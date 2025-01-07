//! Zephyr work wrappers targeted for the `Future` type.
//!
//! The future is similar to our [`SimpleAction`], with a few additional features:
//! - The poll function returns an enum indicating that either it can be suspended, or that it
//!   is finished and has a result.
//! - The poll function takes a `Waker` which is used to "wake" the work item.
//!
//! However, there is a bit of a semantic mismatch between work queues and Futures.  Futures are
//! effectively built with the assumption that the the waking will happen, by Rust code, at the
//! time the event is ready.  However, work queues expect the work to be queued immediately,
//! with a "poll" indicating what kind of even the work.  Work will be scheduled either based on
//! one of these events, or a timeout.

extern crate alloc;

use alloc::boxed::Box;

use core::{
    cell::UnsafeCell,
    ffi::{c_int, c_void, CStr},
    future::Future,
    mem,
    pin::Pin,
    ptr::{self, NonNull},
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use arrayvec::ArrayVec;
use zephyr_sys::{
    k_poll_event, k_poll_event_init, k_poll_modes_K_POLL_MODE_NOTIFY_ONLY, k_work, k_work_poll,
    k_work_poll_init, k_work_poll_submit, k_work_poll_submit_to_queue, k_work_q,
    ZR_POLL_TYPE_DATA_AVAILABLE, ZR_POLL_TYPE_SEM_AVAILABLE, ZR_POLL_TYPE_SIGNAL,
};

use crate::{
    printkln,
    sync::{Arc, Mutex, Weak},
    sys::{queue::Queue, sync::Semaphore},
    time::{Duration, Forever, NoWait, Tick, Timeout},
};

use super::{get_current_workq, Signal, SubmitResult, WorkQueue};

/// An answer to a completed Future.
///
/// The are two times we need to wait on a future running to completion: the outer initial executor
/// invocation from the main thread, and running an async thread which will have a join method.
///
/// For both cases, we will use a Semaphore to indicate when the data is available.
///
/// The main issue is that this type is intended to be one shot.  Trying to load a second value will
/// invalidate the data structure (the item will be replaced, but there is a race with the
/// semaphore).
///
/// TODO: Currently, the data is stored inside of a Mutex. This isn't actually necessary (the
/// semaphore already manages the coordination), and only a memory barrier would be needed, which
/// would be provided by the semaphore. So, this should be changed to just unsafely share the data,
/// similar to how a mutex is implemented.
pub struct Answer<T> {
    item: Mutex<Option<T>>,
    wake: Semaphore,
}

impl<T> Answer<T> {
    /// Construct a new Answer that does not have the result.
    pub fn new() -> Self {
        Self {
            item: Mutex::new(None),
            wake: Semaphore::new(0, 1).expect("Initialize semaphore"),
        }
    }

    /// Place the item into the Answer.
    ///
    /// # Panic
    ///
    /// If the answer already contains an item, this will panic.
    ///
    /// # TODO
    ///
    /// We could check that the Answer has ever been used, not just that it has an answer in it.
    pub fn place(&self, item: T) {
        let mut inner = self.item.lock().expect("Get Mutex");
        if inner.is_some() {
            panic!("Answer already contains a value");
        }
        *inner = Some(item);
        self.wake.give();
    }

    /// Synchronously wait for an Answer.
    ///
    /// Blocks the current thread until an answer is available, returning it.
    pub fn take(&self) -> T {
        self.wake.take(Forever).expect("Forever returned early");
        self.item
            .lock()
            .expect("Get Mutex")
            .take()
            .expect("Answer should contain value")
    }

    /// Asynchronously wait for an answer.
    pub async fn take_async(&self) -> T {
        self.wake
            .take_async(Forever)
            .await
            .expect("Forever returnd early");
        self.item
            .lock()
            .expect("Get Mutex")
            .take()
            .expect("Answer should contain value")
    }
}

/// Build a combiner for Future and a Zephyr work queue.  This encapsulates the idea of starting
/// a new thread of work, and is the basis of both the main `run` for work queues, as well as
/// any calls to spawn that happen within the Future world.
pub struct WorkBuilder {
    queue: Option<NonNull<k_work_q>>,
    // A name for this task, used by debugging and such.
    name: Option<&'static CStr>,
}

impl WorkBuilder {
    /// Construct a new builder for work.
    ///
    /// The builder will default to running on the system workqueue.
    pub fn new() -> Self {
        Self {
            queue: None,
            name: None,
        }
    }

    /// Set the work queue for this worker to run on.
    ///
    /// By default, A Worker will run on the system work-queue.
    pub fn set_worker(&mut self, worker: &WorkQueue) -> &mut Self {
        self.queue = Some(NonNull::new(worker.item.get()).expect("work must not be null"));
        self
    }

    /// Set a name for this worker, for debugging.
    pub fn set_name(&mut self, name: &'static CStr) -> &mut Self {
        self.name = Some(name);
        self
    }

    /// Start this working, consuming the given Future to do the work.
    ///
    /// The work queue is in a pinned Arc to meet requirements of how Futures are used.  The Arc
    /// maintains lifetime while the worker is running.  See notes below for issues of lifetimes
    /// and canceled work.
    pub fn start<F: Future + Send>(&self, future: F) -> JoinHandle<F> {
        JoinHandle::new(self, future)
    }

    /// Start this work, locally running on the current worker thread.
    ///
    /// This is the same as `start`, but the work will always be started on the current work queue
    /// thread.  This relaxes the `Send` requirement, as the data will always be contained in a
    /// single thread.
    ///
    /// # Panics
    ///
    /// If called from other than a Future running on a work queue, will panic.  The System work
    /// queue is not yet supported.
    pub fn start_local<F: Future>(&self, future: F) -> JoinHandle<F> {
        JoinHandle::new_local(self, future)
    }
}

/// A potentially running Work.
///
/// This encapsulates a Future that is potentially running in the Zephyr work queue system.
///
/// # Safety
///
/// Once the worker has been started (meaning once WorkBuilder::start returns this `Work`), all
/// but one field here is owned by the worker itself (it runs on the worker thread, hence the
/// Send constraint).  The exception is the 'answer' field which can be used by the caller to
/// wait for the Work to finish.
pub struct JoinHandle<F: Future> {
    /// The answer will be placed here.  This Arc holds a strong reference, and if the spawning
    /// thread doesn't hold the `Work`, it will be dropped.
    answer: Arc<Answer<F::Output>>,
}

impl<F: Future + Send> JoinHandle<F> {
    /// Construct new [`JoinHandle`] that runs on a specified [`WorkQueue`].
    fn new(builder: &WorkBuilder, future: F) -> Self {
        // Answer holds the result when the work finishes.
        let answer = Arc::new(Answer::new());

        let work = WorkData::new(
            future,
            Arc::downgrade(&answer),
            builder.queue,
            builder.name,
        );
        WorkData::submit(work).expect("Unable to enqueue worker");

        Self { answer }
    }
}

impl<F: Future> JoinHandle<F> {
    /// Construct a new [`JoinHandle`] that runs on the current [`WorkQueue`].
    ///
    /// # Panics
    ///
    /// If `new_local` is called from a context other than running within a worker defined in this
    /// crate, it will panic.
    ///
    /// Note that currently, the system workq is not considered a worked defined in this crate.
    fn new_local(builder: &WorkBuilder, future: F) -> Self {
        let workq = get_current_workq().expect("Called new_local not from worker");
        let answer = Arc::new(Answer::new());

        let work = WorkData::new(
            future,
            Arc::downgrade(&answer),
            Some(NonNull::new(workq).unwrap()),
            builder.name,
        );
        WorkData::submit(work).expect("Unable to enqueue worker");

        Self { answer }
    }
}

impl<F: Future> JoinHandle<F> {
    /// Synchronously wait for this future to have an answer.
    pub fn join(&self) -> F::Output {
        self.answer.take()
    }

    /// Asynchronously wait for this future to have an answer.
    pub async fn join_async(&self) -> F::Output {
        self.answer.take_async().await
    }
}

/// Futures will need to be able to set the events and timeout of this waker.  Because the Waker is
/// parameterized, they will not have access to the whole WorkWaker, but only this WakeInfo.
pub struct WakeInfo {
    /// The work queue to submit this work to.  None indicates the system workq.
    pub(crate) queue: Option<NonNull<k_work_q>>,
    /// Events to use for our next wakeup.  Currently cleared before calling the future (although
    /// this discards the wakeup reason, so needs to be fixed).
    pub events: EventArray,
    /// Timeout to use for the next wakeup.  Will be set to Forever before calling the Future's
    /// poll.
    pub timeout: Timeout,
    /// A Context to use for invoking workers.  This `WakeInfo` can be recovered from this context.
    /// Note that our contexts are `'static` as they are maintained inside of the worker.
    pub context: Context<'static>,
}

impl WakeInfo {
    /// Recover the WakeInfo from a given context.
    ///
    /// # Safety
    ///
    /// Although the lifetime of Context is `'static`, the generic type passed to `Future` does not
    /// specify a lifetime. As such, it is not possible for the future to store the Context, and
    /// rescheduling must be specified before this Future invocation returns.
    ///
    /// This does assume we are only using the Zephyr scheduler.  The Context does have an any-based
    /// data pointer mechanism, but it is nightly.  This recovery would be easier using that
    /// mechanism.
    pub unsafe fn from_context<'b>(context: &'b mut Context) -> &'b mut Self {
        // SAFETY: We're doing pointer arithmetic to recover Self from a reference to the embedded
        // context.  The 'mut' is preserved to keep the rules of mut in Rust.
        unsafe {
            let this: *mut Context = context;
            let this = this
                .cast::<u8>()
                .sub(mem::offset_of!(Self, context))
                .cast::<Self>();
            &mut *this
        }
    }

    /// Add an event that represents waiting for a semaphore to be available for "take".
    pub fn add_semaphore<'a>(&'a mut self, sem: &'a Semaphore) {
        // SAFETY: Fill with zeroed memory, initializatuon happens in the init function next.
        self.events.push(unsafe { mem::zeroed() });
        let ev = self.events.last().unwrap();

        unsafe {
            k_poll_event_init(
                ev.get(),
                ZR_POLL_TYPE_SEM_AVAILABLE,
                k_poll_modes_K_POLL_MODE_NOTIFY_ONLY as i32,
                sem.item.get() as *mut c_void,
            );
        }
    }

    /// Add an event that represents waiting for a signal.
    pub fn add_signal<'a>(&'a mut self, signal: &'a Signal) {
        // SAFETY: Fill with zeroed memory, initializatuon happens in the init function next.
        self.events.push(unsafe { mem::zeroed() });
        let ev = self.events.last().unwrap();

        unsafe {
            k_poll_event_init(
                ev.get(),
                ZR_POLL_TYPE_SIGNAL,
                k_poll_modes_K_POLL_MODE_NOTIFY_ONLY as i32,
                signal.item.get() as *mut c_void,
            );
        }
    }

    /// Add an event that represents waiting for a queue to have a message.
    pub fn add_queue<'a>(&'a mut self, queue: &'a Queue) {
        // SAFETY: Fill with zeroed memory, initializatuon happens in the init function next.
        self.events.push(unsafe { mem::zeroed() });
        let ev = self.events.last().unwrap();

        unsafe {
            k_poll_event_init(
                ev.get(),
                ZR_POLL_TYPE_DATA_AVAILABLE,
                k_poll_modes_K_POLL_MODE_NOTIFY_ONLY as i32,
                queue.item.get() as *mut c_void,
            );
        }
    }
}

