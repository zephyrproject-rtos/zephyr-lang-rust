//! Conversion between Rust and C representations of IP and Socket addresses.
//!
//! The intention is that the Rust networking API presents an interface using the types defined in
//! [`core::net`](https://doc.rust-lang.org/stable/core/net/index.html). Conversion to and from the
//! C types should only be required inside this crate.

use crate::error::{Error, Result};
use crate::raw::{in6_addr, in_addr, sockaddr, sockaddr_in, sockaddr_in6, socklen_t};
use core::mem::MaybeUninit;
use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

/// Convert an IPv4 address from Rust to C representation
pub(crate) fn ipv4_addr_to_c(addr: &Ipv4Addr) -> in_addr {
    let octets = addr.octets();

    // SAFETY: It's not possible to easily construct this type due to the BindgenUnion field, and in
    // any case constructing it would always require unsafe code.
    //
    // The C type and the octets field of the Rust type both have a well-defined layout: 4 bytes in
    // big-endian order. So it is safe to transmute between the two types.
    unsafe { core::mem::transmute(octets) }
}

/// Convert an IPv4 address from C to Rust representation
pub(crate) fn ipv4_addr_from_c(addr: &in_addr) -> Ipv4Addr {
    // SAFETY: The s4_addr union field has no restrictions on which values are valid, so is always
    // safe to access.
    let s4_addr = unsafe { addr.__bindgen_anon_1.s4_addr.as_ref() };

    Ipv4Addr::new(s4_addr[0], s4_addr[1], s4_addr[2], s4_addr[3])
}

/// Convert an IPv6 address from Rust to C representation
pub(crate) fn ipv6_addr_to_c(addr: &Ipv6Addr) -> in6_addr {
    let octets = addr.octets();

    // SAFETY: It's not possible to easily construct this type due to the BindgenUnion field, and in
    // any case constructing it would always require unsafe code.
    //
    // The C type and the octets field of the Rust type both have a well-defined layout: 16 bytes in
    // big-endian order. So it is safe to transmute between the two types.
    unsafe { core::mem::transmute(octets) }
}

/// Convert an IPv6 address from C to Rust representation
pub(crate) fn ipv6_addr_from_c(addr: &in6_addr) -> Ipv6Addr {
    // SAFETY: The s6_addr16 union field has no restrictions on which values are valid, so is always
    // safe to access.
    let s6_addr16 = unsafe { addr.__bindgen_anon_1.s6_addr16.as_ref() };

    Ipv6Addr::new(
        s6_addr16[0],
        s6_addr16[1],
        s6_addr16[2],
        s6_addr16[3],
        s6_addr16[4],
        s6_addr16[5],
        s6_addr16[6],
        s6_addr16[7],
    )
}

/// Convert an IPv4 socket address from Rust to C representation
pub(crate) fn sockaddr_v4_to_c(sa: &SocketAddrV4) -> sockaddr_in {
    sockaddr_in {
        sin_family: crate::raw::AF_INET as u16,
        sin_port: sa.port().to_be(),
        sin_addr: ipv4_addr_to_c(sa.ip()),
    }
}

/// Try to convert an IPv4 socket address from C to Rust representation
///
/// This will return an error if the `sin_family` field of `sa` is not `AF_INET`.
pub(crate) fn try_sockaddr_v4_from_c(sa: &sockaddr_in) -> Result<SocketAddrV4> {
    if sa.sin_family != crate::raw::AF_INET as u16 {
        return Err(Error(crate::raw::EINVAL));
    }

    Ok(SocketAddrV4::new(
        ipv4_addr_from_c(&sa.sin_addr),
        u16::from_be(sa.sin_port),
    ))
}

