// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::device::watchdog::{Options, Reset, TimeoutConfig, WatchdogChannel};
use zephyr::printkln;
use zephyr::time::{sleep, Duration};
use zephyr_panic as _;

#[zephyr::thread(stack_size = 2048)]
fn feed_watchdog(mut channel: WatchdogChannel) {
    for _ in 0..5 {
        printkln!("Feeding watchdog from worker...");
        channel.feed().expect("Cannot feed watchdog");
        sleep(Duration::millis_at_least(250));
    }
}

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("Watchdog sample application from Rust");

    let mut watchdog =
        zephyr::devicetree::aliases::watchdog0::get_instance().expect("Watchdog already acquired");
    let timeout = TimeoutConfig::new(0, 1000, Reset::Soc).expect("Invalid watchdog window");
    let channel = watchdog
        .install_timeout(&timeout)
        .expect("Cannot install watchdog timeout");
    watchdog
        .setup(Options::default())
        .expect("Cannot setup and start watchdog");

    feed_watchdog(channel)
        .start()
        .join()
        .expect("Cannot join watchdog worker");
    printkln!("Watchdog fed 5 times");
    printkln!("Waiting for reset...");

    loop {
        sleep(Duration::millis_at_least(250));
    }
}
