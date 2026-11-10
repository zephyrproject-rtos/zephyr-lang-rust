// SPDX-License-Identifier: Apache-2.0

#![no_std]

use embedded_io::{Read, Write};
use zephyr::printkln;
use zephyr_panic as _;

const MESSAGE: &[u8] = b"Hello from blocking Rust UART!\r\n";

#[no_mangle]
extern "C" fn rust_main() {
    let mut uart =
        zephyr::devicetree::aliases::app_uart::get_instance().expect("No loopback UART alias");
    assert!(uart.is_ready(), "UART is not ready");
    assert!(uart.poll_in().unwrap().is_none());

    uart.poll_out(0xa5).unwrap();
    assert_eq!(uart.poll_in().unwrap(), Some(0xa5));

    uart.write_all(MESSAGE).unwrap();
    let mut received = [0; MESSAGE.len()];
    uart.read_exact(&mut received).unwrap();
    assert_eq!(received, MESSAGE);
    printkln!("UART loopback: received {} bytes", received.len());

    let binary = [0, 0x7f, 0x80, 0xff];
    uart.write_all(&binary).unwrap();
    let mut received = [0; 4];
    uart.read_exact(&mut received).unwrap();
    assert_eq!(received, binary);
    assert_eq!(uart.read(&mut []).unwrap(), 0);
    assert_eq!(uart.write(&[]).unwrap(), 0);
    assert!(uart.poll_in().unwrap().is_none());

    // Polling cannot establish hardware TX completion, even after loopback RX.
    assert_eq!(uart.flush().unwrap_err().0, zephyr::raw::ENOTSUP);
    printkln!("UART loopback: all tests passed");
}
