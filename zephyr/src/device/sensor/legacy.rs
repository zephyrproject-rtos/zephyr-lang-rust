// Copyright (c) 2026 Open Device Partnership and Contributors
// SPDX-License-Identifier: Apache-2.0

//! Wrapper for legacy sensor fetch/get operations.

use super::{Sensor, SensorChannel, SensorInner, SensorValue};
use crate::{error::to_result_void, raw, Result};

impl SensorInner {
    fn sample_fetch(self) -> Result<()> {
        let _guard = self.state.lock();
        // SAFETY: Construction guarantees the legacy API is implemented.
        to_result_void(unsafe { raw::sensor_sample_fetch(self.device) })
    }

    fn sample_fetch_chan(self, chan: SensorChannel) -> Result<()> {
        let _guard = self.state.lock();
        // SAFETY: Construction guarantees the legacy API is implemented.
        to_result_void(unsafe { raw::sensor_sample_fetch_chan(self.device, chan.into_raw()) })
    }

    /// # Safety
    ///
    /// Caller must pass a `vals` buffer with the correct number of values that the underlying
    /// driver expects to write for the specific channel.
    unsafe fn channel_get(self, chan: SensorChannel, vals: &mut [SensorValue]) -> Result<()> {
        let _guard = self.state.lock();
        // SAFETY: Construction guarantees the legacy API is implemented, and caller is responsible
        // for passing a correctly sized buffer. SensorValue is transparent.
        to_result_void(unsafe {
            raw::sensor_channel_get(self.device, chan.into_raw(), vals.as_mut_ptr().cast())
        })
    }

    /// # Safety
    ///
    /// Caller must pass a `vals` buffer with the correct number of values that the underlying
    /// driver expects to write for the specific channel.
    unsafe fn fetch_and_get(self, chan: SensorChannel, vals: &mut [SensorValue]) -> Result<()> {
        let _guard = self.state.lock();
        // SAFETY: Construction guarantees the legacy API is implemented.
        to_result_void(unsafe { raw::sensor_sample_fetch_chan(self.device, chan.into_raw()) })?;
        // SAFETY: The caller guarantees the driver's output buffer requirements.
        // SensorValue is transparent.
        to_result_void(unsafe {
            raw::sensor_channel_get(self.device, chan.into_raw(), vals.as_mut_ptr().cast())
        })
    }
}

impl Sensor {
    /// Fetch samples from all supported channels into the driver's local cache.
    ///
    /// *Note*: The driver may perform bus operations, blocking the calling thread.
    pub fn blocking_sample_fetch(&mut self) -> Result<()> {
        self.inner.sample_fetch()
    }

    /// Fetch a sample from one channel into the driver's local cache.
    ///
    /// *Note*: The driver may perform bus operations, blocking the calling thread.
    pub fn blocking_sample_fetch_chan(&mut self, chan: SensorChannel) -> Result<()> {
        self.inner.sample_fetch_chan(chan)
    }

    /// Get the cached sample values for the given channel.
    ///
    /// Fetch the relevant channel successfully before reading its cached values.
    ///
    /// Retrieved values are written into the provided `vals` slice.
    ///
    /// *Note*: If `async-drivers` and the blocking pool are enabled, this **may** block
    /// in the event that a previous async method was cancelled and a worker thread is
    /// currently processing a sensor operation and thus the internal mutex is still locked.
    /// But otherwise, this will return immediately.
    ///
    /// # Safety
    ///
    /// Caller must pass a `vals` buffer of the correct size that the underlying
    /// driver expects to write into for the specific channel.
    pub unsafe fn channel_get(
        &mut self,
        chan: SensorChannel,
        vals: &mut [SensorValue],
    ) -> Result<()> {
        // SAFETY: The caller guarantees the driver's output buffer requirements.
        unsafe { self.inner.channel_get(chan, vals) }
    }

    /// Fetch and get sample values for the given channel.
    ///
    /// Retrieved values are written into the provided `vals` slice. Other operations through
    /// this wrapper cannot run between fetch and get. If fetching fails, no get is attempted.
    ///
    /// *Note*: The driver may perform bus operations, blocking the calling thread.
    ///
    /// # Safety
    ///
    /// Caller must pass a `vals` buffer of the correct size that the underlying
    /// driver expects to write into for the specific channel.
    pub unsafe fn blocking_fetch_and_get(
        &mut self,
        chan: SensorChannel,
        vals: &mut [SensorValue],
    ) -> Result<()> {
        // SAFETY: The caller guarantees the driver's output buffer requirements.
        unsafe { self.inner.fetch_and_get(chan, vals) }
    }
}

#[cfg(all(feature = "async-drivers", CONFIG_RUST_BLOCKING_POOL))]
impl Sensor {
    /// Fetch samples from all supported channels into the driver's local cache.
    ///
    /// *Note*: This offloads the inherently blocking call onto a worker thread.
    ///
    /// # Cancel safety
    ///
    /// See [`crate::blocking::run`].
    ///
    /// Additionally, it is safe to continue making sensor calls, even if the worker thread is still
    /// accessing the sensor, due to the internal mutex.
    ///
    /// After this future is dropped, its fetch may run between a later fetch and
    /// [`Self::channel_get`], replacing the cached values. Use [`Self::fetch_and_get`]
    /// to keep fetch and get together.
    pub fn sample_fetch(&mut self) -> impl core::future::Future<Output = Result<()>> + '_ {
        let inner = self.inner;
        crate::blocking::run(move || inner.sample_fetch())
    }

    /// Fetch a sample from one channel into the driver's local cache.
    ///
    /// *Note*: This offloads the inherently blocking call onto a worker thread.
    ///
    /// # Cancel safety
    ///
    /// See [`crate::blocking::run`].
    ///
    /// Additionally, it is safe to continue making sensor calls, even if the worker thread is still
    /// accessing the sensor, due to the internal mutex.
    ///
    /// After this future is dropped, its fetch may run between a later fetch and
    /// [`Self::channel_get`], replacing the cached values. Use [`Self::fetch_and_get`]
    /// to keep fetch and get together.
    pub fn sample_fetch_chan(
        &mut self,
        chan: SensorChannel,
    ) -> impl core::future::Future<Output = Result<()>> + '_ {
        let inner = self.inner;
        crate::blocking::run(move || inner.sample_fetch_chan(chan))
    }

    /// Fetch and get sample values for the given channel.
    ///
    /// Retrieved values are returned directly as an array. The fetch and get calls share one
    /// mutex guard, so other operations through this wrapper cannot run between them.
    /// If fetching fails, no get is attempted.
    ///
    /// *Note*: This offloads both inherently blocking calls onto a worker thread.
    ///
    /// # Safety
    ///
    /// Caller must ensure `N` matches the correct size that the underlying
    /// driver expects to write for the specific channel.
    ///
    /// # Cancel safety
    ///
    /// See [`crate::blocking::run`].
    ///
    /// Additionally, it is safe to continue making sensor calls, even if the worker thread is still
    /// accessing the sensor, due to the internal mutex.
    pub unsafe fn fetch_and_get<const N: usize>(
        &mut self,
        chan: SensorChannel,
    ) -> impl core::future::Future<Output = Result<[SensorValue; N]>> + '_ {
        let inner = self.inner;
        crate::blocking::run(move || {
            let mut vals = [SensorValue::default(); N];
            // SAFETY: The caller guarantees the driver's output buffer requirements.
            unsafe { inner.fetch_and_get(chan, &mut vals) }?;
            Ok(vals)
        })
    }
}
