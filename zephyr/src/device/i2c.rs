//! Device wrappers for Zephyr I2C controllers.

use super::{NoStatic, Unique};
use crate::{error::to_result_void, raw, Result};

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
}
