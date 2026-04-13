//! Device wrappers for Zephyr I2C controllers and targets.

use core::cell::UnsafeCell;
use core::ffi::c_int;

use super::{NoStatic, Unique};
use crate::{
    error::{to_result_void, Error},
    raw, Result,
};

// Re-export the raw callback types so users can write `extern "C"` handlers without reaching into
// `zephyr::raw` directly.
pub use raw::{i2c_target_callbacks, i2c_target_config};

/// A single operation in a multi-stage I2C transaction.
///
/// Used with [`I2c::transfer`] to compose general scatter/gather transfers.  A RESTART is
/// inserted automatically between adjacent operations of differing direction; the underlying
/// `i2c_transfer` always emits a STOP after the final operation.
pub enum Operation<'a> {
    /// Write the bytes from the buffer to the bus.
    Write(&'a [u8]),
    /// Read bytes from the bus into the buffer.
    Read(&'a mut [u8]),
}

/// A Zephyr I2C controller device.
///
/// This wrapper maps to Zephyr's blocking I2C controller API (`i2c_write`, `i2c_read`,
/// `i2c_write_read`).  All operations block the calling thread until the transfer completes.
#[allow(dead_code)]
pub struct I2c {
    pub(crate) device: *const raw::device,
}

// SAFETY: `I2c` holds a raw pointer to a static Zephyr device structure with no thread-affine
// Rust state.  The Zephyr I2C driver serialises bus access internally via a semaphore.
unsafe impl Send for I2c {}

impl I2c {
    /// Constructor, intended to be called by devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(
        unique: &Unique,
        _static: &NoStatic,
        device: *const raw::device,
    ) -> Option<I2c> {
        if !unique.once() {
            return None;
        }

        Some(I2c { device })
    }

    /// Verify that the underlying I2C device is ready for use.
    pub fn is_ready(&self) -> bool {
        unsafe { raw::device_is_ready(self.device) }
    }

    /// Write bytes to an I2C device, then read bytes back in a single transaction (RESTART
    /// between the write and read phases).
    pub fn write_read(&mut self, addr: u16, write_buf: &[u8], read_buf: &mut [u8]) -> Result<()> {
        to_result_void(unsafe {
            raw::zr_i2c_write_read(
                self.device,
                addr,
                write_buf.as_ptr().cast(),
                write_buf.len(),
                read_buf.as_mut_ptr().cast(),
                read_buf.len(),
            )
        })
    }

    /// Write bytes to an I2C device.
    pub fn write(&mut self, addr: u16, buf: &[u8]) -> Result<()> {
        to_result_void(unsafe {
            raw::zr_i2c_write(self.device, buf.as_ptr(), buf.len() as u32, addr)
        })
    }

    /// Read bytes from an I2C device.
    pub fn read(&mut self, addr: u16, buf: &mut [u8]) -> Result<()> {
        to_result_void(unsafe {
            raw::zr_i2c_read(self.device, buf.as_mut_ptr(), buf.len() as u32, addr)
        })
    }

    /// Maximum number of operations supported in a single [`transfer`](Self::transfer) call.
    ///
    /// `transfer` builds the underlying `i2c_msg` array on the stack so the bound is fixed at
    /// compile time.  Increase this if longer transactions are needed.
    pub const MAX_TRANSFER_OPS: usize = 8;

    /// Perform a multi-stage I2C transaction.
    ///
    /// Wraps Zephyr's `i2c_transfer`, the general-purpose scatter/gather entry point.  Each
    /// [`Operation`] becomes one `i2c_msg`; a RESTART is inserted between adjacent operations
    /// whose direction differs, and `i2c_transfer` itself appends a STOP after the final
    /// message.
    ///
    /// Up to [`MAX_TRANSFER_OPS`](Self::MAX_TRANSFER_OPS) operations are supported per call;
    /// passing more returns `Err(Error(E2BIG))`.  For simple cases prefer
    /// [`write`](Self::write), [`read`](Self::read), or [`write_read`](Self::write_read).
    pub fn transfer(&mut self, addr: u16, ops: &mut [Operation<'_>]) -> Result<()> {
        if ops.len() > Self::MAX_TRANSFER_OPS {
            return Err(Error(raw::E2BIG));
        }

        let mut msgs: [raw::i2c_msg; Self::MAX_TRANSFER_OPS] =
            core::array::from_fn(|_| Default::default());
        for (i, op) in ops.iter_mut().enumerate() {
            let (buf_ptr, len, mut flags) = match op {
                // i2c_msg.buf is `*mut u8` even for writes; the driver does not mutate write
                // buffers.
                Operation::Write(buf) => {
                    (buf.as_ptr() as *mut u8, buf.len(), raw::ZR_I2C_MSG_WRITE)
                }
                Operation::Read(buf) => (buf.as_mut_ptr(), buf.len(), raw::ZR_I2C_MSG_READ),
            };
            // Insert a RESTART when the direction changes from the previous message, matching
            // what Zephyr's own `i2c_write_read` helper does for the write -> read transition.
            if i > 0 && (msgs[i - 1].flags & raw::ZR_I2C_MSG_READ) != (flags & raw::ZR_I2C_MSG_READ)
            {
                flags |= raw::ZR_I2C_MSG_RESTART;
            }
            msgs[i].buf = buf_ptr;
            msgs[i].len = len as u32;
            msgs[i].flags = flags;
        }

        to_result_void(unsafe {
            raw::i2c_transfer(self.device, msgs.as_mut_ptr(), ops.len() as u8, addr)
        })
    }

    /// Register an I2C target on this bus (low-level).
    ///
    /// Prefer [`I2cTargetData::register`] for a fully safe interface.
    ///
    /// The caller must supply a fully initialised `i2c_target_config` (including the `callbacks`
    /// pointer and target address).  The config and the callbacks it points to must remain valid
    /// for as long as the target is registered — typically they live in a `static`.
    ///
    /// All callback function pointers in the `i2c_target_callbacks` struct are invoked from ISR
    /// context.  The caller is responsible for providing the `extern "C"` callback implementations
    /// and managing any shared state they access.
    ///
    /// Returns an [`I2cTarget`] handle that can be used to unregister.
    ///
    /// # Safety
    ///
    /// The `config` pointer must remain valid and unmodified until [`I2cTarget::unregister`] is
    /// called.  The callback functions must be safe to call from ISR context.
    pub unsafe fn register_target(
        &mut self,
        config: &'static mut i2c_target_config,
    ) -> Result<I2cTarget> {
        let config_ptr = config as *mut _;
        to_result_void(raw::zr_i2c_target_register(self.device, config_ptr))?;
        Ok(I2cTarget {
            device: self.device,
            config: config_ptr,
        })
    }
}

// ---------------------------------------------------------------------------
// Safe I2C target abstraction
// ---------------------------------------------------------------------------

/// Trait for I2C target callbacks.
///
/// Implement this on a type that holds your shared state.  All methods receive
/// `&self` and are called from **ISR context**, so interior mutability must use
/// ISR-safe mechanisms such as [`SpinMutex`](crate::sync::SpinMutex) or
/// atomics.
///
/// The implementing type must be [`Send`] + [`Sync`] because it is shared
/// between ISR callbacks and regular threads.
pub trait I2cTargetCallbacks: Send + Sync + 'static {
    /// Called when the controller initiates a write to this target.
    fn write_requested(&self) -> Result<()> {
        Ok(())
    }

    /// Called for each byte received from the controller.
    fn write_received(&self, val: u8) -> Result<()> {
        let _ = val;
        Ok(())
    }

    /// Called when the controller initiates a read from this target.
    ///
    /// Returns the first byte to send to the controller.
    fn read_requested(&self) -> Result<u8> {
        Ok(0)
    }

    /// Called after the controller reads a byte, requesting the next.
    ///
    /// Returns the next byte to send to the controller.
    fn read_processed(&self) -> Result<u8> {
        Ok(0)
    }

    /// Called when a STOP condition is detected.
    fn stop(&self) -> Result<()> {
        Ok(())
    }
}

