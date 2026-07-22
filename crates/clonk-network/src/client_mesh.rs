use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::time::Duration;

use crate::{AddressInsertion, NetworkAddress, NetworkProtocol};

/// Value used by C++ when deciding whether an address has exhausted its
/// connection attempts (`src/C4Network2Client.h:34-35`).
///
/// `DoConnectAttempt` compares with `>` before incrementing, so counters
/// starting at zero produce four actual dials despite this value being three.
pub const CLIENT_MESH_CONNECT_ATTEMPTS: u32 = 3;

/// Delay after C++ schedules one concrete address dial.
pub const CLIENT_MESH_CONNECT_INTERVAL: Duration = Duration::from_secs(6);

/// Delay used when no address is eligible, all eligible addresses are
/// exhausted, or separate message and data connections already exist.
pub const CLIENT_MESH_CONNECT_BACKOFF: Duration = Duration::from_secs(10);

/// One address in a remote client's ordered `C4Network2Client::Addresses`
/// vector, including the counter used by best-address selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientMeshAddressState {
    address: NetworkAddress,
    connection_attempts: u32,
}

impl ClientMeshAddressState {
    pub const fn address(&self) -> NetworkAddress {
        self.address
    }

    pub const fn connection_attempts(&self) -> u32 {
        self.connection_attempts
    }
}

/// Protocol availability and the routes currently assigned to one peer.
///
/// C++ tests message/data connection pointer identity before looking at
/// address protocols. Keep that fact explicit: two distinct connections may
/// theoretically use the same protocol and must still enter the ten-second
/// backoff (`src/C4Network2Client.cpp:126-159`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientMeshConnectivity {
    pub tcp_available: bool,
    pub udp_available: bool,
    pub message_protocol: Option<NetworkProtocol>,
    pub data_protocol: Option<NetworkProtocol>,
    pub message_and_data_routes_are_distinct: bool,
}

impl ClientMeshConnectivity {
    pub const fn disconnected(tcp_available: bool, udp_available: bool) -> Self {
        Self {
            tcp_available,
            udp_available,
            message_protocol: None,
            data_protocol: None,
            message_and_data_routes_are_distinct: false,
        }
    }

    pub const fn single_route(
        protocol: NetworkProtocol,
        tcp_available: bool,
        udp_available: bool,
    ) -> Self {
        Self {
            tcp_available,
            udp_available,
            message_protocol: Some(protocol),
            data_protocol: Some(protocol),
            message_and_data_routes_are_distinct: false,
        }
    }

    pub const fn distinct_routes(
        message_protocol: NetworkProtocol,
        data_protocol: NetworkProtocol,
        tcp_available: bool,
        udp_available: bool,
    ) -> Self {
        Self {
            tcp_available,
            udp_available,
            message_protocol: Some(message_protocol),
            data_protocol: Some(data_protocol),
            message_and_data_routes_are_distinct: true,
        }
    }

    fn protocol_available(self, protocol: NetworkProtocol) -> bool {
        match protocol {
            NetworkProtocol::Tcp => self.tcp_available,
            NetworkProtocol::Udp => self.udp_available,
            NetworkProtocol::Unknown(_) => false,
        }
    }

    fn protocol_connected(self, protocol: NetworkProtocol) -> bool {
        self.message_protocol == Some(protocol) || self.data_protocol == Some(protocol)
    }
}

/// Concrete dial selected by one C++-ordered `DoConnectAttempt` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientMeshDialAttempt {
    pub address_index: usize,
    pub address: NetworkAddress,
    /// Counter value after C++ increments it and before calling `Connect`.
    pub connection_attempt: u32,
    pub next_attempt_at: Duration,
}

/// Result of explicitly invoking `DoConnectAttempt` for one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMeshConnectDecision {
    /// An already scheduled attempt lies in the future; state is unchanged.
    NotDue { next_attempt_at: Duration },
    /// No dial was emitted and the peer was placed into ten-second backoff.
    Backoff { next_attempt_at: Duration },
    /// One address counter was incremented and should be dialed now.
    Dial(ClientMeshDialAttempt),
}

/// Addresses newly announced by `AddAddrFromPuncher`, in broadcast order.
///
/// Storage order differs because every variant is inserted at the front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientMeshPuncherUpdate {
    pub announcements: Vec<NetworkAddress>,
    pub ipv6_simultaneous_open_address: Option<SocketAddr>,
}

/// Pure per-peer address and retry state from `C4Network2Client`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientMeshPeerState {
    addresses: Vec<ClientMeshAddressState>,
    next_attempt_at: Option<Duration>,
    ipv6_address_from_puncher: Option<SocketAddr>,
}

impl ClientMeshPeerState {
    pub const fn new() -> Self {
        Self {
            addresses: Vec::new(),
            next_attempt_at: None,
            ipv6_address_from_puncher: None,
        }
    }

    pub fn addresses(&self) -> &[ClientMeshAddressState] {
        &self.addresses
    }

