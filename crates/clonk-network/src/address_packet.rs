use std::fmt;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs};

/// `C4PacketType::PID_Addr` (`src/C4PacketBase.h:109-110`).
pub const PID_ADDR: u8 = 0x12;

/// `C4PacketType::PID_TCPSimOpen` (`src/C4PacketBase.h:116`).
pub const PID_TCP_SIM_OPEN: u8 = 0x14;

/// The byte-sized `C4Network2IOProtocol` carried by `C4Network2Address`.
///
/// Binary compilation casts the C++ enum to `uint8_t` without validation
/// (`src/C4Network2Address.cpp:497-503`), so unknown values must survive a
/// decode/encode cycle instead of being rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NetworkProtocol {
    Udp,
    Tcp,
    Unknown(u8),
}

impl NetworkProtocol {
    /// Wire representation produced when a local C++ address still has
    /// `P_NONE == -1`. On decode C++ casts the byte back to enum value 255,
    /// which is not equal to `P_NONE`, so it is intentionally unknown here.
    pub const NONE_WIRE: Self = Self::Unknown(u8::MAX);

    pub const fn from_wire(value: u8) -> Self {
        match value {
            0 => Self::Udp,
            1 => Self::Tcp,
            value => Self::Unknown(value),
        }
    }

    pub const fn to_wire(self) -> u8 {
        match self {
            Self::Udp => 0,
            Self::Tcp => 1,
            Self::Unknown(value) => value,
        }
    }
}

/// A protocol plus endpoint, matching `C4Network2Address`.
#[derive(Debug, Clone, Copy)]
pub struct NetworkAddress {
    pub protocol: NetworkProtocol,
    pub endpoint: SocketAddr,
}

impl PartialEq for NetworkAddress {
    fn eq(&self, other: &Self) -> bool {
        self.protocol == other.protocol
            && cpp_equality_endpoint(self.endpoint) == cpp_equality_endpoint(other.endpoint)
    }
}

impl Eq for NetworkAddress {}

impl Hash for NetworkAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.protocol.hash(state);
        cpp_equality_endpoint(self.endpoint).hash(state);
    }
}

impl NetworkAddress {
    pub fn new(protocol: NetworkProtocol, endpoint: SocketAddr) -> Self {
        Self {
            protocol,
            endpoint: canonicalize_mapped_ipv4(endpoint),
        }
    }

    /// Mirrors the misleadingly named `C4Network2Address::isIPNull`, which
    /// delegates to endpoint `IsNull`: the host must be unspecified and the
    /// port must be zero.
    pub fn is_ip_null(&self) -> bool {
        self.has_null_host() && self.endpoint.port() == 0
    }

    /// Mirrors `C4Network2EndpointAddress::IsNullHost`, which ignores the
    /// endpoint port. Client mesh dialing rejects unspecified hosts even when
    /// an announced address carries a nonzero configured port.
    pub fn has_null_host(&self) -> bool {
        self.endpoint.ip().is_unspecified()
    }

    /// Mirrors `C4Network2Address::SetIP`. Despite its name, C++ delegates to
    /// endpoint `SetAddress` and therefore copies both the peer host and port.
    pub fn with_ip_from_peer(self, peer: SocketAddr) -> Self {
        if !self.is_ip_null() {
            return self;
        }

        Self {
            endpoint: canonicalize_mapped_ipv4(peer),
            ..self
        }
    }
}

/// One `PID_Addr` payload. There is deliberately no count: C++ sends one
/// packet per address (`src/C4Network2Client.cpp:319-337`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AddressPacket {
    pub client_id: i32,
    pub address: NetworkAddress,
}

/// One `PID_TCPSimOpen` payload.
///
/// C++ compiles this as the same packed client ID plus `C4Network2Address`
/// field sequence as [`AddressPacket`], but it has distinct session semantics
/// and therefore remains a separate type
/// (`src/C4Network2Client.cpp:665-670`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TcpSimOpenPacket {
    pub client_id: i32,
    pub address: NetworkAddress,
}

