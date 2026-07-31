//! One place where every listening socket in this crate is created.
//!
//! C++ `C4NetIO` opens a single AF_INET6 socket with `IPV6_V6ONLY` cleared and
//! carries both address families over it
//! (oracle-src-pinned src/C4NetIO.cpp:1560-1633). Every transport here wants
//! that same shape, so the creation and the bind-address mapping it implies
//! live together rather than being repeated per transport.

use std::io;
use std::net::{SocketAddr, SocketAddrV6};

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
pub(crate) fn dual_stack_bind_address(requested: SocketAddr) -> SocketAddr {
    match requested {
        SocketAddr::V4(address) => SocketAddr::V6(SocketAddrV6::new(
            address.ip().to_ipv6_mapped(),
            address.port(),
            0,
            0,
        )),
        SocketAddr::V6(address) => SocketAddr::V6(address),
    }
}
