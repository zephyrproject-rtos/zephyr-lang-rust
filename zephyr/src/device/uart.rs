//! Simple (and unsafe) wrappers around USB devices.

// TODO! Remove this.
#![allow(dead_code)]
#![allow(unused_variables)]

use arraydeque::ArrayDeque;

use crate::raw;
use crate::error::{Error, Result, to_result_void, to_result};
use crate::printkln;
use crate::sys::sync::Semaphore;
use crate::sync::{Arc, SpinMutex};
use crate::time::{Forever, NoWait, Timeout};

use core::ffi::{c_int, c_uchar, c_void};
use core::ptr;

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

    /// Set one of the UART line control values.
    pub unsafe fn line_ctrl_set(&mut self, item: LineControl, value: u32) -> Result<()> {
        to_result_void(unsafe {
            raw::uart_line_ctrl_set(self.device, item as u32, value)
        })
    }

    /// Convenience, return if DTR is asserted.
    pub unsafe fn is_dtr_set(&self) -> Result<bool> {
        let ret = unsafe {
            self.line_ctrl_get(LineControl::DTR)?
        };
        Ok(ret == 1)
    }

    /// Convert this UART into an async one.
    pub unsafe fn into_async(self) -> Result<UartAsync> {
        UartAsync::new(self)
    }

    /// Convert into an IRQ one.
    pub unsafe fn into_irq(self) -> Result<UartIrq> {
        UartIrq::new(self)
    }
} 

/// The uart is safe to Send, as long as it is only used from one thread at a time.  As such, it is
/// not Sync.
unsafe impl Send for Uart {}

/// This is the async interface to the uart.
///
/// Until we can analyze this for safety, it will just be declared as unsafe.
///
/// It is unclear from the docs what context this callback api is called from, so we will assume
/// that it might be called from an irq.  As such, we'll need to use a critical-section and it's
/// mutex to protect the data.
pub struct UartAsync();

impl UartAsync {
    /// Take a Uart device and turn it into an async interface.
    ///
    /// TODO: Return the uart back if this fails.
    pub unsafe fn new(uart: Uart) -> Result<UartAsync> {
        let ret = unsafe {
            raw::uart_callback_set(uart.device, Some(async_callback), ptr::null_mut())
        };
        to_result_void(ret)?;
        Ok(UartAsync())
    }
}

extern "C" fn async_callback(
    _dev: *const raw::device,
    _evt: *mut raw::uart_event,
    _user_data: *mut c_void,
) {
    printkln!("Async");
}

/// Size of the irq buffer used for UartIrq.
///
/// TODO: Make this a parameter of the type.
const BUFFER_SIZE: usize = 256;

/// The "outer" struct holds the semaphore, and the mutex.  The semaphore has to live outside of the
/// mutex because it can only be waited on when the Mutex is not locked.
struct IrqOuterData {
    read_sem: Semaphore,
    write_sem: Semaphore,
    inner: SpinMutex<IrqInnerData>,
}

/// Data for communication with the UART IRQ.
struct IrqInnerData {
    /// The Ring buffer holding incoming and read data.
    buffer: ArrayDeque<u8, BUFFER_SIZE>,
    /// Data to be written, if that is the case.
    ///
    /// If this is Some, then the irq should be enabled.
    write: Option<WriteSlice>,
}

/// Represents a slice of data that the irq is going to write.
struct WriteSlice {
    data: *const u8,
    len: usize,
}

impl WriteSlice {
    /// Add an offset to the beginning of this slice, returning a new slice.  This is equivalent to
    /// &item[count..] with a slice.
    pub unsafe fn add(&self, count: usize) -> WriteSlice {
        WriteSlice {
            data: unsafe { self.data.add(count) },
            len: self.len - count,
        }
    }
}

/// This is the irq-driven interface.
pub struct UartIrq {
    /// The raw device.
    device: *const raw::device,
    /// Critical section protected data.
    data: Arc<IrqOuterData>,
}

// UartIrq is also Send, !Sync, for the same reasons as for Uart.
unsafe impl Send for UartIrq {}

