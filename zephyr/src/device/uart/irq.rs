//! Simple (and unsafe) wrappers around USB devices.

// TODO! Remove this.
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate alloc;

// TODO: This should be a generic Buffer type indicating some type of owned buffer, with Vec as one
// possible implementation.
use alloc::vec::Vec;

use arraydeque::ArrayDeque;

use crate::raw;
use crate::error::{Result, to_result_void};
use crate::sys::sync::Semaphore;
use crate::sync::{Arc, SpinMutex};
use crate::time::{NoWait, Timeout};

use core::ffi::c_void;
use core::ops::Range;
use core::{fmt, result};

use super::Uart;

/// Size of the irq buffer used for UartIrq.
///
/// TODO: Make this a parameter of the type.
const BUFFER_SIZE: usize = 256;

/// The "outer" struct holds the semaphore, and the mutex.  The semaphore has to live outside of the
/// mutex because it can only be waited on when the Mutex is not locked.
struct IrqOuterData<const WS: usize, const RS: usize> {
    read_sem: Semaphore,
    /// Write semaphore.  This should **exactly** match the number of elements in `write_dones`.
    write_sem: Semaphore,
    inner: SpinMutex<IrqInnerData<WS, RS>>,
}

/// Data for communication with the UART IRQ.
struct IrqInnerData<const WS: usize, const RS: usize> {
    /// The Ring buffer holding incoming and read data.
    buffer: ArrayDeque<u8, BUFFER_SIZE>,
    /// Write request.  The 'head' is the one being worked on.  Once completed, they will move into
    /// the completion queue.
    write_requests: ArrayDeque<WriteRequest, WS>,
    /// Completed writes.
    write_dones: ArrayDeque<WriteDone, WS>,
}

/// A single requested write.  This is a managed buffer, and a range of the buffer to actually
/// write.  The write is completed when `pos` == `len`.
struct WriteRequest {
    /// The data to write.
    data: Vec<u8>,
    /// What part to write.
    part: Range<usize>,
}

/// A completed write.  All the requested data will have been written, and this returns the buffer
/// to the user.
struct WriteDone {
    /// The returned buffer.
    data: Vec<u8>,
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

/// The error type from write requests.  Used to return the buffer.
pub struct WriteError(pub Vec<u8>);

// The default Debug for Write error will print the whole buffer, which isn't particularly useful.
impl fmt::Debug for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WriteError(...)")
    }
}

/// The wait for write completion timed out.
pub struct WriteWaitTimedOut;

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
    data: Arc<IrqOuterData<WS, RS>>,
}

// UartIrq is also Send, !Sync, for the same reasons as for Uart.
unsafe impl<const WS: usize, const RS: usize> Send for UartIrq<WS, RS> {}

impl<const WS: usize, const RS: usize> UartIrq<WS, RS> {
    /// Convert uart into irq driven one.
    pub unsafe fn new(uart: Uart) -> Result<Self> {
        let data = Arc::new(IrqOuterData {
            read_sem: Semaphore::new(0, RS as u32)?,
            write_sem: Semaphore::new(0, WS as u32)?,
            inner: SpinMutex::new(IrqInnerData {
                buffer: ArrayDeque::new(),
                write_requests: ArrayDeque::new(),
                write_dones: ArrayDeque::new(),
            }),
        });

        // Clone the arc, and convert to a raw pointer, to give to the callback.
        // This will leak the Arc (which prevents deallocation).
        let data_raw = Arc::into_raw(data.clone());
        let data_raw = data_raw as *mut c_void;

        let ret = unsafe {
            raw::uart_irq_callback_user_data_set(uart.device, Some(irq_callback::<WS, RS>), data_raw)
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

    /// Enqueue a single write request.
    ///
    /// If the queue is full, the `WriteError` returned will return the buffer.
    pub fn write_enqueue(&mut self, data: Vec<u8>, part: Range<usize>) -> result::Result<(), WriteError> {
        let mut inner = self.data.inner.lock().unwrap();

        let req = WriteRequest { data, part };
        match inner.write_requests.push_back(req) {
            Ok(()) => {
                // Make sure the write actually happens.  This needs to happen for the first message
                // queued, if some were already queued, it should already be enabled.
                if inner.write_requests.len() == 1 {
                    unsafe { raw::uart_irq_tx_enable(self.uart.device); }
                }
                Ok(())
            }
            Err(e) => Err(WriteError(e.element.data)),
        }
    }

    /// Return true if the write queue is full.
    ///
    /// There is a race between this and write_enqueue, but only in the safe direction (this may
    /// return false, but a write may complete before being able to call write_enqueue).
    pub fn write_is_full(&self) -> bool {
        let inner = self.data.inner.lock().unwrap();

        inner.write_requests.is_full()
    }

    /// Retrieve a write completion.
    ///
    /// Waits up to `timeout` for a write to complete, and returns the buffer.
    pub fn write_wait<T>(&mut self, timeout: T) -> result::Result<Vec<u8>, WriteWaitTimedOut>
        where T: Into<Timeout>,
    {
        match self.data.write_sem.take(timeout) {
            Ok(()) => (),
            // TODO: Handle other errors?
            Err(_) => return Err(WriteWaitTimedOut),
        }

        let mut inner = self.data.inner.lock().unwrap();
        Ok(inner.write_dones.pop_front().expect("Write done empty, despite semaphore").data)
    }
}

impl<const WS: usize, const RS: usize> IrqOuterData<WS, RS> {
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

extern "C" fn irq_callback<const WS: usize, const RS: usize>(
    dev: *const raw::device,
    user_data: *mut c_void,
) {
    // Convert our user data, back to the CS Mutex.
    let outer = unsafe { &*(user_data as *const IrqOuterData<WS, RS>) };
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

    // Handle any write requests.
    loop {
        if let Some(mut req) = inner.write_requests.pop_front() {
            if req.part.is_empty() {
                // This request is empty.  Move to completion.
                inner.write_dones.push_back(WriteDone { data: req.data })
                    .expect("write done queue is full");
                outer.write_sem.give();
            } else {
                // Try to write this part of the data.
                let piece = &req.data[req.part.clone()];
                let count = unsafe {
                    raw::uart_fifo_fill(dev, piece.as_ptr(), piece.len() as i32)
                };
                if count < 0 {
                    panic!("Incorrect use of device fifo");
                }
                let count = count as usize;

                // Adjust the part.  The next through the loop will notice the write being done.
                req.part.start += count;
                inner.write_requests.push_front(req)
                    .expect("Unexpected write_dones overflow");
            }
        } else {
            // No work.  Turn off the irq, and stop.
            unsafe { raw::uart_irq_tx_disable(dev); }
            break;
        }
    }

    unsafe {
        raw::uart_irq_update(dev);
    }
}
