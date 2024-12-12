// Copyright (c) 2024 EOVE SAS
// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! Zephyr `k_timer` wrapper.
//!
//! This module wraps the `k_timer` object from Zephyr.  This works with either the [`object`]
//! system from the kernel, or can support a pinned boxed version.
//!
//! Cleanup of Zephyr objects, in general, isn't well thought out.  The descriptions of the methods
//! can help to be able to use this type soundly.  The interface is unsafe, as the timers run
//! callbacks in IRQ context.  However, in addition, the data given to the callback must be used
//! carefully to prevent unsound use.  Notable, it is never permissible for the IRQ callback to make
//! a 'mut' reference out of the data.  To synchronize with IRQ handlers, such as this one, it is
//! recommended that the data should be a shared reference to a `[SpinMutex]`.
//!
//! [`object`]: crate::object
//! [`SpinMutex`] crate::sync::SpinMutex

use core::ffi::c_void;
use core::marker::PhantomPinned;
use core::{fmt, mem, ptr};
use core::pin::Pin;

use crate::object::{Fixed, StaticKernelObject, Wrapped};
use crate::raw::{
    k_timer,
    k_timer_init,
    k_timer_start,
    k_timer_status_get,
    k_timer_status_sync,
    k_timer_stop,
    k_timer_user_data_get,
    k_timer_user_data_set,
};
use crate::time::Timeout;

/// A timer callback.  The function will be called, in IRQ context being passed the given data.
/// Note that this handler owns the data, but passes a reference to the handler.  This will
/// typically be a `SpinMutex` to allow for proper sharing with IRQ context.
pub struct Callback<T: Send + Sync> {
    call: fn(data: &T),
    data: T,
}

/// A Zephyr timer.
///
/// A Zephyr timer is an object that can cause something to happen in the future.  The timer has a
/// `duration` and `period` value.  If the `period` is [`Forever`] (or [`NoWait`], then the timer
/// will fire once, otherwise after the first expiry, it will fire every `period` interval.
///
/// Timers can be used two ways (both are permissible).  It is possible to register a callback with
/// the timer which will be called, in IRQ context, each time the timer fires.  This can be used,
/// for example, with an `Arc<SpinMutex<Sender<T>>>` with a bounded channel to send a message each
/// time the timer fires.
///
/// Alternatively, the timer can be left free-running, and the `status` or `status_sync` methods can
/// be used to determine the state of the timer.
///
/// Note that the timer is typed, which is the data payload type for the callback.  If the timer is
/// used without a callback, this can be something like `()`.
pub struct Timer<T: Send + Sync> {
    /// The underlying Zephyr timer.
    inner: Fixed<k_timer>,

    /// The callback used for expiry.
    expiry: Option<Callback<T>>,

    /// Marker to prevent unpinning.
    _marker: PhantomPinned,
}

// For now, don't implement sync or send, as it is unclear if we can implement Drop safely in that
// case.  Send is probably ok?

