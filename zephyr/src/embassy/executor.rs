//! An embassy executor tailored for Zephyr

use core::marker::PhantomData;

use crate::sys::sync::Semaphore;
use crate::time::Forever;
use embassy_executor::{raw, Spawner};

/// Zephyr-thread based executor.
pub struct Executor {
    inner: Option<raw::Executor>,
    poll_needed: Semaphore,
    not_send: PhantomData<*mut ()>,
}

impl Executor {
    /// Create a new Executor.
    pub fn new() -> Self {
        Self {
            inner: None,
            poll_needed: Semaphore::new(0, 1).unwrap(),
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
            let _ = self.poll_needed.take(Forever);
            unsafe {
                // The raw executor's poll only runs things that were queued _before_ this poll
                // itself is actually run. This means, specifically, that if the polled execution
                // causes this, or other threads to enqueue, this will return without running them.
                // `__pender` _will_ be called, so the next time around the semaphore will be taken.
                inner.poll();
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
        let this = context as *const Executor;
        (*this).poll_needed.give();
    }
}
