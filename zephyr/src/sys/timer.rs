// Copyright (c) 2024 EOVE SAS
// SPDX-License-Identifier: Apache-2.0

//! Zephyr `k_timer` wrapper.
//!
//! This module implements a thing wrapper around a `k_timer` object from Zephyr. It works
//! with the [`object`] system from the kernel. This offers to statically allocate a timer.
//!
//! [`object`]: crate::object

use core::fmt;
use core::future::Future;
use core::marker::PhantomPinned;
use core::pin::Pin;

use crate::object::{Fixed, StaticKernelObject, Wrapped};
use crate::raw::{
    k_timer, k_timer_init, k_timer_start, k_timer_stop, k_timer_user_data_get,
    k_timer_user_data_set,
};
use crate::time::{Duration, Timeout};

/// Zephyr `k_timer` usable from safe Rust code.
///
/// This merely wraps a pointer to the kernel object. It implements clone.
pub struct Timer {
    /// The raw Zephyr timer.
    inner: Fixed<k_timer>,

    /// The expired callback if any.
    expired: Option<(fn(&mut Self, *mut ()), *mut ())>,

    /// Marker to prevent from implementing Unpin.
    _marker: PhantomPinned,
}

unsafe impl Sync for StaticKernelObject<k_timer> {}

unsafe impl Send for Timer {}

impl Timer {
    /// Create a new timer from a static pointer.
    pub const fn new_from_ptr(ptr: *mut k_timer) -> Self {
        Timer {
            inner: Fixed::Static(ptr),
            expired: None,
            _marker: PhantomPinned,
        }
    }

    /// Set the expired callback.
    pub fn with_expiry_callback(
        &mut self,
        expired: fn(&mut Self, *mut ()),
        data: *mut (),
    ) -> &mut Self {
        self.expired = Some((expired, data));
        self
    }

    /// Start the Zephyr Timer after a given `duration` and repeat every `period`.
    pub fn start(self: Pin<&mut Self>, delay: impl Into<Timeout>, period: impl Into<Timeout>) {
        let ptr = self.inner.get();
        let user: *mut Self = unsafe { self.get_unchecked_mut() };

        unsafe {
            k_timer_user_data_set(ptr, user as *mut ::core::ffi::c_void);
            k_timer_start(ptr, delay.into().0, period.into().0);
        }
    }

    /// Stop a Zephyr Timer.
    pub fn stop(&mut self) {
        unsafe { k_timer_stop(self.inner.get()) };
    }
}

/// A static Zephyr Timer.
///
/// This is intended to be used from within `kobj_define!` macro. It declares a statically
/// allocated `k_timer` that will be proprely registered with the Zephyr object system. Call
/// [`init_once`] to get the [`Timer`] that is represents.
///
/// [`init_once`]: StaticTimer::init_once
pub type StaticTimer = StaticKernelObject<k_timer>;

impl Wrapped for StaticKernelObject<k_timer> {
    type T = Timer;
    type I = ();

    fn get_wrapped(&self, _: Self::I) -> Self::T {
        let ptr = self.value.get();
        unsafe { k_timer_init(ptr, Some(expired), None) };
        Timer::new_from_ptr(ptr)
    }
}

unsafe extern "C" fn expired(ktimer: *mut k_timer) {
    let user: *mut _ = k_timer_user_data_get(ktimer);
    let timer: &mut Timer = &mut *(user as *mut Timer);

    if let Some((expired, user)) = timer.expired {
        (expired)(timer, user);
    }
}

impl fmt::Debug for Timer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sys::Timer {:?}", self.inner.get())
    }
}

// impl Drop for Timer {
//     #[inline]
//     fn drop(&mut self) {
//         self.stop();
//     }
// }

mod futures {
    use core::cell::RefCell;
    use core::future::Future;
    use core::marker::PhantomData;
    use core::pin::Pin;
    use core::task::{Context, Poll, Waker};

    use crate::sync::{Arc, Mutex};
    use crate::time::{Duration, NoWait};

    use super::Timer;

    #[derive(Default)]
    struct TimerFutureState {
        expired: bool,
        waker: Option<Waker>,
    }

    impl TimerFutureState {
        fn new() -> Self {
            TimerFutureState::default()
        }
    }

    pub struct TimerFuture<'a> {
        /// The state of the future, indicating that the time has expired.
        state: Arc<Mutex<RefCell<TimerFutureState>>>,

        /// Marker to prevent from being dropped before the timer has
        _marker: PhantomData<&'a ()>,
    }

    impl<'a> TimerFuture<'a> {
        pub fn new<'b: 'a>(mut timer: Pin<&'b mut Timer>, delay: Duration) -> Self {
            let state = Arc::new(Mutex::new(RefCell::new(TimerFutureState::new())));
            let user: *const Mutex<_> = &*state;

            {
                let ptr = unsafe { timer.as_mut().get_unchecked_mut() };
                ptr.with_expiry_callback(expired, user as *mut ());
            }

            // Start a one-shot timer.
            timer.as_mut().start(delay, NoWait);

            TimerFuture {
                state,
                _marker: PhantomData,
            }
        }
    }

    fn expired(_: &mut Timer, user: *mut ()) {
        let mutex: &Mutex<_> = unsafe { &*(user as *const Mutex<RefCell<TimerFutureState>>) };

        let mut guard = mutex.lock().expect("mutex lock should never fail");
        let state = guard.get_mut();

        state.expired = true;

        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }

    impl Future for TimerFuture<'_> {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let mut guard = self.state.lock().expect("mutex lock should never fail");
            let state = guard.get_mut();

            if state.expired {
                return Poll::Ready(());
            }

            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
} // mod futures

/// Wait for a given seconds using a timer.
pub fn wait<'a, 'b: 'a>(
    timer: Pin<&'b mut Timer>,
    delay: Duration,
) -> impl Future<Output = ()> + 'a {
    futures::TimerFuture::new(timer, delay)
}
