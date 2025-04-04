//! Networking support.
//!
//! This module provides support for networking over UDP.
//!
//! Currently only a blocking networking API is provided, which builds on top of Zephyr's sockets
//! API.

mod ipaddr;
mod socket;

use crate::error::Result;
use crate::net::socket::{Domain, Protocol, SockType, Socket};
use core::ffi::c_int;
use core::net::SocketAddr;

/// UDP socket.
///
/// A UDP socket can be created by binding to a local socket address. Once bound, data can be sent
/// and received using [`send_to`] and [`recv_from`].
///
/// [`send_to`]: UdpSocket::send_to
/// [`recv_from`]: UdpSocket::recv_from
pub struct UdpSocket {
    sock: Socket,
}

impl UdpSocket {
    /// Create a UDP socket bound to the provided socket address.
    pub fn bind(addr: &SocketAddr) -> Result<Self> {
        let domain = Domain::from_socket_addr(addr);

        let mut sock = Socket::new(domain, SockType::Dgram, Protocol::IpProtoUdp)?;
        sock.bind(addr)?;

        Ok(Self { sock })
    }

    /// Send data to the specified socket address, returning the number of bytes that were actually
    /// sent.
    pub fn send_to(&self, buf: &[u8], addr: &SocketAddr) -> Result<usize> {
        self.sock.send_to(buf, addr)
    }

    /// Receive from the socket, returning the data length and peer address it was received from.
    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        self.sock.recv_from(buf)
    }
}

/// TCP Listener
///
/// A TCP listener can be created by binding to a local socket address. It represents a server
/// socket which is ready to accept incoming connections.
///
/// Connections can be accepted by calling [`accept`] on the TcpListener.
///
/// [`accept`]: TcpListener::accept
pub struct TcpListener {
    sock: Socket,
}

impl TcpListener {
    /// Create a TCP listener bound to and listening on the provided socket address.
    pub fn bind(addr: &SocketAddr) -> Result<Self> {
        let domain = Domain::from_socket_addr(addr);

        let mut sock = Socket::new(domain, SockType::Stream, Protocol::IpProtoTcp)?;
        sock.bind(addr)?;
        sock.listen()?;

        Ok(Self { sock })
    }

    /// Accept a single connection from the TCP listener.
    pub fn accept(&mut self) -> Result<(TcpStream, SocketAddr)> {
        let (sock, peer) = self.sock.accept()?;
        Ok((TcpStream { sock }, peer))
    }
}

/// TCP Stream
///
/// This represents a connected TCP socket, which may be either a client or server socket. Data can
/// be sent and received over the stream using the [`send`] and [`recv`] methods.
///
/// There are two ways to get a `TcpStream`:
/// - Call [`accept`] on a [`TcpListener`] to get a connected stream as a server.
/// - Use [`connect`] to get a connected stream as a client
///
/// [`accept`]: TcpListener::accept
/// [`connect`]: TcpStream::connect
/// [`send`]: TcpStream::send
/// [`recv`]: TcpStream::recv
pub struct TcpStream {
    sock: Socket,
}

impl TcpStream {
    /// Create a TCP stream by connecting to the provided socket address.
    pub fn connect(addr: &SocketAddr) -> Result<Self> {
        let domain = Domain::from_socket_addr(addr);

        let mut sock = Socket::new(domain, SockType::Stream, Protocol::IpProtoTcp)?;
        sock.connect(addr)?;

        Ok(Self { sock })
    }

    /// Send data over the TCP stream, returning the number of bytes that were actually sent.
    pub fn send(&mut self, buf: &[u8]) -> Result<usize> {
        self.sock.send(buf)
    }

    /// Receive data from the TCP stream, returning the number of bytes received.
    pub fn recv(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.sock.recv(buf)
    }
}
