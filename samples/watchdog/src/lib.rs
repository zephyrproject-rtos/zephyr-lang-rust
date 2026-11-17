// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::device::watchdog::{Options, Reset, TimeoutConfig};
use zephyr::printkln;
use zephyr::time::{sleep, Duration};
use zephyr_panic as _;

#[no_mangle]
extern "C" fn rust_main() {
    let mut watchdog =
        zephyr::devicetree::aliases::watchdog0::get_instance().expect("Watchdog already acquired");
    let timeout = TimeoutConfig::new(0, 1000, Reset::Soc).expect("Invalid watchdog window");
    let channel = watchdog
        .install_timeout(&timeout)
        .expect("Cannot install watchdog timeout");
    watchdog
        .setup(Options::default())
        .expect("Cannot start watchdog");

    for _ in 0..5 {
        sleep(Duration::millis_at_least(250));
        watchdog.feed(channel).expect("Cannot feed watchdog");
    }
    printkln!("Watchdog fed 5 times");

    // Some watchdogs cannot be stopped, so keep feeding instead of dropping the handle.
    loop {
        sleep(Duration::millis_at_least(250));
        watchdog.feed(channel).expect("Cannot feed watchdog");
    }
}
