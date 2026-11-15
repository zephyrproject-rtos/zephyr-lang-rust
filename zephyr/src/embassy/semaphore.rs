// Copyright (c) 2026
// SPDX-License-Identifier: Apache-2.0

//! Native semaphore waits driven by the Zephyr executor's polling thread.

extern crate alloc;

use alloc::vec::Vec;
use core::{
    future::poll_fn,
    task::{Poll, Waker},
};

use crate::{
    raw,
    simpletls::SimpleTls,
    sync::{Arc, Mutex},
    sys::sync::{semaphore::SemaphoreRef, Semaphore},
};

static EXECUTORS: Mutex<SimpleTls<Context>> = Mutex::new(SimpleTls::new());

#[derive(Clone, Copy)]
pub(super) struct Context {
    pub registry: &'static Registry,
    pub wake: &'static Semaphore,
}

impl Context {
    pub fn install(self) {
        // SAFETY: Called on the executor's own thread during run(). Both
        // references belong to that permanently borrowed, static executor.
        let thread = unsafe { raw::k_current_get() };
        EXECUTORS.lock().unwrap().insert(thread, self);
    }
}

pub(super) struct Entry {
    token: Arc<()>,
    pub semaphore: SemaphoreRef,
    pub waker: Waker,
}

pub(super) struct Registry(Mutex<Vec<Arc<Entry>>>);

impl Registry {
    pub const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    pub fn snapshot(&self) -> Vec<Arc<Entry>> {
        self.0.lock().unwrap().clone()
    }
}

struct Registration {
    context: Context,
    token: Arc<()>,
}

impl Registration {
    fn update(&self, semaphore: SemaphoreRef, waker: &Waker) {
        // Clone user-owned wakers before locking, and drop replaced wakers
        // after unlocking: either operation may execute arbitrary Rust code.
        let entry = Arc::new(Entry {
            token: self.token.clone(),
            semaphore,
            waker: waker.clone(),
        });
        let old = {
            let mut entries = self.context.registry.0.lock().unwrap();
            if let Some(index) = entries
                .iter()
                .position(|e| Arc::ptr_eq(&e.token, &self.token))
            {
                Some(core::mem::replace(&mut entries[index], entry))
            } else {
                entries.push(entry);
                None
            }
        };
        drop(old);
        self.context.wake.give();
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let removed = {
            let mut entries = self.context.registry.0.lock().unwrap();
            entries
                .iter()
                .position(|e| Arc::ptr_eq(&e.token, &self.token))
                .map(|index| entries.swap_remove(index))
        };
        drop(removed);
        // A snapshot may still be inside k_poll on another CPU. Its references
        // all remain valid; wake it so it rebuilds the event set promptly.
        self.context.wake.give();
    }
}

pub(crate) async fn take(semaphore: SemaphoreRef) {
    let mut registration = None;
    poll_fn(|cx| {
        if semaphore.try_take() {
            registration = None;
            return Poll::Ready(());
        }
        let registration = registration.get_or_insert_with(|| {
            let context = EXECUTORS
                .lock()
                .unwrap()
                .get()
                .expect("take_async must be polled on the Zephyr executor");
            Registration {
                context,
                token: Arc::new(()),
            }
        });
        registration.update(semaphore, cx.waker());
        // k_poll checks availability again when installing the wait, closing
        // the race between the failed take and registering this waker.
        Poll::Pending
    })
    .await
}
