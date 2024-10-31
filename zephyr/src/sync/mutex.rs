//! Higher level Mutex type and friends.
//!
//! These are modeled after the synchronization primitives in
//! [`std::sync`](https://doc.rust-lang.org/stable/std/sync/index.html), notably `Mutex`, and
//! `Condvar`, and the associated types.

use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::time::{Forever, NoWait};
use crate::sys::sync as sys;

/// Until poisoning is implemented, mutexes never return an error, and we just get back the guard.
pub type LockResult<Guard> = Result<Guard, ()>;

/// The return type from [`Mutex::try_lock`].
///
/// The error indicates the reason for the failure.  Until poisoning is
/// implemented, there is only a single type of failure.
pub type TryLockResult<Guard> = Result<Guard, TryLockError>;

/// An enumeration of possible errors associated with a [`TryLockResult`].
///
/// Note that until Poisoning is implemented, there is only one value of this.
pub enum TryLockError {
    /// The lock could not be acquired at this time because the operation would otherwise block.
    WouldBlock,
}

/// A mutual exclusion primitive useful for protecting shared data.
///
/// This mutex will block threads waiting for the lock to become available. This is modeled after
/// [`std::sync::Mutex`](https://doc.rust-lang.org/stable/std/sync/struct.Mutex.html), and attempts
/// to implement that API as closely as makes sense on Zephyr.  Currently, it has the following
/// differences:
/// - Poisoning: This does not yet implement poisoning, as there is no way to recover from panic at
///   this time on Zephyr.
/// - Allocation: `new` is not yet provided, and will be provided once kernel object pools are
///   implemented.  Please use `new_from` which takes a reference to a statically allocated
///   `sys::Mutex`.
pub struct Mutex<T: ?Sized> {
    inner: sys::Mutex,
    // poison: ...
    data: UnsafeCell<T>,
}

// At least if correctly done, the Mutex provides for Send and Sync as long as the inner data
// supports Send.
unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mutex {:?}", self.inner)
    }
}

/// An RAII implementation of a "scoped lock" of a mutex.  When this structure is dropped (faslls
/// out of scope), the lock will be unlocked.
///
/// The data protected by the mutex can be accessed through this guard via its [`Deref`] and
/// [`DerefMut`] implementations.
///
/// This structure is created by the [`lock`] and [`try_lock`] methods on [`Mutex`].
///
/// [`lock`]: Mutex::lock
/// [`try_lock`]: Mutex::try_lock
///
/// Taken directly from
/// [`std::sync::MutexGuard`](https://doc.rust-lang.org/stable/std/sync/struct.MutexGuard.html).
pub struct MutexGuard<'a, T: ?Sized + 'a> {
    lock: &'a Mutex<T>,
    // until <https://github.com/rust-lang/rust/issues/68318> is implemented, we have to mark unsend
    // explicitly.  This can be done by holding Phantom data with an unsafe cell in it.
    _nosend: PhantomData<UnsafeCell<()>>,
}

// Make sure the guard doesn't get sent.
// Negative trait bounds are unstable, see marker above.
// impl<T: ?Sized> !Send for MutexGuard<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for MutexGuard<'_, T> {}

impl<T> Mutex<T> {
    /// Construct a new wrapped Mutex, using the given underlying sys mutex.  This is different that
    /// `std::sync::Mutex` in that in Zephyr, objects are frequently allocated statically, and the
    /// sys Mutex will be taken by this structure.  It is safe to share the underlying Mutex between
    /// different items, but without careful use, it is easy to deadlock, so it is not recommended.
    pub const fn new_from(t: T, raw_mutex: sys::Mutex) -> Mutex<T> {
        Mutex { inner: raw_mutex, data: UnsafeCell::new(t) }
    }

    /// Construct a new Mutex, dynamically allocating the underlying sys Mutex.
    #[cfg(CONFIG_RUST_ALLOC)]
    pub fn new(t: T) -> Mutex<T> {
        Mutex::new_from(t, sys::Mutex::new().unwrap())
    }
}

