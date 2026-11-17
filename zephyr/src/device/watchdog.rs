// SPDX-License-Identifier: Apache-2.0

//! Synchronous watchdog support.
//!
//! Install timeouts before calling [`Watchdog::setup`], then feed every installed
//! channel within its window using [`WatchdogChannel::feed`]. Channels can be moved
//! to different threads. Hardware capabilities, including the number of channels
//! and whether a running watchdog can be disabled or restarted, depend on the driver.
//!
//! Callbacks and multistage timeouts are not exposed. With `CONFIG_WDT_MULTISTAGE`,
//! installed timeouts still have a null next-stage pointer.
//!
//! Driver operations are serialized per device without waiting for the lock:
//! contention returns `EAGAIN`. They are restricted to system threads (not ISRs).
//! Operations return `ENODEV` if the device is not ready, or `EPERM` from an ISR;
//! otherwise driver errors are propagated without retries.

use core::ffi::c_int;

use super::Unique;
use crate::error::{to_result, to_result_void, Error, Result};
use crate::raw;
use crate::sync::{Mutex, MutexGuard};

/// Action taken when a timeout expires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Reset {
    /// Do not reset. Without a callback, this provides no Rust notification.
    None = raw::WDT_FLAG_RESET_NONE as u8,
    /// Reset the CPU core.
    CpuCore = raw::WDT_FLAG_RESET_CPU_CORE as u8,
    /// Reset the entire SoC.
    Soc = raw::WDT_FLAG_RESET_SOC as u8,
}

/// A single-stage timeout with a feed window measured in milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeoutConfig {
    min_ms: u32,
    max_ms: u32,
    reset: Reset,
}

impl TimeoutConfig {
    /// Construct a timeout, returning `EINVAL` if `max_ms` is zero or is not
    /// greater than `min_ms`.
    ///
    /// Set `min_ms` to zero for watchdogs without window support. Drivers may
    /// impose additional range limits and round supported timeouts upward.
    pub fn new(min_ms: u32, max_ms: u32, reset: Reset) -> Result<Self> {
        if min_ms >= max_ms {
            return Err(Error(raw::EINVAL));
        }
        Ok(Self {
            min_ms,
            max_ms,
            reset,
        })
    }
}

/// Global watchdog options. The default does not request either pause mode.
///
/// Unsupported options are reported by the driver, usually as `ENOTSUP`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Options {
    /// Pause while the CPU is sleeping.
    pub pause_in_sleep: bool,
    /// Pause while the CPU is halted by a debugger.
    pub pause_when_debugging: bool,
}

impl Options {
    fn bits(self) -> u8 {
        let mut bits = 0;
        if self.pause_in_sleep {
            bits |= raw::ZR_WDT_OPT_PAUSE_IN_SLEEP;
        }
        if self.pause_when_debugging {
            bits |= raw::ZR_WDT_OPT_PAUSE_HALTED_BY_DBG;
        }
        bits
    }
}

/// Per-device synchronization and channel validity state.
pub struct WatchdogStatic {
    generation: Mutex<usize>,
}

impl WatchdogStatic {
    pub(crate) const fn new() -> Self {
        Self {
            generation: Mutex::new(0),
        }
    }

    fn lock(&self, device: *const raw::device) -> Result<MutexGuard<'_, usize>> {
        // SAFETY: Both handles are constructed with a static Zephyr device.
        if !unsafe { raw::device_is_ready(device) } {
            return Err(Error(raw::ENODEV));
        }
        // SAFETY: This query is valid in both thread and interrupt contexts.
        if unsafe { raw::k_is_in_isr() } {
            return Err(Error(raw::EPERM));
        }
        self.generation.try_lock().map_err(|_| Error(raw::EAGAIN))
    }
}

/// An owned channel returned by [`Watchdog::install_timeout`].
///
/// Channels can be moved to other threads and outlive the controller handle.
/// Dropping a channel does not uninstall its timeout or stop the watchdog.
#[must_use = "keep the channel to feed its installed timeout"]
pub struct WatchdogChannel {
    device: *const raw::device,
    data: &'static WatchdogStatic,
    generation: usize,
    id: c_int,
}

// SAFETY: The device and shared state are static. All driver access is serialized
// by the same per-device mutex; no state is tied to the calling thread.
unsafe impl Send for WatchdogChannel {}

