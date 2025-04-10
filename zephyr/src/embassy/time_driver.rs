//! Embassy time driver for Zephyr.
//!
//! Implements the time driver for Embassy using a `k_timer` in Zephyr.

use core::{
    cell::{RefCell, UnsafeCell},
    mem,
};

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex};
use embassy_time_driver::Driver;
use embassy_time_queue_utils::Queue;

use crate::raw::{k_timeout_t, k_timer, k_timer_init, k_timer_start};
use crate::sys::K_FOREVER;

/// The time base configured into Zephyr.
pub const ZEPHYR_TICK_HZ: u64 = crate::time::SYS_FREQUENCY as u64;

/// The configured Embassy time tick rate.
pub const EMBASSY_TICK_HZ: u64 = embassy_time_driver::TICK_HZ;

/// When the zephyr and embassy rates differ, use this intermediate type.  This can be selected by
/// feature.  At the worst case, with Embassy's tick at 1Mhz, and Zephyr's at 50k, it is a little
/// over 11 years.  Higher of either will reduce that further.  But, 128-bit arithmetic is fairly
/// inefficient.
type InterTime = u128;

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
            unsafe {
                k_timer_init(self.item.get(), Some(Self::timer_tick), None);
            }
            self.initialized = true;
        }

        // There is a +1 here as the `k_timer_start()` for historical reasons, subtracts one from
        // the time, effectively rounding down, whereas we want to wait at least long enough.
        let delta = k_timeout_t {
            ticks: (next - now + 1) as i64,
        };
        let period = K_FOREVER;
        unsafe {
            k_timer_start(self.item.get(), delta, period);
        }

        true
    }

    unsafe extern "C" fn timer_tick(_k_timer: *mut k_timer) {
        DRIVER.check_alarm();
    }
}

/// Convert from a zephyr tick count, to an embassy tick count.
///
/// This is done using an intermediate type defined above.
/// This conversion truncates.
fn zephyr_to_embassy(ticks: u64) -> u64 {
    if ZEPHYR_TICK_HZ == EMBASSY_TICK_HZ {
        // This should happen at compile time.
        return ticks;
    }

    // Otherwise do the intermediate conversion.
    let prod = (ticks as InterTime) * (EMBASSY_TICK_HZ as InterTime);
    (prod / (ZEPHYR_TICK_HZ as InterTime)) as u64
}

/// Convert from an embassy tick count to a zephyr.
///
/// This conversion use ceil so that values are always large enough.
fn embassy_to_zephyr(ticks: u64) -> u64 {
    if ZEPHYR_TICK_HZ == EMBASSY_TICK_HZ {
        return ticks;
    }

    let prod = (ticks as InterTime) * (ZEPHYR_TICK_HZ as InterTime);
    prod.div_ceil(EMBASSY_TICK_HZ as InterTime) as u64
}

fn zephyr_now() -> u64 {
    crate::time::now().ticks()
}

impl Driver for ZephyrTimeDriver {
    fn now(&self) -> u64 {
        zephyr_to_embassy(zephyr_now())
    }

    fn schedule_wake(&self, at: u64, waker: &core::task::Waker) {
        critical_section::with(|cs| {
            let mut queue = self.queue.borrow(cs).borrow_mut();
            let mut timer = self.timer.borrow(cs).borrow_mut();

            // All times below are in Zephyr units.
            let at = embassy_to_zephyr(at);

            if queue.schedule_wake(at, waker) {
                let mut next = queue.next_expiration(zephyr_now());
                while !timer.set_alarm(next, zephyr_now()) {
                    next = queue.next_expiration(zephyr_now());
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

            let mut next = queue.next_expiration(zephyr_now());
            while !timer.set_alarm(next, zephyr_now()) {
                next = queue.next_expiration(zephyr_now());
            }
        })
    }
}

// SAFETY: The timer access is always coordinated through a critical section.
unsafe impl Send for ZTimer {}
