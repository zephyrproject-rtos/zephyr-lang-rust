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
//! ## Waitable events
//!
//! The triggerable work items can be triggered to wake on a set of any of the following:
//!
//! - A signal.  `k_poll_signal` is a type used just for waking work items.  This works similar to a
//!   binary semaphore, but is lighter weight for use just by this mechanism.
//! - A semaphore.  Work can be scheduled to run when a `k_sem` is available.  Since
//!   [`sys::sync::Semaphore`] is built on top of `k_sem`, the "take" operation for these semaphores
//!   can be a trigger source.
//! - A queue/FIFO/LIFO.  The queue is used to implement [`sync::channel`] and thus any blocking
//!   operation on queues can be a trigger source.
//! - Message Queues, and Pipes.  Although not yet provided in Rust, these can also be a source of
//!   triggering.
//!
//! It is important to note that the trigger source may not necessarily still be available by the
//! time the work item is actually run.  This depends on the design of the system.  If there is only
//! a single waiter, then it will still be available (the mechanism does not have false triggers,
//! like CondVar).
//!
//! Also, note, specifically, that Zephyr Mutexes cannot be used as a trigger source.  That means
//! that locking a [`sync::Mutex`] shouldn't be use within work items.  There is another
//! [`kio::sync::Mutex`], which is a simplified Mutex that is implemented with a Semaphore that can
//! be used from work-queue based code.
//!
//! # Rust `Future`
//!
//! The rust language, also has built-in support for something rather similar to Zephyr work queues.
//! The main user-visible type behind this is [`Future`].  The rust compiler has support for
//! functions, as well as code blocks to be declared as `async`.  For this code, instead of directly
//! returning the given data, returns a `Future` that has as its output type the data.  What this
//! does is essentially capture what would be stored on the stack to maintain the state of that code
//! into the data of the `Future` itself.  For rust code running on a typical OS, a crate such as
//! [Tokio](https://tokio.rs/) provides what is known as an executor, which implements the schedule
//! for these `Futures` as well as provides equivalent primitives for Mutex, Semaphores and channels
//! for this code to use for synchronization.
//!
//! It is notable that the Zephyr implementation of `Future` operations under a fairly simple
//! assumption of how this scheduling will work.  Each future is invoked with a Context, which
//! contains a dynamic `Waker` that can be invoked to schedule this Future to run again.  This means
//! that the primitives are typically implemented above OS primitives, where each manages wake
//! queues to determine the work that needs to be woken.
//!
//! # Bringing it together.
//!
//! There are a couple of issues that need to be addressed to bring work-queue support to Rust.
//! First is the question of how they will be used.  On the one hand, there are users that will
//! definitely want to make use of `async` in rust, and it is important to implement a executor,
//! similar to Tokio, that will schedule this `async` code.  On the other hand, it will likely be
//! common for others to want to make more direct use of the work queues themselves.  As such, these
//! users will want more direct access to scheduling and triggering of work.
//!
//! ## Future erasure
//!
//! One challenge with using `Future` for work is that the `Future` type intentionally erases the
//! details of scheduling work, reducing it down to a single `Waker`, which similar to a trait, has
//! a `wake` method to cause the executor to schedule this work.  Unfortunately, this simple
//! mechanism makes it challenging to take advantage of Zephyr's existing mechanisms to be able to
//! automatically trigger work based on primitives.
//!
//! As such, what we do is have a structure `Work` that contains both a `k_work_poll` as well as
//! `Context` from Rust.  Our handler can use a mechanism similar to C's `CONTAINER_OF` macro to
//! recover this outer structure.
//!
//! There is some extra complexity to this process, as the `Future` we are storing associated with
//! the work is `?Sized`, since each particular Future will have a different size.  As such, it is
//! not possible to recover the full work type.  To work around this, we have a Sized struct at the
//! beginning of this structure, that along with judicious use of `#[repr(C)]` allows us to recover
//! this fixed data.  This structure contains the information needed to re-schedule the work, based
//! on what is needed.
//!
//! ## Ownership
//!
//! The remaining challenge with implementing `k_work` for Rust is that of ownership.  The model
//! taken here is that the work items are held in a `Box` that is effectively owned by the work
//! itself.  When the work item is scheduled to Zephyr, ownership of that box is effectively handed
//! off to C, and then when the work item is called, the Box re-constructed.  This repeats until the
//! work is no longer needed (e.g. when a [`Future::poll`] returns `Ready`), at which point the work
//! will be dropped.
//!
//! There are two common ways the lifecycle of work can be managed in an embedded system:
//!
//! - A set of `Future`'s are allocated once at the start, and these never return a value.  Work
//!   Futures inside of this (which correspond to `.await` in async code) can have lives and return
//!   values, but the main loops will not return values, or be dropped.  Embedded Futures will
//!   typically not be boxed.
//! - Work will be dynamically created based on system need, with threads using [`kio::spawn`] to
//!   create additional work (or creating the `Work` items directly).  These can use [`join`] or
//!   [`join_async`] to wait for the results.
//!
//! One consequence of the ownership being passed through to C code is that if the work cancellation
//! mechanism is used on a work queue, the work items themselves will be leaked.
//!
//! The Future mechanism in Rust relies on the use of [`Pin`] to ensure that work items are not
//! moved.  We have the same requirements here, although currently, the pin is only applied while
//! the future is run, and we do not expose the `Box` that we use, thus preventing moves of the work
//! items.
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
//! let main_worker = Box::new(j
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
//! [`kio::sync::Mutex`]: crate::kio::sync::Mutex
//! [`kio::spawn`]: crate::kio::spawn
//! [`join`]: futures::JoinHandle::join
//! [`join_async`]: futures::JoinHandle::join_async

