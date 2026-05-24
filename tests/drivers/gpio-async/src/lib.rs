// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

extern crate alloc;

use embassy_time::{Duration, Ticker};
use zephyr::{
    device::gpio::GpioPin,
    embassy::Executor,
    raw::{GPIO_PULL_DOWN, ZR_GPIO_INPUT, ZR_GPIO_OUTPUT_ACTIVE},
};

use embassy_executor::Spawner;
use log::info;
use static_cell::StaticCell;

static EXECUTOR_MAIN: StaticCell<Executor> = StaticCell::new();

#[no_mangle]
extern "C" fn rust_main() {
    unsafe {
        zephyr::set_logger().unwrap();
    }

    let executor = EXECUTOR_MAIN.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(main(spawner)).unwrap();
    })
}

#[embassy_executor::task]
async fn main(spawner: Spawner) {
    info!("Hello world");
    let _ = spawner;

    let mut col0 = zephyr::devicetree::labels::col0::get_instance().unwrap();
    let mut row0 = zephyr::devicetree::labels::row0::get_instance().unwrap();

    col0.configure(ZR_GPIO_OUTPUT_ACTIVE);
    col0.set(true);
    row0.configure(ZR_GPIO_INPUT | GPIO_PULL_DOWN);

    loop {
        unsafe { row0.wait_for_high().await };
        // Simple debounce, Wait for 20 consecutive high samples.
        debounce(&mut row0, true).await;
        info!("Pressed");
        unsafe { row0.wait_for_low().await };
        debounce(&mut row0, false).await;
        info!("Released");
    }
}

/// Simple debounce.  Scan the gpio periodically, and return when we have 20 consecutive samples of
/// the intended value.
async fn debounce(pin: &mut GpioPin, level: bool) {
    let mut count = 0;
    let mut ticker = Ticker::every(Duration::from_millis(1));
    loop {
        ticker.next().await;

        if pin.get() == level {
            count += 1;

            if count >= 20 {
                return;
            }
        } else {
            count = 0;
        }
    }
}
