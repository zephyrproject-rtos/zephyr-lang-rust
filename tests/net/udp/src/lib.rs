// Copyright (c) 2025 Witekio
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::net::UdpSocket;
use zephyr::printkln;

#[no_mangle]
extern "C" fn rust_main() {
    printkln!("Starting tests");

    let rx_sa = "127.0.0.1:4242"
        .parse()
        .expect("Failed to parse RX SocketAddr");
    let tx_sa = "127.0.0.1:12345"
        .parse()
        .expect("Failed to parse TX SocketAddr");

    let rx_sock = UdpSocket::bind(&rx_sa).expect("Failed to create RX UDP socket");
    let tx_sock = UdpSocket::bind(&tx_sa).expect("Failed to create TX UDP socket");

    let tx_data: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    let sent = tx_sock
        .send_to(&tx_data, &rx_sa)
        .expect("Failed to send data");
    assert_eq!(sent, tx_data.len());

    let mut rx_data: [u8; 8] = [0; 8];
    let (recvd, peer) = rx_sock
        .recv_from(&mut rx_data)
        .expect("Failed to receive data");
    assert_eq!(recvd, tx_data.len());
    assert_eq!(tx_data, rx_data);
    assert_eq!(peer, tx_sa);

    printkln!("All tests passed");
}
