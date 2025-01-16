// Copyright (c) 2024 EOVE SAS
// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr timers
//!
//! This provides a relatively high-level and almost safe interface to Zephyr's underlying
//! `k_timer`.
//!
//! Every timer starts as a [`StoppedTimer`], which has been allocated, but is not tracking any
//! time.  These can either be created through [`StoppedTimer::new`], or by calling `.init_once(())` on a
//! StaticStoppedTimer declared within the `kobject_define!` macro, from [`object`].
//!
//! The `StoppedTimer` has two methods of interest here:
//!
//! - [`start_simple`]: which starts the timer.  This timer has methods for robustly getting counts
//!   of the number of times it has fired, as well as blocking the current thread for the timer to
//!   expire.
//! - [`start_callback`]: which starts the timer, registering a callback handler.  This timer will
//!   (unsafely) call the callback function, from IRQ context, every time the timer expires.
//!
//! Both of these returned timer types [`SimpleTimer`] and [`CallbackTimer`] have a [`stop`] method
//! that will stop the timer, and give back the original `StoppedTimer`.
//!
//! All of the types implement `Drop` and can dynamic timers can be safely dropped.  It is safe to
//! drop a timer allocated through the static object system, but it will then not be possible to
//! re-use that timer.
//!
//! [`object`]: crate::object
//! [`start_simple`]: StoppedTimer::start_simple
//! [`start_callback`]: StoppedTimer::start_callback
//! [`stop`]: SimpleTimer::stop

extern crate alloc;

#[cfg(CONFIG_RUST_ALLOC)]
use alloc::boxed::Box;

use core::ffi::c_void;
use core::marker::PhantomPinned;
use core::pin::Pin;
use core::{fmt, mem};

use crate::object::{Fixed, StaticKernelObject, Wrapped};
use crate::raw::{
    k_timer, k_timer_init, k_timer_start, k_timer_status_get, k_timer_status_sync, k_timer_stop,
    k_timer_user_data_get, k_timer_user_data_set,
};
use crate::time::Timeout;

/// A Zephyr timer that is not running.
///
/// A basic timer, allocated, but not running.
pub struct StoppedTimer {
    /// The underlying Zephyr timer.
    item: Fixed<k_timer>,
}

impl StoppedTimer {
    /// Construct a new timer.
    ///
    /// Allocates a dynamically allocate timer.  The time will not be running.
    #[cfg(CONFIG_RUST_ALLOC)]
    pub fn new() -> Self {
        let item: Fixed<k_timer> = Fixed::new(unsafe { mem::zeroed() });
        unsafe {
            // SAFETY: The `Fixed` type takes care of ensuring the timer is allocate at a fixed or
            // pinned address.
            k_timer_init(item.get(), None, None);
        }
        StoppedTimer { item }
    }

    /// Start the timer, in "simple" mode.
    ///
    /// Returns the [`SimpleTimer`] representing the running timer.  The `delay` specifies the
    /// amount of time before the first expiration happens.  `period` gives the time of subsequent
    /// timer expirations.  If `period` is [`NoWait`] or [`Forever`], then the timer will be
    /// one-shot
    ///
    /// [`NoWait`]: crate::time::NoWait
    /// [`Forever`]: crate::time::Forever
    pub fn start_simple(
        self,
        delay: impl Into<Timeout>,
        period: impl Into<Timeout>,
    ) -> SimpleTimer {
        unsafe {
            // SAFETY: The timer will be registered with Zephyr, using fields within the struct.
            // The `Fixed` type takes care of ensuring that the memory is not used.  Drop will call
            // `stop` to ensure that the timer is unregistered before the memory is returned.
            k_timer_start(self.item.get(), delay.into().0, period.into().0);
        }

        SimpleTimer {
            item: Some(self.item),
        }
    }

    /// Start the timer in "callback" mode.
    ///
    /// Returns the [`CallbackTimer`] representing the running timer.  The `delay` specifies the
    /// amount of time before the first expiration happens.  `period` gives the time of subsequent
    /// timer expirations.  If `period` is [`NoWait`] or [`Forever`], then the timer will be one
    /// shot.
    ///
    /// Each time the timer expires, The callback function given by the `Callback` will be called
    /// from IRQ context.  Much of Zephyr's API is unavailable from within IRQ context.  Some useful
    /// things to use are data that is wrapped in a [`SpinMutex`], a channel [`Sender`] from a
    /// bounded channel, or a [`Semaphore`], which can has it's `give` method available from IRQ
    /// context.
    ///
    /// Because the callback is registered with Zephyr, the resulting CallbackTimer must be pinned.
    ///
    /// [`NoWait`]: crate::time::NoWait
    /// [`Forever`]: crate::time::Forever
    /// [`SpinMutex`]: crate::sync::SpinMutex
    /// [`Semaphore`]: crate::sys::sync::Semaphore
    /// [`Sender`]: crate::sync::channel::Sender
    pub fn start_callback<T>(
        self,
        callback: Callback<T>,
        delay: impl Into<Timeout>,
        period: impl Into<Timeout>,
    ) -> Pin<Box<CallbackTimer<T>>>
    where
        T: Send + Sync,
    {
        CallbackTimer::new(self, callback, delay, period)
    }
}