/// Try to convert an IPv6 socket address from Rust to C representation
///
/// This will return an error in either of the following cases:
/// - The `flowinfo` field is non-zero
/// - The `scope_id` field cannot be represented as a `u8`
///
/// Zephyr's `struct sockaddr_in6` differs slightly from the traditional BSD sockets API in that
/// it does not contain a `flowinfo` field, and the `scope_id` is stored as a `uint8_t`.
pub(crate) fn try_sockaddr_v6_to_c(sa: &SocketAddrV6) -> Result<sockaddr_in6> {
    if sa.flowinfo() != 0 || sa.scope_id() > u8::MAX as u32 {
        return Err(Error(crate::raw::EINVAL));
    }

    Ok(sockaddr_in6 {
        sin6_family: crate::raw::AF_INET6 as u16,
        sin6_port: sa.port().to_be(),
        sin6_addr: ipv6_addr_to_c(sa.ip()),
        sin6_scope_id: sa.scope_id() as u8,
    })
}

/// Try to convert an IPv6 socker address from C to Rust representation
///
/// This will return an error if the `sin6_family` field of `sa` is not `AF_INET6`.
pub(crate) fn try_sockaddr_v6_from_c(sa: &sockaddr_in6) -> Result<SocketAddrV6> {
    if sa.sin6_family != crate::raw::AF_INET6 as u16 {
        return Err(Error(crate::raw::EINVAL));
    }

    Ok(SocketAddrV6::new(
        ipv6_addr_from_c(&sa.sin6_addr),
        u16::from_be(sa.sin6_port),
        0,
        sa.sin6_scope_id as u32,
    ))
}

/// Try to convert a socket address from Rust to C representation
///
/// This will return an error if the socket address is a `SocketAddrV6`, and this address contains
/// fields which cannot be represented in the C struct (see [try_sockaddr_v6_from_c]).
pub(crate) fn try_sockaddr_to_c(sa: &SocketAddr) -> Result<(sockaddr, socklen_t)> {
    let mut sa_out = MaybeUninit::<sockaddr>::zeroed();
    let dst_ptr = sa_out.as_mut_ptr() as *mut u8;

    let socklen = match sa {
        SocketAddr::V4(sa4) => {
            let src = sockaddr_v4_to_c(sa4);
            let src_ptr = (&src as *const sockaddr_in) as *const u8;
            let len = core::mem::size_of::<sockaddr>();

            // SAFETY: The `sockaddr` struct is sized to be able to contain either a `sockaddr_in`
            // or a `sockaddr_in6`, so it is safe to copy either type into it.
            unsafe {
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, len);
            }

            len
        }
        SocketAddr::V6(sa6) => {
            let src = try_sockaddr_v6_to_c(sa6)?;
            let src_ptr = (&src as *const sockaddr_in6) as *const u8;
            let len = core::mem::size_of::<sockaddr_in6>();

            // SAFETY: The `sockaddr` struct is sized to be able to contain either a `sockaddr_in`
            // or a `sockaddr_in6`, so it is safe to copy either type into it.
            unsafe {
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, len);
            }

            len
        }
    };

    // SAFETY: Both match arms initialise `sa_out` with a valid `sockaddr`
    let sa_out = unsafe { sa_out.assume_init() };

    Ok((sa_out, socklen))
}

/// Try to convert a socket address from C to Rust representation
///
/// This will return an error if the `sa_family` field is not either `AF_INET` or `AF_INET6`.
pub(crate) fn try_sockaddr_from_c(sa: &sockaddr, socklen: socklen_t) -> Result<SocketAddr> {
    match sa.sa_family as u32 {
        crate::raw::AF_INET => {
            if socklen != core::mem::size_of::<sockaddr_in>() {
                return Err(Error(crate::raw::EINVAL));
            }

            let sa4_ref: &sockaddr_in = unsafe { core::mem::transmute(sa) };
            let res = try_sockaddr_v4_from_c(sa4_ref)?;
            Ok(SocketAddr::V4(res))
        }
        crate::raw::AF_INET6 => {
            if socklen != core::mem::size_of::<sockaddr_in6>() {
                return Err(Error(crate::raw::EINVAL));
            }

            let sa6_ref: &sockaddr_in6 = unsafe { core::mem::transmute(sa) };
            let res = try_sockaddr_v6_from_c(sa6_ref)?;
            Ok(SocketAddr::V6(res))
        }
        _ => Err(Error(crate::raw::EINVAL)),
    }
}
