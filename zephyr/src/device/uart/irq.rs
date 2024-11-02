//! Simple (and unsafe) wrappers around USB devices.

// TODO! Remove this.
#![allow(dead_code)]
#![allow(unused_variables)]

use arraydeque::ArrayDeque;

use crate::raw;
use crate::error::{Result, to_result_void};
use crate::sys::sync::Semaphore;
use crate::sync::{Arc, SpinMutex};
use crate::time::{NoWait, Timeout};

use core::ffi::c_void;

use super::Uart;

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

/// An interface to the UART, that uses the "legacy" IRQ API.
///
/// The interface is parameterized by two value, `WS` is the number of elements in the write ring,
/// and `RS` is the number of elements in the read ring.  Each direction will have two rings, one
/// for pending operations, and the other for completed operations.  Setting these to 2 is
/// sufficient to avoid stalling writes or dropping reads, but requires the application attend to
/// the buffers.
pub struct UartIrq<const WS: usize, const RS: usize> {
    /// Interior wrapped device, to be able to hand out lifetime managed references to it.
    uart: Uart,
    /// Critical section protected data.
    data: Arc<IrqOuterData>,
}

// UartIrq is also Send, !Sync, for the same reasons as for Uart.
unsafe impl<const WS: usize, const RS: usize> Send for UartIrq<WS, RS> {}

impl<const WS: usize, const RS: usize> UartIrq<WS, RS> {
    /// Convert uart into irq driven one.
    pub unsafe fn new(uart: Uart) -> Result<Self> {
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
            data,
            uart,
        })
    }

    /// Get the underlying UART to be able to change line control and such.
    ///
    /// TODO: This really should return something like `&Uart` to bind the lifetime.  Otherwise the
    /// user can continue to use the uart handle beyond the lifetime of the driver.
    pub unsafe fn inner(&mut self) -> &Uart {
        &self.uart
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
    pub unsafe fn write<T>(&mut self, buf: &[u8], timeout: T) -> usize
        where T: Into<Timeout>
    {
        if buf.len() == 0 {
            return 0;
        }

        // Make the data to be written available to the irq handler, and get it going.
        {
            let mut inner = self.data.inner.lock().unwrap();
            assert!(inner.write.is_none());

            inner.write = Some(WriteSlice {
                data: buf.as_ptr(),
                len: buf.len(),
            });

            unsafe { raw::uart_irq_tx_enable(self.uart.device) };
        }

        // Wait for the transmission to complete.  This shouldn't be racy, as the irq shouldn't be
        // giving the semaphore until there is 'write' data, and it has been consumed.
        let _ = self.data.write_sem.take(timeout);

        // Depending on the driver, there might be a race here.  This would result in the above
        // 'take' returning early, and no actual data being written.

        {
            let mut inner = self.data.inner.lock().unwrap();

            if let Some(write) = inner.write.take() {
                // First, make sure that no more interrupts will come in.
                unsafe { raw::uart_irq_tx_disable(self.uart.device) };

                // The write did not complete, and this represents remaining data to write.
                buf.len() - write.len
            } else {
                // The write completed, the rx irq should be disabled.  Just return the whole
                // buffer.
                buf.len()
            }
        }
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
