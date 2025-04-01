// Copyright (c) 2025 Witekio
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![allow(unexpected_cfgs)]

use log::{error, info, warn};
use static_cell::ConstStaticCell;
use zephyr::error::Result;
use zephyr::net::UdpSocket;

#[no_mangle]
extern "C" fn rust_main() {
    unsafe {
        zephyr::set_logger().unwrap();
    }

    let res = run_echo();

    if let Err(e) = res {
        error!("Echo server terminated with error {}", e);
    }
}

fn run_echo() -> Result<()> {
    // Don't allocate the large RX buffer on the stack
    static RX_BUF: ConstStaticCell<[u8; 2048]> = ConstStaticCell::new([0; 2048]);
    let rx_buf = RX_BUF.take();

    let sockaddr = "0.0.0.0:4242".parse().unwrap();
    let sock = UdpSocket::bind(&sockaddr)?;

    info!("Waiting for UDP packets on port 4242");

    loop {
        let (n, peer) = sock.recv_from(rx_buf)?;

        // Note that being able to set the MSG_TRUNC sockopt is not implemented yet so it should
        // not be possible to get n > rx_buf.len(), but it's probably still worth including the
        // check for when this sockopt is implemented.
        let n_trunc = n.min(rx_buf.len());
        if n != n_trunc {
            warn!("Data truncated, got {} / {} bytes", n_trunc, n);
        }

        info!("Echoing {} bytes back to peer address {:?}", n_trunc, peer);
        let _ = sock.send_to(&rx_buf[0..n_trunc], &peer)?;
    }
}
