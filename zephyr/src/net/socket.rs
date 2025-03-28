//! Low-level network socket support
//!
//! A minimal safe Rust wrapper around Zephyr's sockets API, intended for higher level Rust
//! abstractions to be implemented on top of.

use crate::error::{to_result_errno, Result};
use crate::net::ipaddr::{try_sockaddr_from_c, try_sockaddr_to_c};
use crate::raw::{
    sockaddr, socklen_t, zsock_bind, zsock_close, zsock_recvfrom, zsock_sendto, zsock_socket,
};
use core::ffi::{c_int, c_void};
use core::mem::MaybeUninit;
use core::net::SocketAddr;

#[derive(Debug, Copy, Clone)]
pub(crate) enum Domain {
    AfInet = crate::raw::AF_INET as isize,
    AfInet6 = crate::raw::AF_INET6 as isize,
}

#[derive(Debug, Copy, Clone)]
pub(crate) enum SockType {
    Dgram = crate::raw::net_sock_type_SOCK_DGRAM as isize,
}

#[derive(Debug, Copy, Clone)]
pub(crate) enum Protocol {
    IpprotoUdp = crate::raw::net_ip_protocol_IPPROTO_UDP as isize,
}

/// Socket type implementing minimal safe wrappers around Zephyr's C socket API
pub(crate) struct Socket {
    fd: c_int,
}

impl Socket {
    /// Create a socket
    ///
    /// This is a minimal wrapper around zsock_socket
    pub(crate) fn new(domain: Domain, sock_type: SockType, protocol: Protocol) -> Result<Socket> {
        let res = unsafe { zsock_socket(domain as c_int, sock_type as c_int, protocol as c_int) };

        let fd = to_result_errno(res)?;

        Ok(Socket { fd })
    }

    /// Bind a socket
    ///
    /// This is a minimal wrapper around zsock_bind
    pub(crate) fn bind(&mut self, addr: &SocketAddr) -> Result<()> {
        let (sockaddr, socklen) = try_sockaddr_to_c(addr)?;

        let res = unsafe { zsock_bind(self.fd, &sockaddr as *const sockaddr, socklen) };

        let _ = to_result_errno(res)?;
        Ok(())
    }

    /// Receive from the socket, returning the data length and peer address it was received from
    ///
    /// This is a minimal wrapper around zsock_recvfrom
    pub(crate) fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let mut sa = MaybeUninit::<sockaddr>::uninit();
        let mut socklen: socklen_t = core::mem::size_of::<sockaddr>();

        let res = unsafe {
            zsock_recvfrom(
                self.fd,
                buf.as_mut_ptr() as *mut c_void,
                buf.len(),
                0,
                sa.as_mut_ptr(),
                &mut socklen as *mut socklen_t,
            )
        };

        let recvd = to_result_errno(res)?;

        // SAFETY: `zsock_recvfrom` returned a success code, so it has populated the sockaddr.
        let sa = unsafe { sa.assume_init() };

        let peer_sa = try_sockaddr_from_c(&sa, socklen)?;
        Ok((recvd as usize, peer_sa))
    }

    /// Send data to the specified socket address.
    ///
    /// This is a minimal wrapper around zsock_sendto
    pub fn send_to(&self, buf: &[u8], addr: &SocketAddr) -> Result<usize> {
        let (sa, socklen) = try_sockaddr_to_c(addr)?;

        let res = unsafe {
            zsock_sendto(
                self.fd,
                buf.as_ptr() as *const c_void,
                buf.len(),
                0,
                &sa as *const sockaddr,
                socklen,
            )
        };

        let sent = to_result_errno(res)?;
        Ok(sent as usize)
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        unsafe {
            let _ = zsock_close(self.fd);
        }
    }
}
