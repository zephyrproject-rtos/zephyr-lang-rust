// SPDX-License-Identifier: Apache-2.0

//! Synchronous watchdog support.
//!
//! Install timeouts before calling [`Watchdog::setup`], then feed every installed
//! channel within its window. Hardware capabilities, including whether a running
//! watchdog can be disabled or restarted, depend on the driver.
//!
//! Callbacks and multistage timeouts are not exposed. With `CONFIG_WDT_MULTISTAGE`,
//! installed timeouts still have a null next-stage pointer.
//!
//! Operations return `ENODEV` if the device is not ready; otherwise driver errors
//! are propagated without retries.

use core::ffi::c_int;

use super::{NoStatic, Unique};
use crate::error::{to_result, to_result_void, Error, Result};
use crate::raw;

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

/// A channel returned by [`Watchdog::install_timeout`].
///
/// Only use it with the watchdog that installed it, until that watchdog is
/// successfully disabled. Drivers may reuse channel numbers on reinstallation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChannelId {
    device: *const raw::device,
    id: c_int,
}

/// A watchdog acquired through devicetree `get_instance()`.
///
/// Acquisition and dropping this handle do not start, feed, or stop the watchdog.
/// In particular, dropping an active watchdog leaves it running. Unique Rust
/// acquisition does not reserve the device against C users; applications must
/// coordinate with them and with any watchdog started at boot.
pub struct Watchdog {
    device: *const raw::device,
}

impl Watchdog {
    /// Constructor used by generated devicetree code.
    ///
    /// # Safety
    ///
    /// `device` must point to a static Zephyr device with a valid watchdog API.
    /// `unique` must be shared by every acquisition path for this device.
    /// Other users must not access the driver state unsafely.
    pub(crate) unsafe fn new(
        unique: &Unique,
        _static: &NoStatic,
        device: *const raw::device,
    ) -> Option<Self> {
        if !unique.once() {
            return None;
        }
        Some(Self { device })
    }

    /// Check whether the device initialized successfully.
    pub fn is_ready(&self) -> bool {
        // SAFETY: Construction requires a static Zephyr device.
        unsafe { raw::device_is_ready(self.device) }
    }

    fn ensure_ready(&self) -> Result<()> {
        if self.is_ready() {
            Ok(())
        } else {
            Err(Error(raw::ENODEV))
        }
    }

    /// Install a timeout before setup and return its channel.
    pub fn install_timeout(&mut self, timeout: &TimeoutConfig) -> Result<ChannelId> {
        self.ensure_ready()?;
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
        to_result(unsafe { raw::wdt_install_timeout(self.device, &config) }).map(|id| ChannelId {
            device: self.device,
            id,
        })
    }

    /// Start servicing the installed timeouts with the given global options.
    ///
    /// After success every installed channel must be fed periodically.
    pub fn setup(&mut self, options: Options) -> Result<()> {
        self.ensure_ready()?;
        // SAFETY: The device is a static, ready watchdog.
        to_result_void(unsafe { raw::wdt_setup(self.device, options.bits()) })
    }

    /// Feed a channel returned by this device's `install_timeout`.
    ///
    /// A channel belonging to another device is rejected with `EINVAL`.
    /// `EAGAIN` is returned without retrying when feeding would stall the caller.
    pub fn feed(&mut self, channel: ChannelId) -> Result<()> {
        self.ensure_ready()?;
        if channel.device != self.device {
            return Err(Error(raw::EINVAL));
        }
        // SAFETY: The device is a static, ready watchdog; the driver checks the ID.
        to_result_void(unsafe { raw::wdt_feed(self.device, channel.id) })
    }

    /// Disable the watchdog and uninstall all timeouts.
    ///
    /// On success all previously returned channel IDs are invalid. Not all
    /// watchdogs can be disabled or restarted.
    pub fn disable(&mut self) -> Result<()> {
        self.ensure_ready()?;
        // SAFETY: The device is a static, ready watchdog.
        to_result_void(unsafe { raw::wdt_disable(self.device) })
    }
}
