//! Async IO for Zephyr
//!
//! This implements the basics of using Zephyr's work queues to implement async code on Zephyr.
//!
//! Most of the work happens in [`work`] and in [`futures`]
//!
//! [`work`]: crate::work
//! [`futures`]: crate::work::futures

use core::ffi::CStr;
use core::task::Poll;
use core::{future::Future, pin::Pin};

use crate::time::NoWait;
use crate::work::futures::WakeInfo;
use crate::work::{futures::JoinHandle, futures::WorkBuilder, WorkQueue};

pub mod sync;

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
