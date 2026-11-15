// SPDX-License-Identifier: Apache-2.0
#![no_std]

use zephyr::raw;
use zephyr_panic as _;

#[no_mangle]
extern "C" fn rust_main() {
    // SAFETY: Startup invokes this once before other Rust tasks use the logger.
    unsafe { zephyr::set_logger().unwrap() };
    let value = raw::app_value { value: 40 };
    // SAFETY: value is initialized and its pointer remains valid throughout app_add.
    assert_eq!(unsafe { raw::app_add(&value, 2) }, 42);
    // SAFETY: app_scaled accepts any integer for which the multiplication does not overflow.
    assert_eq!(unsafe { raw::app_scaled(14) }, 42);
    assert_eq!(raw::APP_ANSWER, 42);
    // SAFETY: k_uptime_get has no preconditions in a running Zephyr application.
    let _ = unsafe { raw::k_uptime_get() };
    log::info!("Custom bindings passed");
}
