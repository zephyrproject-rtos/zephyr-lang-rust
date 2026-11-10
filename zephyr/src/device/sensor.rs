// Copyright (c) 2026 Open Device Partnership and Contributors
// SPDX-License-Identifier: Apache-2.0

//! Device wrappers for Zephyr sensors.
//!
//! Safety: Several methods present an interface where an input/output buffer is passed
//! as an argument, but unfortunately there is no way to know generically what size buffer the
//! underlying driver expects.
//!
//! For example, most drivers will treat the ambient temp channel as a single scalar value,
//! but some drivers have multiple ambient temp channels (with no way to differentiate with the
//! legacy API which does not include a channel index) and so will read/write multiple values.
//!
//! Thus, creating a fully safe, driver-agnostic wrapper around these methods is infeasible as it
//! requires additional knowledge of the specific driver's buffer requirements.
//! The caller is therefore responsible for passing correctly sized buffers.

use super::Unique;
use crate::{error::to_result_void, raw, Result};

mod legacy;

/// A Zephyr sensor attribute.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SensorAttribute(raw::sensor_attribute);

impl SensorAttribute {
    /// Construct from a raw Zephyr sensor attribute.
    pub const fn from_raw(value: raw::sensor_attribute) -> Self {
        Self(value)
    }

    /// Return the raw Zephyr sensor attribute.
    pub const fn into_raw(self) -> raw::sensor_attribute {
        self.0
    }
}

/// A Zephyr sensor channel.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SensorChannel(raw::sensor_channel);

impl SensorChannel {
    /// Construct from a raw Zephyr sensor channel.
    pub const fn from_raw(value: raw::sensor_channel) -> Self {
        Self(value)
    }

    /// Return the raw Zephyr sensor channel.
    pub const fn into_raw(self) -> raw::sensor_channel {
        self.0
    }
}

/// A Zephyr sensor value.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SensorValue(raw::sensor_value);

impl SensorValue {
    /// Construct from a raw Zephyr sensor value.
    pub const fn from_raw(value: raw::sensor_value) -> Self {
        Self(value)
    }

    /// Return the raw Zephyr sensor value.
    pub const fn into_raw(self) -> raw::sensor_value {
        self.0
    }
}

// The mutex is only needed for the case where an async method that offloads to the blocking worker
// pool is cancelled, which could result in the background worker thread still accessing
// the raw sensor (which the borrow checker has no way of knowing).
//
// If not using async methods backed by the blocking pool, we can omit it for slightly more efficiency.
cfg_if::cfg_if! {
    if #[cfg(all(feature = "async-drivers", CONFIG_RUST_BLOCKING_POOL))] {
        use crate::sync::{Mutex, MutexGuard};

        /// Per-sensor static data for certain async operations.
        #[doc(hidden)]
        pub struct SensorStatic {
            lock: Mutex<()>,
        }

        impl SensorStatic {
            pub(crate) const fn new() -> Self {
                Self {
                    lock: Mutex::new(()),
                }
            }

            fn lock(&self) -> MutexGuard<'_, ()> {
                self.lock.lock().expect("Locking is infallible")
            }
        }
    } else {
        /// Per-sensor static data for certain async operations.
        #[doc(hidden)]
        pub struct SensorStatic {}

        struct NoopGuard;

        impl SensorStatic {
            pub(crate) const fn new() -> Self {
                Self {}
            }

            #[inline(always)]
            fn lock(&self) -> NoopGuard {
                NoopGuard
            }
        }
    }
}

/// Allows the raw handle and static state to be copied into worker closures without making the
/// public sensor handle cloneable or copyable.
#[derive(Clone, Copy)]
struct SensorInner {
    device: *const raw::device,
    state: &'static SensorStatic,
}

// SAFETY: The device and state are static.
// Additionally, if async is enabled, workers serialize all driver operations with the shared mutex,
// even after cancellation.
unsafe impl Send for SensorInner {}

impl SensorInner {
    /// # Safety
    ///
    /// Caller must pass a `vals` buffer with the correct number of values that the underlying
    /// driver expects to read for the specific channel and attribute combination.
    unsafe fn attr_set(
        self,
        chan: SensorChannel,
        attr: SensorAttribute,
        vals: &[SensorValue],
    ) -> Result<()> {
        let _guard = self.state.lock();
        // SAFETY: The caller provides a correctly sized buffer, and SensorValue is transparent.
        to_result_void(unsafe {
            raw::sensor_attr_set(
                self.device,
                chan.into_raw(),
                attr.into_raw(),
                vals.as_ptr().cast(),
            )
        })
    }

    /// # Safety
    ///
    /// Caller must pass a `vals` buffer of the correct size that the underlying
    /// driver expects to write into for the specific channel and attribute combination.
    unsafe fn attr_get(
        self,
        chan: SensorChannel,
        attr: SensorAttribute,
        vals: &mut [SensorValue],
    ) -> Result<()> {
        let _guard = self.state.lock();
        // SAFETY: The caller provides a correctly sized buffer, and SensorValue is transparent.
        to_result_void(unsafe {
            raw::sensor_attr_get(
                self.device,
                chan.into_raw(),
                attr.into_raw(),
                vals.as_mut_ptr().cast(),
            )
        })
    }
}

