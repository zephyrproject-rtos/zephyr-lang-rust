//! Zephyr Work Queues
//!
//! # Zephyr Work Queues and Work
//!
//! Zephyr has a mechanism called a
//! [Workqueue](https://docs.zephyrproject.org/latest/kernel/services/threads/workqueue.html).
//!
//! Each workqueue is backed by a single Zephyr thread, and has its own stack.  The work queue
//! consists of a FIFO queue of work items that will be run consecutively on that thread.  The
//! underlying types are `k_work_q` for the work queue itself, and `k_work` for the worker.
//!
//! In addition to the simple schedulable work, Zephyr also has two additional types of work:
//! `k_work_delayable` which can be scheduled to run in the future, and `k_work_poll`, described as
//! triggered work in the docs.  This can be scheduled to run when various items within Zephyr
//! become available.  This triggered work also has a timeout.  In this sense the triggered work is
//! a superset of the other types of work.  Both delayable and triggered work are implemented by
//! having the `k_work` embedded in their structure, and Zephyr schedules the work when the given
//! reason happens.
//!
//! At this time, only the basic work queue type is supported.
//!
//! Zephyr's work queues can be used in different ways:
//!
//! - Work can be scheduled as needed.  For example, an IRQ handler can queue a work item to process
//!   data it has received from a device.
//! - Work can be scheduled periodically.
//!
//! As most C use of Zephyr statically allocates things like work, these are typically rescheduled
//! when the work is complete.  The work queue scheduling functions are designed, and intended, for
//! a given work item to be able to reschedule itself, and such usage is common.
//!
//! ## Ownership
//!
//! The remaining challenge with implementing `k_work` for Rust is that of ownership.  The model
//! taken here is that the work items are held in a `Box` that is effectively owned by the work
//! itself.  When the work item is scheduled to Zephyr, ownership of that box is effectively handed
//! off to C, and then when the work item is called, the Box re-constructed.  This repeats until the
//! work is no longer needed, at which point the work will be dropped.
//!
//! There are two common ways the lifecycle of work can be managed in an embedded system:
//!
//! - A set of `Future`'s are allocated once at the start, and these never return a value.  Work
//!   Futures inside of this (which correspond to `.await` in async code) can have lives and return
//!   values, but the main loops will not return values, or be dropped.  Embedded Futures will
//!   typically not be boxed.
//!
//! One consequence of the ownership being passed through to C code is that if the work cancellation
//! mechanism is used on a work queue, the work items themselves will be leaked.
//!
//! These work items are also `Pin`, to ensure that the work actions are not moved.
//!
//! ## The work queues themselves
//!
//! Workqueues themselves are built using [`WorkQueueBuilder`].  This needs a statically defined
//! stack.  Typical usage will be along the lines of:
//! ```rust
//! kobj_define! {
//!   WORKER_STACK: ThreadStack<2048>;
//! }
//! // ...
//! let main_worker = Box::new(
//!     WorkQueueBuilder::new()
//!         .set_priority(2).
//!         .set_name(c"mainloop")
//!         .set_no_yield(true)
//!         .start(MAIN_LOOP_STACK.init_once(()).unwrap())
//!     );
//!
//! let _ = zephyr::kio::spawn(
//!     mainloop(), // Async or function returning Future.
//!     &main_worker,
//!     c"w:mainloop",
//! );
//!
//! ...
//!
//! // Leak the Box so that the worker is never freed.
//! let _ = Box::leak(main_worker);
//! ```
//!
//! It is important that WorkQueues never be dropped.  It has a Drop implementation that invokes
//! panic.  Zephyr provides no mechanism to stop work queue threads, so dropping would result in
//! undefined behavior.
//!
//! # Current Status
//!
//! Although Zephyr has 3 types of work queues, the `k_work_poll` is sufficient to implement all of
//! the behavior, and this implementation only implements this type.  Non Future work could be built
//! around the other work types.
//!
//! As such, this means that manually constructed work is still built using `Future`.  The `_async`
//! primitives throughout this crate can be used just as readily by hand-written Futures as by async
//! code.  Notable, the use of [`Signal`] will likely be common, along with possible timeouts.
//!
//! [`sys::sync::Semaphore`]: crate::sys::sync::Semaphore
//! [`sync::channel`]: crate::sync::channel
//! [`sync::Mutex`]: crate::sync::Mutex
//! [`join`]: futures::JoinHandle::join
//! [`join_async`]: futures::JoinHandle::join_async

