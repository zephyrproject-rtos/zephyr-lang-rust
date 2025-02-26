//! Embassy time driver for Zephyr.
//!
//! Implements the time driver for Embassy using a `k_timer` in Zephyr.

use core::{cell::{RefCell, UnsafeCell}, mem};

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time_driver::Driver;
use embassy_time_queue_utils::Queue;

use crate::raw::{
    k_timer,
    k_timer_init,
    k_timer_start,
    k_timeout_t,
};
use crate::sys::K_FOREVER;

embassy_time_driver::time_driver_impl!(static DRIVER: ZephyrTimeDriver = ZephyrTimeDriver {
    queue: Mutex::new(RefCell::new(Queue::new())),
    timer: Mutex::new(RefCell::new(unsafe { mem::zeroed() })),
});

struct ZephyrTimeDriver {
    queue: Mutex<CriticalSectionRawMutex, RefCell<Queue>>,
    timer: Mutex<CriticalSectionRawMutex, RefCell<ZTimer>>,
}

/// A wrapper around `k_timer`.  In this case, the implementation is a little simpler than the one
/// in the timer module, as we are always called from within a critical section.
struct ZTimer {
    item: UnsafeCell<k_timer>,
    initialized: bool,
}

impl ZTimer {
    fn set_alarm(&mut self, next: u64, now: u64) -> bool {
        if next <= now {
            return false;
        }

        // Otherwise, initialize our timer, and handle it.
        if !self.initialized {
            unsafe { k_timer_init(self.item.get(), Some(Self::timer_tick), None); }
            self.initialized = true;
        }

        // There is a +1 here as the `k_timer_start()` for historical reasons, subtracts one from
        // the time, effectively rounding down, whereas we want to wait at least long enough.
        let delta = k_timeout_t { ticks: (next - now + 1) as i64 };
        let period = K_FOREVER;
        unsafe { k_timer_start(self.item.get(), delta, period); }

        true
    }

    unsafe extern "C" fn timer_tick(_k_timer: *mut k_timer) {
        DRIVER.check_alarm();
    }
}

impl Driver for ZephyrTimeDriver {
    fn now(&self) -> u64 {
        crate::time::now().ticks()
    }

    fn schedule_wake(&self, at: u64, waker: &core::task::Waker) {
        critical_section::with(|cs| {
            let mut queue = self.queue.borrow(cs).borrow_mut();
            let mut timer = self.timer.borrow(cs).borrow_mut();

            if queue.schedule_wake(at, waker) {
                let mut next = queue.next_expiration(self.now());
                while !timer.set_alarm(next, self.now()) {
                    next = queue.next_expiration(self.now());
                }
            }
        })
    }
}

impl ZephyrTimeDriver {
    fn check_alarm(&self) {
        critical_section::with(|cs| {
            let mut queue = self.queue.borrow(cs).borrow_mut();
            let mut timer = self.timer.borrow(cs).borrow_mut();

            let mut next = queue.next_expiration(self.now());
            while !timer.set_alarm(next, self.now()) {
                next = queue.next_expiration(self.now());
            }
        })
    }
}

// SAFETY: The timer access is always coordinated through a critical section.
unsafe impl Send for ZTimer { }
