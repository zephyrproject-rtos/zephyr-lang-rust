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
//!
//! ## IRQ safety
//!
//! These channels are usable from IRQ context on Zephyr in very limited situations.  Notably, all
//! of the following must be true:
//! - The channel has been created with `bounded()`, which pre-allocates all of the messages.
//! - If the type `T` has a Drop implementation, this implementation can be called from IRQ context.
//! - Only `try_send` or `try_recv` are used on the channel.
//!
//! The requirement for Drop is only strictly true if the IRQ handler calls `try_recv` and drops
//! received message.  If the message is *always* sent over another channel or otherwise not
//! dropped, it *might* be safe to use these messages.
//!
//! ## Dropping of Sender/Receiver
//!
//! Crossbeam channels support detecting when all senders or all receivers have been dropped on a
//! channel, which will cause the handles on the other end to error, including waking up current
//! threads waiting on those channels.
//!
//! At this time, this isn't implementable in Zephyr, as there is no API to wake up all threads
//! blocked on a given `k_queue`.  As such, this scenario is not supported.  What actually happens
//! is that when all senders or receivers on a channel are dropped, operations on the other end of
//! the channel may just block (or queue forever with unbounded queues).  If all handles (both
//! sender and receiver) are dropped, the last drop will cause a panic.  It maybe be better to just
//! leak the entire channel, as any data associated with the channels would be leaked at this point,
//! including the underlying Zephyr `k_queue`.  Until APIs are added to Zephyr to allow the channel
//! information to be safely freed, these can't actually be freed.

extern crate alloc;

use alloc::boxed::Box;

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt;
use core::future::Future;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::task::Poll;

use crate::kio::ContextExt;
use crate::sys::queue::Queue;
use crate::time::{Duration, Forever, NoWait, Timeout};

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
        },
    };
    let r = Receiver {
        flavor: ReceiverFlavor::Unbounded {
            queue: r,
            _phantom: PhantomData,
        },
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
        panic!("Zero capacity queues are not supported on Zephyr");
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
        Message { _private: 0, data }
    }
}

/// The sending side of a channel.
pub struct Sender<T> {
    flavor: SenderFlavor<T>,
}

// SAFETY: We implement Send and Sync for the Sender itself, as long as the underlying data can be
// sent.  The underlying zephyr primitives used for the channel provide the Sync safety.
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
    where
        D: Into<Timeout>,
    {
        match &self.flavor {
            SenderFlavor::Unbounded { queue, .. } => {
                let msg = Box::new(Message::new(msg));
                let msg = Box::into_raw(msg);
                // SAFETY: Zephyr requires, for as long as the message remains in the queue, that
                // the first `usize` of the message be available for its use, and that the message
                // not be moved.  The `into_raw` of the box consumes the box, so this is entirely a
                // raw pointer with no references from the Rust code.  The item is not used until it
                // has been removed from the queue.
                unsafe {
                    queue.send(msg as *mut c_void);
                }
            }
            SenderFlavor::Bounded(chan) => {
                // Retrieve a message buffer from the free list.
                // SAFETY: Please see the safety discussion on `Bounded` on what makes this safe.
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

// A little note about the Unpin constraint here.  Because Futures are pinned in Rust Async code,
// and the future stores the messages, we can only send and receive messages that aren't pinned.
impl<T: Unpin> Sender<T> {
    /// Waits for a message to be sent into the channel, but only for a limited time.  Async
    /// version.
    ///
    /// This has the same behavior as [`send_timeout`], but as an Async function.
    ///
    /// [`send_timeout`]: Sender::send_timeout
    pub fn send_timeout_async<'a>(
        &'a self,
        msg: T,
        timeout: impl Into<Timeout>,
    ) -> impl Future<Output = Result<(), SendError<T>>> + 'a {
        SendFuture {
            sender: self,
            msg: Some(msg),
            timeout: timeout.into(),
            waited: false,
        }
    }

    /// Sends a message over the given channel, waiting if necessary. Async version.
    pub async fn send_async(&self, msg: T) -> Result<(), SendError<T>> {
        self.send_timeout_async(msg, Forever).await
    }

    // Note that there is no async version of `try_send`.
}

/// The implementation of Future for Sender::send_timeout_async.
struct SendFuture<'a, T: Unpin> {
    sender: &'a Sender<T>,
    msg: Option<T>,
    timeout: Timeout,
    waited: bool,
}