extern crate alloc;

use core::{
    cell::UnsafeCell,
    ffi::{c_int, c_uint, CStr},
    mem,
    pin::Pin,
};

use zephyr_sys::{
    k_poll_signal, k_poll_signal_check, k_poll_signal_init, k_poll_signal_raise,
    k_poll_signal_reset, k_work, k_work_init, k_work_q, k_work_queue_config, k_work_queue_init,
    k_work_queue_start, k_work_submit, k_work_submit_to_queue,
};

use crate::{
    error::to_result_void,
    object::Fixed,
    simpletls::SimpleTls,
    sync::{Arc, Mutex},
    sys::thread::ThreadStack,
};

/// A builder for work queues themselves.
///
/// A work queue is a Zephyr thread that instead of directly running a piece of code, manages a work
/// queue.  Various types of `Work` can be submitted to these queues, along with various types of
/// triggering conditions.
pub struct WorkQueueBuilder {
    /// The "config" value passed in.
    config: k_work_queue_config,
    /// Priority for the work queue thread.
    priority: c_int,
}

impl WorkQueueBuilder {
    /// Construct a new WorkQueueBuilder with default values.
    pub fn new() -> Self {
        Self {
            config: Default::default(),
            priority: 0,
        }
    }

    /// Set the name for the WorkQueue thread.
    ///
    /// This name shows up in debuggers and some analysis tools.
    pub fn set_name(&mut self, name: &'static CStr) -> &mut Self {
        self.config.name = name.as_ptr();
        self
    }

    /// Set the "no yield" flag for the created worker.
    ///
    /// If this is not set, the work queue will call `k_yield` between each enqueued work item.  For
    /// non-preemptible threads, this will allow other threads to run.  For preemptible threads,
    /// this will allow other threads at the same priority to run.
    ///
    /// This method has a negative in the name, which goes against typical conventions.  This is
    /// done to match the field in the Zephyr config.
    pub fn set_no_yield(&mut self, value: bool) -> &mut Self {
        self.config.no_yield = value;
        self
    }

    /// Set the "essential" flag for the created worker.
    ///
    /// This sets the essential flag on the running thread.  The system considers the termination of
    /// an essential thread to be a fatal error.
    pub fn set_essential(&mut self, value: bool) -> &mut Self {
        self.config.essential = value;
        self
    }

    /// Set the priority for the worker thread.
    ///
    /// See the Zephyr docs for the meaning of priority.
    pub fn set_priority(&mut self, value: c_int) -> &mut Self {
        self.priority = value;
        self
    }

    /// Start the given work queue thread.
    ///
    /// TODO: Implement a 'start' that works from a static work queue.
    pub fn start(&self, stack: ThreadStack) -> WorkQueue {
        let item: Fixed<k_work_q> = Fixed::new(unsafe { mem::zeroed() });
        unsafe {
            // SAFETY: Initialize zeroed memory.
            k_work_queue_init(item.get());

            // SAFETY: This associates the workqueue with the thread ID that runs it.  The thread is
            // a pointer into this work item, which will not move, because of the Fixed.
            let this = &mut *item.get();
            WORK_QUEUES
                .lock()
                .unwrap()
                .insert(&this.thread, WorkQueueRef(item.get()));

            // SAFETY: Start work queue thread.  The main issue here is that the work queue cannot
            // be deallocated once the thread has started.  We enforce this by making Drop panic.
            k_work_queue_start(
                item.get(),
                stack.base,
                stack.size,
                self.priority,
                &self.config,
            );
        }

        WorkQueue { item }
    }
}

