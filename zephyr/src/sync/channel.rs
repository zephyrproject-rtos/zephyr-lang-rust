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

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;

use crate::sys::queue::Queue;
use crate::time::{Forever, NoWait, Timeout};

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
        flavor: SenderFlavor::Unbounded {
            queue: s,
            _phantom: PhantomData,
        }
    };
    let r = Receiver {
        flavor: ReceiverFlavor::Unbounded {
            queue: r,
            _phantom: PhantomData,
        }
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

/// Create a multi-producer multi-consumer channel with bounded capacity.
///
/// The messages are allocated at channel creation time.  If there are no messages at `send` time,
/// send will block (possibly waiting for a timeout).
///
/// At this time, Zephyr does not support crossbeam's 0 capacity queues, which are also called
/// a rendezvous, where both threads wait until in the same region.  `bounded` will panic if called
/// with a capacity of zero.
pub fn bounded<T>(cap: usize) -> (Sender<T>, Receiver<T>) {
    if cap == 0 {
        panic!("Zero capacity queues no supported on Zephyr");
    }

    let (s, r) = counter::new(Bounded::new(cap));
    let s = Sender {
        flavor: SenderFlavor::Bounded(s),
    };
    let r = Receiver {
        flavor: ReceiverFlavor::Bounded(r),
    };
    (s, r)
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
    fn new(data: T) -> Message<T> {
        Message {
            _private: 0,
            data,
        }
    }
}

/// The sending side of a channel.
pub struct Sender<T> {
    flavor: SenderFlavor<T>,
}

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T> Sender<T> {
    /// Waits for a message to be sent into the channel, but only for a limited time.
    ///
    /// This call will block until the send operation can proceed or the operation times out.
    ///
    /// For unbounded channels, this will perform an allocation (and always send immediately).  For
    /// bounded channels, no allocation will be performed.
    pub fn send_timeout<D>(&self, msg: T, timeout: D) -> Result<(), SendError<T>>
        where D: Into<Timeout>,
    {
        match &self.flavor {
            SenderFlavor::Unbounded { queue, .. } => {
                let msg = Box::new(Message::new(msg));
                let msg = Box::into_raw(msg);
                unsafe {
                    queue.send(msg as *mut c_void);
                }
            }
            SenderFlavor::Bounded(chan) => {
                // Retrieve a message buffer from the free list.
                let buf = unsafe { chan.free.recv(timeout) };
                if buf.is_null() {
                    return Err(SendError(msg));
                }
                let buf = buf as *mut Message<T>;
                unsafe {
                    buf.write(Message::new(msg));
                    chan.chan.send(buf as *mut c_void);
                }
            }
        }
        Ok(())
    }

    /// Sends a message over the given channel.  Waiting if necessary.
    ///
    /// For unbounded channels, this will allocate space for a message, and immediately send it.
    /// For bounded channels, this will block until a message slot is available, and then send the
    /// message.
    pub fn send(&self, msg: T) -> Result<(), SendError<T>> {
        self.send_timeout(msg, Forever)
    }

    /// Attempts to send a message into the channel without blocking.
    ///
    /// This message will either send a message into the channel immediately or return an error if
    /// the channel is full.  The returned error contains the original message.
    pub fn try_send(&self, msg: T) -> Result<(), SendError<T>> {
        self.send_timeout(msg, NoWait)
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        match &self.flavor {
            SenderFlavor::Unbounded { queue, .. } => {
                unsafe {
                    queue.release(|_| {
                        crate::printkln!("Release");
                        true
                    })
                }
            }
            SenderFlavor::Bounded(chan) => {
                unsafe {
                    chan.release(|_| {
                        panic!("Bounded queues cannot be dropped");
                    })
                }
            }
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        let flavor = match &self.flavor {
            SenderFlavor::Unbounded { queue, .. } => {
                SenderFlavor::Unbounded {
                    queue: queue.acquire(),
                    _phantom: PhantomData,
                }
            }
            SenderFlavor::Bounded(chan) => {
                SenderFlavor::Bounded(chan.acquire())
            }
        };

        Sender { flavor }
    }
}

/// The "flavor" of a sender.  This maps to the type of channel.
enum SenderFlavor<T> {
    /// An unbounded queue.  Messages are allocated with Box, and sent directly.
    Unbounded {
        queue: counter::Sender<Queue>,
        _phantom: PhantomData<T>,
    },
    Bounded(counter::Sender<Bounded<T>>),
}

impl<T: fmt::Debug> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender")
    }
}

/// The receiving side of a channel.
pub struct Receiver<T> {
    flavor: ReceiverFlavor<T>,
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
        match &self.flavor {
            ReceiverFlavor::Unbounded { queue, .. } => {
                let msg = unsafe {
                    queue.recv(Forever)
                };
                let msg = msg as *mut Message<T>;
                let msg = unsafe { Box::from_raw(msg) };
                Ok(msg.data)
            }
            ReceiverFlavor::Bounded(chan) => {
                let rawbuf = unsafe {
                    chan.chan.recv(Forever)
                };
                let buf = rawbuf as *mut Message<T>;
                let msg: Message<T> = unsafe { buf.read() };
                unsafe {
                    chan.free.send(buf as *mut c_void);
                }
                Ok(msg.data)
            }
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        match &self.flavor {
            ReceiverFlavor::Unbounded { queue, .. } => {
                unsafe {
                    queue.release(|_| {
                        crate::printkln!("Release");
                        true
                    })
                }
            }
            ReceiverFlavor::Bounded(chan) => {
                unsafe {
                    chan.release(|_| {
                        panic!("Bounded channels cannot be dropped");
                    })
                }
            }
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        let flavor = match &self.flavor {
            ReceiverFlavor::Unbounded { queue, .. } => {
                ReceiverFlavor::Unbounded {
                    queue: queue.acquire(),
                    _phantom: PhantomData,
                }
            }
            ReceiverFlavor::Bounded(chan) => {
                ReceiverFlavor::Bounded(chan.acquire())
            }
        };

        Receiver { flavor }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender")
    }
}

/// The "flavor" of a receiver.  This maps to the type of the channel.
enum ReceiverFlavor<T> {
    /// An unbounded queue.  Messages were allocated with Box, and will be freed upon receipt.
    Unbounded {
        queue: counter::Receiver<Queue>,
        _phantom: PhantomData<T>,
    },
    Bounded(counter::Receiver<Bounded<T>>),
}

type Slot<T> = UnsafeCell<MaybeUninit<Message<T>>>;

/// Bounded channel implementation.
struct Bounded<T> {
    /// The messages themselves.  This Box owns the allocation of the messages, although it is
    /// unsafe to drop this with any messages stored in either of the Zephyr queues.
    ///
    /// The UnsafeCell is needed to indicate that this data is handled outside of what Rust is aware
    /// of.  MaybeUninit allows us to create this without allocation.
    _slots: Box<[Slot<T>]>,
    /// The free queue, holds messages that aren't be used.
    free: Queue,
    /// The channel queue.  These are messages that have been sent and are waiting to be received.
    chan: Queue,
}

impl<T> Bounded<T> {
    fn new(cap: usize) -> Self {
        let slots: Box<[Slot<T>]> = (0..cap)
            .map(|_| {
                UnsafeCell::new(MaybeUninit::uninit())
            })
        .collect();

        let free = Queue::new().unwrap();
        let chan = Queue::new().unwrap();

        // Add each of the boxes to the free list.
        for chan in &slots {
            unsafe {
                free.send(chan.get() as *mut c_void);
            }
        }

        Bounded { _slots: slots, free, chan }
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
