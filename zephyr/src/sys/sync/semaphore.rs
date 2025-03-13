// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr Semaphore support
//!
//! This is a thin wrapper around Zephyr's `k_sem`.  This is one of the few of the `sys` primitives
//! in Zephyr that is actually perfectly usable on its own, without needing additional wrappers.
//!
//! Zephyr implements counting semaphores, with both an upper and lower bound on the count.  Note
//! that calling 'give' on a semaphore that is at the maximum count will discard the 'give'
//! operation, which in situation where counting is actually desired, will result in the count being
//! incorrect.

use core::ffi::c_uint;
use core::fmt;

use crate::object::{ObjectInit, ZephyrObject};
#[cfg(CONFIG_RUST_ALLOC)]
use crate::{
    error::{to_result_void, Result},
    raw::{k_sem, k_sem_count_get, k_sem_give, k_sem_init, k_sem_reset, k_sem_take},
    time::Timeout,
};

pub use crate::raw::K_SEM_MAX_LIMIT;

/// General Zephyr Semaphores
pub struct Semaphore(pub(crate) ZephyrObject<k_sem>);

/// By nature, Semaphores are both Sync and Send.  Safety is handled by the underlying Zephyr
/// implementation (which is why Clone is also implemented).
unsafe impl Sync for Semaphore {}
unsafe impl Send for Semaphore {}

impl Semaphore {
    /// Create a new semaphore.
    ///
    /// Create a new dynamically allocated Semaphore.  This semaphore can only be used from system
    /// threads.  The arguments are as described in [the
    /// docs](https://docs.zephyrproject.org/latest/kernel/services/synchronization/semaphores.html).
    ///
    /// Note that this API has changed, and it now doesn't return a Result, since the Result time
    /// generally doesn't work (in stable rust) with const.
    #[cfg(CONFIG_RUST_ALLOC)]
    pub const fn new(initial_count: c_uint, limit: c_uint) -> Semaphore {
        // Due to delayed init, we need to replicate the object checks in the C `k_sem_init`.

        if limit == 0 || initial_count > limit {
            panic!("Invalid semaphore initialization");
        }

        let this = <ZephyrObject<k_sem>>::new_raw();

        unsafe {
            let addr = this.get_uninit();
            (*addr).count = initial_count;
            (*addr).limit = limit;
        }

        // to_result_void(k_sem_init(item.get(), initial_count, limit))?;
        Semaphore(this)
    }

    /// Take a semaphore.
    ///
    /// Can be called from ISR if called with [`NoWait`].
    ///
    /// [`NoWait`]: crate::time::NoWait
    pub fn take<T>(&self, timeout: T) -> Result<()>
    where
        T: Into<Timeout>,
    {
        let timeout: Timeout = timeout.into();
        let ret = unsafe { k_sem_take(self.0.get(), timeout.0) };
        to_result_void(ret)
    }

    /// Give a semaphore.
    ///
    /// This routine gives to the semaphore, unless the semaphore is already at its maximum
    /// permitted count.
    pub fn give(&self) {
        unsafe { k_sem_give(self.0.get()) }
    }

    /// Resets a semaphor's count to zero.
    ///
    /// This resets the count to zero.  Any outstanding [`take`] calls will be aborted with
    /// `Error(EAGAIN)`.
    ///
    /// [`take`]: Self::take
    pub fn reset(&self) {
        unsafe { k_sem_reset(self.0.get()) }
    }

    /// Get a semaphore's count.
    ///
    /// Returns the current count.
    pub fn count_get(&self) -> usize {
        unsafe { k_sem_count_get(self.0.get()) as usize }
    }
}

impl ObjectInit<k_sem> for ZephyrObject<k_sem> {
    fn init(item: *mut k_sem) {
        // SAFEFY: Get the initial values used in new.  The address may have changed, but only due
        // to a move.
        unsafe {
            let count = (*item).count;
            let limit = (*item).limit;

            if k_sem_init(item, count, limit) != 0 {
                // Note that with delayed init, we cannot do anything with invalid values.  We're
                // replicated the check in `new` above, so would only catch semantic changes in the
                // implementation of `k_sem_init`.
                unreachable!();
            }
        }
    }
}

impl fmt::Debug for Semaphore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Semaphore")
    }
}
