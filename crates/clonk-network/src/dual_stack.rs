//! One place where every listening socket in this crate is created.
//!
//! C++ `C4NetIO` opens a single AF_INET6 socket with `IPV6_V6ONLY` cleared and
//! carries both address families over it
//! (oracle-src-pinned src/C4NetIO.cpp:1560-1633). Every transport here wants
//! that same shape, so the creation and the bind-address mapping it implies
//! live together rather than being repeated per transport.

use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};

use socket2::{Domain, Protocol, Socket, Type};

/// Creates one dual-stack socket, the shape every transport in this crate
/// binds.
pub(crate) fn create_dual_stack_socket(
    kind: Type,
    protocol: Option<Protocol>,
) -> io::Result<Socket> {
    let socket = Socket::new(Domain::IPV6, kind, protocol)?;
    socket.set_only_v6(false)?;
    Ok(socket)
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