impl AddressPacket {
    /// Produces the address C++ applies after replacing a null endpoint with
    /// the full peer endpoint (`src/C4Network2Client.cpp:581-597`).
    pub fn announcement_for_peer(self, peer: SocketAddr) -> Self {
        Self {
            address: self.address.with_ip_from_peer(peer),
            ..self
        }
    }
}

/// Result of applying one received address to a client's ordered address list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressInsertion {
    Added { index: usize },
    AlreadyPresent { index: usize },
}

/// Applies the duplicate suppression and default append order used by
/// `C4Network2Client::AddAddr` for received `PID_Addr` packets.
///
/// Both outcomes correspond to C++ `AddAddr` returning true, so the caller
/// still performs a connection attempt; a newly added address is also
/// announced to the other connected clients (`src/C4Network2Client.cpp:259-278,581-597`).
pub fn append_received_address(
    addresses: &mut Vec<NetworkAddress>,
    address: NetworkAddress,
) -> AddressInsertion {
    let address = NetworkAddress::new(address.protocol, address.endpoint);
    addresses
        .iter()
        .position(|known| *known == address)
        .map(|index| AddressInsertion::AlreadyPresent { index })
        .unwrap_or_else(|| {
            let index = addresses.len();
            addresses.push(address);
            AddressInsertion::Added { index }
        })
}

/// Applies `C4Network2Client::AddAddrFromPuncher` to one ordered client
/// address list and returns the newly-added addresses in announcement order.
///
/// Puncher-derived addresses are inserted at the front. C++ first adds the
/// observed UDP endpoint, then a configured-port UDP alternative when NAT
/// translated the port, and finally the configured TCP alternative. Because
/// each insertion is at the front, the retained list order is the reverse of
/// the returned announcement order (`src/C4Network2Client.cpp:237-256`).
pub(crate) fn add_addresses_from_puncher(
    addresses: &mut Vec<NetworkAddress>,
    observed_address: SocketAddr,
    configured_udp_port: u16,
    configured_tcp_port: u16,
) -> Vec<NetworkAddress> {
    let observed_address = NetworkAddress::new(NetworkProtocol::Udp, observed_address).endpoint;
    let mut added = Vec::new();
    let mut add_in_front = |address: NetworkAddress| {
        if addresses.contains(&address) {
            return;
        }
        addresses.insert(0, address);
        added.push(address);
    };

    add_in_front(NetworkAddress::new(NetworkProtocol::Udp, observed_address));
    if observed_address.port() != configured_udp_port {
        let mut configured_udp = observed_address;
        configured_udp.set_port(configured_udp_port);
        add_in_front(NetworkAddress::new(NetworkProtocol::Udp, configured_udp));
    }
    if configured_tcp_port != 0 {
        let mut configured_tcp = observed_address;
        configured_tcp.set_port(configured_tcp_port);
        add_in_front(NetworkAddress::new(NetworkProtocol::Tcp, configured_tcp));
    }

    added
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressPacketDecodeError {
    UnexpectedEof,
    PackedClientIdOverflow,
    MissingEndpointTerminator,
}

impl fmt::Display for AddressPacketDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnexpectedEof => "PID_Addr payload is truncated",
            Self::PackedClientIdOverflow => "PID_Addr client id exceeds packed int32",
            Self::MissingEndpointTerminator => "PID_Addr endpoint has no NUL terminator",
        })
    }
}

impl std::error::Error for AddressPacketDecodeError {}

