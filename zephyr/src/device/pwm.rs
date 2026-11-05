// SPDX-License-Identifier: Apache-2.0

//! PWM controller output.

use super::{NoStatic, Unique};
use crate::{
    error::{to_result_void, Error},
    raw, Result,
};

/// PWM output flags, including controller-specific configuration bits.
///
/// Support for individual flags depends on the underlying driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PwmFlags(raw::pwm_flags_t);

impl PwmFlags {
    /// Active-high pulse.
    pub const NORMAL: Self = Self(raw::PWM_POLARITY_NORMAL as raw::pwm_flags_t);

    /// Active-low pulse.
    pub const INVERTED: Self = Self(raw::PWM_POLARITY_INVERTED as raw::pwm_flags_t);

    /// Retain raw flags without checking whether the driver supports them.
    pub const fn from_bits(bits: raw::pwm_flags_t) -> Self {
        Self(bits)
    }

    /// Return the raw Zephyr flags.
    pub const fn bits(self) -> raw::pwm_flags_t {
        self.0
    }
}

/// A Zephyr PWM controller, obtained through devicetree `get_instance()`.
///
/// Some controllers share a period between channels, so changing one channel's
/// period can affect another. Updates are not necessarily glitch-free, and their
/// latency depends on the driver. A successful call does not wait for the output
/// change to take effect; an error does not guarantee that the output is unchanged.
///
/// Unique acquisition prevents duplicate Rust handles, but does not reserve the
/// controller against C drivers. Applications must coordinate use of shared
/// outputs and periods with those drivers. Dropping this handle does not stop PWM.
pub struct Pwm {
    device: *const raw::device,
}

impl Pwm {
    /// Constructor used by generated devicetree code.
    ///
    /// # Safety
    ///
    /// `device` must point to a static Zephyr PWM device with a valid PWM driver
    /// API. `unique` must be the token for that controller, shared by all paths
    /// that acquire it. Other users must not access driver state unsafely.
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

    /// Check whether the controller is initialized and ready for use.
    pub fn is_ready(&self) -> bool {
        // SAFETY: Construction requires a pointer to a static Zephyr device.
        unsafe { raw::device_is_ready(self.device) }
    }

    fn ensure_ready(&self) -> Result<()> {
        if self.is_ready() {
            Ok(())
        } else {
            Err(Error(raw::ENODEV))
        }
    }

    /// Set a channel's period and active pulse width in hardware clock cycles.
    ///
    /// A zero pulse produces a constant inactive level. A nonzero pulse equal
    /// to the period produces a constant active level. A pulse exceeding the
    /// period returns `EINVAL`; other limits and flags depend on the driver.
    /// Returns `ENODEV` if the controller is not ready, or the driver's error.
    pub fn set_cycles(
        &mut self,
        channel: u32,
        period: u32,
        pulse: u32,
        flags: PwmFlags,
    ) -> Result<()> {
        self.ensure_ready()?;
        // SAFETY: The device is a static, ready PWM controller. Arguments are
        // passed by value, and the mutable receiver prevents shared Rust calls.
        to_result_void(unsafe {
            raw::pwm_set_cycles(self.device, channel, period, pulse, flags.bits())
        })
    }

    /// Set a channel's period and active pulse width in nanoseconds.
    ///
    /// Uses Zephyr's `pwm_set`, which converts durations to cycles with integer
    /// truncation. Zero and full-duty outputs behave as in [`Self::set_cycles`].
    /// Returns `ENODEV` if the controller is not ready, `ENOTSUP` if the converted
    /// durations exceed the cycle range, or an error from the underlying API.
    pub fn set_ns(
        &mut self,
        channel: u32,
        period_ns: u32,
        pulse_ns: u32,
        flags: PwmFlags,
    ) -> Result<()> {
        self.ensure_ready()?;
        // SAFETY: The device is a static, ready PWM controller. Arguments are
        // passed by value, and the mutable receiver prevents shared Rust calls.
        to_result_void(unsafe {
            raw::pwm_set(self.device, channel, period_ns, pulse_ns, flags.bits())
        })
    }

    /// Get a channel's PWM clock rate in cycles per second.
    ///
    /// Returns `ENODEV` if the controller is not ready, or the driver's error.
    pub fn cycles_per_sec(&self, channel: u32) -> Result<u64> {
        self.ensure_ready()?;
        let mut cycles = 0;
        // SAFETY: The device is a static, ready PWM controller, and the output
        // points to a live local value for the duration of this synchronous call.
        to_result_void(unsafe { raw::pwm_get_cycles_per_sec(self.device, channel, &mut cycles) })?;
        Ok(cycles)
    }
}
