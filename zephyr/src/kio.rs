//! Async IO for Zephyr
//!
//! This implements the basics of using Zephyr's work queues to implement async code on Zephyr.
//!
//! Most of the work happens in [`work`] and in [`futures`]
//!
//! [`work`]: crate::work
//! [`futures`]: crate::work::futures

use core::ffi::CStr;
use core::task::{Context, Poll};
use core::{future::Future, pin::Pin};

use crate::sys::queue::Queue;
use crate::sys::sync::Semaphore;
use crate::time::{NoWait, Timeout};
use crate::work::futures::WakeInfo;
use crate::work::Signal;
use crate::work::{futures::JoinHandle, futures::WorkBuilder, WorkQueue};

pub mod sync;

pub use crate::work::futures::sleep;

/// Run an async future on the given worker thread.
///
/// Arrange to have the given future run on the given worker thread.  The resulting `JoinHandle` has
/// `join` and `join_async` methods that can be used to wait for the given thread.
pub fn spawn<F>(future: F, worker: &WorkQueue, name: &'static CStr) -> JoinHandle<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    WorkBuilder::new()
        .set_worker(worker)
        .set_name(name)
        .start(future)
}

/// Run an async future on the current worker thread.
///
/// Arrange to have the given future run on the current worker thread.  The resulting `JoinHandle`
/// has `join` and `join_async` methods that can be used to wait for the given thread.
///
/// The main use for this is to allow work threads to use `Rc` and `Rc<RefCell<T>>` within async
/// tasks.  The main constraint is that references inside cannot be held across an `.await`.
///
/// # Panics
/// If this is called other than from a worker task running on a work thread, it will panic.
pub fn spawn_local<F>(future: F, name: &'static CStr) -> JoinHandle<F>
where
    F: Future + 'static,
    F::Output: Send + 'static,
{
    WorkBuilder::new()
        .set_name(name)
        .start_local(future)
}

/// Yield the current thread, returning it to the work queue to be run after other work on that
/// queue.  (This has to be called `yield_now` in Rust, because `yield` is a keyword.
pub fn yield_now() -> impl Future<Output = ()> {
    YieldNow { waited: false }
}

struct YieldNow {
    waited: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        if self.waited {
            Poll::Ready(())
        } else {
            // Enqueue outselves with no wait and no events.
            let info = unsafe { WakeInfo::from_context(cx) };

            // Unsafely check if the work queue running us is empty.  We only check explicitly
            // specified workers (TODO access the system work queue).  The check is racy, but should
            // always fail indicating that the queue is not empty when it could be.  Checking this
            // avoids re-scheduling the only worker back into the queue.
            // SAFETY: The check is racy, but will fail with us yielding when we didn't need to.
            if let Some(wq) = info.queue {
                let wq = unsafe { wq.as_ref() };
                if wq.pending.head == wq.pending.tail {
                    return Poll::Ready(());
                }
            }

            info.timeout = NoWait.into();
            self.waited = true;

            Poll::Pending
        }
    }
}

/// Extensions on [`Context`] to support scheduling via Zephyr's workqueue system.
///
/// All of these are called from within the context of running work, and indicate what _next_
/// should cause this work to be run again.  If none of these methods are called before the work
/// exits, the work will be scheduled to run after `Forever`, which is not useful.  There may be
/// later support for having a `Waker` that can schedule work from another context.
///
/// Note that the events to wait on, such as Semaphores or channels, if there are multiple threads
/// that can wait for them, might cause this worker to run, but not actually be available.  As such,
/// to maintain the non-blocking requirements of Work, [`Semaphore::take`], and the blocking `send`
/// and `recv` operations on channels should not be used, even after being woken.
///
/// For the timeout [`Forever`] is useful to indicate there is no timeout.  If called with
/// [`NoWait`], the work will be immediately scheduled. In general, it is better to query the
/// underlying object directly rather than have the overhead of being rescheduled.
///
/// # Safety
///
/// The lifetime bounds on the items waited for ensure that these items live at least as long as the
/// work queue.  Practically, this can only be satisfied by using something with 'static' lifetime,
/// or embedding the value in the Future itself.
///
/// With the Zephyr executor, the `Context` is embedded within a `WakeInfo` struct, which this makes
/// use of.  If a different executor were to be used, these calls would result in undefined
/// behavior.
///
/// This could be checked at runtime, but it would have runtime cost.
///
/// [`Forever`]: crate::time::Forever
pub trait ContextExt {
    /// Indicate the work should next be scheduled based on a semaphore being available for "take".
    ///
    /// The work will be scheduled either when the given semaphore becomes available to 'take', or
    /// after the timeout.
    fn add_semaphore<'a>(&'a mut self, sem: &'a Semaphore, timeout: impl Into<Timeout>);

    /// Indicate that the work should be scheduled after receiving the given [`Signal`], or the
    /// timeout occurs.
    fn add_signal<'a>(&'a mut self, signal: &'a Signal, timeout: impl Into<Timeout>);

    /// Indicate that the work should be scheduled when the given [`Queue`] has data available to
    /// recv, or the timeout occurs.
    fn add_queue<'a>(&'a mut self, queue: &'a Queue, timeout: impl Into<Timeout>);

    /// Indicate that the work should just be scheduled after the given timeout.
    ///
    /// Note that this only works if none of the other wake methods are called, as those also set
    /// the timeout.
    fn add_timeout(&mut self, timeout: impl Into<Timeout>);
}

/// Implementation of ContextExt for the Rust [`Context`] type.
impl<'b> ContextExt for Context<'b> {
    fn add_semaphore<'a>(&'a mut self, sem: &'a Semaphore, timeout: impl Into<Timeout>) {
        let info = unsafe { WakeInfo::from_context(self) };
        info.add_semaphore(sem);
        info.timeout = timeout.into();
    }

    fn add_signal<'a>(&'a mut self, signal: &'a Signal, timeout: impl Into<Timeout>) {
        let info = unsafe { WakeInfo::from_context(self) };
        info.add_signal(signal);
        info.timeout = timeout.into();
    }

    fn add_queue<'a>(&'a mut self, queue: &'a Queue, timeout: impl Into<Timeout>) {
        let info = unsafe { WakeInfo::from_context(self) };
        info.add_queue(queue);
        info.timeout = timeout.into();
    }

    fn add_timeout(&mut self, timeout: impl Into<Timeout>) {
        let info = unsafe { WakeInfo::from_context(self) };
        info.timeout = timeout.into();
    }
}