/// Static storage for an I2C target device.
///
/// Wraps a user callback implementation `T` together with the Zephyr
/// `i2c_target_config` and `i2c_target_callbacks` needed for registration.
/// Place this in a `static` and call [`register`](Self::register) to activate
/// the target on an I2C bus.
///
/// The [`data`](Self::data) method provides a shared reference to the user
/// state, usable from any thread.
///
/// # Example
///
/// ```ignore
/// use zephyr::device::i2c::{I2cTargetData, I2cTargetCallbacks};
/// use zephyr::sync::SpinMutex;
///
/// struct MyTarget {
///     inner: SpinMutex<MyState>,
/// }
///
/// impl I2cTargetCallbacks for MyTarget {
///     fn write_received(&self, val: u8) -> zephyr::Result<()> {
///         let mut s = self.inner.lock().unwrap();
///         // handle byte ...
///         Ok(())
///     }
///     // ... other callbacks
/// #   fn read_requested(&self) -> zephyr::Result<u8> { Ok(0) }
/// #   fn read_processed(&self) -> zephyr::Result<u8> { Ok(0) }
/// }
///
/// static TARGET: I2cTargetData<MyTarget> = I2cTargetData::new(0x42, MyTarget {
///     inner: SpinMutex::new(MyState::new()),
/// });
///
/// fn main() {
///     let mut i2c = /* get I2C device */;
///     let _handle = TARGET.register(&mut i2c).unwrap();
///     // Access shared state from any thread:
///     let state = TARGET.data();
/// }
/// ```
#[repr(C)]
pub struct I2cTargetData<T: I2cTargetCallbacks> {
    // First field: the C callbacks receive a `*mut i2c_target_config` that
    // points here.  Because `UnsafeCell` is `#[repr(transparent)]` and this is
    // the first field of a `#[repr(C)]` struct, the pointer value equals the
    // `I2cTargetData` address, enabling a zero-offset container_of cast.
    config: UnsafeCell<i2c_target_config>,
    cbs: UnsafeCell<i2c_target_callbacks>,
    data: T,
}