/// A running work queue thread.
///
/// # Panic
///
/// Allowing a work queue to drop will result in a panic.  There are two ways to handle this,
/// depending on whether the WorkQueue is in a Box, or an Arc:
/// ```
/// // Leak a work queue in an Arc.
/// let wq = Arc::new(WorkQueueBuilder::new().start(...));
/// // If the Arc is used after this:
/// let _ = Arc::into_raw(wq.clone());
/// // If the Arc is no longer needed:
/// let _ = Arc::into_raw(wq);
///
/// // Leak a work queue in a Box.
/// let wq = Box::new(WorkQueueBuilder::new().start(...));
/// let _ = Box::leak(wq);
/// ```
pub struct WorkQueue {
    #[allow(dead_code)]
    item: Fixed<k_work_q>,
}

/// Work queues can be referenced from multiple threads, and thus are Send and Sync.
unsafe impl Send for WorkQueue {}
unsafe impl Sync for WorkQueue {}

impl Drop for WorkQueue {
    fn drop(&mut self) {
        panic!("WorkQueues must not be dropped");
    }
}

/// A simple mapping to get the current work_queue from the currently running thread.
///
/// This assumes that Zephyr's works queues have a 1:1 mapping between the work queue and the
/// thread.
///
/// # Safety
///
/// The work queue is protected with a sync Mutex (which uses an underlying Zephyr mutex).  It is,
/// in general, not a good idea to use a mutex in a work queue, as deadlock can happen.  So it is
/// important to both never .await while holding the lock, as well as to make sure operations within
/// it are relatively fast.  In this case, `insert` and `get` on the SimpleTls are reasonably fast.
/// `insert` is usually done just at startup as well.
///
/// This is a little bit messy as we don't have a lazy mechanism, so we have to handle this a bit
/// manually right now.
static WORK_QUEUES: Mutex<SimpleTls<WorkQueueRef>> = Mutex::new(SimpleTls::new());

/// For the queue mapping, we need a simple wrapper around the underlying pointer, one that doesn't
/// implement stop.
#[derive(Copy, Clone)]
struct WorkQueueRef(*mut k_work_q);

// SAFETY: The work queue reference is also safe for both Send and Sync per Zephyr semantics.
unsafe impl Send for WorkQueueRef {}
unsafe impl Sync for WorkQueueRef {}

/// Retrieve the current work queue, if we are running within one.
pub fn get_current_workq() -> Option<*mut k_work_q> {
    WORK_QUEUES.lock().unwrap().get().map(|wq| wq.0)
}

/// A Rust wrapper for `k_poll_signal`.
///
/// A signal in Zephyr is an event mechanism that can be used to trigger actions in event queues to
/// run.  The work somewhat like a kind of half boolean semaphore.  The signaling is robust in the
/// direction of the event happening, as in a blocked task will definitely wake when the signal happens. However, the clearing of the signal is racy.  Generally, there are two ways to do this:
///
/// - A work action can clear the signal as soon as it wakes up, before it starts processing any
///   data the signal was meant to indicate.  If the race happens, the processing will handle the
///   extra data.
/// - A work action can clear the signal after it does it's processing.  This is useful for things
///   like periodic timers, where if it is still processing when an additional timer tick comes in,
///   that timer tick will be ignored.  This is useful for periodic events where it is better to
///   just skip a tick rather than for them to "stack up" and get behind.
///
/// Notably, as long as the `reset` method is only ever called by the worker that is waiting upon
/// it, there shouldn't ever be a race in the `wait_async` itself.
///
/// Signals can pass a `c_int` from the signalling task to the task that is waiting for the signal.
/// It is not specified in the Zephyr documentation what value will be passed if `raise` is called
/// multiple times before a task waits upon a signal.  The current implementation will return the
/// most recent raised `result` value.
///
/// For most other use cases, channels or semaphores are likely to be better solutions.
pub struct Signal {
    /// The raw Zephyr `k_poll_signal`.
    pub(crate) item: Fixed<k_poll_signal>,
}

// SAFETY: Zephyr's API maintains thread safety.
unsafe impl Send for Signal {}
unsafe impl Sync for Signal {}

