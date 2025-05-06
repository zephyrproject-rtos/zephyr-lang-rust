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
//! At this point, this code supports the simple work queues, with [`Work`] items.
//!
//! Work Queues should be declared with the `define_work_queue!` macro, this macro requires the name
//! of the symbol for the work queue, the stack size, and then zero or more optional arguments,
//! defined by the fields in the [`WorkQueueDeclArgs`] struct.  For example:
//!
//! ```rust
//! define_work_queue!(MY_WORKQ, 2048, no_yield = true, priority = 2);
//! ```
//!
//! Then, in code, the work queue can be started, and used to issue work.
//! ```rust
//! let my_workq = MY_WORKQ.start();
//! let action = Work::new(action_item);
//! action.submit(my_workq);
//! ```

extern crate alloc;

use core::{
    cell::{RefCell, UnsafeCell},
    ffi::{c_char, c_int, c_uint},
    mem,
    sync::atomic::Ordering,
};

use critical_section::Mutex;
use portable_atomic::AtomicBool;
use portable_atomic_util::Arc;
use zephyr_sys::{
    k_poll_signal, k_poll_signal_check, k_poll_signal_init, k_poll_signal_raise,
    k_poll_signal_reset, k_work, k_work_init, k_work_q, k_work_queue_config,
    k_work_queue_init, k_work_queue_start, k_work_submit, k_work_submit_to_queue,
    z_thread_stack_element,
};

use crate::{
    error::to_result_void,
    object::{Fixed, ObjectInit, ZephyrObject},
    simpletls::SimpleTls,
};

/// The WorkQueue decl args as a struct, so we can have a default, and the macro can fill in those
/// specified by the user.
pub struct WorkQueueDeclArgs {
    /// Should this work queue call yield after each queued item.
    pub no_yield: bool,
    /// Is this work queue thread "essential".
    ///
    /// Threads marked essential will panic if they stop running.
    pub essential: bool,
    /// Zephyr thread priority for the work queue thread.
    pub priority: c_int,
}

impl WorkQueueDeclArgs {
    /// Like `Default::default`, but const.
    pub const fn default_values() -> Self {
        Self {
            no_yield: false,
            essential: false,
            priority: 0,
        }
    }
}

/// A static declaration of a work-queue.  This associates a work queue, with a stack, and an atomic
/// to determine if it has been initialized.
// TODO: Remove the pub on the fields, and make a constructor.
pub struct WorkQueueDecl<const SIZE: usize> {
    queue: WorkQueue,
    stack: &'static crate::thread::ThreadStack<SIZE>,
    config: k_work_queue_config,
    priority: c_int,
    started: AtomicBool,
}

// SAFETY: Sync is needed here to make a static declaration, despite the `*const i8` that is burried
// in the config.
unsafe impl<const SIZE: usize> Sync for WorkQueueDecl<SIZE> {}

impl<const SIZE: usize> WorkQueueDecl<SIZE> {
    /// Static constructor.  Mostly for use by the macro.
    pub const fn new(
        stack: &'static crate::thread::ThreadStack<SIZE>,
        name: *const c_char,
        args: WorkQueueDeclArgs,
    ) -> Self {
        Self {
            queue: unsafe { mem::zeroed() },
            stack,
            config: k_work_queue_config {
                name,
                no_yield: args.no_yield,
                essential: args.essential,
            },
            priority: args.priority,
            started: AtomicBool::new(false),
        }
    }