extern crate alloc;

use alloc::boxed::Box;
use core::{
    convert::Infallible,
    ffi::{c_int, c_uint, CStr},
    future::Future,
    mem,
    pin::Pin,
    ptr,
    task::Poll,
};

use zephyr_sys::{
    k_poll_signal, k_poll_signal_check, k_poll_signal_init, k_poll_signal_raise,
    k_poll_signal_reset, k_work, k_work_init, k_work_q, k_work_queue_config, k_work_queue_init,
    k_work_queue_start, k_work_submit, k_work_submit_to_queue, ETIMEDOUT,
};

use crate::{error::to_result_void, kio::ContextExt, object::Fixed, simpletls::StaticTls, sys::thread::ThreadStack, time::Timeout};

pub mod futures;

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
            config: k_work_queue_config {
                name: ptr::null(),
                no_yield: false,
                essential: false,
            },
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
            WORK_QUEUES.insert(&this.thread, WorkQueueRef(item.get()));

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
///
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
/// it are relatively fast.  In this case, `jnsert` and `get` on the SimpleTls are reasonably fast.
/// `insert` is usually done just at startup as well.
///
/// This is a little bit messy as we don't have a lazy mechanism, so we have to handle this a bit
/// manually right now.
static WORK_QUEUES: StaticTls<WorkQueueRef> = StaticTls::new();

/// For the queue mapping, we need a simple wrapper around the underlying pointer, one that doesn't
/// implement stop.
#[derive(Copy, Clone)]
struct WorkQueueRef(*mut k_work_q);

// SAFETY: The work queue reference is also safe for both Send and Sync per Zephyr semantics.
unsafe impl Send for WorkQueueRef {}
unsafe impl Sync for WorkQueueRef {}

/// Retrieve the current work queue, if we are running within one.
pub fn get_current_workq() -> Option<*mut k_work_q> {
    WORK_QUEUES.get().map(|wq| wq.0)
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
    pub fn new() -> Result<Signal, Infallible> {
        // SAFETY: The memory is zero initialized, and Fixed ensure that it never changes address.
        let item: Fixed<k_poll_signal> = Fixed::new(unsafe { mem::zeroed() });
        unsafe {
            k_poll_signal_init(item.get());
        }
        Ok(Signal { item })
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

    /// Asynchronously wait for a signal to be signaled.
    ///
    /// If the signal has not been raised, will wait until it has been.  If the signal has been
    /// raised, the Future will immediately return that value without waiting.
    ///
    /// **Note**: there is no sync wait, as Zephyr does not provide a convenient mechanmism for
    /// this.  It could be implemented with `k_poll` if needed.
    pub fn wait_async<'a>(
        &'a self,
        timeout: impl Into<Timeout>,
    ) -> impl Future<Output = crate::Result<c_int>> + 'a {
        SignalWait {
            signal: self,
            timeout: timeout.into(),
            ran: false,
        }
    }
}