/// Decodes the body after `PID_Addr` in the exact C++ field order.
pub fn decode_address_packet_payload(
    payload: &[u8],
) -> Result<AddressPacket, AddressPacketDecodeError> {
    let (client_id, client_id_len) = decode_packed_i32(payload)?;
    let protocol = payload
        .get(client_id_len)
        .copied()
        .ok_or(AddressPacketDecodeError::UnexpectedEof)?;
    let endpoint_start = client_id_len + 1;
    let endpoint_tail = payload
        .get(endpoint_start..)
        .ok_or(AddressPacketDecodeError::UnexpectedEof)?;
    let endpoint_len = endpoint_tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(AddressPacketDecodeError::MissingEndpointTerminator)?;
    let endpoint = decode_cpp_endpoint(&endpoint_tail[..endpoint_len]);

    Ok(AddressPacket {
        client_id,
        address: NetworkAddress::new(NetworkProtocol::from_wire(protocol), endpoint),
    })
}

/// Encodes the body after `PID_Addr`.
pub fn encode_address_packet_payload(packet: &AddressPacket) -> Vec<u8> {
    let mut payload = Vec::new();
    encode_packed_i32(packet.client_id, &mut payload);
    payload.push(packet.address.protocol.to_wire());
    payload.extend_from_slice(endpoint_wire_text(packet.address.endpoint).as_bytes());
    payload.push(0);
    payload
}

/// Decodes the body after `PID_TCPSimOpen`.
pub fn decode_tcp_sim_open_packet_payload(
    payload: &[u8],
) -> Result<TcpSimOpenPacket, AddressPacketDecodeError> {
    let packet = decode_address_packet_payload(payload)?;
    Ok(TcpSimOpenPacket {
        client_id: packet.client_id,
        address: packet.address,
    })
}

/// Encodes the body after `PID_TCPSimOpen` in C++ field order.
pub fn encode_tcp_sim_open_packet_payload(packet: &TcpSimOpenPacket) -> Vec<u8> {
    encode_address_packet_payload(&AddressPacket {
        client_id: packet.client_id,
        address: packet.address,
    })
}

fn canonicalize_mapped_ipv4(endpoint: SocketAddr) -> SocketAddr {
    match endpoint {
        SocketAddr::V6(endpoint) => endpoint
            .ip()
            .to_ipv4_mapped()
            .map(|ip| SocketAddr::V4(SocketAddrV4::new(ip, endpoint.port())))
            .unwrap_or(SocketAddr::V6(endpoint)),
        endpoint => endpoint,
    }
}

fn cpp_equality_endpoint(endpoint: SocketAddr) -> SocketAddr {
    match canonicalize_mapped_ipv4(endpoint) {
        SocketAddr::V6(endpoint) => SocketAddr::V6(SocketAddrV6::new(
            *endpoint.ip(),
            endpoint.port(),
            0,
            endpoint.scope_id(),
        )),
        endpoint => endpoint,
    }
}

fn endpoint_wire_text(endpoint: SocketAddr) -> String {
    match canonicalize_mapped_ipv4(endpoint) {
        SocketAddr::V4(endpoint) => endpoint.to_string(),
        SocketAddr::V6(endpoint) => {
            SocketAddrV6::new(*endpoint.ip(), endpoint.port(), 0, 0).to_string()
        }
    }
}

pub(crate) fn decode_cpp_endpoint(bytes: &[u8]) -> SocketAddr {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(split_cpp_endpoint)
        .and_then(|(host, port)| resolve_cpp_host(host, port))
        .map(canonicalize_mapped_ipv4)
        .unwrap_or_else(null_endpoint)
}

fn split_cpp_endpoint(text: &str) -> Option<(&str, u16)> {
    if let Some(bracketed) = text.strip_prefix('[') {
        let closing = bracketed.find(']')?;
        let host = &bracketed[..closing];
        let remainder = &bracketed[closing + 1..];
        return remainder.strip_prefix(':').map_or(Some((host, 0)), |port| {
            parse_cpp_port(port).map(|port| (host, port))
        });
    }

    if text.bytes().filter(|byte| *byte == b':').count() > 1 {
        return Some((text, 0));
    }

    text.find(':')
        .map(|separator| {
            parse_cpp_port(&text[separator + 1..]).map(|port| (&text[..separator], port))
        })
        .unwrap_or(Some((text, 0)))
}