    /// Start the work queue thread, if needed, and return a reference to it.
    pub fn start(&'static self) -> &'static WorkQueue {
        critical_section::with(|cs| {
            if self.started.load(Ordering::Relaxed) {
                // Already started, just return it.
                return &self.queue;
            }

            // SAFETY: Starting is coordinated by the atomic, as well as being protected in a
            // critical section.
            unsafe {
                let this = &mut *self.queue.item.get();

                k_work_queue_init(self.queue.item.get());

                // Add to the WORK_QUEUES data.  That needs to be changed to a critical
                // section Mutex from a Zephyr Mutex, as that would deadlock if called while in a
                // critrical section.
                let mut tls = WORK_QUEUES.borrow_ref_mut(cs);
                tls.insert(&this.thread, WorkQueueRef(self.queue.item.get()));

                // Start the work queue thread.
                k_work_queue_start(
                    self.queue.item.get(),
                    self.stack.data.get() as *mut z_thread_stack_element,
                    self.stack.size(),
                    self.priority,
                    &self.config,
                );
            }

            &self.queue
        })
    }
}

/// A running work queue thread.
///
/// This must be declared statically, and initialized once.  Please see the macro
/// [`define_work_queue`] which declares this with a [`WorkQueue`] to help with the
/// association with a stack, and making sure the queue is only started once.
///
/// [`define_work_queue`]: crate::define_work_queue
pub struct WorkQueue {
    #[allow(dead_code)]
    item: UnsafeCell<k_work_q>,
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
static WORK_QUEUES: Mutex<RefCell<SimpleTls<WorkQueueRef>>> =
    Mutex::new(RefCell::new(SimpleTls::new()));

/// For the queue mapping, we need a simple wrapper around the underlying pointer, one that doesn't
/// implement stop.
#[derive(Copy, Clone)]
struct WorkQueueRef(*mut k_work_q);

// SAFETY: The work queue reference is also safe for both Send and Sync per Zephyr semantics.
unsafe impl Send for WorkQueueRef {}
unsafe impl Sync for WorkQueueRef {}

/// Retrieve the current work queue, if we are running within one.
pub fn get_current_workq() -> Option<*mut k_work_q> {
    critical_section::with(|cs| WORK_QUEUES.borrow_ref(cs).get().map(|wq| wq.0))
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
    fn act(self: &Self);
}

/// A basic Zephyr work item.
///
/// Holds a `k_work`, along with the data associated with that work.  When the work is queued, the
/// `act` method will be called on the provided `SimpleAction`.
pub struct Work<T> {
    work: ZephyrObject<k_work>,
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

/// Arc held work.
///
/// Because C code takes ownership of the work, we only support submitting work with very specific
/// pointer types.  This wraps an Arc holding work to allow Work held in an Arc to be queued.
/// Earlier versions of this required the work to be `Pin<Arc<..>>`, however, we use
/// [`ZephyrObject`] to hold work items, anyway, and this already does a runtime check to prevents
/// moves, so we can safely avoid needing to use `Pin`.  However, note that if the work is moved
/// between it's first use, and subsequent use, it will panic.
pub struct ArcWork<T: SimpleAction + Send>(pub Arc<Work<T>>);

/// Clone just passes the clone to the arc.
impl<T: SimpleAction + Send> Clone for ArcWork<T> {
    fn clone(&self) -> Self {
        ArcWork(self.0.clone())
    }
}

impl<T: SimpleAction + Send> ArcWork<T> {
    /// Submit this work to the system work queue.
    ///
    /// This can return several possible `Ok` results.  See the docs on [`SubmitResult`] for an
    /// explanation of them.
    pub fn submit(self) -> crate::Result<SubmitResult> {
        // Leak the arc, so that when the handler runs, it can be safely turned back into an Arc,
        // and then the drop on the Arc will run.
        // SAFETY: As we are leaking the pointer until the C code is done with it, it is safe to get
        // the pointer to the raw work.
        let work = unsafe { self.0.work.get() };
        let _ = Arc::into_raw(self.0);

        let result = SubmitResult::to_result(unsafe { k_work_submit(work) });

        Self::check_drop(work, &result);

        result
    }

    /// Submit this work to the given work queue.
    ///
    /// This can return several possible `Ok` results.  See the docs on [`SubmitResult`] for an
    /// explanation of them.
    pub fn submit_to_queue(self, queue: &'static WorkQueue) -> crate::Result<SubmitResult> {
        // Leak the arc, so that when the handler runs, it can be safely turned back into an Arc,
        // and then the drop on the Arc will run.
        // SAFETY: As we are leaking the pointer until the C code is done with it, it is safe to get
        // the pointer to the raw work.
        let work = unsafe { self.0.work.get() };
        let _ = Arc::into_raw(self.0);

        let result = SubmitResult::to_result(unsafe { k_work_submit_to_queue(queue.item.get(), work) });

        Self::check_drop(work, &result);

        result
    }

    /// Given the raw "C" work pointer, get a pointer back to our work item.
    unsafe fn from_raw(ptr: *const k_work) -> Self {
        let ptr = ptr
            .cast::<u8>()
            .sub(mem::offset_of!(Work<T>, work))
            .cast::<Work<T>>();
        let this = Arc::from_raw(ptr);
        Self(this)
    }

    /// Check if the C code has "dropped" it's reference, and drop our Arc reference as well.  This
    /// should detect the case where the work was not queued, and no callback, of this ownership,
    /// will be called.
    fn check_drop(work: *const k_work, result: &crate::Result<SubmitResult>) {
        if matches!(result, Ok(SubmitResult::AlreadySubmitted) | Err(_)) {
            // SAFETY: If the above matches, it indicates this work was already running, and someone
            // other than the work itself is trying to submit it.  In this case, there will be no
            // callback that belongs to this particular context.  Err also indicates that the work
            // was not enqueued.
            unsafe {
                let this = Self::from_raw(work);
                drop(this);
            }
        }
    }

    /// The handler for Arc based work.
    extern "C" fn handler(work: *mut k_work) {
        // Reconstruct self out of the work.
        // SAFETY: The submit functions will leak the arc any time C has ownership of the Work, and
        // the C will relinquish that ownership when calling this handler.
        let this = unsafe { Self::from_raw(work) };

        let action = &this.0.action;

        action.act();

        // This will be dropped.
    }
}

/// Static Work.
///
/// Work items can also be declared statically.  Note that the work should only be submitted after
/// it has been moved to it's final static location.
pub struct StaticWork<T: SimpleAction + Send + 'static>(pub &'static Work<T>);

impl<T: SimpleAction + Send + 'static> StaticWork<T> {
    /// Submit this work to the system work queue.
    pub fn submit(self) -> crate::Result<SubmitResult> {
        SubmitResult::to_result(unsafe { k_work_submit(self.0.work.get()) })
    }

