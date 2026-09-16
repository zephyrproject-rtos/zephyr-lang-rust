// Copyright (c) 2026 Open Device Partnership and Contributors
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use embassy_executor::Spawner;
use static_cell::StaticCell;
use zephyr::{
    blocking,
    embassy::Executor,
    printkln,
    sys::uptime_get,
    time::{Duration, Tick},
};
use zephyr_panic as _;

#[no_mangle]
extern "C" fn rust_main() {
    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());

    executor.run(|spawner: Spawner| {
        spawner.spawn(sleep_print("fast", 200).unwrap());
        spawner.spawn(sleep_print("slow", 700).unwrap());
    })
}

#[embassy_executor::task(pool_size = 2)]
async fn sleep_print(name: &'static str, period: u64) {
    let delay = Duration::millis(period as Tick);

    loop {
        // In reality you would use an embassy Timer here natively,
        // but a Zephyr blocking sleep call is good for demonstration purposes
        //
        // Note: We must move here because the closure cannot borrow `delay`
        // from the caller's stack since it might outlive the caller on the worker thread
        blocking::run(move || zephyr::time::sleep(delay)).await;
        printkln!(
            "[{:>5} ms] {} finished a {} ms blocking call",
            uptime_get(),
            name,
            period
        );
    }
}