/// The worker-owned information about that worker.
///
/// This holds a single worker, and will be owned by that worker itself.
struct WorkData<F: Future> {
    /// Info needed to reschedule the work.
    info: WakeInfo,
    /// The Zephyr worker.  This struct is allocated in a Box, and only used by the worker thread,
    /// so it is easy to recover.  The UnsafeCell is to indicate that Zephyr is free to mutate the
    /// work.
    work: UnsafeCell<k_work_poll>,
    /// Where the answer is placed.  This is weak because the spawning thread may not be interested
    /// in the result, which will drop the only reference to the Arc, breaking the weak reference.
    answer: Weak<Answer<F::Output>>,
    /// The future that is running this work.
    future: F,
}

// SAFETY: The worker struct is explicitly safe to send by the Zephyr docs.
// unsafe impl<F: Future + Send> Send for WorkData<F> {}

impl<F: Future> WorkData<F> {
    /// Build a new WorkWaker around the given future.  The weak reference to the answer is where
    /// the answer is stored if the task spawner is still interested in the answer.
    fn new(
        future: F,
        answer: Weak<Answer<F::Output>>,
        queue: Option<NonNull<k_work_q>>,
        name: Option<&'static CStr>,
    ) -> Pin<Box<Self>> {
        // name is only used for SystemView debugging, so prevent a warning when that is not
        // enabled.
        let _ = name;

        let this = Box::pin(Self {
            // SAFETY: This will be initialized below, once the Box allocates and the memory won't
            // move.
            work: unsafe { mem::zeroed() },
            future,
            answer,
            info: WakeInfo {
                queue,
                events: EventArray::new(),
                // Initial timeout is NoWait so work starts as soon as submitted.
                timeout: NoWait.into(),
                context: Context::from_waker(&VOID_WAKER),
            },
        });

        unsafe {
            // SAFETY: The above Arc allocates the worker.  The code here is careful to not move it.
            k_work_poll_init(this.work.get(), Some(Self::handler));

            // If we have a name, send it to Segger.
            #[cfg(CONFIG_SEGGER_SYSTEMVIEW)]
            {
                let ww = &(&*this.work.get()).work;
                if let Some(name) = name {
                    let info = crate::raw::SEGGER_SYSVIEW_TASKINFO {
                        TaskID: this.work.get() as ::core::ffi::c_ulong,
                        sName: name.as_ptr(),
                        Prio: 1,
                        StackBase: 0,
                        StackSize: 32,
                    };
                    crate::raw::SEGGER_SYSVIEW_OnTaskCreate(this.work.get() as ::core::ffi::c_ulong);
                    crate::raw::SEGGER_SYSVIEW_SendTaskInfo(&info);
                }
            }
        }

        this
    }