    /// Submit this work to the a specific work queue.
    pub fn submit_to_queue(self, queue: &'static WorkQueue) -> crate::Result<SubmitResult> {
        SubmitResult::to_result(unsafe { k_work_submit_to_queue(queue.item.get(), self.0.work.get()) })
    }

    /// The handler for static work.
    extern "C" fn handler(work: *mut k_work) {
        let ptr = unsafe {
            work
                .cast::<u8>()
                .sub(mem::offset_of!(Work<T>, work))
                .cast::<Work<T>>()
        };
        let this = unsafe { &*ptr };
        let action = &this.action;
        action.act();
    }
}

impl<T: SimpleAction + Send> Work<T> {
    /// Construct a new Work from the given action.
    ///
    /// Note that the data will be moved into the pinned Work.  The data is internal, and only
    /// accessible to the work thread (the `act` method).  If shared data is needed, normal
    /// inter-thread sharing mechanisms are needed.
    ///
    /// TODO: Can we come up with a way to allow sharing on the same worker using Rc instead of Arc?
    pub fn new_arc(action: T) -> ArcWork<T> {
        let work = <ZephyrObject<k_work>>::new_raw();

        // SAFETY: Initializes the above zero-initialized struct.  Initialization once is handled by
        // ZephyrObject.
        unsafe {
            let addr = work.get_uninit();
            (*addr).handler = Some(ArcWork::<T>::handler);
        }

        let this = Arc::new(Work { work, action });

        ArcWork(this)
    }
}

impl<T: SimpleAction + Send + 'static> Work<T> {
    /// Construct a static worker.
    pub const fn new_static(action: T) -> Work<T> {
        let work = <ZephyrObject<k_work>>::new_raw();

        unsafe { 
            let addr = work.get_uninit();
            (*addr).handler = Some(StaticWork::<T>::handler);
        }

        Work { work, action }
    }

    /// Access the inner action.
    pub fn action(&self) -> &T {
        &self.action
    }
}

/// Capture the kinds of pointers that are safe to submit to work queues.
pub trait SubmittablePointer<T> {
    /// Submit this work to the system work queue.
    fn submit(self) -> crate::Result<SubmitResult>;

    /// Submit this work to the given work queue.
    fn submit_to_queue(self, queue: &'static WorkQueue) -> crate::Result<SubmitResult>;
}

impl<T: SimpleAction + Send> SubmittablePointer<T> for Arc<Work<T>> {
    fn submit(self) -> crate::Result<SubmitResult> {
        ArcWork(self).submit()
    }

    fn submit_to_queue(self, queue: &'static WorkQueue) -> crate::Result<SubmitResult> {
        ArcWork(self).submit_to_queue(queue)
    }
}

impl<T: SimpleAction + Send + 'static> SubmittablePointer<T> for &'static Work<T> {
    fn submit(self) -> crate::Result<SubmitResult> {
        StaticWork(self).submit()
    }

    fn submit_to_queue(self, queue: &'static WorkQueue) -> crate::Result<SubmitResult> {
        StaticWork(self).submit_to_queue(queue)
    }
}

impl ObjectInit<k_work> for ZephyrObject<k_work> {
    fn init(item: *mut k_work) {
        // SAFETY: The handler was stashed in this field when constructing.  At this point, the item
        // will be pinned.
        unsafe {
            let handler = (*item).handler;
            k_work_init(item, handler);
        }
    }
}

/// Declare a static work queue.
///
/// This declares a static work queue (of type [`WorkQueueDecl`]).  This will have a single method
/// `.start()` which can be used to start the work queue, as well as return the persistent handle
/// that can be used to enqueue to it.
#[macro_export]
macro_rules! define_work_queue {
    ($name:ident, $stack_size:expr) => {
        $crate::define_work_queue!($name, $stack_size,);
    };
    ($name:ident, $stack_size:expr, $($key:ident = $value:expr),* $(,)?) => {
        static $name: $crate::work::WorkQueueDecl<$stack_size> = {
            #[link_section = concat!(".noinit.workq.", stringify!($name))]
            static _ZEPHYR_STACK: $crate::thread::ThreadStack<$stack_size> =
                $crate::thread::ThreadStack::new();
            const _ZEPHYR_C_NAME: &[u8] = concat!(stringify!($name), "\0").as_bytes();
            const _ZEPHYR_ARGS: $crate::work::WorkQueueDeclArgs = $crate::work::WorkQueueDeclArgs {
                $($key: $value,)*
                ..$crate::work::WorkQueueDeclArgs::default_values()
            };
            $crate::work::WorkQueueDecl::new(
                &_ZEPHYR_STACK,
                _ZEPHYR_C_NAME.as_ptr() as *const ::core::ffi::c_char,
                _ZEPHYR_ARGS,
            )
        };
    };
}
