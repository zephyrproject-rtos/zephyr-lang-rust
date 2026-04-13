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

use zephyr::{
    device::i2c::{I2cTargetCallbacks, I2cTargetData},
    printkln,
    sync::SpinMutex,
};

/// I2C target address (matches protocol.h SAMPLE_TARGET_ADDR).
const TARGET_ADDR: u16 = 0x42;

/// Size of the register file.
const REG_SIZE: usize = 16;

/// Internal mutable state, protected by [`SpinMutex`].
struct TargetInner {
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

impl TargetInner {
    const fn new() -> Self {
        Self {
            reg_file: [0; REG_SIZE],
            reg_ptr: 0,
            reg_start: 0,
            write_pos: 0,
        }
    }
}

/// Shared state accessed by the ISR callbacks and the main thread.
///
/// The [`SpinMutex`] ensures that accesses are properly synchronised and no
/// aliased mutable references are created.  The spin-lock also acts as a
/// memory barrier, ensuring updates made inside one callback are visible to
/// subsequent callbacks or to the main thread.
struct TargetState {
    inner: SpinMutex<TargetInner>,
}

impl I2cTargetCallbacks for TargetState {
    fn write_requested(&self) -> zephyr::Result<()> {
        let mut s = self.inner.lock().unwrap();
        s.write_pos = 0;
        Ok(())
    }

    fn write_received(&self, val: u8) -> zephyr::Result<()> {
        let mut s = self.inner.lock().unwrap();
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
        Ok(())
    }

    fn read_requested(&self) -> zephyr::Result<u8> {
        let mut s = self.inner.lock().unwrap();
        // On a write-read (RESTART), reset the pointer to the start of the
        // write phase so the controller reads back the data it just wrote.
        s.reg_ptr = s.reg_start;
        let ptr = s.reg_ptr as usize;
        let val = s.reg_file[ptr];
        s.reg_ptr = (ptr as u8 + 1) % REG_SIZE as u8;
        Ok(val)
    }

    fn read_processed(&self) -> zephyr::Result<u8> {
        let mut s = self.inner.lock().unwrap();
        let ptr = s.reg_ptr as usize;
        let val = s.reg_file[ptr];
        s.reg_ptr = (ptr as u8 + 1) % REG_SIZE as u8;
        Ok(val)
    }
}

/// The I2C target, combining config, callbacks, and shared state in one static.
static TARGET: I2cTargetData<TargetState> = I2cTargetData::new(
    TARGET_ADDR,
    TargetState {
        inner: SpinMutex::new(TargetInner::new()),
    },
);

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("I2C target test starting");

    // Get the I2C device and register as a target.
    let mut i2c = zephyr::devicetree::aliases::i2c_bus::get_instance()
        .expect("Failed to get I2C bus from devicetree alias");
    assert!(i2c.is_ready(), "I2C device is not ready");

    let _target = TARGET
        .register(&mut i2c)
        .expect("Failed to register I2C target");

    printkln!("i2c target registered at address 0x{:02x}", TARGET_ADDR);

    // The target is now servicing requests via callbacks.  Keep the main
    // thread alive.
    loop {
        zephyr::time::sleep(zephyr::time::Duration::millis_at_least(1000));
    }
}
