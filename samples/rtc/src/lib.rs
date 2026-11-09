// SPDX-License-Identifier: Apache-2.0

#![no_std]

use log::info;
use zephyr::device::rtc::RtcTime;
use zephyr::time::{sleep, Duration};
use zephyr_panic as _;

#[no_mangle]
extern "C" fn rust_main() {
    // SAFETY: rust_main runs once at startup, before any Rust tasks are spawned.
    unsafe {
        zephyr::set_logger().unwrap();
    }

    let mut rtc = zephyr::devicetree::aliases::rtc::get_instance().expect("RTC already acquired");
    assert!(rtc.is_ready(), "RTC is not ready");

    // This demonstration overwrites the clock, including on physical hardware.
    // Use whole seconds because RTC precision depends on the driver.
    let initial = RtcTime::new(2024, 2, 29, 12, 34, 0, 0).expect("Invalid sample date");
    rtc.set_time(&initial).expect("Failed to set RTC time");

    let first = rtc.get_time().expect("Failed to read RTC time");
    check_date(&first);
    print_time(&first);

    // Allow a tick between set_time and get_time; do not require exact equality.
    // Poll for at most five seconds instead of assuming a particular tick phase.
    let mut advanced = false;
    for _ in 0..50 {
        sleep(Duration::millis_at_least(100));
        let current = rtc.get_time().expect("Failed to read RTC time");
        check_date(&current);
        if current.second() > first.second() {
            print_time(&current);
            advanced = true;
            break;
        }
    }
    assert!(advanced, "RTC did not advance within five seconds");
    info!("RTC sample successful");

    loop {
        sleep(Duration::secs(1));
        let current = rtc.get_time().expect("Failed to read RTC time");
        print_time(&current);
    }
}

fn check_date(time: &RtcTime) {
    assert_eq!(
        (
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
        ),
        (2024, 2, 29, 12, 34),
        "RTC did not retain the sample date and time"
    );
}

fn print_time(time: &RtcTime) {
    info!(
        "RTC time: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.year(),
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        time.second(),
    );
}
