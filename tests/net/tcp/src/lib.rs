// Copyright (c) 2025 Witekio
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use zephyr::net::{TcpListener, TcpStream};
use zephyr::printkln;

#[no_mangle]
extern "C" fn rust_main() {
    let tx_data: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    let mut server_recv_buf: [u8; 64] = [0; 64];
    let mut client_recv_buf: [u8; 64] = [0; 64];

    // The test creates client and server TcpStreams, and echo's some data between them to verify
    // that the TCP connection is functioning
    printkln!("Starting tests");

    let server_sockaddr = "0.0.0.0:4242"
        .parse()
        .expect("Failed to parse server sockaddr");

    let connect_sockaddr = "127.0.0.1:4242"
        .parse()
        .expect("Failed to parse connect sockaddr");

    let mut listener = TcpListener::bind(&server_sockaddr).expect("Failed to create TcpListener");

    let mut client_stream =
        TcpStream::connect(&connect_sockaddr).expect("Failed to create client TcpStream");

    let sent = client_stream
        .send(&tx_data)
        .expect("Client failed to send data");

    assert_eq!(sent, tx_data.len());

    let (mut server_stream, _) = listener.accept().expect("Failed to accept connection");

    let recvd = server_stream
        .recv(&mut server_recv_buf)
        .expect("Server failed to receive data");

    assert_eq!(recvd, tx_data.len());
    assert_eq!(server_recv_buf[0..recvd], tx_data);

    let sent = server_stream
        .send(&server_recv_buf[0..recvd])
        .expect("Failed to send data");

    assert_eq!(sent, recvd);

    let recvd = client_stream
        .recv(&mut client_recv_buf)
        .expect("Client failed to receive data");

    assert_eq!(recvd, sent);
    assert_eq!(client_recv_buf[0..recvd], tx_data);

    printkln!("All tests passed");
}
