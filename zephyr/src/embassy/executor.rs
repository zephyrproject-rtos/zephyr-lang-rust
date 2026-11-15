//! An embassy executor tailored for Zephyr

use core::marker::PhantomData;

use crate::sys::sync::Semaphore;
#[cfg(not(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC)))]
use crate::time::Forever;
use embassy_executor::{raw, Spawner};

/// Zephyr-thread based executor.
pub struct Executor {
    inner: Option<raw::Executor>,
    poll_needed: Semaphore,
    not_send: PhantomData<*mut ()>,
    #[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
    waiters: super::semaphore::Registry,
}

impl Executor {
    /// Create a new Executor.
    pub fn new() -> Self {
        Self {
            inner: None,
            poll_needed: Semaphore::new(0, 1),
            not_send: PhantomData,
            #[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
            waiters: super::semaphore::Registry::new(),
        }
    }

    /// Run the executor.
    pub fn run(&'static mut self, init: impl FnOnce(Spawner)) -> ! {
        let context = self as *mut _ as *mut ();
        self.inner.replace(raw::Executor::new(context));
        #[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
        super::semaphore::Context {
            registry: &self.waiters,
            wake: &self.poll_needed,
        }
        .install();
        let inner = self.inner.as_mut().unwrap();
        init(inner.spawner());

        loop {
            #[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
            wait_for_events(&self.poll_needed, &self.waiters);
            #[cfg(not(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC)))]
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

#[cfg(all(feature = "async-semaphores", CONFIG_POLL, CONFIG_RUST_ALLOC))]
fn wait_for_events(wake: &Semaphore, registry: &super::semaphore::Registry) {
    extern crate alloc;
    use crate::{raw, time::NoWait};
    use alloc::vec::Vec;

    let entries = registry.snapshot();
    if entries.is_empty() {
        let _ = wake.take(crate::time::Forever);
        return;
    }
    // Allocate the complete event array before initializing it. Zephyr may
    // link events internally during k_poll; it must not move until that call
    // has returned. The snapshot owns every waker and all semaphores are static.
    let mut events: Vec<raw::k_poll_event> = core::iter::repeat_with(Default::default)
        .take(entries.len() + 1)
        .collect();
    for (index, event) in events.iter_mut().enumerate() {
        let semaphore = if index == 0 {
            // SAFETY: wake is borrowed for the entire blocking poll.
            unsafe { wake.0.get() }
        } else {
            entries[index - 1].semaphore.as_raw()
        };
        // SAFETY: The array and semaphore stay alive and stationary for the
        // synchronous k_poll call. Only Zephyr accesses the events during it.
        unsafe {
            raw::k_poll_event_init(
                event,
                raw::ZR_POLL_TYPE_SEM_AVAILABLE,
                raw::k_poll_modes_K_POLL_MODE_NOTIFY_ONLY as i32,
                semaphore.cast(),
            );
        }
    }
    // SAFETY: Every array element was initialized above. k_poll removes its
    // registrations before returning, including on interruption errors.
    let result = unsafe {
        raw::k_poll(
            events.as_mut_ptr(),
            events.len().try_into().unwrap(),
            crate::sys::K_FOREVER,
        )
    };
    let _ = wake.take(NoWait);
    for (entry, event) in entries.iter().zip(&events[1..]) {
        // Repoll on interruption as well. Availability is advisory and the
        // future must always try taking again.
        if result != 0 || event.state() != raw::K_POLL_STATE_NOT_READY {
            entry.waker.wake_by_ref();
        }
    }
}
