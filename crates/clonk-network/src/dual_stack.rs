//! One place where every listening socket in this crate is created.
//!
//! C++ `C4NetIO` opens a single AF_INET6 socket with `IPV6_V6ONLY` cleared and
//! carries both address families over it
//! (oracle-src-pinned src/C4NetIO.cpp:1560-1633). Every transport here wants
//! that same shape, so the creation and the bind-address mapping it implies
//! live together rather than being repeated per transport.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

use socket2::{Domain, Protocol, Socket, Type};

/// Destination families a bound socket can actually carry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SocketFamily {
    /// AF_INET6 with `IPV6_V6ONLY` cleared, bound to an address that is not
    /// v4-mapped. Both families reach their peers over it.
    DualStack,
    /// AF_INET6 pinned to a v4-mapped address. It can reach only IPv4 peers,
    /// but they must still be passed to `sendto` as mapped IPv6 sockaddrs.
    MappedIpv4,
    /// AF_INET. Callers must pass it IPv4 sockaddrs and must not offer it an
    /// IPv6 destination.
    Ipv4Only,
}

/// The `socket(2)` seam. Production passes [`new_socket`]; tests substitute a
/// constructor that refuses `AF_INET6` the way a kernel booted with
/// `ipv6.disable=1` does.
pub(crate) type SocketConstructor<'a> =
    &'a dyn Fn(Domain, Type, Option<Protocol>) -> io::Result<Socket>;

pub(crate) fn new_socket(
    domain: Domain,
    kind: Type,
    protocol: Option<Protocol>,
) -> io::Result<Socket> {
    Socket::new(domain, kind, protocol)
}

/// Error codes with which a host reports that it has no IPv6 stack at all,
/// rather than a transient failure worth retrying.
#[cfg(unix)]
const IPV6_UNAVAILABLE_CODES: &[i32] = &[
    libc::EAFNOSUPPORT,
    libc::EPFNOSUPPORT,
    libc::EPROTONOSUPPORT,
];
#[cfg(windows)]
const IPV6_UNAVAILABLE_CODES: &[i32] = &[
    10_047, // WSAEAFNOSUPPORT
    10_046, // WSAEPFNOSUPPORT
    10_043, // WSAEPROTONOSUPPORT
];
#[cfg(not(any(unix, windows)))]
const IPV6_UNAVAILABLE_CODES: &[i32] = &[];

fn ipv6_stack_unavailable(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported
        || error
            .raw_os_error()
            .is_some_and(|code| IPV6_UNAVAILABLE_CODES.contains(&code))
}

/// The error a host without an IPv6 stack answers `socket(AF_INET6, ...)`
/// with. Only tests build one; production reads it from the kernel.
#[cfg(test)]
pub(crate) fn ipv6_unavailable_error() -> io::Error {
    IPV6_UNAVAILABLE_CODES.first().map_or_else(
        || io::Error::from(io::ErrorKind::Unsupported),
        |code| io::Error::from_raw_os_error(*code),
    )
}

/// Creates the socket `bind_address` needs, together with the address it must
/// actually be bound to.
///
/// A host whose kernel has no IPv6 stack fails `socket(AF_INET6, ...)` outright
/// with `EAFNOSUPPORT`, and a hard failure there leaves it unable to host a game
/// at all. Degrading to plain IPv4 keeps every IPv4 peer reachable; the caller
/// reads back [`bound_socket_family`] to learn that IPv6 destinations are not.
pub(crate) fn create_bound_socket_with(
    bind_address: SocketAddr,
    kind: Type,
    protocol: Option<Protocol>,
    constructor: SocketConstructor<'_>,
) -> io::Result<(Socket, SocketAddr)> {
    match constructor(Domain::IPV6, kind, protocol) {
        Ok(socket) => socket
            .set_only_v6(false)
            .map(|()| (socket, dual_stack_bind_address(bind_address))),
        Err(error) if ipv6_stack_unavailable(&error) => ipv4_bind_address(bind_address)
            .ok_or(error)
            .and_then(|address| {
                constructor(Domain::IPV4, kind, protocol).map(|socket| (socket, address))
            }),
        Err(error) => Err(error),
    }
}

pub(crate) fn create_bound_socket(
    bind_address: SocketAddr,
    kind: Type,
    protocol: Option<Protocol>,
) -> io::Result<(Socket, SocketAddr)> {
    create_bound_socket_with(bind_address, kind, protocol, &new_socket)
}

/// Which destination families a socket already bound to `local` can carry.
pub(crate) fn bound_socket_family(local: SocketAddr) -> SocketFamily {
    match local {
        SocketAddr::V6(address) if address.ip().to_ipv4_mapped().is_none() => {
            SocketFamily::DualStack
        }
        SocketAddr::V6(_) => SocketFamily::MappedIpv4,
        SocketAddr::V4(_) => SocketFamily::Ipv4Only,
    }
}

/// IPv4 form of `requested`, or `None` when it names an endpoint only IPv6 can
/// carry and an IPv4 socket therefore cannot serve at all.
fn ipv4_bind_address(requested: SocketAddr) -> Option<SocketAddr> {
    match requested {
        SocketAddr::V4(address) => Some(SocketAddr::V4(address)),
        SocketAddr::V6(address) if address.ip().is_unspecified() => Some(SocketAddr::new(
            Ipv4Addr::UNSPECIFIED.into(),
            address.port(),
        )),
        SocketAddr::V6(address) => address
            .ip()
            .to_ipv4_mapped()
            .map(|ip| SocketAddr::new(ip.into(), address.port())),
    }
}

/// Address a dual-stack socket must actually bind in order to serve
/// `requested`.
///
/// The IPv4 wildcard becomes the IPv6 wildcard rather than `::ffff:0.0.0.0`:
/// Linux pins a socket bound to a v4-mapped address to IPv4 and answers
/// `EAFNOSUPPORT` for every IPv6 destination sent over it, which is what kept a
/// host from reaching a netpuncher resolved from an AAAA record. Both spellings
/// of "any interface" therefore have to produce the same genuinely dual-stack
/// socket.
pub(crate) fn dual_stack_bind_address(requested: SocketAddr) -> SocketAddr {
    let address = match requested {
        SocketAddr::V4(address) if address.ip().is_unspecified() => {
            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, address.port(), 0, 0)
        }
        SocketAddr::V4(address) => {
            SocketAddrV6::new(address.ip().to_ipv6_mapped(), address.port(), 0, 0)
        }
        SocketAddr::V6(address) => address,
    };
    SocketAddr::V6(address)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn both_spellings_of_any_interface_stay_dual_stack() {
        let ipv4_wildcard = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 11_115);
        let ipv6_wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 11_115, 0, 0));
        assert_eq!(dual_stack_bind_address(ipv4_wildcard), ipv6_wildcard);
        assert_eq!(dual_stack_bind_address(ipv6_wildcard), ipv6_wildcard);
    }

    #[test]
    fn a_named_ipv4_interface_keeps_its_mapped_form() {
        let requested = SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 11_115);
        assert_eq!(
            dual_stack_bind_address(requested),
            SocketAddr::V6(SocketAddrV6::new(
                Ipv4Addr::new(192, 0, 2, 1).to_ipv6_mapped(),
                11_115,
                0,
                0
            ))
        );
    }
}
