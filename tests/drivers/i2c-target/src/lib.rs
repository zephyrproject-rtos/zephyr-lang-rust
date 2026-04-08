// Copyright (c) 2026 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! I2C target test.
//!
//! Implements the same 16-byte register-file protocol as the Zephyr
//! `samples/drivers/i2c/controller_target` sample's target side, so it can be
//! tested against the C controller sample (or the Rust controller test).
//!
//! Protocol:
//! - Write transaction: first byte is the register address, subsequent bytes
//!   are stored starting at that address (pointer auto-increments, wraps at 16).
//! - Read transaction: returns bytes starting at the last register address set
//!   by a preceding write (pointer auto-increments, wraps at 16).
//! - Write-read (RESTART): the register pointer is reset to the start address
//!   from the write phase before serving the read, so the controller reads back
//!   what it just wrote.

#![no_std]

use core::ffi::c_int;

use static_cell::StaticCell;
use zephyr::{
    device::i2c::{i2c_target_callbacks, i2c_target_config},
    printkln,
    raw::sys_snode_t,
    sync::SpinMutex,
};

/// I2C target address (matches protocol.h SAMPLE_TARGET_ADDR).
const TARGET_ADDR: u16 = 0x42;

/// Size of the register file.
const REG_SIZE: usize = 16;

/// Shared state accessed by the ISR callbacks and the main thread.
///
/// Protected by a [`SpinMutex`] so that accesses are properly synchronised
/// and no aliased mutable references are created.  The spin-lock also acts
/// as a memory barrier, ensuring updates made inside one callback are
/// visible to subsequent callbacks or to the main thread.
struct TargetState {
    reg_file: [u8; REG_SIZE],
    /// Current register pointer (auto-incremented by reads and writes).
    reg_ptr: u8,
    /// Register address captured at the start of the write phase; restored
    /// before the read phase of a write-read (RESTART) transaction.
    reg_start: u8,
    /// Position within the current write transaction (0 = register address
    /// byte, 1+ = data bytes).
    write_pos: u8,
}

impl TargetState {
    const fn new() -> Self {
        Self {
            reg_file: [0; REG_SIZE],
            reg_ptr: 0,
            reg_start: 0,
            write_pos: 0,
        }
    }
}

/// Shared target state, protected by a spin-lock.
///
/// The `SpinMutex` ensures that the callbacks (running in ISR context) and
/// the main thread never hold aliased mutable references, and that memory
/// ordering is correct across lock/unlock boundaries.
static STATE: SpinMutex<TargetState> = SpinMutex::new(TargetState::new());

/// The static cell holding the callback table.
static CALLBACKS: StaticCell<i2c_target_callbacks> = StaticCell::new();

/// The static cell holding the target config.
static CONFIG: StaticCell<i2c_target_config> = StaticCell::new();

// ---------------------------------------------------------------------------
// extern "C" callbacks — called from ISR context by the I2C driver.
// ---------------------------------------------------------------------------

unsafe extern "C" fn write_requested(_config: *mut i2c_target_config) -> c_int {
    let mut s = STATE.lock().unwrap();
    s.write_pos = 0;
    0
}

unsafe extern "C" fn write_received(_config: *mut i2c_target_config, val: u8) -> c_int {
    let mut s = STATE.lock().unwrap();
    if s.write_pos == 0 {
        // First byte is the register address.
        s.reg_ptr = val % REG_SIZE as u8;
        s.reg_start = s.reg_ptr;
    } else {
        // Subsequent bytes are data.
        let ptr = s.reg_ptr as usize;
        s.reg_file[ptr] = val;
        s.reg_ptr = (ptr as u8 + 1) % REG_SIZE as u8;
    }
    s.write_pos += 1;
    0
}

unsafe extern "C" fn read_requested(_config: *mut i2c_target_config, val: *mut u8) -> c_int {
    let mut s = STATE.lock().unwrap();
    // On a write-read (RESTART), reset the pointer to the start of the write
    // phase so the controller reads back the data it just wrote.
    s.reg_ptr = s.reg_start;
    let ptr = s.reg_ptr as usize;
    unsafe { *val = s.reg_file[ptr] };
    s.reg_ptr = (ptr as u8 + 1) % REG_SIZE as u8;
    0
}

unsafe extern "C" fn read_processed(_config: *mut i2c_target_config, val: *mut u8) -> c_int {
    let mut s = STATE.lock().unwrap();
    let ptr = s.reg_ptr as usize;
    unsafe { *val = s.reg_file[ptr] };
    s.reg_ptr = (ptr as u8 + 1) % REG_SIZE as u8;
    0
}

unsafe extern "C" fn stop(_config: *mut i2c_target_config) -> c_int {
    0
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("I2C target test starting");

    // Build the callback table.
    let cbs = CALLBACKS.init(i2c_target_callbacks {
        write_requested: Some(write_requested),
        write_received: Some(write_received),
        read_requested: Some(read_requested),
        read_processed: Some(read_processed),
        stop: Some(stop),
        ..Default::default()
    });

    // Build the target config.
    let config = CONFIG.init(i2c_target_config {
        node: sys_snode_t {
            next: core::ptr::null_mut(),
        },
        flags: 0,
        address: TARGET_ADDR,
        callbacks: cbs as *const _,
    });

    // Get the I2C device and register as a target.
    let mut i2c = zephyr::devicetree::aliases::i2c_bus::get_instance()
        .expect("Failed to get I2C bus from devicetree alias");
    assert!(i2c.is_ready(), "I2C device is not ready");

    let _target = unsafe { i2c.register_target(config) }.expect("Failed to register I2C target");

    printkln!("i2c target registered at address 0x{:02x}", TARGET_ADDR);

    // The target is now servicing requests via callbacks.  Keep the main
    // thread alive.
    loop {
        zephyr::time::sleep(zephyr::time::Duration::millis_at_least(1000));
    }
}
