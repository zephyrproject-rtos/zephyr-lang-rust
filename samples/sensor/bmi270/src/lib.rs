// Copyright (c) 2025
// SPDX-License-Identifier: Apache-2.0
//
// BMI270 6-axis IMU sample for cy8ckit_062s2_ai, ported from samples/sensor/bmi270 (C) to Rust.
//
// Configures the accelerometer and gyroscope to sample at 100 Hz and writes
// the readings to the console using the Zephyr sensor API bindings.

#![no_std]
#![allow(unexpected_cfgs)]

use core::mem::MaybeUninit;
use log::info;
use zephyr::raw;
use zephyr::time::{sleep, Duration};

#[no_mangle]
extern "C" fn rust_main() {
    // SAFETY: Called once at program startup before any logging occurs,
    // satisfying the logger initialization contract.
    unsafe {
        zephyr::set_logger().unwrap();
    }

    info!("Starting BMI270 sample for cy8ckit_062s2_ai");

    // SAFETY: Gets device from device tree via DEVICE_DT_GET_ONE macro
    let dev = unsafe { raw::zr_device_dt_get_bosch_bmi270() };
    
    if dev.is_null() {
        info!("BMI270 device not found in device tree");
        return;
    }

    handle_device(dev);
}

fn handle_device(dev: *const raw::device) {
    info!("Device found at {:p}", dev);

    // SAFETY: device_is_ready checks if device is valid
    if !unsafe { raw::device_is_ready(dev) } {
        info!("Device is not ready");
        info!("Check:");
        info!("  - BMI270 is connected to I2C0 (P0.2=SCL, P0.3=SDA)");
        info!("  - Device address is 0x68 (SDO pin to GND)");
        info!("  - Power and pull-up resistors are properly connected");
        return;
    }

    info!("Device {:p} is ready", dev);

    // Configure accelerometer: 2G full scale, 100 Hz sampling
    let mut attr_val = raw::sensor_value {
        val1: 2,
        val2: 0,
    };
    // SAFETY: Valid device pointer and sensor_value struct
    unsafe {
        raw::sensor_attr_set(dev, raw::ZR_SENSOR_CHAN_ACCEL_XYZ, raw::sensor_attribute_SENSOR_ATTR_FULL_SCALE, &attr_val);
    }

    attr_val.val1 = 1;
    attr_val.val2 = 0;
    // SAFETY: Valid device pointer and sensor_value struct
    unsafe {
        raw::sensor_attr_set(dev, raw::ZR_SENSOR_CHAN_ACCEL_XYZ, raw::sensor_attribute_SENSOR_ATTR_OVERSAMPLING, &attr_val);
    }

    attr_val.val1 = 100;
    attr_val.val2 = 0;
    // SAFETY: Valid device pointer and sensor_value struct
    unsafe {
        raw::sensor_attr_set(dev, raw::ZR_SENSOR_CHAN_ACCEL_XYZ, raw::sensor_attribute_SENSOR_ATTR_SAMPLING_FREQUENCY, &attr_val);
    }

    info!("Accelerometer configured");

    // Configure gyroscope: 500 dps full scale, 100 Hz sampling
    attr_val.val1 = 500;
    attr_val.val2 = 0;
    // SAFETY: Valid device pointer and sensor_value struct
    unsafe {
        raw::sensor_attr_set(dev, raw::ZR_SENSOR_CHAN_GYRO_XYZ, raw::sensor_attribute_SENSOR_ATTR_FULL_SCALE, &attr_val);
    }

    attr_val.val1 = 1;
    attr_val.val2 = 0;
    // SAFETY: Valid device pointer and sensor_value struct
    unsafe {
        raw::sensor_attr_set(dev, raw::ZR_SENSOR_CHAN_GYRO_XYZ, raw::sensor_attribute_SENSOR_ATTR_OVERSAMPLING, &attr_val);
    }

    attr_val.val1 = 100;
    attr_val.val2 = 0;
    // SAFETY: Valid device pointer and sensor_value struct
    unsafe {
        raw::sensor_attr_set(dev, raw::ZR_SENSOR_CHAN_GYRO_XYZ, raw::sensor_attribute_SENSOR_ATTR_SAMPLING_FREQUENCY, &attr_val);
    }

    info!("Gyroscope configured, starting sampling loop");

    let duration = Duration::millis_at_least(10);
    // SAFETY: Zeroing these arrays is safe; they will be populated by sensor_channel_get
    let mut acc: [raw::sensor_value; 3] = unsafe { MaybeUninit::zeroed().assume_init() };
    // SAFETY: Zeroing these arrays is safe; they will be populated by sensor_channel_get
    let mut gyr: [raw::sensor_value; 3] = unsafe { MaybeUninit::zeroed().assume_init() };

    loop {
        sleep(duration);

        // SAFETY: Valid device pointer, arrays large enough for 3 sensor_value entries
        unsafe {
            raw::sensor_sample_fetch(dev);
            raw::sensor_channel_get(dev, raw::ZR_SENSOR_CHAN_ACCEL_XYZ, acc.as_mut_ptr());
            raw::sensor_channel_get(dev, raw::ZR_SENSOR_CHAN_GYRO_XYZ, gyr.as_mut_ptr());
        }

        info!(
            "AX: {}.{:06}; AY: {}.{:06}; AZ: {}.{:06}; \
             GX: {}.{:06}; GY: {}.{:06}; GZ: {}.{:06};",
            acc[0].val1, acc[0].val2.abs(),
            acc[1].val1, acc[1].val2.abs(),
            acc[2].val1, acc[2].val2.abs(),
            gyr[0].val1, gyr[0].val2.abs(),
            gyr[1].val1, gyr[1].val2.abs(),
            gyr[2].val1, gyr[2].val2.abs(),
        );
    }
}