// SAFETY: T: Send + Sync.  The config and cbs cells are only written during
// the single-threaded `register()` call; afterwards they are effectively
// read-only (the driver reads `callbacks`, and the `node` field is managed by
// Zephyr's internal linked list under its own lock).
unsafe impl<T: I2cTargetCallbacks> Send for I2cTargetData<T> {}
unsafe impl<T: I2cTargetCallbacks> Sync for I2cTargetData<T> {}

impl<T: I2cTargetCallbacks> I2cTargetData<T> {
    /// Create a new I2C target data instance.
    ///
    /// `address` is the 7-bit I2C target address.  `data` is the user state
    /// that implements [`I2cTargetCallbacks`].
    ///
    /// Internal pointers (callback table, config→callbacks link) are set up
    /// lazily by [`register`](Self::register).
    pub const fn new(address: u16, data: T) -> Self {
        Self {
            config: UnsafeCell::new(i2c_target_config {
                node: raw::sys_snode_t {
                    next: core::ptr::null_mut(),
                },
                flags: 0,
                address,
                callbacks: core::ptr::null(),
            }),
            // SAFETY: A zeroed `i2c_target_callbacks` is valid — every field
            // is an `Option<unsafe extern "C" fn(…)>`, which is `None` when
            // zero.  Using `zeroed()` avoids enumerating fields that vary with
            // Kconfig (e.g. CONFIG_I2C_TARGET_BUFFER_MODE).
            cbs: UnsafeCell::new(unsafe { core::mem::zeroed() }),
            data,
        }
    }

    /// Shared reference to the user data.
    ///
    /// This is the same `T` that the ISR callbacks see via `&self`, so both
    /// sides share the same synchronisation primitives inside `T`.
    pub fn data(&self) -> &T {
        &self.data
    }