/// A Zephyr sensor device.
///
/// This wrapper currently maps to:
/// - The attribute (sensor_attr_get/sensor_attr_set) API
/// - The legacy (sensor_sample_fetch/sensor_channel_get) API
///
/// The legacy trigger API and modern RTIO (read/decode) API are planned but not yet supported.
pub struct Sensor {
    inner: SensorInner,
}

impl Sensor {
    /// Construct a handle from generated devicetree state.
    ///
    /// # Safety
    ///
    /// `device` must point to a static Zephyr sensor implementing the API methods that the
    /// application will use.
    pub(crate) unsafe fn new(
        unique: &Unique,
        state: &'static SensorStatic,
        device: *const raw::device,
    ) -> Option<Self> {
        if !unique.once() {
            return None;
        }

        Some(Self {
            inner: SensorInner { device, state },
        })
    }

    /// Verify that the underlying sensor device is ready for use.
    pub fn is_ready(&self) -> bool {
        // SAFETY: Construction guarantees a valid, static Zephyr device.
        unsafe { raw::device_is_ready(self.inner.device) }
    }

    /// Set an attribute for the given channel with the given values.
    ///
    /// Values are read from the provided `vals` slice.
    ///
    /// *Note*: The driver may perform bus operations, blocking the calling thread.
    ///
    /// # Safety
    ///
    /// Caller must pass a `vals` buffer with the correct number of values that the underlying
    /// driver expects to read for the specific channel and attribute combination.
    pub unsafe fn blocking_attr_set(
        &mut self,
        chan: SensorChannel,
        attr: SensorAttribute,
        vals: &[SensorValue],
    ) -> Result<()> {
        // SAFETY: The caller guarantees the driver's input buffer requirements.
        unsafe { self.inner.attr_set(chan, attr, vals) }
    }

    /// Get the values for the given channel and attribute.
    ///
    /// Retrieved values are written into the provided `vals` slice.
    ///
    /// *Note*: The driver may perform bus operations, blocking the calling thread.
    ///
    /// # Safety
    ///
    /// Caller must pass a `vals` buffer of the correct size that the underlying
    /// driver expects to write into for the specific channel and attribute combination.
    pub unsafe fn blocking_attr_get(
        &mut self,
        chan: SensorChannel,
        attr: SensorAttribute,
        vals: &mut [SensorValue],
    ) -> Result<()> {
        // SAFETY: The caller guarantees the driver's output buffer requirements.
        unsafe { self.inner.attr_get(chan, attr, vals) }
    }
}

// The attribute API is used regardless of which API is used to read samples from the sensor,
// and since Zephyr provides no intrinsically async API for these methods, the blocking pool
// must be enabled to use these in async Rust, even if using the async read/decode API.
#[cfg(all(feature = "async-drivers", CONFIG_RUST_BLOCKING_POOL))]
impl Sensor {
    /// Set an attribute for the given channel with the given values.
    ///
    /// Values are read from the provided `vals` array.
    ///
    /// *Note*: This offloads the inherently blocking call onto a worker thread.
    /// Thus, unlike [`Self::blocking_attr_set`], this method takes an owned buffer instead of
    /// a slice, since it must move the buffer to the worker thread.
    ///
    /// # Safety
    ///
    /// Caller must ensure `N` matches the correct size that the underlying
    /// driver expects to read for the specific channel and attribute combination.
    ///
    /// # Cancel safety
    ///
    /// See [`crate::blocking::run`].
    ///
    /// Additionally, it is safe to continue making sensor calls, even if the worker thread is still
    /// accessing the sensor, due to the internal mutex.
    pub unsafe fn attr_set<const N: usize>(
        &mut self,
        chan: SensorChannel,
        attr: SensorAttribute,
        vals: [SensorValue; N],
    ) -> impl core::future::Future<Output = Result<()>> + '_ {
        let inner = self.inner;
        // SAFETY: The caller guarantees the driver's input buffer requirements.
        crate::blocking::run(move || unsafe { inner.attr_set(chan, attr, &vals) })
    }

    /// Get the values for the given channel and attribute.
    ///
    /// Retrieved values are returned directly as an array.
    ///
    /// *Note*: This offloads the inherently blocking call onto a worker thread.
    /// Thus, unlike [`Self::blocking_attr_get`], this method returns an owned buffer instead of
    /// writing into a provided buffer, since the worker thread must own the output buffer.
    ///
    /// # Safety
    ///
    /// Caller must ensure `N` matches the correct size that the underlying
    /// driver expects to write for the specific channel and attribute combination.
    ///
    /// # Cancel safety
    ///
    /// See [`crate::blocking::run`].
    ///
    /// Additionally, it is safe to continue making sensor calls, even if the worker thread is still
    /// accessing the sensor, due to the internal mutex.
    pub unsafe fn attr_get<const N: usize>(
        &mut self,
        chan: SensorChannel,
        attr: SensorAttribute,
    ) -> impl core::future::Future<Output = Result<[SensorValue; N]>> + '_ {
        let inner = self.inner;
        crate::blocking::run(move || {
            let mut vals = [SensorValue::default(); N];
            // SAFETY: The caller guarantees the driver's output buffer requirements.
            unsafe { inner.attr_get(chan, attr, &mut vals) }?;
            Ok(vals)
        })
    }
}