    /// Submit this work to the Zephyr work queue.  This consumes the Box, with the primary owner
    /// being the work thread itself.  Not that canceling work will leak the worker.
    fn submit(mut this: Pin<Box<Self>>) -> crate::Result<SubmitResult> {
        // SAFETY: This is unsafe because the pointer lose the Pin guarantee, but C code will not
        // move it.
        let this_ref = unsafe {
            Pin::get_unchecked_mut(this.as_mut())
        };

        let result = if let Some(queue) = this_ref.info.queue {
            unsafe {
                // SAFETY: We're transferring ownership of the box to the enqueued work.  For
                // regular re-submission as the worker runs, the worker won't be run until this
                // method exits.  For initial creation, there is a possible period where our
                // reference here survives while the worker is schedule (when the work queue is
                // higher priority than this.  I'm not sure if this fully followes the rules, as
                // there is still a reference to this here, but as long as we only use it to leak
                // the box, I believe we are safe.  If this is deemed unsafe, these values could be
                // copied to variables and the box leaked before we enqueue.
                k_work_poll_submit_to_queue(
                    queue.as_ptr(),
                    this_ref.work.get(),
                    this_ref.info.events.as_mut_ptr() as *mut k_poll_event,
                    this.info.events.len() as c_int,
                    this.info.timeout.0,
                )
            }
        } else {
            unsafe {
                // SAFETY: See above, safety here is the same.
                k_work_poll_submit(
                    this_ref.work.get(),
                    this_ref.info.events.as_mut_ptr() as *mut k_poll_event,
                    this_ref.info.events.len() as c_int,
                    this_ref.info.timeout.0,
                )
            }
        };

        // The Box has been handed to C.  Consume the box, leaking the value.  We use `into_raw` as
        // it is the raw pointer we will be recovering the Box with when the worker runs.
        let _ = Self::into_raw(this);

        match result {
            0 => Ok(SubmitResult::Enqueued),
            code => panic!("Unexpected result from work poll submit: {}", code),
        }
    }