impl Signal {
    /// Create a new `Signal`.
    ///
    /// The Signal will be in the non-signaled state.
    pub fn new() -> Signal {
        // SAFETY: The memory is zero initialized, and Fixed ensure that it never changes address.
        let item: Fixed<k_poll_signal> = Fixed::new(unsafe { mem::zeroed() });
        unsafe {
            k_poll_signal_init(item.get());
        }
        Signal { item }
    }

    /// Reset the Signal
    ///
    /// This resets the signal state to unsignaled.
    ///
    /// Please see the [`Signal`] documentation on how to handle the races that this implies.
    pub fn reset(&self) {
        // SAFETY: This is safe with a non-mut reference, as the purpose of the Zephyr API is to
        // coordinate this information between threads.
        unsafe {
            k_poll_signal_reset(self.item.get());
        }
    }

    /// Check the status of a signal.
    ///
    /// This reads the status of the signal.  If the state is "signalled", this will return
    /// `Some(result)` where the `result` is the result value given to [`raise`].
    ///
    /// [`raise`]: Self::raise
    pub fn check(&self) -> Option<c_int> {
        let mut signaled: c_uint = 0;
        let mut result: c_int = 0;
        unsafe {
            // SAFETY: Zephyr's signal API coordinates access across threads.
            k_poll_signal_check(self.item.get(), &mut signaled, &mut result);
        }

        if signaled != 0 {
            Some(result)
        } else {
            None
        }
    }

    /// Signal a signal object.
    ///
    /// This will signal to any worker that is waiting on this object that the event has happened.
    /// The `result` will be returned from the worker's `wait` call.
    ///
    /// As per the Zephyr docs, this could return an EAGAIN error if the polling thread is in the
    /// process of expiring.  The implication is that the signal will not be raised in this case.
    /// ...
    pub fn raise(&self, result: c_int) -> crate::Result<()> {
        to_result_void(unsafe { k_poll_signal_raise(self.item.get(), result) })
    }
}

impl Default for Signal {
    fn default() -> Self {
        Signal::new()
    }
}

/// Possible returns from work queue submission.
#[derive(Debug, Clone, Copy)]
pub enum SubmitResult {
    /// This work was already in a queue.
    AlreadySubmitted,
    /// The work has been added to the specified queue.
    Enqueued,
    /// The queue was called from the worker itself, and has been queued to the queue that was
    /// running it.
    WasRunning,
}

impl SubmitResult {
    /// Does this result indicate that the work was enqueued?
    pub fn enqueued(self) -> bool {
        matches!(self, Self::Enqueued | Self::WasRunning)
    }

    /// Convert an int result from a work submit function.
    fn to_result(value: c_int) -> crate::Result<Self> {
        crate::error::to_result(value).map(|code| match code {
            0 => Self::AlreadySubmitted,
            1 => Self::Enqueued,
            2 => Self::WasRunning,
            _ => panic!("Unexpected result {} from Zephyr work submission", code),
        })
    }
}

/// A simple action that just does something with its data.
///
/// This is similar to a Future, except there is no concept of it completing.  It manages its
/// associated data however it wishes, and is responsible for re-queuing as needed.
///
/// Note, specifically, that the Act does not take a mutable reference.  This is because the Work
/// below uses an Arc, so this data can be shared.
pub trait SimpleAction {
    /// Perform the action.
    fn act(self: Pin<&Self>);
}

/// A basic Zephyr work item.
///
/// Holds a `k_work`, along with the data associated with that work.  When the work is queued, the
/// `act` method will be called on the provided `SimpleAction`.
pub struct Work<T> {
    work: UnsafeCell<k_work>,
    action: T,
}

/// SAFETY: Work queues can be sent as long as the action itself can be.
unsafe impl<F> Send for Work<F>
where
    F: SimpleAction,
    F: Send,
{
}

/// SAFETY: Work queues are Sync when the action is.
unsafe impl<F> Sync for Work<F>
where
    F: SimpleAction,
    F: Sync,
{
}

