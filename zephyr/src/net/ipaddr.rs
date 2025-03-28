//!
//! docs todo...
//!

use crate::error::{Error, Result};
use crate::raw::{in6_addr, in_addr, sockaddr, sockaddr_in, sockaddr_in6, socklen_t};
use core::mem::MaybeUninit;
use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

/// docs todo...
pub(crate) fn ipv4_addr_to_c(addr: &Ipv4Addr) -> in_addr {
    let octets = addr.octets();

    // It's not possible to easily construct this type due to the BindgenUnion field, and in any
    // case constructing it would always require unsafe code. Since the layout of the C type is
    // well defined and can be interpreted as a 4-byte array, we can just transmute the octets.
    unsafe { core::mem::transmute(octets) }
}

/// docs todo...
pub(crate) fn ipv4_addr_from_c(addr: &in_addr) -> Ipv4Addr {
    let s4_addr = unsafe { addr.__bindgen_anon_1.s4_addr.as_ref() };

    Ipv4Addr::new(s4_addr[0], s4_addr[1], s4_addr[2], s4_addr[3])
}

/// docs todo...
pub(crate) fn ipv6_addr_to_c(addr: &Ipv6Addr) -> in6_addr {
    let octets = addr.octets();

    // It's not possible to easily construct this type due to the BindgenUnion field, and in any
    // case constructing it would always require unsafe code. Since the layout of the C type is
    // well defined and can be interpreted as a 16-byte array, we can just transmute the octets.
    unsafe { core::mem::transmute(octets) }
}

/// docs todo...
pub(crate) fn ipv6_addr_from_c(addr: &in6_addr) -> Ipv6Addr {
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

/// docs todo...
pub(crate) fn sockaddr_v4_to_c(sa: &SocketAddrV4) -> sockaddr_in {
    sockaddr_in {
        sin_family: crate::raw::AF_INET as u16,
        sin_port: sa.port().to_be(),
        sin_addr: ipv4_addr_to_c(sa.ip()),
    }
}

/// docs todo...
pub(crate) fn try_sockaddr_v4_from_c(sa: &sockaddr_in) -> Result<SocketAddrV4> {
    if sa.sin_family != crate::raw::AF_INET as u16 {
        return Err(Error(crate::raw::EINVAL));
    }

    Ok(SocketAddrV4::new(
        ipv4_addr_from_c(&sa.sin_addr),
        u16::from_be(sa.sin_port),
    ))
}

/// docs todo...
pub(crate) fn sockaddr_v6_to_c(sa: &SocketAddrV6) -> sockaddr_in6 {
    sockaddr_in6 {
        sin6_family: crate::raw::AF_INET6 as u16,
        sin6_port: sa.port().to_be(),
        sin6_addr: ipv6_addr_to_c(sa.ip()),
        // TODO: It seems to be possible that Rust scope_id is > 255. Is panic the correct behaviour?
        sin6_scope_id: sa.scope_id() as u8,
    }
}

/// docs todo...
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

/// docs todo...
pub(crate) fn sockaddr_to_c(sa: &SocketAddr) -> (sockaddr, socklen_t) {
    // TODO: this way of getting a `struct sockaddr` is pretty horrible, see if there is a better approach
    let mut sa_out = MaybeUninit::<sockaddr>::zeroed();
    let dst_ptr = sa_out.as_mut_ptr() as *mut u8;

    let socklen = match sa {
        SocketAddr::V4(sa4) => {
            let src = sockaddr_v4_to_c(sa4);
            let src_ptr = (&src as *const sockaddr_in) as *const u8;
            let len = core::mem::size_of::<sockaddr>();

            unsafe {
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, len);
            }

            len
        }
        SocketAddr::V6(sa6) => {
            let src = sockaddr_v6_to_c(sa6);
            let src_ptr = (&src as *const sockaddr_in6) as *const u8;
            let len = core::mem::size_of::<sockaddr_in6>();

            unsafe {
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, len);
            }

            len
        }
    };

    (unsafe { sa_out.assume_init() }, socklen)
}

/// docs todo...
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
