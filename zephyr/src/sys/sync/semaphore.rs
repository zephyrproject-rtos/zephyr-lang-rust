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

use crate::{
    error::{Result, to_result_void},
    object::{StaticKernelObject, Wrapped},
    raw::{
        k_sem,
        k_sem_init,
        k_sem_take,
        k_sem_give,
        k_sem_reset,
        k_sem_count_get,
    },
    time::Timeout,
};

pub use crate::raw::K_SEM_MAX_LIMIT;

/// A zephyr `k_sem` usable from safe Rust code.
#[derive(Clone)]
pub struct Semaphore {
    /// The raw Zephyr `k_sem`.
    item: *mut k_sem,
}

/// By nature, Semaphores are both Sync and Send.  Safety is handled by the underlying Zephyr
/// implementation (which is why Clone is also implemented).
unsafe impl Sync for Semaphore {}
unsafe impl Send for Semaphore {}

impl Semaphore {
    /// Take a semaphore.
    ///
    /// Can be called from ISR if called with [`NoWait`].
    ///
    /// [`NoWait`]: crate::time::NoWait
    pub fn take<T>(&self, timeout: T) -> Result<()>
        where T: Into<Timeout>,
    {
        let timeout: Timeout = timeout.into();
        let ret = unsafe {
            k_sem_take(self.item, timeout.0)
        };
        to_result_void(ret)
    }

    /// Give a semaphore.
    ///
    /// This routine gives to the semaphore, unless the semaphore is already at its maximum
    /// permitted count.
    pub fn give(&self) {
        unsafe {
            k_sem_give(self.item)
        }
    }

    /// Resets a semaphor's count to zero.
    ///
    /// This resets the count to zero.  Any outstanding [`take`] calls will be aborted with
    /// `Error(EAGAIN)`.
    ///
    /// [`take`]: Self::take
    pub fn reset(&mut self) {
        unsafe {
            k_sem_reset(self.item)
        }
    }

    /// Get a semaphore's count.
    ///
    /// Returns the current count.
    pub fn count_get(&mut self) -> usize {
        unsafe {
            k_sem_count_get(self.item) as usize
        }
    }
}

/// A static Zephyr `k_sem`.
///
/// This is intended to be used from within the `kobj_define!` macro.  It declares a static ksem
/// that will be properly registered with the Zephyr kernel object system.  Call [`init_once`] to
/// get the [`Semaphore`] that is represents.
///
/// [`init_once`]: StaticKernelObject::init_once
pub type StaticSemaphore = StaticKernelObject<k_sem>;

unsafe impl Sync for StaticSemaphore {}

impl Wrapped for StaticKernelObject<k_sem> {
    type T = Semaphore;

    /// The initializer for Semaphores is the initial count, and the count limit (which can be
    /// K_SEM_MAX_LIMIT, re-exported here.
    type I = (c_uint, c_uint);

    // TODO: Thoughts about how to give parameters to the initialzation.
    fn get_wrapped(&self, arg: Self::I) -> Semaphore {
        let ptr = self.value.get();
        unsafe {
            k_sem_init(ptr, arg.0, arg.1);
        }
        Semaphore {
            item: ptr,
        }
    }
}

impl fmt::Debug for Semaphore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Semaphore")
    }
}