    /// Register this target on the given I2C bus.
    ///
    /// Populates the C callback trampolines, links the internal config to the
    /// callback table, and calls `i2c_target_register`.
    ///
    /// Returns an [`I2cTarget`] handle whose [`unregister`](I2cTarget::unregister)
    /// method reverses the registration.
    ///
    /// The `&'static self` requirement ensures the backing storage outlives the
    /// registration.
    pub fn register(&'static self, i2c: &mut I2c) -> Result<I2cTarget> {
        // SAFETY: We are the sole writer — `register` is called once during
        // single-threaded init, before any callbacks can fire.
        unsafe {
            let cbs = &mut *self.cbs.get();
            cbs.write_requested = Some(Self::write_requested_trampoline);
            cbs.read_requested = Some(Self::read_requested_trampoline);
            cbs.write_received = Some(Self::write_received_trampoline);
            cbs.read_processed = Some(Self::read_processed_trampoline);
            cbs.stop = Some(Self::stop_trampoline);

            let config = &mut *self.config.get();
            config.callbacks = self.cbs.get() as *const _;
        }

        to_result_void(unsafe { raw::zr_i2c_target_register(i2c.device, self.config.get()) })?;

        Ok(I2cTarget {
            device: i2c.device,
            config: self.config.get(),
        })
    }

    // -- internal helpers -----------------------------------------------------

    /// Recover `&Self` from the config pointer passed into C callbacks.
    ///
    /// # Safety
    ///
    /// `ptr` must originate from the `config` field of a live `I2cTargetData<T>`.
    unsafe fn from_config(ptr: *mut i2c_target_config) -> &'static Self {
        unsafe { &*(ptr as *const Self) }
    }

    // -- C-ABI trampoline functions -------------------------------------------

    unsafe extern "C" fn write_requested_trampoline(config: *mut i2c_target_config) -> c_int {
        let this = unsafe { Self::from_config(config) };
        match this.data.write_requested() {
            Ok(()) => 0,
            Err(e) => -(e.0 as c_int),
        }
    }

    unsafe extern "C" fn write_received_trampoline(
        config: *mut i2c_target_config,
        val: u8,
    ) -> c_int {
        let this = unsafe { Self::from_config(config) };
        match this.data.write_received(val) {
            Ok(()) => 0,
            Err(e) => -(e.0 as c_int),
        }
    }

    unsafe extern "C" fn read_requested_trampoline(
        config: *mut i2c_target_config,
        val: *mut u8,
    ) -> c_int {
        let this = unsafe { Self::from_config(config) };
        match this.data.read_requested() {
            Ok(byte) => {
                unsafe { *val = byte };
                0
            }
            Err(e) => -(e.0 as c_int),
        }
    }

    unsafe extern "C" fn read_processed_trampoline(
        config: *mut i2c_target_config,
        val: *mut u8,
    ) -> c_int {
        let this = unsafe { Self::from_config(config) };
        match this.data.read_processed() {
            Ok(byte) => {
                unsafe { *val = byte };
                0
            }
            Err(e) => -(e.0 as c_int),
        }
    }

    unsafe extern "C" fn stop_trampoline(config: *mut i2c_target_config) -> c_int {
        let this = unsafe { Self::from_config(config) };
        match this.data.stop() {
            Ok(()) => 0,
            Err(e) => -(e.0 as c_int),
        }
    }
}

/// A registered I2C target.
///
/// Created by [`I2cTargetData::register`] or [`I2c::register_target`].  Holds
/// the device pointer and config so it can unregister cleanly.  Dropping
/// without calling [`unregister`](I2cTarget::unregister) will **not**
/// automatically unregister — the caller must manage the lifetime explicitly.
pub struct I2cTarget {
    device: *const raw::device,
    config: *mut i2c_target_config,
}

// SAFETY: Same justification as `I2c` — the raw device pointer is to a static Zephyr struct.
unsafe impl Send for I2cTarget {}

impl I2cTarget {
    /// Unregister this I2C target from the bus.
    pub fn unregister(self) -> Result<()> {
        to_result_void(unsafe { raw::zr_i2c_target_unregister(self.device, self.config) })
    }
}
