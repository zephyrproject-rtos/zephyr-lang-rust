// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use embassy_executor::{Executor, Spawner};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use log::info;
use static_cell::StaticCell;

static EXECUTOR_LOW: StaticCell<Executor> = StaticCell::new();

#[no_mangle]
extern "C" fn rust_main() {
    unsafe {
        zephyr::set_logger().unwrap();
    }

    info!("Hello world from Rust on {}", zephyr::kconfig::CONFIG_BOARD);

    let executor = EXECUTOR_LOW.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(sample_task(spawner)).unwrap();
    })
}

static CHAN: Channel<CriticalSectionRawMutex, usize, 1> = Channel::new();

#[embassy_executor::task]
async fn sample_task(spawner: Spawner) {
    info!("Started once");
    spawner.spawn(other_task(spawner)).unwrap();
    loop {
        // Wait for a message.
        let msg = CHAN.receive().await;
        info!("main task got: {}", msg);
    }
}

#[embassy_executor::task]
async fn other_task(_spawner: Spawner) {
    info!("The other task");
    let mut count = 0;
    loop {
        CHAN.send(count).await;
        count = count.wrapping_add(1);

        Timer::after(Duration::from_secs(1)).await;
    }
}
