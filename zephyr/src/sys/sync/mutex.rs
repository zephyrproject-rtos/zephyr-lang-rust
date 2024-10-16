// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr `k_mutex` wrapper.
//!
//! This module implements a thing wrapper around the `k_mutex` type in Zephyr.  It works with the
//! kernel [`object`] system, to allow the mutexes to be defined statically.
//!
//! [`object`]: crate::object

use core::fmt;
use crate::{
    error::{Result, to_result_void},
    raw::{
        k_condvar,
        k_condvar_init,
        k_condvar_broadcast,
        k_condvar_signal,
        k_condvar_wait,
        k_mutex,
        k_mutex_init,
        k_mutex_lock,
        k_mutex_unlock,
    },
    time::Timeout,
};
use crate::object::{
    StaticKernelObject,
    Wrapped,
};
use crate::sys::K_FOREVER;

/// A Zephyr `k_mutux` usable from safe Rust code.
///
/// This merely wraps a pointer to the kernel object.  It implements clone, send and sync as it is
/// safe to have multiple instances of these, as well as use them across multiple threads.
///
/// Note that these are Safe in the sense that memory safety is guaranteed.  Attempts to
/// recursively lock, or incorrect nesting can easily result in deadlock.
///
/// Safety: Typically, the Mutex type in Rust does not implement Clone, and must be shared between
/// threads using Arc.  However, these sys Mutexes are wrappers around static kernel objects, and
/// Drop doesn't make sense for them.  In addition, Arc requires alloc, and one possible place to
/// make use of the sys Mutex is to be able to do so in an environment without alloc.
///
/// This mutex type of only of limited use to application programs.  It can be used as a simple
/// binary semaphore, although it has strict semantics, requiring the release to be called by the
/// same thread that called lock.  It can be used to protect data that Rust itself is either not
/// managing, or is managing in an unsafe way.
///
/// For a Mutex type that is useful in a Rust type of manner, please see the regular [`sync::Mutex`]
/// type.
///
/// [`sync::Mutex`]: http://example.com/TODO
#[derive(Clone)]
pub struct Mutex {
    /// The raw Zephyr mutex.
    item: *mut k_mutex,
}

impl Mutex {
    /// Lock a Zephyr Mutex.
    ///
    /// Will wait for the lock, returning status, with `Ok(())` indicating the lock has been
    /// acquired, and an error indicating a timeout (Zephyr returns different errors depending on
    /// the reason).
    pub fn lock<T>(&self, timeout: T) -> Result<()>
        where T: Into<Timeout>,
    {
        let timeout: Timeout = timeout.into();
        to_result_void(unsafe { k_mutex_lock(self.item, timeout.0) })
    }

    /// Unlock a Zephyr Mutex.
    ///
    /// The mutex must already be locked by the calling thread.  Mutexes may not be unlocked in
    /// ISRs.
    pub unsafe fn unlock(&self) -> Result<()> {
        to_result_void(unsafe { k_mutex_unlock(self.item) })
    }
}


/// A static Zephyr `k_mutex`
///
/// This is intended to be used from within the `kobj_define!` macro.  It declares a static
/// `k_mutex` that will be properly registered with the Zephyr object system.  Call [`init_once`] to
/// get the [`Mutex`] that it represents.
///
/// [`init_once`]: StaticMutex::init_once
pub type StaticMutex = StaticKernelObject<k_mutex>;

unsafe impl Sync for Mutex {}
unsafe impl Send for Mutex {}

// Sync and Send are meaningful, as the underlying Zephyr API can use these values from any thread.
// Care must be taken to use these in a safe manner.
unsafe impl Sync for StaticMutex {}
unsafe impl Send for StaticMutex {}

impl Wrapped for StaticKernelObject<k_mutex> {
    type T = Mutex;

    /// Mutex initializers take no argument.
    type I = ();

    fn get_wrapped(&self, _arg: Self::I) -> Mutex {
        let ptr = self.value.get();
        unsafe {
            k_mutex_init(ptr);
        }
        Mutex { 
            item: ptr,
        }
    }
}

impl fmt::Debug for Mutex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Mutex {:?}", self.item)
    }
}

/// A Condition Variable
///
/// Lightweight wrappers for Zephyr's `k_condvar`.
#[derive(Clone)]
pub struct Condvar {
    /// The underlying `k_condvar`.
    item: *mut k_condvar,
}

#[doc(hidden)]
pub type StaticCondvar = StaticKernelObject<k_condvar>;

unsafe impl Sync for StaticKernelObject<k_condvar> { }

unsafe impl Sync for Condvar {}
unsafe impl Send for Condvar {}

impl Condvar {
    /// Wait for someone else using this mutex/condvar pair to notify.
    ///
    /// Note that this requires the lock to be held by use, but as this is a low-level binding to
    /// Zephyr's interfaces, this is not enforced.  See [`sync::Condvar`] for a safer and easier to
    /// use interface.
    ///
    /// [`sync::Condvar`]: http://www.example.com/TODO
    // /// [`sync::Condvar`]: crate::sync::Condvar
    pub fn wait(&self, lock: &Mutex) {
        unsafe { k_condvar_wait(self.item, lock.item, K_FOREVER); }
    }

    // TODO: timeout.

    /// Wake a single thread waiting on this condition variable.
    pub fn notify_one(&self) {
        unsafe { k_condvar_signal(self.item); }
    }

    /// Wake all threads waiting on this condition variable.
    pub fn notify_all(&self) {
        unsafe { k_condvar_broadcast(self.item); }
    }
}

impl fmt::Debug for Condvar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Condvar {:?}", self.item)
    }
}

impl Wrapped for StaticCondvar {
    type T = Condvar;

    /// Condvar initializers take no argument.
    type I = ();

    fn get_wrapped(&self, _arg: Self::I) -> Condvar {
        let ptr = self.value.get();
        unsafe {
            k_condvar_init(ptr);
        }
        Condvar {
            item: ptr,
        }
    }
}