impl<'a, T: Unpin> Future for SendFuture<'a, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        /*
        let this = unsafe {
            Pin::get_unchecked_mut(self)
        };
        */
        let this = Pin::get_mut(self);

        // Take the message out in preparation to try sending it.  It is a logic error if the unwrap
        // fails.
        let msg = this.msg.take().unwrap();

        // Try sending the message, with no timeout.
        let msg = match this.sender.try_send(msg) {
            Ok(()) => return Poll::Ready(Ok(())),
            Err(SendError(msg)) => msg,
        };

        if this.waited {
            // We already waited, and no message, so give the messagre back, indiciating a timeout.
            return Poll::Ready(Err(SendError(msg)));
        }

        // Send didn't happen, put the message back to have for the next call.
        this.msg = Some(msg);

        // Otherwise, schedule to wake up on receipt or timeout.
        match &this.sender.flavor {
            SenderFlavor::Unbounded { .. } => {
                panic!("Implementation error: unbounded queues should never fail");
            }
            SenderFlavor::Bounded(chan) => {
                cx.add_queue(&chan.free, this.timeout);
            }
        }

        Poll::Pending
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        match &self.flavor {
            SenderFlavor::Unbounded { queue, .. } => {
                // SAFETY: It is not possible to free from Zephyr queues.  This means drop has to
                // either leak or panic.  We will panic for now.
                unsafe {
                    queue.release(|_| {
                        panic!("Unbounded queues cannot currently be dropped");
                    })
                }
            }
            SenderFlavor::Bounded(chan) => {
                // SAFETY: It is not possible to free from Zephyr queues.  This means drop has to
                // either leak or panic.  We will panic for now.
                unsafe {
                    chan.release(|_| {
                        panic!("Bounded queues cannot currently be dropped");
                    })
                }
            }
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        let flavor = match &self.flavor {
            SenderFlavor::Unbounded { queue, .. } => SenderFlavor::Unbounded {
                queue: queue.acquire(),
                _phantom: PhantomData,
            },
            SenderFlavor::Bounded(chan) => SenderFlavor::Bounded(chan.acquire()),
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

// SAFETY: We implement Send and Sync for the Receiver itself, as long as the underlying data can be
// sent.  The underlying zephyr primitives used for the channel provide the Sync safety.
unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T> Receiver<T> {
    /// Waits for a message to be received from the channel, but only for a limited time.
    ///
    /// If the channel is empty and not disconnected, this call will block until the receive
    /// operation can proceed or the operation times out.
    /// wake up and return an error.
    pub fn recv_timeout<D>(&self, timeout: D) -> Result<T, RecvError>
    where
        D: Into<Timeout>,
    {
        match &self.flavor {
            ReceiverFlavor::Unbounded { queue, .. } => {
                // SAFETY: Messages were sent by converting a Box through `into_raw()`.
                let msg = unsafe {
                    let msg = queue.recv(timeout);
                    if msg.is_null() {
                        return Err(RecvError);
                    }
                    msg
                };
                let msg = msg as *mut Message<T>;
                // SAFETY: After receiving the message from the queue's `recv` method, Zephyr will
                // no longer use the `usize` at the beginning, and it is safe for us to convert the
                // message back into a box, copy the field out of it, an allow the Box itself to be
                // freed.
                let msg = unsafe { Box::from_raw(msg) };
                Ok(msg.data)
            }
            ReceiverFlavor::Bounded(chan) => {
                // SAFETY: Please see the safety discussion on Bounded.
                let rawbuf = unsafe {
                    let buf = chan.chan.recv(timeout);
                    if buf.is_null() {
                        return Err(RecvError);
                    }
                    buf
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

    /// Blocks the current thread until a message is received or the channel is empty and
    /// disconnected.
    ///
    /// If the channel is empty and not disconnected, this call will block until the receive
    /// operation can proceed.
    pub fn recv(&self) -> Result<T, RecvError> {
        self.recv_timeout(Forever)
    }

    /// Attempts to receive a message from the channel without blocking.
    ///
    /// This method will either receive a message from the channel immediately, or return an error
    /// if the channel is empty.
    ///
    /// This method is safe to use from IRQ context, if and only if the channel was created as a
    /// bounded channel.
    pub fn try_recv(&self) -> Result<T, RecvError> {
        self.recv_timeout(NoWait)
    }
}

// Note that receive doesn't need the Unpin constraint, as we aren't storing any message.
impl<T> Receiver<T> {
    /// Waits for a message to be received from the channel, but only for a limited time.
    /// Async version.
    ///
    /// If the channel is empty and not disconnected, this call will block until the receive
    /// operation can proceed or the operation times out.
    /// wake up and return an error.
    pub fn recv_timeout_async<'a>(
        &'a self,
        timeout: impl Into<Timeout>,
    ) -> impl Future<Output = Result<T, RecvError>> + 'a {
        RecvFuture {
            receiver: self,
            timeout: timeout.into(),
            waited: false,
        }
    }

    /// Blocks the current thread until a message is received or the channel is empty and
    /// disconnected.  Async version.
    ///
    /// If the channel is empty and not disconnected, this call will block until the receive
    /// operation can proceed.
    pub async fn recv_async(&self) -> Result<T, RecvError> {
        self.recv_timeout_async(Forever).await
    }

    /// Return a reference to the inner queue.
    fn as_queue(&self) -> &Queue {
        match &self.flavor {
            ReceiverFlavor::Unbounded { queue, .. } => queue,
            ReceiverFlavor::Bounded(chan) => &chan.chan,
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        match &self.flavor {
            ReceiverFlavor::Unbounded { queue, .. } => {
                // SAFETY: As the Zephyr channel cannot be freed we must either leak or panic.
                // Chose panic for now.
                unsafe {
                    queue.release(|_| {
                        panic!("Unnbounded channel cannot be dropped");
                    })
                }
            }
            ReceiverFlavor::Bounded(chan) => {
                // SAFETY: As the Zephyr channel cannot be freed we must either leak or panic.
                // Chose panic for now.
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
            ReceiverFlavor::Unbounded { queue, .. } => ReceiverFlavor::Unbounded {
                queue: queue.acquire(),
                _phantom: PhantomData,
            },
            ReceiverFlavor::Bounded(chan) => ReceiverFlavor::Bounded(chan.acquire()),
        };

        Receiver { flavor }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender")
    }
}

struct RecvFuture<'a, T> {
    receiver: &'a Receiver<T>,
    timeout: Timeout,
    waited: bool,
}

impl<'a, T> Future for RecvFuture<'a, T> {
    type Output = Result<T, RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> Poll<Self::Output> {
        // Try to receive a message.
        if let Ok(msg) = self.receiver.try_recv() {
            return Poll::Ready(Ok(msg));
        }

        if self.waited {
            // Wait already happened, so this is a timeout.
            return Poll::Ready(Err(RecvError));
        }

        // Otherwise, schedule to wakeup on receipt or timeout.
        cx.add_queue(self.receiver.as_queue(), self.timeout);
        self.waited = true;

        Poll::Pending
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

// SAFETY: A Bounded channel contains an array of messages that are allocated together in a Box.
// This Box is held for an eventual future implementation that is able to free the messages, once
// they have all been taken from Zephyr's knowledge.  For now, they could also be leaked.
// It is a `Pin<Box<...>>` because it is important that the data never be moved, as we maintain
// pointers to the items in Zephyr queues.
//
// There are two `Queue`s used here: `free` is the free list of messages that are not being sent,
// and `chan` for messages that have been sent but not received.  Initially, all slots are placed on
// the `free` queue.  At any time, outside of the calls in this module, each slot must live inside
// of one of the two queues.  This means that the messages cannot be moved or accessed, except
// inside of the individual send/receive operations.  Zephyr makes use of the initial `usize` field
// at the beginning of each Slot.
//
// We use MaybeUninit for the messages to avoid needing to initialize the messages.  The individual
// messages are accessed through pointers when they are retrieved from the Zephyr `Queue`, so these
// values are never marked as initialized.
/// Bounded channel implementation.
struct Bounded<T> {
    /// The messages themselves.  This Box owns the allocation of the messages, although it is
    /// unsafe to drop this with any messages stored in either of the Zephyr queues.
    ///
    /// The UnsafeCell is needed to indicate that this data is handled outside of what Rust is aware
    /// of.  MaybeUninit allows us to create this without allocation.
    _slots: Pin<Box<[Slot<T>]>>,
    /// The free queue, holds messages that aren't be used.
    free: Queue,
    /// The channel queue.  These are messages that have been sent and are waiting to be received.
    chan: Queue,
}

impl<T> Bounded<T> {
    fn new(cap: usize) -> Self {
        let slots: Box<[Slot<T>]> = (0..cap)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect();
        let slots = Box::into_pin(slots);

        let free = Queue::new().unwrap();
        let chan = Queue::new().unwrap();

        // Add each of the boxes to the free list.
        for slot in slots.as_ref().iter() {
            // SAFETY: See safety discussion on `Bounded`.
            unsafe {
                free.send(slot.get() as *mut c_void);
            }
        }

        Bounded {
            _slots: slots,
            free,
            chan,
        }
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

/// Wait loop
///
/// A common scenario for async work tasks is to wait for, and process messages off of a queue, but
/// to also wake periodically to perform some task.
///
/// This performs this periodic loop.  It has some support for handling the case where the
/// processing takes longer than the loop duration, but it merely re-schedules for the period past
/// the current time.  This means the phase of the period will change upon dropped ticks.
///
/// Each time an event is received, 'handle' is called with `Some(ev)`.  In addition, periodically
/// (based on `period`) `handle` will be called with None.
///
/// **Note**: It needs to be a single handler, because this closure will frequently be in a move
/// closure, and this would force shared data to be shared in Sync types of wrappers.  The main
/// purpose of combining the event handling and the periodic is to avoid that.
///
/// Note that also, if the timer is just barely able to run, it will still be scheduled "shortly" in
/// the future.
///
/// T is the type of the messages expected to be received.
///
/// TODO: This function, in general, is completely worthless without Rust support for [async
/// closures](https://rust-lang.github.io/rfcs/3668-async-closures.html).
pub async fn event_loop_useless<T, EF, EFF>(
    events: Receiver<T>,
    period: Duration,
    mut handle: EF,
) -> !
where
    EF: FnMut(Option<T>) -> EFF,
    EFF: Future<Output = ()>,
{
    // Start with a deadline 'period' out in the future.
    let mut next = crate::time::now() + period;
    loop {
        if let Ok(ev) = events.recv_timeout_async(next).await {
            handle(Some(ev)).await;
            continue;
        }

        // We either reached, or exceeded our timeout.
        handle(None).await;

        // Calculate the next time.
        next += period;

        // If this is passed, just reschedule after our Duration from "now".
        let now = crate::time::now();
        if next <= now {
            next = now + period;
        }
    }
}

/// Wait loop, as a macro.
///
/// This is the `event loop` above, implemented as a macro, which becomes more useful as the async
/// closures aren't needed.
#[macro_export]
macro_rules! event_loop {
    ($events:expr, $period:expr,
     Some($eventvar:ident) => $event_body:block,
     None => $periodic_body: block $(,)?) =>
    {
        let events = $events;
        let period = $period;
        let mut next = $crate::time::now() + period;
        loop {
            if let Ok($eventvar) = events.recv_timeout_async(next).await {
                $event_body
            } else {
                // Note that ':block' above requires the braces, so this body can't introduce
                // bindings that shadow our local variables.
                $periodic_body
                next += period;

                // If this is passed, just reschedule after our Duration from "now".
                let now = $crate::time::now();
                if next <= now {
                    ::log::warn!("periodic overflow: {} ticks, {}:{}",
                                 (now - next).ticks(),
                                 core::file!(), core::line!());
                    next = now + period;
                }
            }
        }
    };
}