impl<T: ?Sized> Mutex<T> {
    /// Acquires a mutex, blocking the current thread until it is able to do so.
    ///
    /// This function will block the local thread until it is available to acquire the mutex. Upon
    /// returning, the thread is the only thread with the lock held. An RAII guard is returned to
    /// allow scoped unlock of the lock. When the guard goes out of scope, the mutex will be
    /// unlocked.
    ///
    /// In `std`, an attempt to lock a mutex by a thread that already holds the mutex is
    /// unspecified.  Zephyr explicitly supports this behavior, by simply incrementing a lock
    /// count.
    pub fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        // With `Forever`, should never return an error.
        self.inner.lock(Forever).unwrap();
        unsafe {
            Ok(MutexGuard::new(self))
        }
    }

    /// Attempts to acquire this lock.
    ///
    /// If the lock could not be acquired at this time, then [`Err`] is returned. Otherwise, an RAII
    /// guard is returned. The lock will be unlocked when the guard is dropped.
    ///
    /// This function does not block.
    pub fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        match self.inner.lock(NoWait) {
            Ok(()) => {
                unsafe {
                    Ok(MutexGuard::new(self))
                }
            }
            // TODO: It might be better to distinguish these errors, and only return the WouldBlock
            // if that is the corresponding error. But, the lock shouldn't fail in Zephyr.
            Err(_) => {
                Err(TryLockError::WouldBlock)
            }
        }
    }
}

impl<'mutex, T: ?Sized> MutexGuard<'mutex, T> {
    unsafe fn new(lock: &'mutex Mutex<T>) -> MutexGuard<'mutex, T> {
        // poison todo
        MutexGuard { lock, _nosend: PhantomData }
    }
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe {
            &*self.lock.data.get()
        }
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
        self.lock.inner.unlock().unwrap();
    }
}

/// Inspired by
/// [`std::sync::Condvar`](https://doc.rust-lang.org/stable/std/sync/struct.Condvar.html),
/// implemented directly using `z_condvar` in Zephyr.
///
/// Condition variables represent the ability to block a thread such that it consumes no CPU time
/// while waiting for an even to occur.  Condition variables are typically associated with a
/// boolean predicate (a condition) and a mutex.  The predicate is always verified inside of the
/// mutex before determining that a thread must block.
///
/// Functions in this module will block the current **thread** of execution.  Note that any attempt
/// to use multiple mutexces on the same condition variable may result in a runtime panic.
pub struct Condvar {
    inner: sys::Condvar,
}

impl Condvar {
    /// Construct a new wrapped Condvar, using the given underlying `k_condvar`.
    ///
    /// This is different from `std::sync::Condvar` in that in Zephyr, objects are frequently
    /// allocated statically, and the sys Condvar will be taken by this structure.
    pub const fn new_from(raw_condvar: sys::Condvar) -> Condvar {
        Condvar { inner: raw_condvar }
    }

    /// Construct a new Condvar, dynamically allocating the underlying Zephyr `k_condvar`.
    #[cfg(CONFIG_RUST_ALLOC)]
    pub fn new() -> Condvar {
        Condvar::new_from(sys::Condvar::new().unwrap())
    }

    /// Blocks the current thread until this conditional variable receives a notification.
    ///
    /// This function will automatically unlock the mutex specified (represented by `guard`) and
    /// block the current thread.  This means that any calls to `notify_one` or `notify_all` which
    /// happen logically after the mutex is unlocked are candidates to wake this thread up.  When
    /// this function call returns, the lock specified will have been re-equired.
    ///
    /// Note that this function is susceptable to spurious wakeups.  Condition variables normally
    /// have a boolean predicate associated with them, and the predicate must always be checked
    /// each time this function returns to protect against spurious wakeups.
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        self.inner.wait(&guard.lock.inner);
        Ok(guard)
    }

    // TODO: wait_while
    // TODO: wait_timeout_ms
    // TODO: wait_timeout
    // TODO: wait_timeout_while

    /// Wakes up one blocked thread on this condvar.
    ///
    /// If there is a blocked thread on this condition variable, then it will be woken up from its
    /// call to `wait` or `wait_timeout`. Calls to `notify_one` are not buffered in any way.
    ///
    /// To wakeup all threads, see `notify_all`.
    pub fn notify_one(&self) {
        self.inner.notify_one();
    }

    /// Wakes up all blocked threads on this condvar.
    ///
    /// This methods will ensure that any current waiters on the condition variable are awoken.
    /// Calls to `notify_all()` are not buffered in any way.
    ///
    /// To wake up only one thread, see `notify_one`.
    pub fn notify_all(&self) {
        self.inner.notify_all();
    }
}

impl fmt::Debug for Condvar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Condvar {:?}", self.inner)
    }
}