/// The Future for Signal::wait_async.
struct SignalWait<'a> {
    /// The signal we are waiting on.
    signal: &'a Signal,
    /// The timeout to use.
    timeout: Timeout,
    /// Set after we've waited once,
    ran: bool,
}

impl<'a> Future for SignalWait<'a> {
    type Output = crate::Result<c_int>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        // We can check if the even happened immediately, and avoid blocking if we were already
        // signaled.
        if let Some(result) = self.signal.check() {
            return Poll::Ready(Ok(result));
        }

        if self.ran {
            // If it is not ready, assuming a timeout.  Note that if a thread other than this work
            // thread resets the signal, it is possible to see a timeout even if `Forever` was given
            // as the timeout.
            return Poll::Ready(Err(crate::Error(ETIMEDOUT)));
        }

        cx.add_signal(self.signal, self.timeout);
        self.ran = true;

        Poll::Pending
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
pub trait SimpleAction<T> {
    /// Perform the action.
    fn act(self: Pin<&mut Self>);
}

/// A basic Zephyr work item.
///
/// Holds a `k_work`, along with the data associated with that work.  When the work is queued, the
/// `act` method will be called on the provided `SimpleAction`.
#[repr(C)]
pub struct Work<T> {
    work: k_work,
    action: T,
}

impl<T: SimpleAction<T>> Work<T> {
    /// Construct a new Work from the given action.
    ///
    /// Note that the data will be moved into the pinned Work.  The data is internal, and only
    /// accessible to the work thread (the `act` method).  If shared data is needed, normal
    /// inter-thread sharing mechanisms are needed.
    ///
    /// TODO: Can we come up with a way to allow sharing on the same worker using Rc instead of Arc?
    pub fn new(action: T) -> Pin<Box<Self>> {
        let mut this = Box::pin(Self {
            // SAFETY: will be initialized below, after this is pinned.
            work: unsafe { mem::zeroed() },
            action,
        });
        let ptr = this.as_mut().as_k_work();
        // SAFETY: Initializes the zero allocated struct.
        unsafe {
            k_work_init(ptr, Some(Self::handler));
        }

        this
    }

    /// Submit this work to the system work queue.
    ///
    /// This can return several possible `Ok` results.  See the docs on [`SubmitResult`] for an
    /// explanation of them.
    pub fn submit(self: Pin<&mut Self>) -> crate::Result<SubmitResult> {
        // SAFETY: The Pin ensures this will not move.  Our implementation of drop ensures that the
        // work item is no longer queued when the data is dropped.
        SubmitResult::to_result(unsafe { k_work_submit(self.as_k_work()) })
    }

    /// Submit this work to a specified work queue.
    ///
    /// TODO: Change when we have better wrappers for work queues.
    pub fn submit_to_queue(
        self: Pin<&mut Self>,
        queue: *mut k_work_q,
    ) -> crate::Result<SubmitResult> {
        // SAFETY: The Pin ensures this will not move.  Our implementation of drop ensures that the
        // work item is no longer queued when the data is dropped.
        SubmitResult::to_result(unsafe { k_work_submit_to_queue(queue, self.as_k_work()) })
    }

    /// Get the pointer to the underlying work queue.
    fn as_k_work(self: Pin<&mut Self>) -> *mut k_work {
        // SAFETY: This is private, and no code here will move the pinned item.
        unsafe { self.map_unchecked_mut(|s| &mut s.work).get_unchecked_mut() }
    }

    /// Get a pointer into our action.
    fn as_action(self: Pin<&mut Self>) -> Pin<&mut T> {
        // SAFETY: We rely on the worker itself not moving the data.
        unsafe { self.map_unchecked_mut(|s| &mut s.action) }
    }

    /// Callback, through C, but bound by a specific type.
    extern "C" fn handler(work: *mut k_work) {
        // SAFETY: We rely on repr(C) placing the first field of a struct at the same address as the
        // struct.  This avoids needing a Rust equivalent to `CONTAINER_OF`.
        let this: Pin<&mut Self> = unsafe { Pin::new_unchecked(&mut *(work as *mut Self)) };
        this.as_action().act();
    }
}