impl<T: SimpleAction + Send> Work<T> {
    /// Construct a new Work from the given action.
    ///
    /// Note that the data will be moved into the pinned Work.  The data is internal, and only
    /// accessible to the work thread (the `act` method).  If shared data is needed, normal
    /// inter-thread sharing mechanisms are needed.
    ///
    /// TODO: Can we come up with a way to allow sharing on the same worker using Rc instead of Arc?
    pub fn new(action: T) -> Pin<Arc<Self>> {
        let this = Arc::pin(Self {
            // SAFETY: will be initialized below, after this is pinned.
            work: unsafe { mem::zeroed() },
            action,
        });

        // SAFETY: Initializes above zero-initialized struct.
        unsafe {
            k_work_init(this.work.get(), Some(Self::handler));
        }

        this
    }

    /// Submit this work to the system work queue.
    ///
    /// This can return several possible `Ok` results.  See the docs on [`SubmitResult`] for an
    /// explanation of them.
    pub fn submit(this: Pin<Arc<Self>>) -> crate::Result<SubmitResult> {
        // We "leak" the arc, so that when the handler runs, it can be safely turned back into an
        // Arc, and the drop on the arc will then run.
        let work = this.work.get();

        // SAFETY: C the code does not perform moves on the data, and the `from_raw` below puts it
        // back into a Pin when it reconstructs the Arc.
        let this = unsafe { Pin::into_inner_unchecked(this) };
        let _ = Arc::into_raw(this);

        // SAFETY: The Pin ensures this will not move.  Our implementation of drop ensures that the
        // work item is no longer queued when the data is dropped.
        SubmitResult::to_result(unsafe { k_work_submit(work) })
    }

    /// Submit this work to a specified work queue.
    ///
    /// TODO: Change when we have better wrappers for work queues.
    pub fn submit_to_queue(this: Pin<Arc<Self>>, queue: &WorkQueue) -> crate::Result<SubmitResult> {
        let work = this.work.get();

        // "leak" the arc to give to C.  We'll reconstruct it in the handler.
        // SAFETY: The C code does not perform moves on the data, and the `from_raw` below puts it
        // back into a Pin when it reconstructs the Arc.
        let this = unsafe { Pin::into_inner_unchecked(this) };
        let _ = Arc::into_raw(this);

        // SAFETY: The Pin ensures this will not move.  Our implementation of drop ensures that the
        // work item is no longer queued when the data is dropped.
        SubmitResult::to_result(unsafe { k_work_submit_to_queue(queue.item.get(), work) })
    }

    /// Callback, through C, but bound by a specific type.
    extern "C" fn handler(work: *mut k_work) {
        // We want to avoid needing a `repr(C)` on our struct, so the `k_work` pointer is not
        // necessarily at the beginning of the struct.
        // SAFETY: Converts raw pointer to work back into the box.
        let this = unsafe { Self::from_raw(work) };

        // Access the action within, still pinned.
        // SAFETY: It is safe to keep the pin on the interior.
        let action = unsafe { this.as_ref().map_unchecked(|p| &p.action) };

        action.act();
    }

    /*
    /// Consume this Arc, returning the internal pointer.  Needs to have a complementary `from_raw`
    /// called to avoid leaking the item.
    fn into_raw(this: Pin<Arc<Self>>) -> *const Self {
        // SAFETY: This removes the Pin guarantee, but is given as a raw pointer to C, which doesn't
        // generally use move.
        let this = unsafe { Pin::into_inner_unchecked(this) };
        Arc::into_raw(this)
    }
    */

    /// Given a pointer to the work_q burried within, recover the Pinned Box containing our data.
    unsafe fn from_raw(ptr: *const k_work) -> Pin<Arc<Self>> {
        // SAFETY: This fixes the pointer back to the beginning of Self.  This also assumes the
        // pointer is valid.
        let ptr = ptr
            .cast::<u8>()
            .sub(mem::offset_of!(Self, work))
            .cast::<Self>();
        let this = Arc::from_raw(ptr);
        Pin::new_unchecked(this)
    }

    /// Access the inner action.
    pub fn action(&self) -> &T {
        &self.action
    }
}
