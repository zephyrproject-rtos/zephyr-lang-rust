// Copyright (c) 2024 TDK Invensense
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![allow(unexpected_cfgs)]

use log::{info, warn};
use zephyr::raw::{self, sensor_value};
use zephyr::time::{sleep, Duration};

#[no_mangle]
extern "C" fn rust_main() {
    // SAFETY: set_logger must be called once at startup before any logging occurs.
    unsafe {
        zephyr::set_logger().unwrap();
    }

    info!("Starting pressure, temperature and altitude polling sample");

    do_polling();
}

#[cfg(dt = "aliases::pressure_sensor")]
fn do_polling() {
    // SAFETY: Access device pointer from static device tree structure.
    // __device_dts_ord_109 is a static defined by Zephyr's device tree for the DPS368 sensor.
    let dev = unsafe {
        &raw::__device_dts_ord_109 as *const raw::device as *mut raw::device
    };

    // SAFETY: device_is_ready performs a safe check on the device pointer (doesn't dereference).
    let is_ready = unsafe { raw::device_is_ready(dev) };

    if !is_ready {
        warn!("Device is not ready; check driver initialization logs");
        loop {
            core::hint::spin_loop();
        }
    }

    info!("Found pressure sensor device");

    // Main polling loop
    loop {
        // SAFETY: sensor_sample_fetch_chan is called with a valid device pointer from the device tree.
        // This function is safe to call from a single thread during normal operation.
        let ret = unsafe { raw::sensor_sample_fetch_chan(dev, raw::ZR_SENSOR_CHAN_ALL) };

        if ret == 0 {
            // Get pressure value
            let mut pressure = sensor_value {
                val1: 0,
                val2: 0,
            };
            // SAFETY: sensor_channel_get is called with valid device pointer and mutable reference
            // to locally-owned sensor_value struct. No concurrent access.
            unsafe {
                raw::sensor_channel_get(dev, raw::ZR_SENSOR_CHAN_PRESS, &mut pressure);
            }

            // Get temperature value
            let mut temperature = sensor_value {
                val1: 0,
                val2: 0,
            };
            // SAFETY: Same as pressure read above - valid device and local mutable reference.
            unsafe {
                raw::sensor_channel_get(dev, raw::ZR_SENSOR_CHAN_AMBIENT_TEMP, &mut temperature);
            }

            // Try to get altitude value (optional)
            let mut altitude = sensor_value {
                val1: 0,
                val2: 0,
            };
            // SAFETY: Same as pressure and temperature reads - valid device and local mutable reference.
            let altitude_ret = unsafe {
                raw::sensor_channel_get(dev, raw::ZR_SENSOR_CHAN_ALTITUDE, &mut altitude)
            };

            // Display the readings (all calculations are safe, working with local copies)
            let temp_d = (temperature.val1 as f64) + (temperature.val2 as f64) / 1_000_000.0;
            let press_d = (pressure.val1 as f64) + (pressure.val2 as f64) / 1_000_000.0;

            if altitude_ret == 0 {
                let alt_d = (altitude.val1 as f64) + (altitude.val2 as f64) / 1_000_000.0;
                info!(
                    "temp {:.2} Cel, pressure {:.1} kPa, altitude {:.1} m",
                    temp_d, press_d, alt_d,
                );
            } else {
                info!("temp {:.2} Cel, pressure {:.1} kPa", temp_d, press_d);
            }
        }

        sleep(Duration::millis_at_least(1000));
    }
}

#[cfg(not(dt = "aliases::pressure_sensor"))]
fn do_polling() {
    warn!("No pressure_sensor alias found in device tree");
    loop {
        core::hint::spin_loop();
    }
}
