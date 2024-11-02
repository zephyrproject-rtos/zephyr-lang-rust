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
use crate::time::Timeout;

use core::ffi::c_void;
use core::ops::Range;
use core::{fmt, result};

use super::Uart;

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
    /// Write request.  The 'head' is the one being worked on.  Once completed, they will move into
    /// the completion queue.
    write_requests: ArrayDeque<WriteRequest, WS>,
    /// Completed writes.
    write_dones: ArrayDeque<WriteDone, WS>,
    /// Read requests.  The 'head' is the one data will come into.
    read_requests: ArrayDeque<ReadRequest, RS>,
    /// Completed writes.  Generally, these will be full, but a read might move an early one here.
    read_dones: ArrayDeque<ReadDone, RS>,
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

/// A single read request.  This is a buffer to hold data being read, along with the part still
/// valid to hold data.
struct ReadRequest {
    /// The data to read.
    data: Vec<u8>,
    /// How much of the data has been read so far.
    len: usize,
}

impl ReadRequest {
    fn into_done(self) -> ReadDone {
        ReadDone { data: self.data, len: self.len }
    }
}

/// A completed read.
struct ReadDone {
    /// The buffer holding the data.
    data: Vec<u8>,
    /// How much of `data` contains read data.  Should always be > 0.
    len: usize,
}

impl ReadDone {
    fn into_result(self) -> ReadResult {
        ReadResult { data: self.data, len: self.len }
    }
}

/// The result of a read.
pub struct ReadResult {
    data: Vec<u8>,
    len: usize,
}

/// The error type from write requests.  Used to return the buffer.
pub struct WriteError(pub Vec<u8>);

// The default Debug for Write error will print the whole buffer, which isn't particularly useful.
impl fmt::Debug for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WriteError(...)")
    }
}

/// The error type from read requests.  Used to return the buffer.
pub struct ReadError(pub Vec<u8>);

// The default Debug for Write error will print the whole buffer, which isn't particularly useful.
impl fmt::Debug for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReadError(...)")
    }
}

/// The wait for write completion timed out.
pub struct WriteWaitTimedOut;

/// The wait for read completion timed out.
pub struct ReadWaitTimedOut;

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
                write_requests: ArrayDeque::new(),
                write_dones: ArrayDeque::new(),
                read_requests: ArrayDeque::new(),
                read_dones: ArrayDeque::new(),
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

    /// Enqueue a buffer for reading data.
    ///
    /// Enqueues a buffer to hold read data.  Can enqueue up to RS of these.
    pub fn read_enqueue(&mut self, data: Vec<u8>) -> result::Result<(), ReadError> {
        let mut inner = self.data.inner.lock().unwrap();

        let req = ReadRequest { data, len: 0 };
        match inner.read_requests.push_back(req) {
            Ok(()) => {
                // Enable the rx fifo so incoming data will be placed.
                if inner.read_requests.len() == 1 {
                    unsafe { raw::uart_irq_rx_enable(self.uart.device); }
                }
                Ok(())
            }
            Err(e) => Err(ReadError(e.element.data))
        }
    }

    /// Wait up to 'timeout' for a read to complete, and returns the data.
    ///
    /// Note that if there is a buffer that has been partially filled, this will return that buffer,
    /// so that there isn't a delay with read data.
    pub fn read_wait<T>(&mut self, timeout: T) -> result::Result<ReadResult, ReadWaitTimedOut>
        where T: Into<Timeout>,
    {
        // If there is no read data available, see if we have a partial block we can consider a
        // completion.
        let mut inner = self.data.inner.lock().unwrap();
        if inner.read_dones.is_empty() {
            if let Some(req) = inner.read_requests.pop_front() {
                // TODO: User defined threshold?
                if req.len > 0 {
                    // Queue this up as a completion.
                    inner.read_dones.push_back(req.into_done()).unwrap();

                    // Signal the sem, as we've pushed.
                    self.data.read_sem.give();
                } else {
                    // Stick it back on the queue.
                    inner.read_requests.push_front(req).unwrap();
                }
            }
        }
        drop(inner);

        match self.data.read_sem.take(timeout) {
            Ok(()) => (),
            // TODO: Handle other errors?
            Err(_) => return Err(ReadWaitTimedOut),
        }

        let mut inner = self.data.inner.lock().unwrap();
        let done = inner.read_dones.pop_front().expect("Semaphore mismatched with read done queue");
        Ok(done.into_result())
    }
}

// TODO: It could actually be possible to implement drop, but we would need to make sure the irq
// handlers are deregistered.  These is also the issue of the buffers being dropped.  For now, just
// panic, as this isn't normal.
impl<const WS: usize, const RS: usize> Drop for UartIrq<WS, RS> {
    fn drop(&mut self) {
        panic!("UartIrq dropped");
    }
}

extern "C" fn irq_callback<const WS: usize, const RS: usize>(
    dev: *const raw::device,
    user_data: *mut c_void,
) {
    // Convert our user data, back to the CS Mutex.
    let outer = unsafe { &*(user_data as *const IrqOuterData<WS, RS>) };
    let mut inner = outer.inner.lock().unwrap();

    // Handle any read requests.
    loop {
        if let Some(mut req) = inner.read_requests.pop_front() {
            if req.len == req.data.len() {
                // This buffer is full, make it a completion.
                inner.read_dones.push_back(req.into_done())
                    .expect("Completion queue not large enough");
                outer.read_sem.give();
            } else {
                // Read as much as we can.
                let piece = &mut req.data[req.len..];
                let count = unsafe {
                    raw::uart_fifo_read(dev, piece.as_mut_ptr(), piece.len() as i32)
                };
                if count < 0 {
                    panic!("Incorrect use of read");
                }
                let count = count as usize;

                // Adjust the piece.  The next time through the loop will notice if the write is
                // full.
                req.len += count;
                inner.read_requests.push_front(req)
                    .expect("Unexpected read request overflow");

                if count == 0 {
                    // There is no more data in the fifo.
                    break;
                }
            }
        } else {
            // No place to store results.  Turn off the irq and stop.
            // The doc's don't describe this as being possible, but hopefully the implementations
            // are sane.
            unsafe { raw::uart_irq_rx_disable(dev); }
            break;
        }
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

                // If the count reaches 0, the fifo is full.
                if count == 0 {
                    break;
                }
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