    pub const fn next_attempt_at(&self) -> Option<Duration> {
        self.next_attempt_at
    }

    pub const fn ipv6_address_from_puncher(&self) -> Option<SocketAddr> {
        self.ipv6_address_from_puncher
    }

    /// Whether `C4Network2ClientList::DoConnectAttempts` would call this peer
    /// on the current one-second network tick.
    pub fn scheduled_attempt_due(&self, now: Duration) -> bool {
        self.next_attempt_at
            .is_some_and(|next_attempt_at| next_attempt_at <= now)
    }

    /// Applies an ordinary received/local address with C++ append order.
    pub fn add_address(&mut self, address: NetworkAddress, now: Duration) -> AddressInsertion {
        self.insert_address(address, false, now)
    }

    /// Expands and applies the externally observed endpoint exactly like
    /// `C4Network2Client::AddAddrFromPuncher`.
    ///
    /// New addresses are announced in call order (outside UDP, optional
    /// configured-port UDP, optional configured-port TCP), while repeated
    /// front insertion leaves their storage order reversed.
    pub fn add_address_from_puncher(
        &mut self,
        observed_address: SocketAddr,
        configured_udp_port: u16,
        configured_tcp_port: u16,
        now: Duration,
    ) -> ClientMeshPuncherUpdate {
        let variants = client_mesh_puncher_variants(
            observed_address,
            configured_udp_port,
            configured_tcp_port,
        );
        let mut announcements = Vec::with_capacity(variants.len());
        for address in variants {
            if matches!(
                self.insert_address(address, true, now),
                AddressInsertion::Added { .. }
            ) {
                announcements.push(address);
            }
        }

        let observed_address = NetworkAddress::new(NetworkProtocol::Udp, observed_address).endpoint;
        let ipv6_simultaneous_open_address = observed_address.is_ipv6().then_some(observed_address);
        if let Some(address) = ipv6_simultaneous_open_address {
            self.ipv6_address_from_puncher = Some(address);
        }

        ClientMeshPuncherUpdate {
            announcements,
            ipv6_simultaneous_open_address,
        }
    }

    /// Executes the pure scheduling and best-address portion of
    /// `C4Network2Client::DoConnectAttempt`.
    ///
    /// Call this directly after every received `PID_Addr`, including a
    /// duplicate. Periodic callers should first use [`Self::scheduled_attempt_due`].
    pub fn do_connect_attempt(
        &mut self,
        now: Duration,
        connectivity: ClientMeshConnectivity,
    ) -> ClientMeshConnectDecision {
        // Native checks distinct message/data connection pointers before its
        // due-time gate, so an explicit call pushes this backoff out again.
        if connectivity.message_and_data_routes_are_distinct {
            return self.backoff(now);
        }

        if let Some(next_attempt_at) = self.next_attempt_at {
            if next_attempt_at > now {
                return ClientMeshConnectDecision::NotDue { next_attempt_at };
            }
        }

        let best_index = self
            .addresses
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                !candidate.address.has_null_host()
                    && !connectivity.protocol_connected(candidate.address.protocol)
                    && connectivity.protocol_available(candidate.address.protocol)
            })
            .fold(None, |best: Option<usize>, (index, candidate)| match best {
                Some(best_index)
                    if self.addresses[best_index].connection_attempts
                        <= candidate.connection_attempts =>
                {
                    Some(best_index)
                }
                _ => Some(index),
            });

        let Some(address_index) = best_index else {
            return self.backoff(now);
        };
        if self.addresses[address_index].connection_attempts > CLIENT_MESH_CONNECT_ATTEMPTS {
            return self.backoff(now);
        }

        let candidate = &mut self.addresses[address_index];
        candidate.connection_attempts += 1;
        let next_attempt_at = now.saturating_add(CLIENT_MESH_CONNECT_INTERVAL);
        self.next_attempt_at = Some(next_attempt_at);
        ClientMeshConnectDecision::Dial(ClientMeshDialAttempt {
            address_index,
            address: candidate.address,
            connection_attempt: candidate.connection_attempts,
            next_attempt_at,
        })
    }

    fn insert_address(
        &mut self,
        address: NetworkAddress,
        in_front: bool,
        now: Duration,
    ) -> AddressInsertion {
        let address = NetworkAddress::new(address.protocol, address.endpoint);
        if let Some(index) = self
            .addresses
            .iter()
            .position(|candidate| candidate.address == address)
        {
            return AddressInsertion::AlreadyPresent { index };
        }

        let index = if in_front { 0 } else { self.addresses.len() };
        self.addresses.insert(
            index,
            ClientMeshAddressState {
                address,
                connection_attempts: 0,
            },
        );
        if self.next_attempt_at.is_none() {
            self.next_attempt_at = Some(now);
        }
        AddressInsertion::Added { index }
    }

    fn backoff(&mut self, now: Duration) -> ClientMeshConnectDecision {
        let next_attempt_at = now.saturating_add(CLIENT_MESH_CONNECT_BACKOFF);
        self.next_attempt_at = Some(next_attempt_at);
        ClientMeshConnectDecision::Backoff { next_attempt_at }
    }
}

