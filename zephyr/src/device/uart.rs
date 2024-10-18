//! Simple (and unsafe) wrappers around USB devices.

use crate::raw;
use crate::error::{Error, Result, to_result_void, to_result};

use core::ffi::{c_uchar, c_int};

use super::Unique;

/// A wrapper around a UART device on Zephyr.
pub struct Uart {
    /// The underlying device itself.
    #[allow(dead_code)]
    pub(crate) device: *const raw::device,
}

/// Uart control values.
///
/// This mirrors these definitions from C, but as an enum.
#[repr(u32)]
pub enum LineControl {
    /// Baud rate
    BaudRate = raw::uart_line_ctrl_UART_LINE_CTRL_BAUD_RATE,
    /// Request To Send (RTS)
    RTS = raw::uart_line_ctrl_UART_LINE_CTRL_RTS,
    /// Data Terminal Ready (DTR)
    DTR = raw::uart_line_ctrl_UART_LINE_CTRL_DTR,
    /// Data Carrier Detect (DCD)
    DCD = raw::uart_line_ctrl_UART_LINE_CTRL_DCD,
    /// Data Set Ready (DSR)
    DSR = raw::uart_line_ctrl_UART_LINE_CTRL_DSR,
}

impl Uart {
    // Note that the `poll_in` and `poll_out` are terrible.

    /// Constructor, used by the devicetree generated code.
    #[allow(dead_code)]
    pub(crate) unsafe fn new(unique: &Unique, device: *const raw::device) -> Option<Uart> {
        if !unique.once() {
            return None;
        }

        Some(Uart { device })
    }

    /// Attempt to read a character from the UART fifo.
    ///
    /// Will return Ok(Some(ch)) if there is a character available, `Ok(None)` if no character
    /// is available, or `Err(e)` if there was an error.
    pub unsafe fn poll_in(&mut self) -> Result<Option<u8>> {
        let mut ch: c_uchar = 0;

        match to_result_void(unsafe { raw::uart_poll_in(self.device, &mut ch) }) {
            Ok(()) => Ok(Some(ch as u8)),
            Err(Error(1)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Attempt to write to the outgoing FIFO.
    ///
    /// This writes to the outgoing UART fifo.  This will block if the outgoing fifo is full.
    pub unsafe fn poll_out(&mut self, out_char: u8) {
        unsafe { raw::uart_poll_out(self.device, out_char as c_uchar) }
    }

    /// Fill FIFO with data.
    ///
    /// This is unspecified what happens if this is not called from IRQ context.
    /// Returns Ok(n) for the number of bytes sent.
    pub unsafe fn fifo_fill(&mut self, data: &[u8]) -> Result<usize> {
        to_result(unsafe {
            raw::uart_fifo_fill(self.device, data.as_ptr(), data.len() as c_int)
        }).map(|count| count as usize)
    }

    /// Drain FIFO.
    ///
    /// This is unspecified as to what happens if not called from IRQ context.
    pub unsafe fn fifo_read(&mut self, data: &mut [u8]) -> Result<usize> {
        to_result(unsafe {
            raw::uart_fifo_read(self.device, data.as_mut_ptr(), data.len() as c_int)
        }).map(|count| count as usize)
    }

    /// Read one of the UART line control values.
    pub unsafe fn line_ctrl_get(&self, item: LineControl) -> Result<u32> {
        let mut result: u32 = 0;
        to_result_void(unsafe {
            raw::uart_line_ctrl_get(self.device, item as u32, &mut result)
        }).map(|()| result)
    }
}
