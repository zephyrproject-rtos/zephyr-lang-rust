//! Device wrappers for Zephyr I2C controllers and targets.

use super::{NoStatic, Unique};
use crate::{error::to_result_void, raw, Result};

// Re-export the raw callback types so users can write `extern "C"` handlers without reaching into
// `zephyr::raw` directly.
pub use raw::{i2c_target_callbacks, i2c_target_config};

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

    /// Register an I2C target on this bus.
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
        to_result_void(raw::zr_i2c_target_register(self.device, config))?;
        Ok(I2cTarget {
            device: self.device,
            config,
        })
    }
}

/// A registered I2C target.
///
/// Created by [`I2c::register_target`].  Holds the device pointer and a reference to the
/// `i2c_target_config` so it can unregister cleanly.  Dropping without calling
/// [`unregister`](I2cTarget::unregister) will **not** automatically unregister — the caller must
/// manage the lifetime explicitly.
pub struct I2cTarget {
    device: *const raw::device,
    config: &'static mut i2c_target_config,
}

// SAFETY: Same justification as `I2c` — the raw device pointer is to a static Zephyr struct.
unsafe impl Send for I2cTarget {}

impl I2cTarget {
    /// Unregister this I2C target from the bus.
    pub fn unregister(self) -> Result<()> {
        to_result_void(unsafe { raw::zr_i2c_target_unregister(self.device, self.config) })
    }
}
