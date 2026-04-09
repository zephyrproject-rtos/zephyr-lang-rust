// Copyright (c) 2026 Linaro LTD
// SPDX-License-Identifier: Apache-2.0

//! I2C controller test.
//!
//! Exercises the Rust I2C controller bindings by talking to a peer running the
//! Zephyr `samples/drivers/i2c/controller_target` sample in target mode.  The
//! target implements a 16-byte register file at I2C address 0x42.

#![no_std]

use zephyr::{
    device::i2c::{I2c, Operation},
    printkln,
    time::{sleep, Duration},
};

/// I2C address of the target device (matches protocol.h SAMPLE_TARGET_ADDR).
const TARGET_ADDR: u16 = 0x42;

/// Test a plain write followed by a plain read.
fn test_write_then_read(i2c: &mut I2c) {
    printkln!("Test: write then read");

    // Write 8 data bytes starting at register 0x00.
    let write_buf: [u8; 9] = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    i2c.write(TARGET_ADDR, &write_buf).expect("write failed");

    sleep(Duration::millis_at_least(10));

    // Read back: first set the register pointer, then read.
    let reg_addr: [u8; 1] = [0x00];
    i2c.write(TARGET_ADDR, &reg_addr)
        .expect("write reg addr failed");

    let mut read_buf = [0u8; 8];
    i2c.read(TARGET_ADDR, &mut read_buf).expect("read failed");

    assert_eq!(
        read_buf,
        [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        "read-back data mismatch"
    );

    printkln!("  write then read: OK");
}

/// Test a combined write/read transaction built with the general `transfer` API.
///
/// Mirrors `test_write_read` but composes the transaction from explicit
/// [`Operation`] entries instead of using the dedicated `write_read` helper.
fn test_transfer(i2c: &mut I2c) {
    printkln!("Test: transfer (write + read)");

    // Seed a known pattern at register 0x08 with a single write operation.
    let write_buf: [u8; 5] = [0x08, 0x10, 0x20, 0x30, 0x40];
    i2c.transfer(TARGET_ADDR, &mut [Operation::Write(&write_buf)])
        .expect("transfer write failed");

    sleep(Duration::millis_at_least(10));

    // Now do a combined write+read in a single transaction: set the register
    // pointer, RESTART, then read 4 bytes.
    let reg_addr: [u8; 1] = [0x08];
    let mut read_buf = [0u8; 4];
    i2c.transfer(
        TARGET_ADDR,
        &mut [Operation::Write(&reg_addr), Operation::Read(&mut read_buf)],
    )
    .expect("transfer write+read failed");

    assert_eq!(read_buf, [0x10, 0x20, 0x30, 0x40], "transfer data mismatch");

    printkln!("  transfer: OK");
}

/// Test a combined write_read (RESTART) transaction.
fn test_write_read(i2c: &mut I2c) {
    printkln!("Test: write_read (combined)");

    // First, write some known data at register 0x04.
    let write_buf: [u8; 5] = [0x04, 0xAA, 0xBB, 0xCC, 0xDD];
    i2c.write(TARGET_ADDR, &write_buf).expect("write failed");

    sleep(Duration::millis_at_least(10));

    // Now do a combined write_read: set register address, then read 4 bytes.
    let reg_addr: [u8; 1] = [0x04];
    let mut read_buf = [0u8; 4];
    i2c.write_read(TARGET_ADDR, &reg_addr, &mut read_buf)
        .expect("write_read failed");

    assert_eq!(
        read_buf,
        [0xAA, 0xBB, 0xCC, 0xDD],
        "write_read data mismatch"
    );

    printkln!("  write_read: OK");
}

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("I2C controller test starting");

    let mut i2c = zephyr::devicetree::aliases::i2c_bus::get_instance()
        .expect("Failed to get I2C bus from devicetree alias");

    assert!(i2c.is_ready(), "I2C device is not ready");
    printkln!("I2C device is ready");

    // Give the target time to initialise and register its callbacks.
    sleep(Duration::millis_at_least(100));

    test_write_then_read(&mut i2c);
    test_write_read(&mut i2c);
    test_transfer(&mut i2c);

    printkln!("All tests passed");
}
