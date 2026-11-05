// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::{
    device::pwm::PwmFlags,
    printkln,
    time::{sleep, Duration},
};
use zephyr_panic as _;

const CHANNEL: u32 = 1;
const PERIOD_NS: u32 = 40_000;

#[no_mangle]
extern "C" fn rust_main() {
    let mut pwm = zephyr::devicetree::aliases::pwm_controller::get_instance()
        .expect("No PWM controller alias");
    assert!(pwm.is_ready(), "PWM controller is not ready");

    let rate = pwm.cycles_per_sec(CHANNEL).expect("Clock query failed");
    assert!(rate > 0);
    let period = u32::try_from(rate / 25_000).expect("Period does not fit");
    assert!(period > 0);

    for percent in [0_u32, 25, 50, 75, 100] {
        printkln!("Cycle PWM duty: {}%", percent);
        let pulse = (u64::from(period) * u64::from(percent) / 100) as u32;
        pwm.set_cycles(CHANNEL, period, pulse, PwmFlags::NORMAL)
            .unwrap();
        sleep(Duration::millis_at_least(200));
    }
    assert!(pwm
        .set_cycles(CHANNEL, period, period + 1, PwmFlags::NORMAL)
        .is_err());

    for percent in [0_u32, 25, 50, 75, 100] {
        printkln!("Nanosecond PWM duty: {}%", percent);
        pwm.set_ns(
            CHANNEL,
            PERIOD_NS,
            PERIOD_NS * percent / 100,
            PwmFlags::NORMAL,
        )
        .unwrap();
        sleep(Duration::millis_at_least(200));
    }
    pwm.set_ns(CHANNEL, PERIOD_NS, 0, PwmFlags::NORMAL).unwrap();
    printkln!("All tests passed");
}
