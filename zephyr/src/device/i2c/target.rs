//! Safe wrappers for acting as an I2C target device.

use core::cell::UnsafeCell;
use core::ffi::c_int;

use super::controller::I2c;
use super::{i2c_target_callbacks, i2c_target_config};
use crate::{error::to_result_void, raw, Result};

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

        to_result_void(unsafe { raw::i2c_target_register(i2c.device, self.config.get()) })?;

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
    pub(super) device: *const raw::device,
    pub(super) config: *mut i2c_target_config,
}

// SAFETY: Same justification as `I2c` — the raw device pointer is to a static Zephyr struct.
unsafe impl Send for I2cTarget {}

impl I2cTarget {
    /// Unregister this I2C target from the bus.
    pub fn unregister(self) -> Result<()> {
        to_result_void(unsafe { raw::i2c_target_unregister(self.device, self.config) })
    }
}