/// Returns the addresses `AddAddrFromPuncher` passes to `AddAddr`, in call and
/// announcement order.
pub fn client_mesh_puncher_variants(
    observed_address: SocketAddr,
    configured_udp_port: u16,
    configured_tcp_port: u16,
) -> Vec<NetworkAddress> {
    let observed = NetworkAddress::new(NetworkProtocol::Udp, observed_address).endpoint;
    let mut variants = vec![NetworkAddress::new(NetworkProtocol::Udp, observed)];
    if observed.port() != configured_udp_port {
        let mut inside_udp = observed;
        inside_udp.set_port(configured_udp_port);
        variants.push(NetworkAddress::new(NetworkProtocol::Udp, inside_udp));
    }
    if configured_tcp_port > 0 {
        let mut tcp = observed;
        tcp.set_port(configured_tcp_port);
        variants.push(NetworkAddress::new(NetworkProtocol::Tcp, tcp));
    }
    variants
}

/// Exact initiator gate used before C++ attempts an IPv6 TCP simultaneous
/// open for a selected mesh address.
pub fn client_mesh_tcp_sim_open_eligible(
    local_client_id: i32,
    remote_client_id: i32,
    address: NetworkAddress,
    socket_already_pending: bool,
) -> bool {
    let SocketAddr::V6(endpoint) = address.endpoint else {
        return false;
    };
    let ip = endpoint.ip();
    matches!(address.protocol, NetworkProtocol::Tcp)
        && !socket_already_pending
        && local_client_id < remote_client_id
        && !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && !ip.is_unicast_link_local()
        && ip.octets()[0] & 0xfe != 0xfc
}

#[derive(Debug, Clone, Copy)]
struct ClientMeshLocalHost {
    endpoint: SocketAddr,
    tcp: bool,
    udp: bool,
}

/// Constructs the local address vector produced by
/// `C4Network2Client::AddLocalAddrs` from already-bound session endpoints and
/// an injected interface snapshot.
///
/// Wildcard IPv4 entries lead the vector in TCP/UDP order. An explicitly
/// bound host is advertised only for its protocol, including an intentional
/// loopback bind. A protocol bound to an unspecified host instead receives
/// every non-loopback interface in C++ decreasing-rank order; each host emits
/// TCP before UDP and retains its IPv6 scope ID.
pub fn client_mesh_local_addresses(
    tcp_bound_address: Option<SocketAddr>,
    udp_bound_address: Option<SocketAddr>,
    interface_endpoints: impl IntoIterator<Item = SocketAddr>,
) -> Vec<NetworkAddress> {
    let mut addresses = Vec::new();
    if let Some(bound) = tcp_bound_address {
        push_mesh_address(
            &mut addresses,
            NetworkAddress::new(
                NetworkProtocol::Tcp,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, bound.port())),
            ),
        );
    }
    if let Some(bound) = udp_bound_address {
        push_mesh_address(
            &mut addresses,
            NetworkAddress::new(
                NetworkProtocol::Udp,
                SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, bound.port())),
            ),
        );
    }

    let tcp_enumerates_interfaces =
        tcp_bound_address.is_some_and(|bound| bound.ip().is_unspecified());
    let udp_enumerates_interfaces =
        udp_bound_address.is_some_and(|bound| bound.ip().is_unspecified());
    let mut hosts = Vec::<ClientMeshLocalHost>::new();

    if let Some(bound) = tcp_bound_address.filter(|bound| !bound.ip().is_unspecified()) {
        merge_local_host(&mut hosts, bound, true, false);
    }
    if let Some(bound) = udp_bound_address.filter(|bound| !bound.ip().is_unspecified()) {
        merge_local_host(&mut hosts, bound, false, true);
    }
    if tcp_enumerates_interfaces || udp_enumerates_interfaces {
        for endpoint in sorted_client_mesh_interface_endpoints(interface_endpoints) {
            merge_local_host(
                &mut hosts,
                endpoint,
                tcp_enumerates_interfaces,
                udp_enumerates_interfaces,
            );
        }
    }
    hosts.sort_by_key(|host| std::cmp::Reverse(cpp_local_address_rank(host.endpoint)));

    for host in hosts {
        if host.tcp {
            if let Some(bound) = tcp_bound_address {
                let mut endpoint = host.endpoint;
                endpoint.set_port(bound.port());
                push_mesh_address(
                    &mut addresses,
                    NetworkAddress::new(NetworkProtocol::Tcp, endpoint),
                );
            }
        }
        if host.udp {
            if let Some(bound) = udp_bound_address {
                let mut endpoint = host.endpoint;
                endpoint.set_port(bound.port());
                push_mesh_address(
                    &mut addresses,
                    NetworkAddress::new(NetworkProtocol::Udp, endpoint),
                );
            }
        }
    }
    addresses
}

fn push_mesh_address(addresses: &mut Vec<NetworkAddress>, address: NetworkAddress) {
    if !addresses.contains(&address) {
        addresses.push(address);
    }
}

