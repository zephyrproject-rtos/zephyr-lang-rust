//! Synchronization mechanisms that work with async.
//!
//! Notably, Zephyr's `k_mutex` type isn't supported as a type that can be waited for
//! asynchronously.
//!
//! The main problem with `k_mutex` (meaning [`crate::sync::Mutex`]) is that the `lock` operation
//! can block, and since multiple tasks may be scheduled for the same work queue, the system can
//! deadlock, as the scheduler may not run to allow the task that actually holds the mutex to run.
//!
//! As an initial stopgap. We provide a [`Mutex`] type that is usable within an async context.  We
//! do not currently implement an associated `Condvar`.
//!
//! Note that using Semaphores for locking means that this mechanism doesn't handle priority
//! inversion issues.  Be careful with workers that run at different priorities.

// Use the same error types from the regular sync version.

use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    sync::{LockResult, TryLockError, TryLockResult},
    sys::sync::Semaphore,
    time::{Forever, NoWait},
};

/// A mutual exclusion primitive useful for protecting shared data.  Async version.
///
/// This mutex will block a task waiting for the lock to become available.
pub struct Mutex<T: ?Sized> {
    /// The semaphore indicating ownership of the data.  When it is "0" the task that did the 'take'
    /// on it owns the data, and will use `give` when it is unlocked.  This mechanism works for
    /// simple Mutex that protects the data without needing a condition variable.
    inner: Semaphore,
    data: UnsafeCell<T>,
}

// SAFETY: The semaphore, with the semantics provided here, provide Send and Sync.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mutex {:?}", self.inner)
    }
}

/// An RAII implementation of a held lock.
pub struct MutexGuard<'a, T: ?Sized + 'a> {
    lock: &'a Mutex<T>,
    // Mark !Send explicitly until support is added to Rust for this.
    _nosend: PhantomData<UnsafeCell<()>>,
}

unsafe impl<T: ?Sized + Sync> Sync for MutexGuard<'_, T> {}

impl<T> Mutex<T> {
    /// Construct a new Mutex.
    pub fn new(t: T) -> Mutex<T> {
        Mutex {
            inner: Semaphore::new(1, 1).unwrap(),
            data: UnsafeCell::new(t),
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Acquire the mutex, blocking the current thread until it is able to do so.
    ///
    /// This is a sync version, and calling it from an async task will possibly block the async work
    /// thread, potentially causing deadlock.
    pub fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        self.inner.take(Forever).unwrap();
        unsafe { Ok(MutexGuard::new(self)) }
    }

    /// Aquire the mutex, async version.
    pub async fn lock_async(&self) -> LockResult<MutexGuard<'_, T>> {
        self.inner.take_async(Forever).await.unwrap();
        unsafe { Ok(MutexGuard::new(self)) }
    }

    /// Attempt to aquire the lock.
    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        match self.inner.take(NoWait) {
            Ok(()) => unsafe { Ok(MutexGuard::new(self)) },
            // TODO: Distinguish timeout from other errors.
            Err(_) => Err(TryLockError::WouldBlock),
        }
    }
}

impl<'mutex, T: ?Sized> MutexGuard<'mutex, T> {
    unsafe fn new(lock: &'mutex Mutex<T>) -> MutexGuard<'mutex, T> {
        MutexGuard {
            lock,
            _nosend: PhantomData,
        }
    }
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.inner.give();
    }
}
