// Copyright (c) 2026 Open Device Partnership and Contributors
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use core::ffi::c_int;
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use zephyr::device::fuel_gauge::{FuelGauge, FuelGaugeString};

// Entry point into the Rust program from Zephyr.
#[unsafe(no_mangle)]
extern "C" fn rust_main() {
    const MAIN_PRIO: c_int = 2;

    // SAFETY: `rust_main` runs once during application startup before any rust tasks
    // are spawned, so the global logger is initialized before concurrent use.
    unsafe {
        zephyr::set_logger().unwrap();
    }

    // SAFETY: `rust_main` runs in a thread context.
    unsafe {
        zephyr::raw::k_thread_priority_set(zephyr::raw::k_current_get(), MAIN_PRIO);
    }

    log::info!("Initialized rust_main.");

    // Spawn the main task.
    static EXECUTOR_MAIN: StaticCell<zephyr::embassy::Executor> = StaticCell::new();
    let executor = EXECUTOR_MAIN.init(zephyr::embassy::Executor::new());
    executor.run(|spawner| {
        spawner
            .spawn(fuel_gauge())
            .expect("Failed to spawn fuel_gauge()");
    })
}

/// Struct to store the fuel gauge data we want to get.
#[allow(dead_code)]
#[derive(Debug)]
struct FuelGaugeData {
    current_ua: i32,
    voltage_uv: i32,
    temperature_dk: u16,
    remaining_capacity_uah: u32,
    runtime_to_empty_mins: u32,
    manufacturer_name: FuelGaugeString,
    device_name: FuelGaugeString,
    device_chemistry: FuelGaugeString,
}

impl FuelGaugeData {
    /// Reads data from `gauge` and packs it into a struct.
    pub fn get(gauge: &FuelGauge) -> Result<Self, zephyr::error::Error> {
        Ok(Self {
            current_ua: gauge
                .current()
                .inspect_err(|err| log::error!("Failed to read fuel gauge current: {}", err))?,
            voltage_uv: gauge
                .voltage()
                .inspect_err(|err| log::error!("Failed to read fuel gauge voltage: {}", err))?,
            temperature_dk: gauge
                .temperature()
                .inspect_err(|err| log::error!("Failed to read fuel gauge temperature: {}", err))?,
            remaining_capacity_uah: gauge.remaining_capacity().inspect_err(|err| {
                log::error!("Failed to read fuel gauge remaining_capacity: {}", err)
            })?,
            runtime_to_empty_mins: gauge.runtime_to_empty().inspect_err(|err| {
                log::error!("Failed to read fuel gauge runtime_to_empty: {}", err)
            })?,
            manufacturer_name: gauge.manufacturer_name().inspect_err(|err| {
                log::error!("Failed to read fuel gauge manufacturer_name: {}", err)
            })?,
            device_name: gauge
                .device_name()
                .inspect_err(|err| log::error!("Failed to read fuel gauge device_name: {}", err))?,
            device_chemistry: gauge.device_chemistry().inspect_err(|err| {
                log::error!("Failed to read fuel gauge device_chemistry: {}", err)
            })?,
        })
    }
}

#[embassy_executor::task]
async fn fuel_gauge() {
    let gauge: FuelGauge = zephyr::devicetree::labels::fuel_gauge::get_instance()
        .expect("Failed to call zephyr::devicetree::labels::fuel_gauge::get_instance()");

    if !gauge.is_ready() {
        log::error!("Fuel Gauge is not ready to use!");
        return;
    }

    loop {
        if let Ok(data) = FuelGaugeData::get(&gauge) {
            log::info!("{:#?}", data);
        }

        Timer::after(Duration::from_secs(1)).await;
    }
}