fn merge_local_host(
    hosts: &mut Vec<ClientMeshLocalHost>,
    endpoint: SocketAddr,
    tcp: bool,
    udp: bool,
) {
    let endpoint = client_mesh_host_endpoint(endpoint);
    if let Some(host) = hosts
        .iter_mut()
        .find(|host| same_client_mesh_host(host.endpoint, endpoint))
    {
        host.tcp |= tcp;
        host.udp |= udp;
        return;
    }
    hosts.push(ClientMeshLocalHost { endpoint, tcp, udp });
}

fn sorted_client_mesh_interface_endpoints(
    endpoints: impl IntoIterator<Item = SocketAddr>,
) -> Vec<SocketAddr> {
    let mut endpoints = endpoints
        .into_iter()
        .filter(|endpoint| !endpoint.ip().is_unspecified() && !endpoint.ip().is_loopback())
        .map(client_mesh_host_endpoint)
        .fold(Vec::<SocketAddr>::new(), |mut unique, endpoint| {
            if !unique
                .iter()
                .any(|known| same_client_mesh_host(*known, endpoint))
            {
                unique.push(endpoint);
            }
            unique
        });
    endpoints.sort_by_key(|endpoint| std::cmp::Reverse(cpp_local_address_rank(*endpoint)));
    endpoints
}

fn client_mesh_host_endpoint(endpoint: SocketAddr) -> SocketAddr {
    let endpoint = NetworkAddress::new(NetworkProtocol::Udp, endpoint).endpoint;
    match endpoint {
        SocketAddr::V4(endpoint) => SocketAddr::V4(SocketAddrV4::new(*endpoint.ip(), 0)),
        SocketAddr::V6(endpoint) => {
            SocketAddr::V6(SocketAddrV6::new(*endpoint.ip(), 0, 0, endpoint.scope_id()))
        }
    }
}

fn same_client_mesh_host(left: SocketAddr, right: SocketAddr) -> bool {
    NetworkAddress::new(NetworkProtocol::Udp, client_mesh_host_endpoint(left))
        == NetworkAddress::new(NetworkProtocol::Udp, client_mesh_host_endpoint(right))
}

fn cpp_local_address_rank(endpoint: SocketAddr) -> i32 {
    if cpp_is_link_local(endpoint) {
        100
    } else if cpp_is_private(endpoint) {
        150
    } else if endpoint.is_ipv6() {
        300
    } else {
        200
    }
}

fn cpp_is_link_local(endpoint: SocketAddr) -> bool {
    match endpoint.ip() {
        std::net::IpAddr::V4(ip) => {
            let octets = ip.octets();
            octets[0] == 169 && octets[1] == 254
        }
        std::net::IpAddr::V6(ip) => ip.is_unicast_link_local(),
    }
}

fn cpp_is_private(endpoint: SocketAddr) -> bool {
    match endpoint.ip() {
        std::net::IpAddr::V4(ip) => {
            let octets = ip.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
        }
        std::net::IpAddr::V6(ip) => ip.octets()[0] & 0xfe == 0xfc,
    }
}

/// Enumerates the non-loopback IPv4/IPv6 interface endpoints used by
/// [`client_mesh_local_addresses`]. The returned port is always zero; IPv6
/// link-local scope IDs are retained or recovered from the interface name.
#[cfg(unix)]
pub(crate) fn client_mesh_os_interface_endpoints() -> Vec<SocketAddr> {
    struct InterfaceAddresses(*mut libc::ifaddrs);

    impl Drop for InterfaceAddresses {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: this pointer is the successful allocation returned
                // by `getifaddrs` and this guard releases it exactly once.
                unsafe { libc::freeifaddrs(self.0) };
            }
        }
    }

    let mut head = std::ptr::null_mut();
    // SAFETY: `getifaddrs` initializes `head` on success. The guard below owns
    // and eventually releases that complete linked list.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Vec::new();
    }
    let addresses = InterfaceAddresses(head);
    let mut endpoints = Vec::new();
    let mut current = addresses.0;
    while !current.is_null() {
        // SAFETY: `current` belongs to the live list owned by `addresses`.
        let interface = unsafe { &*current };
        let raw = interface.ifa_addr;
        if !raw.is_null() && interface.ifa_flags & libc::IFF_LOOPBACK as u32 == 0 {
            // SAFETY: every sockaddr variant starts with its family field.
            let family = unsafe { (*raw).sa_family as i32 };
            match family {
                libc::AF_INET => {
                    // SAFETY: the family check establishes `sockaddr_in`.
                    let address = unsafe { &*raw.cast::<libc::sockaddr_in>() };
                    let ip = Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes());
                    if !ip.is_loopback() && !ip.is_unspecified() {
                        endpoints.push(SocketAddr::V4(SocketAddrV4::new(ip, 0)));
                    }
                }
                libc::AF_INET6 => {
                    // SAFETY: the family check establishes `sockaddr_in6`.
                    let address = unsafe { &*raw.cast::<libc::sockaddr_in6>() };
                    let ip = Ipv6Addr::from(address.sin6_addr.s6_addr);
                    if !ip.is_loopback() && !ip.is_unspecified() {
                        let scope_id = if address.sin6_scope_id != 0 {
                            address.sin6_scope_id
                        } else if ip.is_unicast_link_local() && !interface.ifa_name.is_null() {
                            // SAFETY: `ifa_name` is a NUL-terminated interface
                            // name for the lifetime of this list node.
                            unsafe { libc::if_nametoindex(interface.ifa_name) }
                        } else {
                            0
                        };
                        endpoints.push(SocketAddr::V6(SocketAddrV6::new(ip, 0, 0, scope_id)));
                    }
                }
                _ => {}
            }
        }
        current = interface.ifa_next;
    }
    sorted_client_mesh_interface_endpoints(endpoints)
}