impl fmt::Debug for StoppedTimer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StoppedTimer {:?}", self.item.get())
    }
}

/// A statically allocated `k_timer` (StoppedTimer).
///
/// This is intended to be used from within the `kobj_define!` macro.  It declares a static
/// `k_timer` that will be properly registered with the Zephyr object system (and can be used from
/// userspace).  Call `[init_once`] to get the `StoppedTimer` that it represents.
///
/// [`init_once`]: StaticStoppedTimer::init_once
pub type StaticStoppedTimer = StaticKernelObject<k_timer>;

// SAFETY: The timer itself is not associated with any particular thread, but it is unclear if they
// are safe to use from multiple threads.  As such, we'll declare this as Send, !Sync.
unsafe impl Send for StoppedTimer {}

impl Wrapped for StaticKernelObject<k_timer> {
    type T = StoppedTimer;

    /// No initializers.
    type I = ();

    fn get_wrapped(&self, _arg: Self::I) -> StoppedTimer {
        let ptr = self.value.get();
        unsafe {
            // SAFETY: The ptr is static, so it is safe to have Zephyr initialize.  The callback is
            // safe as it checks if the user data has been set.  The callback is needed for the
            // callback version of the timer.
            k_timer_init(ptr, None, None);
        }
        StoppedTimer {
            item: Fixed::Static(ptr),
        }
    }
}

/// A simple timer.
///
/// A SimpleTimer represents a running Zephyr `k_timer` that does not have a callback registered.
/// It can only be created by calling [`StoppedTimer::start_simple`].
pub struct SimpleTimer {
    /// The underlying Zephyr timer.  Option is needed to coordinate 'stop' and 'drop'.
    item: Option<Fixed<k_timer>>,
}

impl SimpleTimer {
    /// Read the count from the timer.
    ///
    /// Returns the number of times the timer has fired since the last time either this method or
    /// [`read_count_wait`] was called.
    ///
    /// This works via an internal counter, that is atomically reset to zero when the current value
    /// of the counter is read.
    ///
    /// [`read_count_wait`]: Self::read_count_wait
    pub fn read_count(&mut self) -> u32 {
        unsafe {
            // SAFETY: As long as the timer's data is allocated, this call is safe in Zephyr.
            k_timer_status_get(self.item_ptr())
        }
    }

    /// Read the count from the timer, waiting for it to become non-zero.
    ///
    /// Blocks the current thread until the timer has fired at least once since the last call to
    /// this method or [`read_count`].  Once it has fired, will return the count.  This will return
    /// immediately if the timer has already fired once since the last time.
    ///
    /// [`read_count`]: Self::read_count
    pub fn read_count_wait(&mut self) -> u32 {
        unsafe {
            // SAFETY: As long as the timer's data is allocated, this call is safe in Zephyr.
            k_timer_status_sync(self.item_ptr())
        }
    }

    /// Restart the current timer.
    ///
    /// This resets the fired counter back to zero, and sets a new `delay` and `period` for the
    /// timer.  It is mostly equivalent to `self.stop().start_simple(delay, period)`, but saves the
    /// step of having to stop the timer.
    pub fn restart(&mut self, delay: impl Into<Timeout>, period: impl Into<Timeout>) {
        unsafe {
            // SAFETY: According to zephyr docs, it is safe to `start` a running timer, and the
            // behavior is as described here.
            k_timer_start(self.item_ptr(), delay.into().0, period.into().0);
        }
    }

    /// Get the item pointer, assuming it is still present.
    fn item_ptr(&self) -> *mut k_timer {
        self.item
            .as_ref()
            .expect("Use of SimpleTimer after stop")
            .get()
    }

    /// Stop the timer.
    ///
    /// Stops the timer, so that it will not fire any more, converting the timer back into a
    /// StoppedTimer.
    pub fn stop(mut self) -> StoppedTimer {
        // Actually do the stop.
        let item = self.raw_stop();

        let item = item.expect("Error in stop/drop interaction");

        StoppedTimer { item }
    }

    /// Attempt to stop the timer, if it is still present.  Returns the possible inner item.
    fn raw_stop(&mut self) -> Option<Fixed<k_timer>> {
        let item = self.item.take();
        if let Some(ref item) = item {
            unsafe {
                // SAFETY: This call, in Zephyr, removes the timer from any queues.  There must also
                // not be any threads blocked on `read_count_wait`, which will be the case because
                // this is `self` and there can be no other references to the timer in Rust.
                k_timer_stop(item.get())
            }
        }
        item
    }
}

