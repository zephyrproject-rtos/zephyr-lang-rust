// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! # Zephyr low-level synchronization primities.
//!
//! The `zephyr-sys` crate contains direct calls into the Zephyr C API.  This interface, however,
//! cannot be used from safe Rust.  This crate attempts to be as direct an interface to some of
//! these synchronization mechanisms, but without the need for unsafe.  The other module
//! `crate::sync` provides higher level interfaces that help manage synchronization in coordination
//! with Rust's borrowing and sharing rules, and will generally provide much more usable
//! interfaces.
//!
//! # Kernel objects
//!
//! Zephyr's primitives work with the concept of a kernel object.  These are the data structures
//! that are used by the Zephyr kernel to coordinate the operation of the primitives.  In addition,
//! they are where the protection barrier provided by `CONFIG_USERSPACE` is implemented.  In order
//! to use these primitives from a userspace thread two things must happen:
//!
//! - The kernel objects must be specially declared.  All kernel objects in Zephyr will be built,
//!   at compile time, into a perfect hash table that is used to validate them.  The special
//!   declaration will take care of this.
//! - The objects must be granted permission to be used by the userspace thread.  This can be
//!   managed either by specifically granting permission, or by using inheritance when creating the
//!   thread.
//!
//! At this time, only the first mechanism is implemented, and all kernel objects should be
//! declared using the `crate::kobj_define!` macro.  These then must be initialized, and then the
//! special method `.get()` called, to retrieve the Rust-style value that is used to manage them.
//! Later, there will be a pool mechanism to allow these kernel objects to be allocated and freed
//! from a pool, although the objects will still be statically allocated.

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
        K_SEM_MAX_LIMIT,
    },
    time::Timeout,
};

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
    pub fn take<T>(&mut self, timeout: T) -> Result<()>
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
    pub fn give(&mut self) {
        unsafe {
            k_sem_give(self.item)
        }
    }

    /// Resets a semaphor's count to zero.
    ///
    /// This resets the count to zero.  Any outstanding [`take`] calls will be aborted with
    /// `Error(EAGAIN)`.
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
/// that will be properly registered with the Zephyr kernel object system.  Call [`take`] to get the
/// [`Semaphore`] that is represents.
pub type StaticSemaphore = StaticKernelObject<k_sem>;

impl Wrapped for StaticKernelObject<k_sem> {
    type T = Semaphore;

    // TODO: Thoughts about how to give parameters to the initialzation.
    fn get_wrapped(&self) -> Semaphore {
        let ptr = self.value.get();
        unsafe {
            k_sem_init(ptr, 0, K_SEM_MAX_LIMIT);
        }
        Semaphore {
            item: ptr,
        }
    }
}
