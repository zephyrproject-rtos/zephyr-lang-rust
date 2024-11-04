//! Spinlock-based Mutexes
//!
//! The [`sync::Mutex`] type is quite handy in Rust for sharing data between multiple contexts.
//! However, it is not possible to aquire a mutex from interrupt context.
//!
//! This module provides a [`SpinMutex`] which has some similarities to the above, but with some
//! restrictions.  In contrast, however, it is usable from interrupt context.
//!
//! It works by use a spinlock to protect the contents of the SpinMutex.  This allows for use in
//! user threads as well as from irq context, the spinlock even protecting the data on SMP machines.
//!
//! In contract to [`critical_section::Mutex`], this has an API much closer to [`sync::Mutex`] (and
//! as such to [`std::sync::Mutex`].  In addition, it has slightly less overhead on Zephyr, and
//! since the mutex isn't shared, might allow for slightly better use on SMP systems, when the other
//! CPU(s) don't need access to the SyncMutex.
//!
//! Note that `SpinMutex` doesn't have anything comparable to `Condvar`.  Generally, they can be
//! used with a `Semaphore` to allow clients to be waken, but this usage is racey, and if not done
//! carefully can result uses of the semaphore not waking.

use core::{cell::UnsafeCell, convert::Infallible, fmt, marker::PhantomData, ops::{Deref, DerefMut}};

use crate::raw;

/// Result from the lock call.  We keep the result for consistency of the API, but these can never
/// fail.
pub type SpinLockResult<Guard> = core::result::Result<Guard, Infallible>;

/// Result from the `try_lock` call.  There is only a single type of failure, indicating that this
/// would block.
pub type SpinTryLockResult<Guard> = core::result::Result<Guard, SpinTryLockError>;

/// The single error type that can be returned from `try_lock`.
pub enum SpinTryLockError {
    /// The lock could not be acquired at this time because the operation would otherwise block.
    WouldBlock,
}

/// A lower-level mutual exclusion primitive for protecting data.
///
/// This is modeled after [`sync::Mutex`] but instead of using `k_mutex` from Zephyr, it uses
/// `k_spinlock`.  It's main advantage is that it is usable from IRQ context.  However, it also is
/// uninterruptible, and prevents even IRQ handlers from running.
pub struct SpinMutex<T: ?Sized> {
    inner: UnsafeCell<raw::k_spinlock>,
    data: UnsafeCell<T>,
}

/// As the data is protected by spinlocks, with RAII ensuring the lock is always released, this
/// satisfies Rust's requirements for Send and Sync.  The dependency of both on "Send" of the data
/// type is intentional, as it is the Mutex that is providing the Sync semantics.  However, it only
/// makes sense for types that are usable from multiple thread contexts.
unsafe impl<T: ?Sized + Send> Send for SpinMutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for SpinMutex<T> {}

impl<T> fmt::Debug for SpinMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mutex {:?}", self.inner)
    }
}

/// An RAII implementation of a "scoped lock" of a SpinMutex.  When this structure is dropped (falls
/// out of scope), the lock will be unlocked.
///
/// The data protected by the SpinMutex can be accessed through this guard via it's [`Deref'] and
/// [`DerefMut`] implementations.
///
/// This structure is created by the [`lock`] and [`try_lock`] methods on [`SpinMutex`].
///
/// [`lock`]: SpinMutex::lock
/// [`try_lock`]: SpinMutex::try_lock
///
/// Borrowed largely from std's `MutexGuard`, but adapted to use spinlocks.
pub struct SpinMutexGuard<'a, T: ?Sized + 'a> {
    lock: &'a SpinMutex<T>,
    key: raw::k_spinlock_key_t,
    // Mark as not Send.
    _nosend: PhantomData<UnsafeCell<()>>,
}

// Negative trait bounds are unstable, see the _nosend field above.
/// Mark as Sync if the contained data is sync.
unsafe impl<T: ?Sized + Sync> Sync for SpinMutexGuard<'_, T> {}

impl<T> SpinMutex<T> {
    /// Construct a new wrapped Mutex.
    pub const fn new(t: T) -> SpinMutex<T> {
        SpinMutex {
            inner: UnsafeCell::new(unsafe { core::mem::zeroed() }),
            data: UnsafeCell::new(t),
        }
    }
}

impl<T: ?Sized> SpinMutex<T> {
    /// Acquire a mutex, spinning as needed to aquire the controlling spinlock.
    ///
    /// This function will spin the current thread until it is able to acquire the spinlock.
    /// Returns an RAII guard to allow scoped unlock of the lock.  When the guard goes out of scope,
    /// the SpinMutex will be unlocked.
    pub fn lock(&self) -> SpinLockResult<SpinMutexGuard<'_, T>> {
        let key = unsafe { raw::k_spin_lock(self.inner.get()) };
        unsafe { Ok(SpinMutexGuard::new(self, key)) }
    }

    /// Attempts to aquire this lock.
    ///
    /// If the lock could not be aquired at this time, then [`Err`] is returned.  Otherwise an RAII
    /// guard is returned.  The lock will be unlocked when the guard is dropped.
    ///
    /// This function does not block.
    pub fn try_lock(&self) -> SpinTryLockResult<SpinMutexGuard<'_, T>> {
        let mut key = raw::k_spinlock_key_t { key: 0 };
        match unsafe { raw::k_spin_trylock(self.inner.get(), &mut key) } {
            0 => {
                unsafe { 
                    Ok(SpinMutexGuard::new(self, key))
                }
            }
            _ => {
                Err(SpinTryLockError::WouldBlock)
            }
        }
    }
}

impl<'mutex, T: ?Sized> SpinMutexGuard<'mutex, T> {
    unsafe fn new(lock: &'mutex SpinMutex<T>, key: raw::k_spinlock_key_t) -> SpinMutexGuard<'mutex, T> {
        SpinMutexGuard { lock, key, _nosend: PhantomData }
    }
}

impl<T: ?Sized> Deref for SpinMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe {
            &*self.lock.data.get()
        }
    }
}

impl<T: ?Sized> DerefMut for SpinMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for SpinMutexGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            raw::k_spin_unlock(self.lock.inner.get(), self.key);
        }
    }
}