impl<T> Timer<T>
where
    T: Send + Sync,
{
    /// Construct a new Timer.
    ///
    /// Allocate a new Timer.  There will be no callback set, and the timer is not running.
    #[cfg(CONFIG_RUST_ALLOC)]
    pub fn new() -> Self {
        let item: Fixed<k_timer> = Fixed::new(unsafe { mem::zeroed() });
        unsafe {
            k_timer_init(item.get(), Some(Self::timer_expiry), None);
        }
        Timer {
            inner: item,
            expiry: None,
            _marker: PhantomPinned,
        }
    }

    /// Set the callback.
    ///
    /// Note that this is racy if called after the timer has been started (meaning the timer may
    /// expire before this can be called.
    ///
    /// Replacing a callback is likely unsound, as the callback data is not SMP protected, and there
    /// is no guarantee that an IRQ handler won't see the old value after we have dropped it.
    pub unsafe fn set_expiry_callback(
        mut self: Pin<&mut Self>,
        expiry: Callback<T>,
    ) {
        // SAFETY: It is unclear if the user data set and get methods have proper memory
        // synchronization (it doesn't appear so).  This is the reason for the above comment that it
        // is only safe to call this function when the timer is not running. Nonetheless, set the
        // user data pointer to null before calling drop, which will make this safe, aside from the
        // memory barrier issues.
        unsafe {
            k_timer_user_data_set(self.inner.get(), ptr::null_mut());
        }
        // SAFETY: Now that we have no references to the callback, we can safely replace the value
        // by moving in the new field.
        unsafe {
            self.as_mut().get_unchecked_mut().expiry = Some(expiry);
        }
        // Move into our struct.  This ensures that we are giving the address of the pinned value.
        unsafe {
            let raw = self.expiry.as_ref().unwrap() as *const _ as *const c_void;
            k_timer_user_data_set(self.inner.get(), raw as *mut c_void);
        }
    }

    /// The timer callback.  This is called in IRQ context, by Zephyr.
    unsafe extern "C" fn timer_expiry(ktimer: *mut k_timer) {
        // The user data comes back from Zephyr as a `*mut`, even though that is not sound.
        let data = unsafe {
            k_timer_user_data_get(ktimer)
        };
        if data.is_null() {
            return;
        }
        let cb: &Callback<T> = &*(data as *const Callback<T>);
        (cb.call)(&cb.data);
    }

    /// Start the timer.
    ///
    /// The [`delay`] specifies the amount of time before the first time expiration happens.
    /// [`period`] gives the time of subsequent timer expirations.  If [`period`] is [`NoWait`] or
    /// [`Forever`], then the timer will be one-shot.
    ///
    /// If a callback was registered with `set_expiry_callback`, it will be called each time the
    /// timer expires. In addition, the status, and status_wait methods can be used to monitor or
    /// wait for the timer.
    pub fn start(&mut self, delay: impl Into<Timeout>, period: impl Into<Timeout>) {
        unsafe {
            k_timer_start(self.inner.get(), delay.into().0, period.into().0);
        }
    }

    /// Stop the timer.
    ///
    /// Stop the possibly running timer.  This method may be called from IRQ context.
    ///
    /// There is some hint on Discord that this is "a little bit racy".  But, it seems that as long
    /// as there is nothing waiting on the timer, it should be safe for the Timer itself to be
    /// dropped.  Drop::drop for this will always call `stop`.
    pub fn stop(&mut self) {
        unsafe {
            k_timer_stop(self.inner.get());
        }
    }

    /// Read the timer's "status".
    ///
    /// Return a count of the number of times this timer has expired.  The internal count will be
    /// atomically reset to zero after this call.
    pub fn status_get(&mut self) -> u32 {
        unsafe {
            k_timer_status_get(self.inner.get())
        }
    }

    /// Read the timer's status, waiting until it fires at least once
    ///
    /// If the timer has expired since the last time the status was read, will return the same count
    /// `status_get` would.  Otherwise, block until the timer fires, and return the status.
    /// The internal counter is reset to zero upon return.
    pub fn status_sync(&mut self) -> u32 {
        unsafe {
            k_timer_status_sync(self.inner.get())
        }
    }
}

impl<T: Send + Sync> Drop for Timer<T> {
    fn drop(&mut self) {
        // As long as the timer is not running, it is safe to drop it.
        self.stop();
    }
}

impl<T: Send + Sync> fmt::Debug for Timer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Timer {:?}", self.inner.get())
    }
}

// TODO: Figure out how to actually make a static one of these.
//
// The type is a little weird, because it needs a type, but I'm not sure Rust types are sufficiently
// powerful enough to express this relationship.

/*
/// A static Zephyr Timer.
///
/// This is intended to be used from within the `kobj_define!`.  It declares a statically allocated
/// `k_timer` that will be properly registered with the Zephyr object system.  Call [`init_once`] do
/// get the [`Timer`] that it represents.
///
/// [`init_once`]: StaticTimer::init_once
pub type StaticTimer = StaticKernelObject<k_timer>;

impl<TT: Send + Sync> Wrapped for StaticKernelObject<k_timer> {
    type T = Timer<TT>;

    type I= ();

    fn get_wrapped(&self, _: Self::I) -> Self::T {
        let ptr = self.value.get();
        unsafe {
            k_timer_init(ptr, Some(Timer::<TT>::timer_expiry), None);
        }
        todo!()
    }
}
*/
