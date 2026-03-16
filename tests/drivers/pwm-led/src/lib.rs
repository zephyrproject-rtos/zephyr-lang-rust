// Copyright (c) 2026 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::{
    device::led::Led,
    printkln,
    time::{sleep, Duration},
};

fn test_led(name: &str, led: &mut Led) {
    assert!(led.is_ready(), "{} controller is not ready", name);

    printkln!("Testing {} through LED API", name);

    led.off().unwrap();
    sleep(Duration::millis_at_least(200));

    for value in [0_u8, 25, 50, 75, 100] {
        printkln!("Setting {} brightness to {}%", name, value);
        led.set_brightness(value).unwrap();
        sleep(Duration::millis_at_least(200));
    }

    assert!(
        led.set_brightness(101).is_err(),
        "brightness values above 100 must fail"
    );

    led.on().unwrap();
    sleep(Duration::millis_at_least(200));

    led.off().unwrap();
}

#[no_mangle]
extern "C" fn rust_main() {
    let mut led0 =
        zephyr::devicetree::aliases::pwm_led0::get_instance().expect("No pwm-led0 alias");
    let mut led1 =
        zephyr::devicetree::aliases::pwm_led1::get_instance().expect("No pwm-led1 alias");
    let mut led2 =
        zephyr::devicetree::aliases::pwm_led2::get_instance().expect("No pwm-led2 alias");

    led0.off().unwrap();
    led1.off().unwrap();
    led2.off().unwrap();
    sleep(Duration::millis_at_least(200));

    test_led("pwm-led0", &mut led0);
    test_led("pwm-led1", &mut led1);
    test_led("pwm-led2", &mut led2);

    printkln!("All tests passed");
}
