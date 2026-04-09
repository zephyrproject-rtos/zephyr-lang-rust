// Copyright (c) 2026 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::{
    device::led_strip::{LedRgb, LedStrip},
    printkln,
    time::{sleep, Duration},
};

fn show_color(strip: &mut LedStrip, pixels: &mut [LedRgb], r: u8, g: u8, b: u8) {
    for px in pixels.iter_mut() {
        px.r = r;
        px.g = g;
        px.b = b;
    }
    strip.update_rgb(pixels).unwrap();
    sleep(Duration::millis_at_least(500));
}

#[no_mangle]
extern "C" fn rust_main() {
    let mut strip =
        zephyr::devicetree::aliases::led_strip::get_instance().expect("No led-strip alias");

    assert!(strip.is_ready(), "LED strip device is not ready");

    let num_pixels = strip.length();
    printkln!("LED strip has {} pixel(s)", num_pixels);
    assert!(num_pixels > 0, "LED strip reports zero length");

    // Use a fixed-size buffer on the stack. The SparkFun Pro Micro has 1 pixel;
    // adjust if testing on a board with more.
    let mut pixels = [LedRgb::default(); 1];
    assert!(
        num_pixels <= pixels.len(),
        "strip length {} exceeds test buffer size {}",
        num_pixels,
        pixels.len(),
    );
    let pixels = &mut pixels[..num_pixels];

    printkln!("Testing LED strip: cycling through red, green, blue, white, off");

    show_color(&mut strip, pixels, 32, 0, 0); // red
    show_color(&mut strip, pixels, 0, 32, 0); // green
    show_color(&mut strip, pixels, 0, 0, 32); // blue
    show_color(&mut strip, pixels, 32, 32, 32); // white
    show_color(&mut strip, pixels, 0, 0, 0); // off

    printkln!("All tests passed");
}