fn parse_cpp_port(text: &str) -> Option<u16> {
    (!text.is_empty())
        .then(|| text.parse::<u16>().ok())
        .flatten()
}

fn resolve_cpp_host(host: &str, port: u16) -> Option<SocketAddr> {
    host.parse::<IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, port))
        .or_else(|| (host, port).to_socket_addrs().ok()?.next())
}

fn null_endpoint() -> SocketAddr {
    SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))
}

fn encode_packed_i32(mut value: i32, output: &mut Vec<u8>) {
    loop {
        let chunk = (value << 25) >> 25;
        if chunk == value {
            output.push(chunk as u8);
            break;
        }
        output.push((chunk ^ 0x80) as u8);
        value >>= 7;
    }
}

fn decode_packed_i32(payload: &[u8]) -> Result<(i32, usize), AddressPacketDecodeError> {
    let first = *payload
        .first()
        .ok_or(AddressPacketDecodeError::UnexpectedEof)?;
    let mut current = first;
    let mut signed = (i32::from(current) << 25) >> 25;
    let mut value = signed;
    let mut length = 1usize;
    let mut shift = 7u32;

    while signed as u8 != current {
        if length == 5 {
            return Err(AddressPacketDecodeError::PackedClientIdOverflow);
        }
        current = *payload
            .get(length)
            .ok_or(AddressPacketDecodeError::UnexpectedEof)?;
        signed = (i32::from(current) << 25) >> 25;
        let lower_mask = (1i64 << shift) - 1;
        value = (((i64::from(signed)) << shift) | (i64::from(value) & lower_mask)) as i32;
        length += 1;
        shift += 7;
    }

    Ok((value, length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    #[test]
    fn cpp_ipv4_tcp_vector_has_exact_field_order() {
        // C4PacketAddr writes ClientID before Addr (src/C4Network2Client.cpp:658-662),
        // and C4Network2Address writes the protocol byte before the endpoint
        // (src/C4Network2Address.cpp:489-505).
        let packet = AddressPacket {
            client_id: 42,
            address: NetworkAddress::new(
                NetworkProtocol::Tcp,
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 11_112),
            ),
        };

        assert_eq!(
            encode_address_packet_payload(&packet),
            [
                0x2a, 0x01, b'2', b'0', b'3', b'.', b'0', b'.', b'1', b'1', b'3', b'.', b'7', b':',
                b'1', b'1', b'1', b'1', b'2', 0x00,
            ]
        );
    }

    #[test]
    fn cpp_ipv6_udp_vector_round_trips() {
        // Endpoint compilation uses the bracketed, zone-free numeric string
        // (src/C4Network2Address.cpp:455-483), and the binary string includes
        // its NUL terminator (src/StdCompiler.cpp:115-122).
        let payload = [
            0xff, 0x00, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':', b'7',
            b']', b':', b'1', b'1', b'1', b'1', b'3', 0x00,
        ];

        let packet = decode_address_packet_payload(&payload).unwrap();
        assert_eq!(packet.client_id, -1);
        assert_eq!(packet.address.protocol, NetworkProtocol::Udp);
        assert_eq!(
            packet.address.endpoint,
            SocketAddr::new(
                IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 7)),
                11_113,
            )
        );
        assert_eq!(encode_address_packet_payload(&packet), payload);
    }

    #[test]
    fn cpp_tcp_sim_open_vector_uses_the_address_packet_field_layout() {
        // C4PacketTCPSimOpen compiles packed ClientID followed by Addr, in the
        // same order and representation as C4PacketAddr
        // (src/C4Network2Client.cpp:655-670).
        let payload = [
            0x07, 0x01, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':', b'7',
            b']', b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ];
        let packet = TcpSimOpenPacket {
            client_id: 7,
            address: NetworkAddress::new(
                NetworkProtocol::Tcp,
                "[2001:db8::7]:11112".parse().unwrap(),
            ),
        };

        assert_eq!(encode_tcp_sim_open_packet_payload(&packet), payload);
        assert_eq!(
            decode_tcp_sim_open_packet_payload(&payload).unwrap(),
            packet
        );
    }

    #[test]
    fn receive_substitutes_full_peer_endpoint_only_for_null_endpoint() {
        // Despite its name, isIPNull delegates to endpoint IsNull (unspecified
        // host AND port zero). SetIP then copies the peer host and peer port
        // (src/C4Network2Client.cpp:588-593; src/C4Network2Address.h:216,223;
        // src/C4Network2Address.cpp:332-354).
        let unspecified_host_with_port = AddressPacket {
            client_id: 7,
            address: NetworkAddress::new(NetworkProtocol::Tcp, "0.0.0.0:11112".parse().unwrap()),
        };
        let peer = "198.51.100.9:4242".parse().unwrap();

        assert_eq!(
            unspecified_host_with_port
                .announcement_for_peer(peer)
                .address
                .endpoint,
            "0.0.0.0:11112".parse().unwrap()
        );

        let null_endpoint = AddressPacket {
            address: NetworkAddress::new(NetworkProtocol::Tcp, "0.0.0.0:0".parse().unwrap()),
            ..unspecified_host_with_port
        };
        assert_eq!(
            null_endpoint.announcement_for_peer(peer).address.endpoint,
            peer
        );
    }

    #[test]
    fn null_host_detection_ignores_the_endpoint_port() {
        let configured_port =
            NetworkAddress::new(NetworkProtocol::Tcp, "0.0.0.0:11112".parse().unwrap());
        assert!(configured_port.has_null_host());
        assert!(!configured_port.is_ip_null());

        let routable =
            NetworkAddress::new(NetworkProtocol::Tcp, "198.51.100.1:11112".parse().unwrap());
        assert!(!routable.has_null_host());
    }

    #[test]
    fn received_addresses_append_in_packet_order_without_a_count_field() {
        // SendAddresses emits one PID_Addr per vector element in order
        // (src/C4Network2Client.cpp:319-337); AddAddr suppresses duplicates and
        // appends ordinary received addresses (src/C4Network2Client.cpp:259-270).
        let first =
            NetworkAddress::new(NetworkProtocol::Tcp, "198.51.100.1:11112".parse().unwrap());
        let second =
            NetworkAddress::new(NetworkProtocol::Udp, "198.51.100.1:11113".parse().unwrap());
        let mut addresses = Vec::new();

        assert_eq!(
            append_received_address(&mut addresses, first),
            AddressInsertion::Added { index: 0 }
        );
        assert_eq!(
            append_received_address(&mut addresses, second),
            AddressInsertion::Added { index: 1 }
        );
        assert_eq!(
            append_received_address(&mut addresses, first),
            AddressInsertion::AlreadyPresent { index: 0 }
        );
        assert_eq!(addresses, [first, second]);
    }

    #[test]
    fn received_address_duplicates_ignore_ipv6_flowinfo_like_cpp() {
        // C4Network2EndpointAddress equality compares IPv6 address, port, and
        // scope ID, but deliberately not sin6_flowinfo
        // (src/C4Network2Address.cpp:407-431).
        let first = NetworkAddress::new(
            NetworkProtocol::Tcp,
            SocketAddr::V6(SocketAddrV6::new("fe80::1".parse().unwrap(), 11_112, 3, 7)),
        );
        let same_cpp_address = NetworkAddress::new(
            NetworkProtocol::Tcp,
            SocketAddr::V6(SocketAddrV6::new("fe80::1".parse().unwrap(), 11_112, 99, 7)),
        );
        let mut addresses = vec![first];

        assert_eq!(
            append_received_address(&mut addresses, same_cpp_address),
            AddressInsertion::AlreadyPresent { index: 0 }
        );
        assert_eq!(addresses, [first]);
    }

    #[test]
    fn puncher_address_adds_external_udp_and_configured_udp_tcp_variants_in_front() {
        let retained =
            NetworkAddress::new(NetworkProtocol::Tcp, "192.0.2.9:11112".parse().unwrap());
        let observed: SocketAddr = "203.0.113.8:49152".parse().unwrap();
        let mut addresses = vec![retained];

        let added = add_addresses_from_puncher(&mut addresses, observed, 11_113, 11_112);
        let external_udp = NetworkAddress::new(NetworkProtocol::Udp, observed);
        let configured_udp =
            NetworkAddress::new(NetworkProtocol::Udp, "203.0.113.8:11113".parse().unwrap());
        let configured_tcp =
            NetworkAddress::new(NetworkProtocol::Tcp, "203.0.113.8:11112".parse().unwrap());

        assert_eq!(added, [external_udp, configured_udp, configured_tcp]);
        assert_eq!(
            addresses,
            [configured_tcp, configured_udp, external_udp, retained]
        );
        assert!(add_addresses_from_puncher(&mut addresses, observed, 11_113, 11_112).is_empty());
    }

    #[test]
    fn puncher_address_omits_redundant_udp_and_disabled_tcp_variants() {
        let observed: SocketAddr = "[2001:db8::8]:11113".parse().unwrap();
        let mut addresses = Vec::new();

        let added = add_addresses_from_puncher(&mut addresses, observed, 11_113, 0);

        assert_eq!(added, [NetworkAddress::new(NetworkProtocol::Udp, observed)]);
        assert_eq!(addresses, added);
    }

    #[test]
    fn protocol_byte_preserves_cpp_none_and_unknown_values() {
        // StdEnumAdapt uses a raw uint8 for the non-verbose binary compiler;
        // it does not validate against the UDP/TCP name table
        // (src/StdAdaptors.h:830-846; src/C4Network2Address.cpp:497-503).
        for (wire, expected) in [
            (u8::MAX, NetworkProtocol::NONE_WIRE),
            (0x7e, NetworkProtocol::Unknown(0x7e)),
        ] {
            let payload = [
                0x00, wire, b'1', b'2', b'7', b'.', b'0', b'.', b'0', b'.', b'1', b':', b'0', 0x00,
            ];
            let packet = decode_address_packet_payload(&payload).unwrap();
            assert_eq!(packet.address.protocol, expected);
            assert_eq!(encode_address_packet_payload(&packet), payload);
        }
    }

    #[test]
    fn packed_client_id_supports_full_cpp_int32_bounds() {
        // C4PacketAddr applies StdIntPackAdapt<int32_t> without narrowing the
        // client ID (src/C4Network2Client.cpp:658-662; src/StdAdaptors.h:748-809).
        for (client_id, prefix) in [
            (i32::MIN, &[0x80, 0x80, 0x80, 0x80, 0xf8][..]),
            (i32::MAX, &[0x7f, 0x7f, 0x7f, 0x7f, 0x07][..]),
        ] {
            let packet = AddressPacket {
                client_id,
                address: NetworkAddress::new(
                    NetworkProtocol::Tcp,
                    "127.0.0.1:65535".parse().unwrap(),
                ),
            };
            let encoded = encode_address_packet_payload(&packet);
            assert!(encoded.starts_with(prefix));
            assert_eq!(decode_address_packet_payload(&encoded).unwrap(), packet);
        }
    }

    #[test]
    fn decoder_reports_each_payload_bound() {
        // The binary compiler requires every field and a terminated std::string
        // (src/StdCompiler.cpp:194-207). A valid packed int32 uses at most five
        // bytes (src/StdAdaptors.h:737-745).
        assert_eq!(
            decode_address_packet_payload(&[]),
            Err(AddressPacketDecodeError::UnexpectedEof)
        );
        assert_eq!(
            decode_address_packet_payload(&[0x00]),
            Err(AddressPacketDecodeError::UnexpectedEof)
        );
        assert_eq!(
            decode_address_packet_payload(&[0x00, 0x01, b'x']),
            Err(AddressPacketDecodeError::MissingEndpointTerminator)
        );
        assert_eq!(
            decode_address_packet_payload(&[0x80; 5]),
            Err(AddressPacketDecodeError::PackedClientIdOverflow)
        );
    }

    #[test]
    fn cpp_compile_from_buf_ignores_bytes_after_endpoint() {
        // CompileFromBuf does not require StdCompilerBinRead to finish at EOF
        // (src/StdCompiler.h:372-385; src/StdCompiler.cpp:241-244).
        let payload = [
            0x07, 0x01, b'1', b'2', b'7', b'.', b'0', b'.', b'0', b'.', b'1', b':', b'0', 0x00,
            0xaa, 0xbb,
        ];

        let packet = decode_address_packet_payload(&payload).unwrap();
        assert_eq!(packet.client_id, 7);
        assert_eq!(packet.address.endpoint, "127.0.0.1:0".parse().unwrap());
    }

    #[test]
    fn cpp_endpoint_parser_accepts_hosts_without_ports() {
        // SetAddress treats an unbracketed multi-colon value as IPv6 with no
        // service, and a colon-free value as a host with no service
        // (src/C4Network2Address.cpp:263-324).
        let ipv6 = decode_address_packet_payload(&[0x00, 0x00, b':', b':', b'1', 0x00]).unwrap();
        let ipv4 = decode_address_packet_payload(&[
            0x00, 0x00, b'1', b'9', b'2', b'.', b'0', b'.', b'2', b'.', b'4', b'4', 0x00,
        ])
        .unwrap();

        assert_eq!(ipv6.address.endpoint, "[::1]:0".parse().unwrap());
        assert_eq!(ipv4.address.endpoint, "192.0.2.44:0".parse().unwrap());
    }

    #[test]
    fn invalid_cpp_endpoint_clears_to_ipv6_any() {
        // C4Network2Address clears its endpoint before compilation, and
        // SetAddress leaves it cleared when parsing fails
        // (src/C4Network2Address.cpp:263-267,489-505).
        let packet = decode_address_packet_payload(&[
            0x00, 0x01, b'n', b'o', b't', b'-', b'a', b'n', b'-', b'a', b'd', b'd', b'r', b':',
            0x00,
        ])
        .unwrap();

        assert_eq!(packet.address.endpoint, "[::]:0".parse().unwrap());
        assert_eq!(
            encode_address_packet_payload(&packet),
            [0x00, 0x01, b'[', b':', b':', b']', b':', b'0', 0x00]
        );
    }

    #[test]
    fn encoder_normalizes_mapped_ipv4_and_omits_ipv6_scope() {
        // Endpoint ToString converts IPv4-mapped IPv6 to IPv4 and uses
        // TSF_SkipZoneId for serialized IPv6 addresses
        // (src/C4Network2Address.cpp:455-477).
        let mapped = AddressPacket {
            client_id: 0,
            address: NetworkAddress::new(
                NetworkProtocol::Tcp,
                SocketAddr::V6(SocketAddrV6::new(
                    "::ffff:203.0.113.8".parse().unwrap(),
                    11_112,
                    9,
                    7,
                )),
            ),
        };
        let scoped = AddressPacket {
            client_id: 0,
            address: NetworkAddress::new(
                NetworkProtocol::Udp,
                SocketAddr::V6(SocketAddrV6::new("fe80::1".parse().unwrap(), 11_113, 9, 7)),
            ),
        };

        assert_eq!(
            encode_address_packet_payload(&mapped),
            [
                0x00, 0x01, b'2', b'0', b'3', b'.', b'0', b'.', b'1', b'1', b'3', b'.', b'8', b':',
                b'1', b'1', b'1', b'1', b'2', 0x00,
            ]
        );
        assert_eq!(
            encode_address_packet_payload(&scoped),
            [
                0x00, 0x00, b'[', b'f', b'e', b'8', b'0', b':', b':', b'1', b']', b':', b'1', b'1',
                b'1', b'1', b'3', 0x00,
            ]
        );
    }
}
