// SPDX-License-Identifier: Apache-2.0

//! Byte-oriented UART polling and blocking [`embedded_io`] access.
//!
//! Trait reads and writes transfer one byte per call; use `read_exact()` and
//! `write_all()` for larger transfers. Empty reads sleep between polls, so these
//! blocking traits must be used from thread context, not an interrupt handler.
//! Prefer a non-blocking API when servicing a high-rate stream or an executor.

use super::{NoStatic, Unique};
use crate::{
    error::Error,
    raw,
    time::{sleep, Duration},
    Result,
};

/// A Zephyr UART device, obtained through devicetree `get_instance()`.
///
/// The handle uses the existing device configuration; acquisition does not
/// change baud rate, framing, flow control, or callbacks. Only 8-bit data is
/// supported. Unique acquisition prevents duplicate Rust handles, but does not
/// reserve the UART against C users. Use a dedicated UART rather than sharing
/// a console, shell, or an active interrupt/async UART receiver.
pub struct Uart {
    device: *const raw::device,
}

impl Uart {
    /// Constructor used by generated devicetree code.
    ///
    /// # Safety
    ///
    /// `device` must point to a static Zephyr UART device with a valid UART API.
    /// `unique` must be the token shared by all paths acquiring that device.
    /// Other users must not access driver state unsafely.
    pub(crate) unsafe fn new(
        unique: &Unique,
        _static: &NoStatic,
        device: *const raw::device,
    ) -> Option<Self> {
        if !unique.once() {
            return None;
        }

        Some(Self { device })
    }

    /// Check whether the UART is initialized and ready for use.
    pub fn is_ready(&self) -> bool {
        // SAFETY: Construction requires a pointer to a static Zephyr device.
        unsafe { raw::device_is_ready(self.device) }
    }

    fn ensure_ready(&self) -> Result<()> {
        if self.is_ready() {
            Ok(())
        } else {
            Err(Error(raw::ENODEV))
        }
    }

    /// Try to read one byte without waiting for input.
    ///
    /// Returns `Ok(None)` when no byte is available, `ENODEV` if the device is
    /// not ready, or the driver's error. Zephyr's `-1` empty-input result is not
    /// an errno. Unexpected positive return values are reported as `EIO`.
    pub fn poll_in(&mut self) -> Result<Option<u8>> {
        self.ensure_ready()?;
        let mut byte = 0;
        // SAFETY: The device is a static, ready UART. The output points to a
        // live initialized byte for the duration of this synchronous call.
        match unsafe { raw::uart_poll_in(self.device, &mut byte) } {
            0 => Ok(Some(byte)),
            -1 => Ok(None),
            status if status < 0 => Err(Error(status.unsigned_abs())),
            _ => Err(Error(raw::EIO)),
        }
    }

    /// Write one byte, blocking as required by the underlying polling driver.
    ///
    /// Returns `ENODEV` if the device is not ready. Zephyr's `uart_poll_out()`
    /// returns no error status, so success cannot report transmission failures
    /// or guarantee that the hardware is idle. Flow control may block this call
    /// indefinitely while CTS is deasserted.
    pub fn poll_out(&mut self, byte: u8) -> Result<()> {
        self.ensure_ready()?;
        // SAFETY: The device is a static, ready UART, and the byte is passed by
        // value. The mutable receiver prevents concurrent shared Rust calls.
        unsafe { raw::uart_poll_out(self.device, byte) };
        Ok(())
    }
}

impl embedded_io::ErrorType for Uart {
    type Error = Error;
}

impl embedded_io::Read for Uart {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            if let Some(byte) = self.poll_in()? {
                buf[0] = byte;
                return Ok(1);
            }
            sleep(Duration::millis_at_least(1));
        }
    }
}

impl embedded_io::Write for Uart {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        match buf.first() {
            Some(&byte) => {
                self.poll_out(byte)?;
                Ok(1)
            }
            None => Ok(0),
        }
    }

    /// Polling provides no portable TX-drain query; return `ENOTSUP` rather
    /// than claiming that all intermediate hardware buffers have drained.
    fn flush(&mut self) -> Result<()> {
        self.ensure_ready()?;
        Err(Error(raw::ENOTSUP))
    }
}
