//! An embassy executor tailored for Zephyr

use core::{marker::PhantomData, sync::atomic::Ordering};

use embassy_executor::{raw, Spawner};
use zephyr_sys::{k_current_get, k_thread_resume, k_thread_suspend, k_tid_t};

use crate::sync::atomic::AtomicBool;

/// Zephyr-thread based executor.
pub struct Executor {
    inner: Option<raw::Executor>,
    id: k_tid_t,
    pend: AtomicBool,
    not_send: PhantomData<*mut ()>,
}

impl Executor {
    /// Create a new Executor.
    pub fn new() -> Self {
        let id = unsafe { k_current_get() };

        Self {
            inner: None,
            pend: AtomicBool::new(false),
            id,
            not_send: PhantomData,
        }
    }

    /// Run the executor.
    pub fn run(&'static mut self, init: impl FnOnce(Spawner)) -> ! {
        let context = self as *mut _ as *mut ();
        self.inner.replace(raw::Executor::new(context));
        let inner = self.inner.as_mut().unwrap();
        init(inner.spawner());

        loop {
            unsafe {
                // The raw executor's poll only runs things that were queued _before_ this poll
                // itself is actually run. This means, specifically, that if the polled execution
                // causes this, or other threads to enqueue, this will return without running them.
                // `__pender` _will_ be called, but it isn't "sticky" like `wfe/sev` are.  To
                // simulate this, we will use the 'pend' atomic to count
                inner.poll();
                if !self.pend.swap(false, Ordering::SeqCst) {
                    k_thread_suspend(k_current_get());
                }
            }
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

#[export_name = "__pender"]
fn __pender(context: *mut ()) {
    unsafe {
        let myself = k_current_get();

        let this = context as *const Executor;
        let other = (*this).id;

        // The atomic is our equivalent to causing an event (wfe) or pending an IRQ.  We need to do
        // this before waking the other thread in case that thread runs early.  This is needed for
        // both the case of another thread, to prevent a race between the `inner.poll()` above, and
        // new items being added to the queue, as well as running entirely locally, also to prevent
        // the same race.
        (*this).pend.store(true, Ordering::SeqCst);

        // If the other is a different thread, resume it.
        if other != myself {
            k_thread_resume(other);
        }
    }
}