impl Drop for SimpleTimer {
    fn drop(&mut self) {
        // Stop the timer, discarding the inner item.
        let _ = self.raw_stop();
    }
}

/// A timer callback.  The function will be called in IRQ context being passed the given data.
/// Note that this handler owns the data, but passes a reference to the handler.  This will
/// typically be a `SpinMutex` to allow for proper sharing with IRQ context.
pub struct Callback<T: Send + Sync> {
    /// The callback function.
    pub call: fn(data: &T),
    /// The data passed into the callback.
    pub data: T,
}

/// A zephyr timer that calls a callback each time the timer expires.
///
/// Each time the timer fires, the callback will be called.  It is important to note the data
/// associated with the timer must be both `Send` and `Sync`.  As the callback will be called from
/// interrupt context, a normal `Mutex` cannot be used.  For this purpose, there is a [`SpinMutex`]
/// type that protects the data with a spin lock.  Other useful things a pass as data to the
/// callback are [`Sender`] from a bounded channel, and a [`Semaphore`].
///
/// [`SpinMutex`]: crate::sync::SpinMutex
/// [`Sender`]: crate::sync::channel::Sender
/// [`Semaphore`]: crate::sys::sync::Semaphore
pub struct CallbackTimer<T: Send + Sync> {
    /// The underlying Zephyr timer.
    item: Option<Fixed<k_timer>>,

    /// The callback used for expiry.
    expiry: Callback<T>,

    /// Marker to prevent unpinning.
    _marker: PhantomPinned,
}

impl<T: Send + Sync> CallbackTimer<T> {
    fn new(
        item: StoppedTimer,
        callback: Callback<T>,
        delay: impl Into<Timeout>,
        period: impl Into<Timeout>,
    ) -> Pin<Box<CallbackTimer<T>>> {
        let this = Box::pin(CallbackTimer {
            item: Some(item.item),
            expiry: callback,
            _marker: PhantomPinned,
        });

        // Set the timer's expiry function.
        unsafe {
            // SAFETY: The timer is not running as this came from a stopped timer.  Therefore there
            // are no races with timers potentially using the callback function.
            //
            // After we set the expiry function, the timer will be started with `k_timer_start`,
            // which includes the necessary memory barrier to that the timer irq will see the updated
            // callback function and user data.
            let item_ptr = this.item_ptr();
            (*item_ptr).expiry_fn = Some(Self::timer_expiry);
            let raw = &this.expiry as *const _ as *const c_void;
            k_timer_user_data_set(item_ptr, raw as *mut c_void);

            k_timer_start(item_ptr, delay.into().0, period.into().0);
        }

        this
    }

    /// The timer callback.  Called in IRQ context, by Zephyr.
    unsafe extern "C" fn timer_expiry(ktimer: *mut k_timer) {
        // The user data comes back from Zephyr as a `* mut`, even though that is not sound.
        let data = unsafe {
            // SAFETY: The user data pointer was set above to the pinned expiry.  It will be
            // unregistered, as set as null when drop is called.  Although the timer will also be
            // stopped, the callback should be safe as this function checks.
            k_timer_user_data_get(ktimer)
        };
        if data.is_null() {
            return;
        }
        let cb: &Callback<T> = &*(data as *const Callback<T>);
        (cb.call)(&cb.data);
    }

    /// Get the item pointer, assuming it is still present.
    fn item_ptr(&self) -> *mut k_timer {
        self.item
            .as_ref()
            .expect("Use of SimpleTimer after stop")
            .get()
    }

    /// Stop the timer.
    ///
    /// Stops the timer, so that it will not fire any more, converting the timer back into a
    /// StoppedTimer.
    pub fn stop(mut self) -> StoppedTimer {
        // Actually do the stop.
        let item = self.raw_stop();

        let item = item.expect("Error in stop/drop interaction");

        StoppedTimer { item }
    }

    /// Stop the timer.  Returns the inner item.
    fn raw_stop(&mut self) -> Option<Fixed<k_timer>> {
        let item = self.item.take();
        if let Some(ref item) = item {
            unsafe {
                // SAFETY: Stopping the timer removes it from any queues.  There must not be threads
                // blocked, which is enforced by this only being called from either `stop` or
                // `drop`.  Once this has been stopped, it is then safe to remove the callback.  As
                // there will be no more timer operations until the timer is restarted, which will
                // have a barrier, this is also safe.
                let raw_item = item.get();
                k_timer_stop(raw_item);
                (*raw_item).expiry_fn = None;
            }
        }
        item
    }
}

impl<T: Send + Sync> Drop for CallbackTimer<T> {
    fn drop(&mut self) {
        // Stop the timer, discarding the inner item.
        let _ = self.raw_stop();
    }
}