    /// The work callback, coming from the Zephyr C world.  The box was into_raw(), We recover the
    /// WorkWaker by using container_of and recovering it back into a box, which we will leak when
    /// we re-submit it.
    extern "C" fn handler(work: *mut k_work) {
        // Note that we want to avoid needing a `repr(C)` on our struct, so the k_work pointer is
        // not necessarily at the beginning of the struct.
        let mut this = unsafe { Self::from_raw(work) };

        let this_ref = unsafe {
            Pin::get_unchecked_mut(this.as_mut())
        };

        // Set the next work to Forever, with no events.  TODO: This prevents the next poll from
        // being able to determine the reason for the wakeup.
        this_ref.info.events.clear();
        this_ref.info.timeout = Forever.into();

        // SAFETY: poll requires the pointer to be pinned, in case that is needed.  We rely on the
        // Boxing of the pointer, and that our code does not move the future.
        let future = unsafe { Pin::new_unchecked(&mut this_ref.future) };
        #[cfg(CONFIG_SEGGER_SYSTEMVIEW)]
        unsafe {
            crate::raw::SEGGER_SYSVIEW_OnTaskStartExec(work as u32);
        }
        match future.poll(&mut this_ref.info.context) {
            Poll::Pending => {
                #[cfg(CONFIG_SEGGER_SYSTEMVIEW)]
                unsafe {
                    crate::raw::SEGGER_SYSVIEW_OnTaskStopExec();
                }
                // With pending, use the timeout and events to schedule ourselves to do more work.
                // TODO: If we want to support a real Waker, this would need to detect that, and
                // schedule a possible wake on this no wake case.
                // Currently, this check is only testing that something is missed, and is really
                // more of a debug assertion.
                if this.info.events.is_empty() && this.info.timeout == Forever.into() {
                    printkln!("Warning: worker scheduled to never wake up");
                }

                // The re-submission will give ownership of the box back to the scheduled work.
                Self::submit(this).expect("Unable to schedule work");
            }
            Poll::Ready(answer) => {
                #[cfg(CONFIG_SEGGER_SYSTEMVIEW)]
                unsafe {
                    crate::raw::SEGGER_SYSVIEW_OnTaskStopExec();
                }
                // TODO: Delete the task as well.
                // If the spawning task is still interested in the answer, provide it.
                if let Some(store) = this.answer.upgrade() {
                    store.place(answer);
                }

                // Work is finished, so allow the Box to be dropped.
            }
        }
    }

