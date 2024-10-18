// Copyright (c) 2024 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

// Sigh. The check config system requires that the compiler be told what possible config values
// there might be.  This is completely impossible with both Kconfig and the DT configs, since the
// whole point is that we likely need to check for configs that aren't otherwise present in the
// build.  So, this is just always necessary.
#![allow(unexpected_cfgs)]

use log::warn;

use core::ffi::c_void;

use zephyr::raw::GPIO_OUTPUT_ACTIVE;
use zephyr::time::{ Duration, sleep };

#[no_mangle]
extern "C" fn rust_main() {
    zephyr::set_logger();

    warn!("Starting blinky");
    // println!("Blinky!");
    // Invoke "blink" as a user thread.
    // blink();
    if false {
        unsafe {
            zephyr::raw::k_thread_user_mode_enter
                (Some(blink),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut());
        }
    } else {
        unsafe {
            blink(core::ptr::null_mut(),
                  core::ptr::null_mut(),
                  core::ptr::null_mut());
        }
    }
}

// fn blink() {
unsafe extern "C" fn blink(_p1: *mut c_void, _p2: *mut c_void, _p3: *mut c_void) {
    // Just call a "safe" rust function.
    do_blink();
}

#[cfg(dt = "aliases::led0")]
fn do_blink() {
    warn!("Inside of blinky");

    let mut led0 = zephyr::devicetree::aliases::led0::get_instance().unwrap();
    let mut gpio_token = unsafe { zephyr::device::gpio::GpioToken::get_instance().unwrap() };

    if !led0.is_ready() {
        warn!("LED is not ready");
        loop {
        }
        // return;
    }

    unsafe { led0.configure(&mut gpio_token, GPIO_OUTPUT_ACTIVE); }
    let duration = Duration::millis_at_least(500);
    loop {
        unsafe { led0.toggle_pin(&mut gpio_token); }
        sleep(duration);
    }
}

#[cfg(not(dt = "aliases::led0"))]
fn do_blink() {
    warn!("No leds configured");
    loop {
    }
}
