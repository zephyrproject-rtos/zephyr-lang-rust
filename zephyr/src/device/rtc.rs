// SPDX-License-Identifier: Apache-2.0

//! Blocking real-time clock access.

use super::{NoStatic, Unique};
use crate::{
    error::{to_result_void, Error},
    raw, Result,
};

/// A validated Gregorian calendar date and time, without a timezone.
///
/// Years are full calendar years starting at 1900, and months are numbered 1–12.
/// Leap seconds are not supported. Hardware may impose a narrower year range or
/// lower precision; those restrictions are handled by the underlying driver.
/// Weekday and day of year are derived when setting the clock. Daylight-saving
/// information is not represented, and no timezone conversion is performed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtcTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

impl RtcTime {
    /// Construct a calendar date and time.
    ///
    /// Returns `EINVAL` for years before 1900, invalid dates, hours outside
    /// 0–23, minutes or seconds outside 0–59, or nanoseconds at least 1 billion.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        nanosecond: u32,
    ) -> Result<Self> {
        if year < 1900
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
            || nanosecond >= 1_000_000_000
        {
            return Err(Error(raw::EINVAL));
        }

        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanosecond,
        })
    }

    /// Full calendar year, not an offset from 1900.
    pub fn year(&self) -> u16 {
        self.year
    }

    /// Month, numbered 1–12.
    pub fn month(&self) -> u8 {
        self.month
    }

    /// Day of the month, numbered from 1.
    pub fn day(&self) -> u8 {
        self.day
    }

    /// Hour, numbered 0–23.
    pub fn hour(&self) -> u8 {
        self.hour
    }

    /// Minute, numbered 0–59.
    pub fn minute(&self) -> u8 {
        self.minute
    }

    /// Second, numbered 0–59.
    pub fn second(&self) -> u8 {
        self.second
    }

    /// Nanosecond within the second, less than 1 billion.
    pub fn nanosecond(&self) -> u32 {
        self.nanosecond
    }

    fn to_raw(self) -> raw::rtc_time {
        let mut yearday = i32::from(self.day) - 1;
        for month in 1..self.month {
            yearday += i32::from(days_in_month(self.year, month));
        }

        // Days before this year in the Gregorian calendar. January 1, year 1,
        // was a Monday; Zephyr numbers Sunday as 0. All arithmetic fits i32
        // for the u16 year range.
        let year = i32::from(self.year) - 1;
        let weekday = (365 * year + year / 4 - year / 100 + year / 400 + yearday + 1) % 7;

        raw::rtc_time {
            tm_sec: i32::from(self.second),
            tm_min: i32::from(self.minute),
            tm_hour: i32::from(self.hour),
            tm_mday: i32::from(self.day),
            tm_mon: i32::from(self.month) - 1,
            tm_year: i32::from(self.year) - 1900,
            tm_wday: weekday,
            tm_yday: yearday,
            tm_isdst: -1,
            tm_nsec: self.nanosecond as i32,
        }
    }

    fn from_raw(time: &raw::rtc_time) -> Result<Self> {
        let year = time.tm_year.checked_add(1900).ok_or(Error(raw::EINVAL))?;
        let month = time.tm_mon.checked_add(1).ok_or(Error(raw::EINVAL))?;

        Self::new(
            u16::try_from(year).map_err(|_| Error(raw::EINVAL))?,
            u8::try_from(month).map_err(|_| Error(raw::EINVAL))?,
            u8::try_from(time.tm_mday).map_err(|_| Error(raw::EINVAL))?,
            u8::try_from(time.tm_hour).map_err(|_| Error(raw::EINVAL))?,
            u8::try_from(time.tm_min).map_err(|_| Error(raw::EINVAL))?,
            u8::try_from(time.tm_sec).map_err(|_| Error(raw::EINVAL))?,
            u32::try_from(time.tm_nsec).map_err(|_| Error(raw::EINVAL))?,
        )
    }
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

/// A Zephyr RTC device, obtained through devicetree `get_instance()`.
///
/// Acquisition and dropping the handle do not set or stop the clock. Unique
/// acquisition prevents duplicate Rust handles, but does not reserve the RTC
/// against C users. Applications must coordinate clock changes with those users.
pub struct Rtc {
    device: *const raw::device,
}

impl Rtc {
    /// Constructor used by generated devicetree code.
    ///
    /// # Safety
    ///
    /// `device` must point to a static Zephyr RTC device with a valid RTC driver
    /// API. `unique` must be the token shared by all paths acquiring that device.
    /// Other users must not access driver state unsafely.
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

    /// Check whether the RTC is initialized and ready for use.
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

    /// Read the current calendar date and time.
    ///
    /// Returns `ENODEV` if the device is not ready, the driver's error (including
    /// `ENODATA` if unset), or `EINVAL` if a successful read yields an invalid or
    /// unrepresentable calendar value. Auxiliary weekday, year-day, and
    /// daylight-saving fields are not used to construct the returned value.
    pub fn get_time(&self) -> Result<RtcTime> {
        self.ensure_ready()?;
        // Fully initialize the buffer: drivers may leave auxiliary fields
        // untouched. Invalid sentinels detect missing required calendar fields.
        let mut time = raw::rtc_time {
            tm_sec: -1,
            tm_min: -1,
            tm_hour: -1,
            tm_mday: 0,
            tm_mon: -1,
            tm_year: -1,
            tm_wday: -1,
            tm_yday: -1,
            tm_isdst: -1,
            tm_nsec: 0,
        };
        // SAFETY: The device is a static, ready RTC. The initialized output
        // buffer remains live for the duration of this synchronous call.
        to_result_void(unsafe { raw::rtc_get_time(self.device, &mut time) })?;
        RtcTime::from_raw(&time)
    }

    /// Set the clock to a validated calendar date and time.
    ///
    /// Weekday and year-day are derived from the date; daylight-saving status
    /// is passed as unknown. Returns `ENODEV` if the device is not ready, or the
    /// driver's error. Supported years and precision depend on the driver.
    pub fn set_time(&mut self, time: &RtcTime) -> Result<()> {
        self.ensure_ready()?;
        let time = time.to_raw();
        // SAFETY: The device is a static, ready RTC. RtcTime construction
        // validates the fields, and the converted value is fully initialized
        // and remains live for the duration of this synchronous call.
        to_result_void(unsafe { raw::rtc_set_time(self.device, &time) })
    }
}
