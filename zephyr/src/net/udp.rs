//!
//! docs todo...
//!

use crate::error::Result;
use crate::net::socket::{Domain, Protocol, SockType, Socket};
use core::net::SocketAddr;

/// docs todo...
pub struct UdpSocket {
    sock: Socket,
}

impl UdpSocket {
    /// Create a UDP socket bound to the provided SocketAddr
    pub fn bind(addr: &SocketAddr) -> Result<Self> {
        let domain = match addr {
            SocketAddr::V4(_) => Domain::AfInet,
            SocketAddr::V6(_) => Domain::AfInet6,
        };

        let mut sock = Socket::new(domain, SockType::Dgram, Protocol::IpprotoUdp)?;
        sock.bind(addr)?;

        Ok(Self { sock })
    }

    /// Send data to the specified socket address.
    pub fn send_to(&self, buf: &[u8], addr: &SocketAddr) -> Result<usize> {
        self.sock.send_to(buf, addr)
    }

    /// Receive from the socket, returning the data length and peer address it was received from
    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        self.sock.recv_from(buf)
    }
}