#[cfg(not(unix))]
pub(crate) fn client_mesh_os_interface_endpoints() -> Vec<SocketAddr> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

    use super::*;

    fn tcp(last_octet: u8, port: u16) -> NetworkAddress {
        NetworkAddress::new(
            NetworkProtocol::Tcp,
            SocketAddr::from(([198, 51, 100, last_octet], port)),
        )
    }

    fn udp(last_octet: u8, port: u16) -> NetworkAddress {
        NetworkAddress::new(
            NetworkProtocol::Udp,
            SocketAddr::from(([198, 51, 100, last_octet], port)),
        )
    }

    fn stored_addresses(state: &ClientMeshPeerState) -> Vec<NetworkAddress> {
        state
            .addresses()
            .iter()
            .map(|entry| entry.address())
            .collect()
    }

    #[test]
    fn local_addresses_use_cpp_wildcard_rank_and_per_interface_protocol_order() {
        let tcp_bound = SocketAddr::from(([0, 0, 0, 0], 11_112));
        let udp_bound = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 11_113, 0, 0));
        let global_v6 = SocketAddr::V6(SocketAddrV6::new(
            "2001:db8::7".parse().unwrap(),
            41_000,
            0,
            0,
        ));
        let global_v4 = SocketAddr::from(([203, 0, 113, 7], 42_000));
        let private_v6 =
            SocketAddr::V6(SocketAddrV6::new("fd00::7".parse().unwrap(), 43_000, 0, 0));
        let private_v4 = SocketAddr::from(([192, 168, 1, 7], 44_000));
        let link_local_v6 =
            SocketAddr::V6(SocketAddrV6::new("fe80::7".parse().unwrap(), 45_000, 91, 7));
        let link_local_v4 = SocketAddr::from(([169, 254, 1, 7], 46_000));

        let actual = client_mesh_local_addresses(
            Some(tcp_bound),
            Some(udp_bound),
            [
                link_local_v6,
                private_v6,
                global_v4,
                private_v4,
                global_v6,
                link_local_v4,
                SocketAddr::from(([127, 0, 0, 1], 47_000)),
                // Ports do not distinguish interface hosts; keep the first.
                SocketAddr::from(([203, 0, 113, 7], 48_000)),
            ],
        );

        let with_port = |mut endpoint: SocketAddr, port: u16| {
            endpoint.set_port(port);
            endpoint
        };
        let mut expected = vec![
            NetworkAddress::new(
                NetworkProtocol::Tcp,
                SocketAddr::from(([0, 0, 0, 0], 11_112)),
            ),
            NetworkAddress::new(
                NetworkProtocol::Udp,
                SocketAddr::from(([0, 0, 0, 0], 11_113)),
            ),
        ];
        for endpoint in [
            global_v6,
            global_v4,
            private_v6,
            private_v4,
            link_local_v6,
            link_local_v4,
        ] {
            expected.push(NetworkAddress::new(
                NetworkProtocol::Tcp,
                with_port(endpoint, 11_112),
            ));
            expected.push(NetworkAddress::new(
                NetworkProtocol::Udp,
                with_port(endpoint, 11_113),
            ));
        }
        assert_eq!(actual, expected);

        let scoped = actual
            .iter()
            .find(|address| {
                address.protocol == NetworkProtocol::Tcp
                    && address.endpoint.ip() == link_local_v6.ip()
            })
            .expect("scoped IPv6 TCP address");
        let SocketAddr::V6(scoped) = scoped.endpoint else {
            panic!("link-local IPv6 address changed family");
        };
        assert_eq!(scoped.scope_id(), 7);
        assert_eq!(scoped.flowinfo(), 0);
    }

    #[test]
    fn explicit_loopback_bind_is_advertised_without_interface_expansion() {
        let tcp_bound = SocketAddr::from(([127, 0, 0, 1], 21_112));
        let udp_bound = SocketAddr::from(([127, 0, 0, 1], 21_113));
        assert_eq!(
            client_mesh_local_addresses(
                Some(tcp_bound),
                Some(udp_bound),
                [
                    SocketAddr::from(([203, 0, 113, 8], 0)),
                    SocketAddr::from(([127, 0, 0, 1], 0)),
                ],
            ),
            [
                NetworkAddress::new(
                    NetworkProtocol::Tcp,
                    SocketAddr::from(([0, 0, 0, 0], 21_112)),
                ),
                NetworkAddress::new(
                    NetworkProtocol::Udp,
                    SocketAddr::from(([0, 0, 0, 0], 21_113)),
                ),
                NetworkAddress::new(NetworkProtocol::Tcp, tcp_bound),
                NetworkAddress::new(NetworkProtocol::Udp, udp_bound),
            ]
        );
    }

    #[test]
    fn mixed_explicit_and_unspecified_binds_expand_only_the_unspecified_protocol() {
        let tcp_bound = SocketAddr::from(([127, 0, 0, 1], 31_112));
        let udp_bound = SocketAddr::from(([0, 0, 0, 0], 31_113));
        let interface = SocketAddr::from(([203, 0, 113, 9], 0));
        assert_eq!(
            client_mesh_local_addresses(
                Some(tcp_bound),
                Some(udp_bound),
                [interface, SocketAddr::from(([127, 0, 0, 1], 0))],
            ),
            [
                NetworkAddress::new(
                    NetworkProtocol::Tcp,
                    SocketAddr::from(([0, 0, 0, 0], 31_112)),
                ),
                NetworkAddress::new(
                    NetworkProtocol::Udp,
                    SocketAddr::from(([0, 0, 0, 0], 31_113)),
                ),
                NetworkAddress::new(NetworkProtocol::Tcp, tcp_bound),
                NetworkAddress::new(
                    NetworkProtocol::Udp,
                    SocketAddr::from(([203, 0, 113, 9], 31_113)),
                ),
            ]
        );
    }

    #[test]
    fn absent_listener_omits_its_wildcard_and_interface_protocol() {
        let interface = SocketAddr::from(([192, 168, 1, 9], 0));
        assert_eq!(
            client_mesh_local_addresses(
                None,
                Some(SocketAddr::from(([0, 0, 0, 0], 41_113))),
                [interface],
            ),
            [
                NetworkAddress::new(
                    NetworkProtocol::Udp,
                    SocketAddr::from(([0, 0, 0, 0], 41_113)),
                ),
                NetworkAddress::new(
                    NetworkProtocol::Udp,
                    SocketAddr::from(([192, 168, 1, 9], 41_113)),
                ),
            ]
        );
        assert!(client_mesh_local_addresses(None, None, [interface]).is_empty());
    }

    #[test]
    fn ordinary_addresses_append_dedupe_and_preserve_attempt_state() {
        let now = Duration::from_secs(100);
        let first = tcp(1, 11_112);
        let second = udp(1, 11_113);
        let mut state = ClientMeshPeerState::new();

        assert_eq!(
            state.add_address(first, now),
            AddressInsertion::Added { index: 0 }
        );
        assert_eq!(
            state.add_address(second, now + Duration::from_secs(1)),
            AddressInsertion::Added { index: 1 }
        );
        assert_eq!(state.next_attempt_at(), Some(now));
        assert!(matches!(
            state.do_connect_attempt(now, ClientMeshConnectivity::disconnected(true, true)),
            ClientMeshConnectDecision::Dial(ClientMeshDialAttempt {
                address_index: 0,
                connection_attempt: 1,
                ..
            })
        ));

        assert_eq!(
            state.add_address(first, now + Duration::from_secs(2)),
            AddressInsertion::AlreadyPresent { index: 0 }
        );
        assert_eq!(stored_addresses(&state), [first, second]);
        assert_eq!(state.addresses()[0].connection_attempts(), 1);
        assert_eq!(
            state.next_attempt_at(),
            Some(now + CLIENT_MESH_CONNECT_INTERVAL)
        );
    }

    #[test]
    fn cpp_three_attempt_constant_produces_four_dials_then_ten_second_backoff() {
        let start = Duration::from_secs(100);
        let address = tcp(2, 11_112);
        let connectivity = ClientMeshConnectivity::disconnected(true, false);
        let mut state = ClientMeshPeerState::new();
        state.add_address(address, start);

        for (ordinal, offset) in [0, 6, 12, 18].into_iter().enumerate() {
            let now = start + Duration::from_secs(offset);
            assert!(state.scheduled_attempt_due(now));
            assert_eq!(
                state.do_connect_attempt(now, connectivity),
                ClientMeshConnectDecision::Dial(ClientMeshDialAttempt {
                    address_index: 0,
                    address,
                    connection_attempt: ordinal as u32 + 1,
                    next_attempt_at: now + CLIENT_MESH_CONNECT_INTERVAL,
                })
            );
        }

        let before_fifth = start + Duration::from_secs(23);
        assert!(!state.scheduled_attempt_due(before_fifth));
        assert_eq!(
            state.do_connect_attempt(before_fifth, connectivity),
            ClientMeshConnectDecision::NotDue {
                next_attempt_at: start + Duration::from_secs(24),
            }
        );

        let exhausted = start + Duration::from_secs(24);
        assert_eq!(
            state.do_connect_attempt(exhausted, connectivity),
            ClientMeshConnectDecision::Backoff {
                next_attempt_at: start + Duration::from_secs(34),
            }
        );
        assert_eq!(state.addresses()[0].connection_attempts(), 4);
        assert!(!state.scheduled_attempt_due(start + Duration::from_secs(33)));
        assert_eq!(
            state.do_connect_attempt(start + Duration::from_secs(34), connectivity),
            ClientMeshConnectDecision::Backoff {
                next_attempt_at: start + Duration::from_secs(44),
            }
        );
    }

    #[test]
    fn best_address_selection_is_stable_and_balances_attempt_counts() {
        let start = Duration::from_secs(200);
        let addresses = [tcp(1, 11_112), tcp(2, 11_112), tcp(3, 11_112)];
        let connectivity = ClientMeshConnectivity::disconnected(true, false);
        let mut state = ClientMeshPeerState::new();
        for address in addresses {
            state.add_address(address, start);
        }

        for (ordinal, expected_index) in [0, 1, 2, 0, 1, 2].into_iter().enumerate() {
            let now = start + CLIENT_MESH_CONNECT_INTERVAL * ordinal as u32;
            let ClientMeshConnectDecision::Dial(attempt) =
                state.do_connect_attempt(now, connectivity)
            else {
                panic!("expected a concrete dial at ordinal {ordinal}");
            };
            assert_eq!(attempt.address_index, expected_index);
            assert_eq!(attempt.address, addresses[expected_index]);
        }
        assert_eq!(
            state
                .addresses()
                .iter()
                .map(ClientMeshAddressState::connection_attempts)
                .collect::<Vec<_>>(),
            [2, 2, 2]
        );
    }

    #[test]
    fn selection_filters_null_hosts_unavailable_unknown_and_connected_protocols() {
        let start = Duration::from_secs(300);
        let null_host_with_port = NetworkAddress::new(
            NetworkProtocol::Tcp,
            SocketAddr::from(([0, 0, 0, 0], 11_112)),
        );
        let unknown = NetworkAddress::new(NetworkProtocol::Unknown(9), tcp(9, 11_112).endpoint);
        let tcp_address = tcp(4, 11_112);
        let udp_address = udp(4, 11_113);
        let mut state = ClientMeshPeerState::new();
        for address in [null_host_with_port, unknown, tcp_address, udp_address] {
            state.add_address(address, start);
        }

        let ClientMeshConnectDecision::Dial(udp_attempt) =
            state.do_connect_attempt(start, ClientMeshConnectivity::disconnected(false, true))
        else {
            panic!("available UDP address was not selected");
        };
        assert_eq!(udp_attempt.address, udp_address);

        let ClientMeshConnectDecision::Dial(tcp_attempt) = state.do_connect_attempt(
            start + CLIENT_MESH_CONNECT_INTERVAL,
            ClientMeshConnectivity::single_route(NetworkProtocol::Udp, true, true),
        ) else {
            panic!("TCP address was not selected around the connected UDP protocol");
        };
        assert_eq!(tcp_attempt.address, tcp_address);

        let backoff_now = start + CLIENT_MESH_CONNECT_INTERVAL * 2;
        assert_eq!(
            state.do_connect_attempt(
                backoff_now,
                ClientMeshConnectivity::single_route(NetworkProtocol::Tcp, true, false),
            ),
            ClientMeshConnectDecision::Backoff {
                next_attempt_at: backoff_now + CLIENT_MESH_CONNECT_BACKOFF,
            }
        );
    }

    #[test]
    fn distinct_routes_refresh_backoff_before_the_due_time_gate() {
        let start = Duration::from_secs(400);
        let mut state = ClientMeshPeerState::new();
        state.add_address(tcp(5, 11_112), start);
        assert!(matches!(
            state.do_connect_attempt(start, ClientMeshConnectivity::disconnected(true, true)),
            ClientMeshConnectDecision::Dial(_)
        ));

        let explicit_call = start + Duration::from_secs(1);
        assert_eq!(
            state.do_connect_attempt(
                explicit_call,
                ClientMeshConnectivity::distinct_routes(
                    NetworkProtocol::Udp,
                    NetworkProtocol::Tcp,
                    true,
                    true,
                ),
            ),
            ClientMeshConnectDecision::Backoff {
                next_attempt_at: explicit_call + CLIENT_MESH_CONNECT_BACKOFF,
            }
        );
    }

    #[test]
    fn adding_an_address_does_not_interrupt_an_existing_backoff() {
        let start = Duration::from_secs(500);
        let mut state = ClientMeshPeerState::new();
        state.add_address(
            NetworkAddress::new(NetworkProtocol::Unknown(7), tcp(6, 11_112).endpoint),
            start,
        );
        assert_eq!(
            state.do_connect_attempt(start, ClientMeshConnectivity::disconnected(true, true)),
            ClientMeshConnectDecision::Backoff {
                next_attempt_at: start + CLIENT_MESH_CONNECT_BACKOFF,
            }
        );

        let tcp_address = tcp(6, 11_112);
        state.add_address(tcp_address, start + Duration::from_secs(1));
        assert_eq!(
            state.do_connect_attempt(
                start + Duration::from_secs(1),
                ClientMeshConnectivity::disconnected(true, true),
            ),
            ClientMeshConnectDecision::NotDue {
                next_attempt_at: start + CLIENT_MESH_CONNECT_BACKOFF,
            }
        );
    }

    #[test]
    fn puncher_variants_announce_in_call_order_and_store_in_reverse_front_order() {
        let now = Duration::from_secs(600);
        let observed = SocketAddr::from(([203, 0, 113, 9], 40_000));
        let outside_udp = NetworkAddress::new(NetworkProtocol::Udp, observed);
        let inside_udp = NetworkAddress::new(
            NetworkProtocol::Udp,
            SocketAddr::from(([203, 0, 113, 9], 11_113)),
        );
        let tcp_address = NetworkAddress::new(
            NetworkProtocol::Tcp,
            SocketAddr::from(([203, 0, 113, 9], 11_112)),
        );
        let mut state = ClientMeshPeerState::new();

        let update = state.add_address_from_puncher(observed, 11_113, 11_112, now);
        assert_eq!(update.announcements, [outside_udp, inside_udp, tcp_address]);
        assert_eq!(
            stored_addresses(&state),
            [tcp_address, inside_udp, outside_udp]
        );
        assert_eq!(state.next_attempt_at(), Some(now));
        assert_eq!(update.ipv6_simultaneous_open_address, None);

        let duplicate =
            state.add_address_from_puncher(observed, 11_113, 11_112, now + Duration::from_secs(1));
        assert!(duplicate.announcements.is_empty());
        assert_eq!(
            stored_addresses(&state),
            [tcp_address, inside_udp, outside_udp]
        );
    }

    #[test]
    fn puncher_skips_equal_inside_udp_and_disabled_tcp_but_keeps_zero_udp_port() {
        let observed = SocketAddr::from(([203, 0, 113, 10], 11_113));
        assert_eq!(
            client_mesh_puncher_variants(observed, 11_113, 0),
            [NetworkAddress::new(NetworkProtocol::Udp, observed)]
        );

        let observed_other_port = SocketAddr::from(([203, 0, 113, 10], 40_000));
        assert_eq!(
            client_mesh_puncher_variants(observed_other_port, 0, 0),
            [
                NetworkAddress::new(NetworkProtocol::Udp, observed_other_port),
                NetworkAddress::new(
                    NetworkProtocol::Udp,
                    SocketAddr::from(([203, 0, 113, 10], 0)),
                ),
            ]
        );
    }

    #[test]
    fn puncher_remembers_only_real_ipv6_for_later_simultaneous_open() {
        let now = Duration::from_secs(700);
        let ipv6 = SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 7),
            40_000,
            0,
            0,
        ));
        let mut state = ClientMeshPeerState::new();
        let update = state.add_address_from_puncher(ipv6, 11_113, 11_112, now);
        assert_eq!(update.ipv6_simultaneous_open_address, Some(ipv6));
        assert_eq!(state.ipv6_address_from_puncher(), Some(ipv6));

        let mapped = SocketAddr::V6(SocketAddrV6::new(
            Ipv4Addr::new(192, 0, 2, 3).to_ipv6_mapped(),
            40_001,
            0,
            0,
        ));
        let mapped_update =
            state.add_address_from_puncher(mapped, 11_113, 11_112, now + Duration::from_secs(1));
        assert_eq!(mapped_update.ipv6_simultaneous_open_address, None);
        assert_eq!(state.ipv6_address_from_puncher(), Some(ipv6));
        assert!(mapped_update
            .announcements
            .iter()
            .all(|address| address.endpoint.is_ipv4()));
    }

    #[test]
    fn tcp_simultaneous_open_requires_lower_id_global_ipv6_tcp_and_no_pending_socket() {
        let global =
            NetworkAddress::new(NetworkProtocol::Tcp, "[2001:db8::7]:11112".parse().unwrap());
        assert!(client_mesh_tcp_sim_open_eligible(1, 2, global, false));
        assert!(!client_mesh_tcp_sim_open_eligible(2, 1, global, false));
        assert!(!client_mesh_tcp_sim_open_eligible(1, 2, global, true));
        assert!(!client_mesh_tcp_sim_open_eligible(
            1,
            2,
            NetworkAddress::new(NetworkProtocol::Udp, global.endpoint),
            false,
        ));
        for endpoint in ["[fe80::7]:11112", "[fd00::7]:11112", "127.0.0.1:11112"] {
            assert!(!client_mesh_tcp_sim_open_eligible(
                1,
                2,
                NetworkAddress::new(NetworkProtocol::Tcp, endpoint.parse().unwrap()),
                false,
            ));
        }
    }
}