impl WatchdogChannel {
    /// Feed this channel, returning `EINVAL` if a successful disable invalidated it.
    ///
    /// This also rejects stale channels after the driver reuses their numeric IDs.
    /// `EAGAIN`, from lock contention or the driver, is returned without retrying.
    pub fn feed(&mut self) -> Result<()> {
        let generation = self.data.lock(self.device)?;
        if self.generation != *generation {
            return Err(Error(raw::EINVAL));
        }
        // SAFETY: The ready device has a watchdog API. The lock prevents disable
        // or other driver operations between the validity check and feeding.
        to_result_void(unsafe { raw::wdt_feed(self.device, self.id) })
    }
}

/// A watchdog acquired through devicetree `get_instance()`.
///
/// Acquisition and dropping this handle do not start, feed, or stop the watchdog.
/// In particular, dropping an active watchdog leaves it running. Unique Rust
/// acquisition does not reserve the device against C users; applications must
/// coordinate with them and with any watchdog started at boot. C calls bypass
/// this module's synchronization and channel invalidation tracking.
pub struct Watchdog {
    device: *const raw::device,
    data: &'static WatchdogStatic,
}

// SAFETY: The device and shared state are static, with all driver access protected
// by the per-device mutex. Moving the controller does not bypass channel locking.
unsafe impl Send for Watchdog {}

impl Watchdog {
    /// Constructor used by generated devicetree code.
    ///
    /// # Safety
    ///
    /// `device` must point to a static Zephyr device with a valid watchdog API.
    /// `unique` and `data` must be shared by every acquisition path for this device.
    /// Other users must not access the driver state unsafely.
    pub(crate) unsafe fn new(
        unique: &Unique,
        data: &'static WatchdogStatic,
        device: *const raw::device,
    ) -> Option<Self> {
        if !unique.once() {
            return None;
        }
        Some(Self { device, data })
    }

    /// Check whether the device initialized successfully.
    pub fn is_ready(&self) -> bool {
        // SAFETY: Construction requires a static Zephyr device.
        unsafe { raw::device_is_ready(self.device) }
    }

    /// Install a timeout before setup and return its channel.
    pub fn install_timeout(&mut self, timeout: &TimeoutConfig) -> Result<WatchdogChannel> {
        let generation = self.data.lock(self.device)?;
        let config = raw::wdt_timeout_cfg {
            window: raw::wdt_window {
                min: timeout.min_ms,
                max: timeout.max_ms,
            },
            callback: None,
            #[cfg(CONFIG_WDT_MULTISTAGE)]
            next: core::ptr::null_mut(),
            flags: timeout.reset as u8,
        };
        // SAFETY: The device has a watchdog API. The fully initialized single-stage
        // configuration remains live throughout this synchronous call.
        let id = to_result(unsafe { raw::wdt_install_timeout(self.device, &config) })?;
        Ok(WatchdogChannel {
            device: self.device,
            data: self.data,
            generation: *generation,
            id,
        })
    }

    /// Start the watchdog with the installed timeouts and given options.
    ///
    /// After success every installed channel must be fed periodically.
    pub fn setup(&mut self, options: Options) -> Result<()> {
        let _guard = self.data.lock(self.device)?;
        // SAFETY: The device is a static, ready watchdog, with driver access locked.
        to_result_void(unsafe { raw::wdt_setup(self.device, options.bits()) })
    }

    /// Disable the watchdog and uninstall all timeouts.
    ///
    /// On success all previously returned channels are invalid; on failure they
    /// remain valid. Not all watchdogs can be disabled or restarted.
    ///
    /// Returns `EOVERFLOW` without calling the driver if the channel generation
    /// counter is exhausted, rather than allowing stale channels to become valid.
    pub fn disable(&mut self) -> Result<()> {
        let mut generation = self.data.lock(self.device)?;
        let next = generation.checked_add(1).ok_or(Error(raw::EOVERFLOW))?;
        // SAFETY: The device is a static, ready watchdog. The lock also excludes
        // feeds until successful disable has invalidated all existing channels.
        to_result_void(unsafe { raw::wdt_disable(self.device) })?;
        *generation = next;
        Ok(())
    }
}
