//! Base Semaphore
//!
//! This is an experiment into a different approach to Zephyr kernel objects.
//!
//! Currently, these kernel objects are directed through "Fixed", which is an enum referencing with
//! a pointer to something static declared, or to a `Pin<Box<UnsafeCell<T>>>`.  This was done in an
//! attempt to keep things performant, but we actually always still end up with both an enum
//! discriminant, as well as an extra indirection for the static one.
//!
//! The deep issue here is that Zephyr objects inherently cannot be moved.  Zephyr uses a `dlist`
//! structure in most objects that has a pointer back to itself to indicate the empty list.
//!
//! To work around this, we will implement objects as a pairing of an `AtomicUsize` and a
//! `UnsafeCell<k_sem>` (for whatever underlying type).  The atomic will go through a small number
//! of states:
//!
//! - 0: indicates that this object is uninitialized.
//! - ptr: where ptr is the address of Self for an initialized object.
//!
//! On each use, the atomic value can be read (Relaxed is fine here), and if a 0 is seen, perform an
//! initialization.  The initialization will lock a simple critical section, checking the atomic
//! again, to make sure it didn't get initialized by something intercepting it.  If the check sees a
//! 'ptr' value that is not the same as Self, it indicates the object has been moved after
//! initialization, and will simply panic.

// To measure performance, this module implements this for `k_sem` without abstractions around it.
// The idea is to compare performance with the above `Fixed` implementation.

use core::{cell::UnsafeCell, ffi::c_uint, mem, sync::atomic::Ordering};

use zephyr::{error::to_result_void, raw::{k_sem, k_sem_give, k_sem_init, k_sem_take}, sync::atomic::AtomicUsize, time::Timeout};
use zephyr::Result;

pub struct Semaphore {
    state: AtomicUsize,
    item: UnsafeCell<k_sem>,
}

// SAFETY: These are both Send and Sync. The semaphore itself is safe, and the atomic+critical
// section protects the state.
unsafe impl Send for Semaphore { }
unsafe impl Sync for Semaphore { }

impl Semaphore {
    /// Construct a new semaphore, with the given initial_count and limit.  There is a bit of
    /// trickery to pass the initial values through to the initializer, but otherwise this is just a
    /// basic initialization.
    pub fn new(initial_count: c_uint, limit: c_uint) -> Semaphore {
        let this = Self {
            state: AtomicUsize::new(0),
            item: unsafe { UnsafeCell::new(mem::zeroed()) },
        };

        // Set the initial count and limit in the semaphore to use for later initialization.
        unsafe {
            let ptr = this.item.get();
            (*ptr).count = initial_count;
            (*ptr).limit = limit;
        }

        this
    }

    /// Get the raw pointer, initializing the `k_sem` if needed.
    fn get(&self) -> *mut k_sem {
        // First load can be relaxed, for performance reasons.  If it is seen as uninitialized, the
        // below Acquire load will see the correct value.
        let state = self.state.load(Ordering::Relaxed);
        if state == self as *const Self as usize {
            return self.item.get();
        } else if state != 0 {
            panic!("Semaphore was moved after first use");
        }

        critical_section::with(|_| {
            // Reload, with Acquire ordering to see a determined value.
            let state = self.state.load(Ordering::Acquire);
            if state == self as *const Self as usize {
                return self.item.get();
            } else if state != 0 {
                panic!("Semaphore was moved after first use");
            }

            // Perform the initialization.  We're within the critical section, and know that nobody
            // could be using this.
            unsafe {
                let ptr = self.item.get();
                let initial_count = (*ptr).count;
                let limit = (*ptr).limit;

                k_sem_init(ptr, initial_count, limit);
            }

            self.state.store(self as *const Self as usize, Ordering::Release);
            self.item.get()
        })
    }

    /// Synchronous take.
    pub fn take(&self, timeout: impl Into<Timeout>) -> Result<()> {
        let timeout: Timeout = timeout.into();
        let ptr = self.get();
        let ret = unsafe { k_sem_take(ptr, timeout.0) };
        to_result_void(ret)
    }

    pub fn give(&self) {
        let ptr = self.get();
        unsafe { k_sem_give(ptr) };
    }
}