impl UartIrq {
    /// Convert uart into irq driven one.
    pub unsafe fn new(uart: Uart) -> Result<UartIrq> {
        let data = Arc::new(IrqOuterData {
            read_sem: Semaphore::new(0, 1)?,
            write_sem: Semaphore::new(0, 1)?,
            inner: SpinMutex::new(IrqInnerData {
                buffer: ArrayDeque::new(),
                write: None,
            }),
        });

        // Clone the arc, and convert to a raw pointer, to give to the callback.
        // This will leak the Arc (which prevents deallocation).
        let data_raw = Arc::into_raw(data.clone());
        let data_raw = data_raw as *mut c_void;

        let ret = unsafe {
            raw::uart_irq_callback_user_data_set(uart.device, Some(irq_callback), data_raw)
        };
        to_result_void(ret)?;
        // Should this be settable?
        unsafe {
            // raw::uart_irq_tx_enable(uart.device);
            raw::uart_irq_rx_enable(uart.device);
        }
        Ok(UartIrq {
            device: uart.device,
            data,
        })
    }

    /// Get the underlying UART to be able to change line control and such.
    pub unsafe fn inner(&mut self) -> Uart {
        Uart {
            device: self.device
        }
    }

    /// Attempt to read data from the UART into the buffer.  If no data is available, it will
    /// attempt, once, to wait using the given timeout.
    ///
    /// Returns the number of bytes that were read, with zero indicating that a timeout occurred.
    pub unsafe fn try_read<T>(&mut self, buf: &mut [u8], timeout: T) -> usize
        where T: Into<Timeout>,
    {
        // Start with a read, before any blocking.
        let count = self.data.try_read(buf);
        if count > 0 {
            return count;
        }

        // Otherwise, wait for the semaphore.  Ignore the result, as we will try to read again, in
        // case there was a race.
        let _ = self.data.read_sem.take(timeout);

        self.data.try_read(buf)
    }

    /// A blocking write to the UART.
    ///
    /// By making this blocking, we don't need to make an extra copy of the data.
    ///
    /// TODO: Async write.
    pub unsafe fn write(&mut self, buf: &[u8]) {
        if buf.len() == 0 {
            return;
        }

        // Make the data to be written available to the irq handler, and get it going.
        {
            let mut inner = self.data.inner.lock().unwrap();
            assert!(inner.write.is_none());

            inner.write = Some(WriteSlice {
                data: buf.as_ptr(),
                len: buf.len(),
            });

            unsafe { raw::uart_irq_tx_enable(self.device) };
        }

        // Wait for the transmission to complete.  This shouldn't be racy, as the irq shouldn't be
        // giving the semaphore until there is 'write' data, and it has been consumed.
        let _ = self.data.write_sem.take(Forever);

        // TODO: Should we check that the write actually finished?
    }
}

impl IrqOuterData {
    /// Try reading from the inner data, filling the buffer with as much data as makes sense.
    /// Returns the number of bytes actually read, or Zero if none.
    fn try_read(&self, buf: &mut [u8]) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let mut pos = 0;
        while pos < buf.len() {
            if let Some(elt) = inner.buffer.pop_front() {
                buf[pos] = elt;
                pos += 1;
            } else {
                break;
            }
        }

        if pos > 0 {
            // Any time we do a read, clear the semaphore.
            let _ = self.read_sem.take(NoWait);
        }
        pos
    }
}

extern "C" fn irq_callback(
    dev: *const raw::device,
    user_data: *mut c_void,
) {
    // Convert our user data, back to the CS Mutex.
    let outer = unsafe { &*(user_data as *const IrqOuterData) };
    let mut inner = outer.inner.lock().unwrap();

    // TODO: Make this more efficient.
    let mut byte = 0u8;
    let mut did_read = false;
    loop {
        match unsafe { raw::uart_fifo_read(dev, &mut byte, 1) } {
            0 => break,
            1 => {
                // TODO: should we warn about overflow here?
                let _ = inner.buffer.push_back(byte);
                did_read = true;
            }
            e => panic!("Uart fifo read not implemented: {}", e),
        }
    }

    // This is safe (and important) to do while the mutex is held.
    if did_read {
        outer.read_sem.give();
    }

    // If there is data to write, ensure the fifo is full, and when we run out of data, disable the
    // interrupt and signal the waiting thread.
    if let Some(write) = inner.write.take() {
        let count = unsafe {
            raw::uart_fifo_fill(dev, write.data, write.len as i32)
        };
        if count < 0 {
            panic!("Incorrect use of device fifo");
        }
        let count = count as usize;

        if count == write.len {
            // The write finished, leave 'write' empty, and let the thread know we're done.
            outer.write_sem.give();

            // Disable the tx fifo, as we don't need it any more.
            unsafe { raw::uart_irq_tx_disable(dev) };
        } else {
            // We're not finished, so remember how much is left.
            inner.write = Some(unsafe { write.add(count) });
        }
    }

    unsafe {
        raw::uart_irq_update(dev);
    }
}