    /// Consume the pinned box containing Self, and return the internal pointer.
    fn into_raw(this: Pin<Box<Self>>) -> *mut Self {
        // SAFETY: This removes the Pin guarantee, but is given as a raw pointer to C, which doesn't
        // generally use move.
        let this = unsafe { Pin::into_inner_unchecked(this) };
        Box::into_raw(this)
    }

    /// Given a pointer to the work_q burried within, recover the Pinned Box containing our data.
    unsafe fn from_raw(ptr: *mut k_work) -> Pin<Box<Self>> {
        // SAFETY: This fixes the pointer back to the beginning of Self.  This also assumes the
        // pointer is valid.
        let ptr = ptr
            .cast::<u8>()
            .sub(mem::offset_of!(k_work_poll, work))
            .sub(mem::offset_of!(Self, work))
            .cast::<Self>();
        let this = Box::from_raw(ptr);
        Pin::new_unchecked(this)
    }
}

/// A VoidWaker is used when we don't use the Waker mechanism.  There is no data associated with
/// this waker, and it panics if anyone tries to clone it or use it to wake a task.
/// This is static to simplify lifetimes.
static VOID_WAKER: Waker = unsafe {
    Waker::from_raw(RawWaker::new(
        ptr::null(),
        &RawWakerVTable::new(void_clone, void_wake, void_wake_by_ref, void_drop),
    ))
};

/// Void clone operation.  Panics for now.  If we want to implement a real waker, this will need
/// to be managed.
unsafe fn void_clone(_: *const ()) -> RawWaker {
    panic!("Zephyr Wakers not yet supported for general 'Waker' use");
}

/// Void wake operation.  Panics for now.  If we want to implement a real waker, this will need
/// to be managed.
unsafe fn void_wake(_: *const ()) {
    panic!("Zephyr Wakers not yet supported for general 'Waker' use");
}

/// Void wake_by_ref operation.  Panics for now.  If we want to implement a real waker, this will need
/// to be managed.
unsafe fn void_wake_by_ref(_: *const ()) {
    panic!("Zephyr Wakers not yet supported for general 'Waker' use");
}

/// The void drop will be called when the Context is dropped after the first invocation.  Because
/// clone above panics, we know there aren't references hanging around.  So, it is safe to just
/// do nothing.
unsafe fn void_drop(_: *const ()) {}
/// To avoid having to parameterize everything, we limit the size of the ArrayVec of events to
/// this amount.  The amount needed her depends on overall use, but so far, 1 is sufficient.
type EventArray = ArrayVec<UnsafeCell<k_poll_event>, 1>;

/// Async sleep.
pub fn sleep(duration: Duration) -> Sleep {
    Sleep {
        ticks_left: duration.ticks(),
    }
}

/// A future that sleeps for a while.
pub struct Sleep {
    // How much time is left.  TODO: Change this into an absolute sleep once we have the ability to
    // determine why were were scheduled.
    ticks_left: Tick,
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // If the sleep is done, so are we.
        if self.ticks_left == 0 {
            return Poll::Ready(());
        }

        // Otherwise, queue outselves back.
        let this = unsafe { WakeInfo::from_context(cx) };

        this.timeout = Duration::from_ticks(self.ticks_left).into();
        self.ticks_left = 0;

        Poll::Pending
    }
}
