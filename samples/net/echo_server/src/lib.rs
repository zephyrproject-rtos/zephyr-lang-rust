// Copyright (c) 2025 Witekio
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![allow(unexpected_cfgs)]

use log::{error, info, warn};
use static_cell::ConstStaticCell;
use zephyr::error::Result;
use zephyr::kobj_define;
use zephyr::net::{TcpListener, TcpStream, UdpSocket};

#[no_mangle]
extern "C" fn rust_main() {
    unsafe {
        zephyr::set_logger().unwrap();
    }

    // Don't allocate the large RX buffers on the stack
    static UDP_RX_BUF: ConstStaticCell<[u8; 2048]> = ConstStaticCell::new([0; 2048]);
    static TCP_RX_BUF: ConstStaticCell<[u8; 2048]> = ConstStaticCell::new([0; 2048]);
    let udp_rx_buf = UDP_RX_BUF.take();
    let tcp_rx_buf = TCP_RX_BUF.take();

    // Create the sockets
    let sockaddr = "0.0.0.0:4242".parse().unwrap();
    let udp_sock = UdpSocket::bind(&sockaddr).expect("Failed to bind UdpSocket");
    let mut tcp_listener = TcpListener::bind(&sockaddr).expect("Failed to bind TcpListener");

    info!("Echo server bound to address {:?}", sockaddr);

    // Spawn the UDP echo server in a thread
    let udp_thread = UDP_THREAD
        .init_once(UDP_STACK.init_once(()).unwrap())
        .unwrap();

    udp_thread.spawn(move || {
        if let Err(e) = run_udp_echo(udp_sock, udp_rx_buf) {
            error!("UDP echo thread failed with error {:?}", e);
        }
    });

    // Run the TCP echo server in the main thread
    loop {
        match tcp_listener.accept() {
            Ok((sock, peer)) => {
                info!("Accepted connection from peer address {:?}", peer);
                let _ = run_tcp_echo(sock, tcp_rx_buf);
            }
            Err(e) => {
                error!("Failed to accept TCP connection with error {:?}", e);
                break;
            }
        }
    }

    info!("TCP echo server exited");
}

fn run_udp_echo(sock: UdpSocket, buf: &mut [u8]) -> Result<()> {
    loop {
        let (n, peer) = sock.recv_from(buf)?;

        // Note that being able to set the MSG_TRUNC sockopt is not implemented yet so it should
        // not be possible to get n > rx_buf.len(), but it's probably still worth including the
        // check for when this sockopt is implemented.
        let n_trunc = n.min(buf.len());
        if n != n_trunc {
            warn!("Data truncated, got {} / {} bytes", n_trunc, n);
        }

        info!(
            "Echoing {} bytes to peer address {:?} via UDP",
            n_trunc, peer
        );
        let _ = sock.send_to(&buf[0..n_trunc], &peer)?;
    }
}

fn run_tcp_echo(mut sock: TcpStream, buf: &mut [u8]) -> Result<()> {
    loop {
        let n = sock.recv(buf)?;

        if 0 == n {
            info!("TCP client disconected");
            return Ok(());
        }

        info!("Echoing {} bytes via TCP", n);
        let mut remaining = &buf[0..n];
        while !remaining.is_empty() {
            match sock.send(remaining)? {
                0 => {
                    info!("TCP client disconnected");
                    return Ok(());
                }
                n => remaining = &remaining[n..],
            };
        }
    }
}

const STACK_SIZE: usize = zephyr::kconfig::CONFIG_MAIN_STACK_SIZE as usize;

kobj_define! {
    static UDP_THREAD: StaticThread;
    static UDP_STACK: ThreadStack<STACK_SIZE>;
}
