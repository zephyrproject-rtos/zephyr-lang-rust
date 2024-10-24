//! Close-to-Zephyr channels
//!
//! This module attempts to provide a mechanism as close as possible to `crossbeam-channel` as we
//! can get, directly using Zephyr primitives.
//!
//! The channels are built around `k_queue` in Zephyr.  As is the case with most Zephyr types,
//! these are typically statically allocated.  Similar to the other close-to-zephyr primitives,
//! this means that there is a constructor that can directly take one of these primitives.
//!
//! In other words, `zephyr::sys::Queue` is a Rust friendly implementation of `k_queue` in Zephyr.
//! This module provides `Sender` and `Receiver`, which can be cloned and behave as if they had an
//! internal `Arc` inside them, but without the overhead of an actual Arc.

extern crate alloc;

use alloc::boxed::Box;

use core::ffi::c_void;
use core::fmt;
use core::marker::PhantomData;

use crate::sys::queue::Queue;

mod counter;

// The zephyr queue does not allocate or manage the data of the messages, so we need to handle
// allocation as such as well.  However, we don't need to manage anything, so it is sufficient to
// simply Box the message, leak it out of the box, and give it to Zephyr, and then on receipt, wrap
// it back into a Box, and give it to the recipient.

/// Create a multi-producer multi-consumer channel of unbounded capacity, using an existing Queue
/// object.
///
/// The messages are allocated individually as "Box", and the queue is managed by the underlying
/// Zephyr queue.
pub fn unbounded_from<T>(queue: Queue) -> (Sender<T>, Receiver<T>) {
    let (s, r) = counter::new(queue);
    let s = Sender {
        queue: s,
        _phantom: PhantomData,
    };
    let r = Receiver {
        queue: r,
        _phantom: PhantomData,
    };
    (s, r)
}

/// Create a multi-producer multi-consumer channel of unbounded capacity.
///
/// The messages are allocated individually as "Box".  The underlying Zephyr queue will be
/// dynamically allocated.
///
/// **Note**: Currently Drop is not propertly supported on Zephyr.  If all senders are dropped, any
/// receivers will likely be blocked forever.  Any data that has been queued and not received will
/// be leaked when all receivers have been droped.
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    unbounded_from(Queue::new().unwrap())
}

/// The underlying type for Messages through Zephyr's [`Queue`].
///
/// This wrapper is used internally to wrap user messages through the queue.  It is not useful in
/// safe code, but may be useful for implementing other types of message queues.
#[repr(C)]
pub struct Message<T> {
    /// The private data used by the kernel to enqueue messages and such.
    _private: usize,
    /// The actual data being transported.
    data: T,
}

impl<T> Message<T> {
    /// Construct a new message from the data.
    ///
    /// This is safe in itself, but sending them is unsafe.
    pub fn new(data: T) -> Message<T> {
        Message {
            _private: 0,
            data,
        }
    }
}

/// The sending side of a channel.
pub struct Sender<T> {
    queue: counter::Sender<Queue>,
    _phantom: PhantomData<T>,
}

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    /// Sends a message over the given channel. This will perform an alloc of the message, which
    /// will have an accompanied free on the recipient side.
    pub fn send(&self, msg: T) -> Result<(), SendError<T>> {
        let msg = Box::new(Message::new(msg));
        let msg = Box::into_raw(msg);
        unsafe {
            self.queue.send(msg as *mut c_void);
        }
        Ok(())
    }

    /// Sends a message that has already been boxed.  The box will be dropped upon receipt.  This is
    /// safe to call from interrupt context, and presumably the box will be allocate from a thread.
    pub unsafe fn send_boxed(&self, msg: Box<Message<T>>) -> Result<(), SendError<T>> {
        let msg = Box::into_raw(msg);
        unsafe {
            self.queue.send(msg as *mut c_void);
        }
        Ok(())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        unsafe {
            self.queue.release(|_| {
                crate::printkln!("Release");
                true
            })
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender {
            queue: self.queue.acquire(),
            _phantom: PhantomData,
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender {:?}", *self.queue)
    }
}

/// The receiving side of a channel.
pub struct Receiver<T> {
    queue: counter::Receiver<Queue>,
    _phantom: PhantomData<T>,
}

unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T> Receiver<T> {
    /// Blocks the current thread until a message is received or the channel is empty and
    /// disconnected.
    ///
    /// If the channel is empty and not disconnected, this call will block until the receive
    /// operation can proceed.  If the channel is empty and becomes disconnected, this call will
    /// wake up and return an error.
    pub fn recv(&self) -> Result<T, RecvError> {
        let msg = unsafe {
            self.queue.recv()
        };
        let msg = msg as *mut Message<T>;
        let msg = unsafe { Box::from_raw(msg) };
        Ok(msg.data)
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        unsafe {
            self.queue.release(|_| {
                crate::printkln!("Release");
                true
            })
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        Receiver {
            queue: self.queue.acquire(),
            _phantom: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender {:?}", *self.queue)
    }
}

// TODO: Move to err

/// An error returned from the [`send`] method.
///
/// The message could not be sent because the channel is disconnected.
///
/// The error contains the message so it can be recovered.
///
/// [`send`]: Sender::send
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "SendError(..)".fmt(f)
    }
}

/// An error returned from the [`recv`] method.
///
/// A message could not be received because the channel is empty and disconnected.
///
/// [`recv`]: Receiver::recv
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct RecvError;
