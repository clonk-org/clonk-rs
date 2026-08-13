//! Startup network-game discovery matching `C4StartupNetDlg` and
//! `C4Network2IODiscoverClient`.

#[cfg(unix)]
use std::collections::BTreeSet;
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::ops::Not as _;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use socket2::{Protocol, SockRef, Socket, Type};
use thiserror::Error;
use tokio::net::UdpSocket;

use crate::address_packet::decode_cpp_endpoint;
use crate::{NetworkAddress, NetworkProtocol};

pub const DEFAULT_MASTER_SERVER_URL: &str = "https://league.clonkspot.org/";
pub const DEFAULT_REFERENCE_PORT: u16 = 11_111;
pub const DEFAULT_DISCOVERY_PORT: u16 = 11_114;
pub const MAX_LAN_DISCOVERS: usize = 64;
pub const REFERENCE_QUERY_TIMEOUT: Duration = Duration::from_secs(12);
pub const GAME_SEARCH_INTERVAL: Duration = Duration::from_secs(30);
/// How often the startup browser re-probes the LAN for games.
///
/// Deliberately shorter than the oracle's `C4NetGameDiscoveryInterval`, which
/// leaves a game opened while the browser is on screen invisible for half a
/// minute. Every probe makes each host on the group multicast an announce, and
/// a C++ client re-fetches a reference for each announce it sees
/// (src/C4StartupNetDlg.cpp:1133-1154,590-600), so this is not free to shorten
/// further; the host's own opening announce covers the first seconds instead.
pub const LAN_DISCOVERY_INTERVAL: Duration = Duration::from_secs(5);
const MASTERSERVER_FAST_RETRIES: u8 = 2;
const REFERENCE_LIFETIME: Duration = Duration::from_secs(42);
const EMPTY_REFERENCE_LIFETIME: Duration = Duration::from_secs(10);

const DISCOVERY_PROBE: u8 = 0x03;
const DISCOVERY_REPLY: u8 = 0x04;
/// `MCGrpInfo.ipv6mr_interface = 0; // Default interface` — the only interface
/// C++ ever joins on (pinned oracle src/C4NetIO.cpp:1624).
const DEFAULT_MULTICAST_INTERFACE: u32 = 0;
const SCOPED_IPV6_REQUEST_HOST: &str = "clonk-rust-lan.invalid";
pub(crate) const DISCOVERY_MULTICAST: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

pub const CURRENT_GAME_VERSION: [i32; 4] = [4, 9, 11, 0];
pub const CURRENT_GAME_BUILD: i32 = 362;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MasterserverVersion {
    pub version: [i32; 4],
    pub build: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MasterserverReplyInfo {
    pub version: Option<MasterserverVersion>,
    pub motd: String,
    pub motd_url: String,
    pub league_server_redirect: String,
    /// Number of references returned by this masterserver request before
    /// they are merged with LAN and direct-query results.
    pub game_count: usize,
    /// Active, visible players in those references.
    pub player_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceQueryResponse {
    pub references: Vec<NetworkGameReference>,
    pub masterserver: MasterserverReplyInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkGameReference {
    pub icon: i32,
    pub title: String,
    pub host_name: String,
    pub host_nick: String,
    pub state: String,
    pub control_mode: i32,
    /// Elapsed game time serialized by `C4Network2Reference::Time`.
    pub time: i32,
    pub start_time: i64,
    pub comment: String,
    pub join_allowed: bool,
    pub password_needed: bool,
    pub official_server: bool,
    pub use_fair_crew: bool,
    pub goals: Vec<String>,
    pub league: String,
    pub league_address: String,
    pub max_players: i32,
    /// Active, non-invisible `PlayerInfos` names in wire order.
    pub player_names: Vec<String>,
    pub game: String,
    pub version: [i32; 4],
    pub build: i32,
    /// Complete ordered `C4Network2Reference::Addrs` transport set.
    pub addresses: Vec<NetworkAddress>,
    /// Server endpoint retained by `C4Network2Reference::SetSourceAddress`.
    pub source_address: SocketAddr,
    /// IPv4 and IPv6 game IDs assigned by the C++ netpuncher.
    pub netpuncher_ipv4: u32,
    pub netpuncher_ipv6: u32,
    /// Configured puncher endpoint advertised by the host.
    pub netpuncher_address: String,
    /// Transitional TCP display projection retained for existing consumers.
    pub tcp_addresses: Vec<SocketAddr>,
}

/// Prepared C++ client routes before `InitClient` starts the transports.
///
/// `logical_addresses` retains one prepared reference address per advertised
/// route for progress/diagnostic presentation. `dial_attempts` expands local
/// routes over the machine's interface IDs for the actual transports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkJoinRoutePlan {
    pub logical_addresses: Vec<NetworkAddress>,
    pub dial_attempts: Vec<NetworkAddress>,
}

impl Default for NetworkGameReference {
    fn default() -> Self {
        Self {
            icon: 0,
            title: String::new(),
            host_name: String::new(),
            host_nick: String::new(),
            state: "None".to_string(),
            control_mode: -1,
            time: 0,
            start_time: 0,
            comment: String::new(),
            join_allowed: true,
            password_needed: false,
            official_server: false,
            use_fair_crew: false,
            goals: Vec::new(),
            league: String::new(),
            league_address: String::new(),
            max_players: 0,
            player_names: Vec::new(),
            game: "None".to_string(),
            version: [0; 4],
            build: -1,
            addresses: Vec::new(),
            source_address: SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)),
            netpuncher_ipv4: 0,
            netpuncher_ipv6: 0,
            netpuncher_address: String::new(),
            tcp_addresses: Vec::new(),
        }
    }
}

impl NetworkGameReference {
    /// Reference-backed Rust clients present the selected host's advertised
    /// build during admission, so build/version display differences do not
    /// make an otherwise open game unjoinable.
    pub fn is_joinable(&self) -> bool {
        self.join_allowed
    }

    pub fn is_lobby_active(&self) -> bool {
        self.state.eq_ignore_ascii_case("lobby")
    }

    pub fn is_past_lobby(&self) -> bool {
        self.state.eq_ignore_ascii_case("paused") || self.state.eq_ignore_ascii_case("running")
    }

    /// Copies and prepares the reference addresses in C++ client join order.
    pub fn join_addresses(&self, have_global_ipv6: bool) -> Vec<NetworkAddress> {
        let source_scope_id = match self.source_address {
            SocketAddr::V4(_) => 0,
            SocketAddr::V6(source) => source.scope_id(),
        };
        let mut addresses = self.addresses.clone();
        for address in &mut addresses {
            if let SocketAddr::V6(endpoint) = &mut address.endpoint {
                if endpoint.ip().is_unicast_link_local() {
                    endpoint.set_scope_id(source_scope_id);
                }
            }
        }
        addresses.sort_by_key(|address| {
            std::cmp::Reverse(cpp_join_address_rank(address.endpoint, have_global_ipv6))
        });
        addresses
    }

    /// Expands prepared addresses into the endpoints C++ actually attempts.
    pub fn join_attempts(
        &self,
        have_global_ipv6: bool,
        local_interface_ids: &[u32],
    ) -> Vec<NetworkAddress> {
        self.join_route_plan(have_global_ipv6, local_interface_ids)
            .dial_attempts
    }

    /// Keeps C++'s original prepared address list distinct from the scoped
    /// endpoints generated for transport connection attempts.
    pub fn join_route_plan(
        &self,
        have_global_ipv6: bool,
        local_interface_ids: &[u32],
    ) -> NetworkJoinRoutePlan {
        let logical_addresses = self
            .join_addresses(have_global_ipv6)
            .into_iter()
            .filter(|address| !address.is_ip_null())
            .collect::<Vec<_>>();
        let mut dial_attempts = Vec::new();
        for address in logical_addresses.iter().copied() {
            if cpp_is_local_address(address.endpoint) {
                for &scope_id in local_interface_ids {
                    let mut attempt = address;
                    if let SocketAddr::V6(endpoint) = &mut attempt.endpoint {
                        endpoint.set_scope_id(scope_id);
                    }
                    dial_attempts.push(attempt);
                }
            } else {
                dial_attempts.push(address);
            }
        }
        NetworkJoinRoutePlan {
            logical_addresses,
            dial_attempts,
        }
    }

    /// Prepares the complete reference attempt list against this machine's
    /// IPv6 capabilities, matching the inputs C++ obtains from its local
    /// client/interface inventory before `InitClient` starts every route.
    pub fn join_attempts_for_local_host(&self) -> Vec<NetworkAddress> {
        self.join_route_plan_for_local_host().dial_attempts
    }

    /// Prepares logical reference routes and expanded dial attempts from the
    /// same snapshot of this machine's IPv6 capabilities.
    pub fn join_route_plan_for_local_host(&self) -> NetworkJoinRoutePlan {
        let (have_global_ipv6, local_interface_ids) = local_join_capabilities();
        self.join_route_plan(have_global_ipv6, &local_interface_ids)
    }

    fn is_same_host_and_address(&self, other: &Self) -> bool {
        self.host_name == other.host_name
            && if self.addresses.is_empty() || other.addresses.is_empty() {
                self.tcp_addresses
                    .iter()
                    .any(|address| other.tcp_addresses.contains(address))
            } else {
                self.addresses
                    .iter()
                    .any(|address| other.addresses.contains(address))
            }
    }

    fn sort_order(&self, use_alternate_server: bool) -> i32 {
        i32::from(self.official_server && !use_alternate_server) * 50
            + i32::from(self.is_joinable()) * 25
            + i32::from(!self.league_address.is_empty()) * 5
            + i32::from(self.state == "Lobby") * 3
            + i32::from(!self.password_needed)
    }
}

#[cfg(unix)]
fn local_join_capabilities() -> (bool, Vec<u32>) {
    let mut addresses = std::ptr::null_mut();
    // SAFETY: `getifaddrs` initializes a linked list owned by the caller on
    // success. Every pointer is checked before access and the list is released
    // exactly once with `freeifaddrs` below.
    if unsafe { libc::getifaddrs(&mut addresses) } != 0 {
        return (false, Vec::new());
    }
    let mut have_global_ipv6 = false;
    let mut interface_ids = BTreeSet::new();
    let mut current = addresses;
    while !current.is_null() {
        // SAFETY: `current` belongs to the live list returned above.
        let interface = unsafe { &*current };
        let address = interface.ifa_addr;
        if !address.is_null()
            // SAFETY: all sockaddr variants begin with `sa_family`.
            && unsafe { (*address).sa_family as i32 } == libc::AF_INET6
            && interface.ifa_flags & (libc::IFF_LOOPBACK as u32) == 0
        {
            // SAFETY: the family check establishes an IPv6 sockaddr.
            let address = unsafe { &*(address.cast::<libc::sockaddr_in6>()) };
            let ip = Ipv6Addr::from(address.sin6_addr.s6_addr);
            have_global_ipv6 |= cpp_is_global_ipv6(ip);
            if ip.is_unicast_link_local() && !interface.ifa_name.is_null() {
                // SAFETY: `ifa_name` is a NUL-terminated interface name for
                // the lifetime of the enclosing `ifaddrs` node.
                let index = if address.sin6_scope_id != 0 {
                    address.sin6_scope_id
                } else {
                    unsafe { libc::if_nametoindex(interface.ifa_name) }
                };
                if index != 0 {
                    interface_ids.insert(index);
                }
            }
        }
        current = interface.ifa_next;
    }
    // SAFETY: this is the successful allocation returned by `getifaddrs`.
    unsafe { libc::freeifaddrs(addresses) };
    (have_global_ipv6, interface_ids.into_iter().collect())
}

#[cfg(not(unix))]
fn local_join_capabilities() -> (bool, Vec<u32>) {
    // Conservative fallback: IPv4/global routes remain usable and global IPv6
    // is ranked after them. Link-local expansion requires platform interface
    // enumeration and is therefore omitted rather than guessing a scope.
    (false, Vec::new())
}

fn cpp_is_global_ipv6(ip: Ipv6Addr) -> bool {
    let first = ip.octets()[0];
    !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && !ip.is_unicast_link_local()
        && first & 0xfe != 0xfc
}

fn cpp_join_address_rank(endpoint: SocketAddr, have_global_ipv6: bool) -> i32 {
    if cpp_is_local_address(endpoint) {
        100
    } else if cpp_is_private_address(endpoint) {
        150
    } else {
        match endpoint {
            SocketAddr::V4(_) => 200,
            SocketAddr::V6(_) if have_global_ipv6 => 300,
            SocketAddr::V6(_) => 0,
        }
    }
}

fn cpp_is_local_address(endpoint: SocketAddr) -> bool {
    match endpoint {
        SocketAddr::V4(endpoint) => {
            let octets = endpoint.ip().octets();
            octets[0] == 169 && octets[1] == 254
        }
        SocketAddr::V6(endpoint) => endpoint.ip().is_unicast_link_local(),
    }
}

fn cpp_is_private_address(endpoint: SocketAddr) -> bool {
    match endpoint {
        SocketAddr::V4(endpoint) => {
            let octets = endpoint.ip().octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
        }
        SocketAddr::V6(endpoint) => endpoint.ip().octets()[0] & 0xfe == 0xfc,
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReferenceParseError {
    #[error("invalid reference address `{0}`")]
    InvalidAddress(String),
    #[error("invalid integer `{value}` for reference key `{key}`")]
    InvalidInteger { key: String, value: String },
    #[error("unsupported reference charset `{0}`")]
    UnsupportedCharset(String),
}

#[derive(Debug, Error)]
pub enum ReferenceFetchError {
    #[error(
        "reference request failed: {message}",
        message = reference_request_error_message(.0)
    )]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Parse(#[from] ReferenceParseError),
}

fn reference_request_error_message(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        let cause_message = cause.to_string();
        if !message.ends_with(&cause_message) {
            message.push_str(": ");
            message.push_str(&cause_message);
        }
        source = cause.source();
    }
    message
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkGameSearchConfig {
    pub internet_enabled: bool,
    pub use_alternate_server: bool,
    pub master_server_url: String,
    pub discovery_port: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceQueryConfig {
    /// `Config.General.LanguageCharset`, before C++ code-page canonicalization.
    pub language_charset: String,
    /// `Config.General.LanguageEx`, preserved as the HTTP language preference.
    pub language_sequence: String,
    /// `Config.Network.UseCurl` (`C4Network2Reference.cpp:410-413`).
    pub http_backend: crate::HttpBackend,
}

impl ReferenceQueryConfig {
    fn charset_code_name(&self) -> &'static str {
        match self.language_charset.to_ascii_uppercase().as_str() {
            "SHIFTJIS" => "CP932",
            "HANGUL" => "CP949",
            "JOHAB" => "CP1361",
            "CHINESEBIG5" => "CP950",
            "GREEK" => "CP1253",
            "TURKISH" => "CP1254",
            "VIETNAMESE" => "CP1258",
            "HEBREW" => "CP1255",
            "ARABIC" => "CP1256",
            "BALTIC" => "CP1257",
            "RUSSIAN" => "CP1251",
            "THAI" => "CP874",
            "EASTEUROPE" => "CP1250",
            "UTF-8" => "UTF-8",
            _ => "CP1252",
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<String, ReferenceParseError> {
        let charset = self.charset_code_name();
        iconv_native::decode_lossy(bytes, charset)
            .map_err(|_| ReferenceParseError::UnsupportedCharset(charset.to_string()))
    }
}

impl Default for NetworkGameSearchConfig {
    fn default() -> Self {
        Self {
            internet_enabled: true,
            use_alternate_server: false,
            master_server_url: DEFAULT_MASTER_SERVER_URL.to_string(),
            discovery_port: DEFAULT_DISCOVERY_PORT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceQuerySource {
    GameDiscovery,
    Masterserver,
    DirectJoin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReferenceEndpoint {
    Url(String),
    Address(SocketAddr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanProbeTrigger {
    Initial,
    ExplicitRefresh,
    Periodic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchCommand {
    SendLanProbe {
        target: SocketAddrV6,
        payload: Vec<u8>,
        trigger: LanProbeTrigger,
    },
    QueryReferences {
        endpoint: ReferenceEndpoint,
        source: ReferenceQuerySource,
        timeout: Duration,
    },
}

#[derive(Clone, Debug)]
pub struct NetworkGameSearch {
    config: NetworkGameSearchConfig,
    lan_discover_count: usize,
    references: Vec<NetworkGameReference>,
    reference_expirations: Vec<Instant>,
}

impl NetworkGameSearch {
    pub fn new(mut config: NetworkGameSearchConfig) -> Self {
        config.master_server_url = normalize_master_server_url(&config.master_server_url);
        Self {
            config,
            lan_discover_count: 0,
            references: Vec::new(),
            reference_expirations: Vec::new(),
        }
    }

    pub fn initial_commands(&mut self) -> Vec<SearchCommand> {
        self.refresh_commands(LanProbeTrigger::Initial)
    }

    pub fn refresh(&mut self) -> Vec<SearchCommand> {
        self.refresh_commands(LanProbeTrigger::ExplicitRefresh)
    }

    fn refresh_commands(&mut self, trigger: LanProbeTrigger) -> Vec<SearchCommand> {
        self.lan_discover_count = 0;
        self.references.clear();
        self.reference_expirations.clear();
        let mut commands = vec![SearchCommand::SendLanProbe {
            target: SocketAddrV6::new(DISCOVERY_MULTICAST, self.config.discovery_port, 0, 0),
            payload: vec![DISCOVERY_PROBE],
            trigger,
        }];
        if self.config.internet_enabled {
            commands.push(SearchCommand::QueryReferences {
                endpoint: ReferenceEndpoint::Url(self.config.master_server_url.clone()),
                source: ReferenceQuerySource::Masterserver,
                timeout: REFERENCE_QUERY_TIMEOUT,
            });
        }
        commands
    }

    pub fn handle_lan_datagram(
        &mut self,
        source: SocketAddr,
        payload: &[u8],
    ) -> Option<SearchCommand> {
        // The C++ wire format is the native ABI layout of
        // `{ char c; uint16_t Port; }`: four bytes, with one ignored padding
        // byte before the native-endian port.
        if self.lan_discover_count >= MAX_LAN_DISCOVERS
            || payload.len() != 4
            || payload[0] != DISCOVERY_REPLY
        {
            return None;
        }
        let port = u16::from_ne_bytes([payload[2], payload[3]]);
        let endpoint = match source {
            SocketAddr::V4(mut address) => {
                address.set_port(port);
                SocketAddr::V4(address)
            }
            SocketAddr::V6(mut address) => {
                address.set_port(port);
                SocketAddr::V6(address)
            }
        };
        self.lan_discover_count += 1;
        Some(SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Address(endpoint),
            source: ReferenceQuerySource::GameDiscovery,
            timeout: REFERENCE_QUERY_TIMEOUT,
        })
    }

    pub fn references(&self) -> &[NetworkGameReference] {
        &self.references
    }

    pub fn set_internet_enabled(&mut self, enabled: bool) -> Option<SearchCommand> {
        if self.config.internet_enabled == enabled {
            return None;
        }
        self.config.internet_enabled = enabled;
        enabled.then(|| self.masterserver_query())
    }

    pub fn periodic_commands(&mut self) -> Vec<SearchCommand> {
        let mut commands = vec![self.periodic_lan_command()];
        if self.config.internet_enabled {
            commands.push(self.masterserver_query());
        }
        commands
    }

    fn periodic_lan_command(&mut self) -> SearchCommand {
        self.lan_discover_count = 0;
        SearchCommand::SendLanProbe {
            target: SocketAddrV6::new(DISCOVERY_MULTICAST, self.config.discovery_port, 0, 0),
            payload: vec![DISCOVERY_PROBE],
            trigger: LanProbeTrigger::Periodic,
        }
    }

    fn masterserver_query(&self) -> SearchCommand {
        SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Url(self.config.master_server_url.clone()),
            source: ReferenceQuerySource::Masterserver,
            timeout: REFERENCE_QUERY_TIMEOUT,
        }
    }

    pub fn merge_references(&mut self, references: impl IntoIterator<Item = NetworkGameReference>) {
        self.merge_references_at(Instant::now(), references);
    }

    fn merge_references_at(
        &mut self,
        observed_at: Instant,
        references: impl IntoIterator<Item = NetworkGameReference>,
    ) {
        let expires_at = observed_at
            .checked_add(REFERENCE_LIFETIME)
            .unwrap_or(observed_at);
        for incoming in references {
            if let Some(index) = self.references.iter().position(|existing| {
                existing.is_same_host_and_address(&incoming)
                    && existing.start_time <= incoming.start_time
            }) {
                self.references[index] = incoming;
                self.reference_expirations[index] = expires_at;
            } else {
                let sort_order = incoming.sort_order(self.config.use_alternate_server);
                let index = self
                    .references
                    .iter()
                    .position(|existing| {
                        existing.sort_order(self.config.use_alternate_server) < sort_order
                    })
                    .unwrap_or(self.references.len());
                self.references.insert(index, incoming);
                self.reference_expirations.insert(index, expires_at);
            }
        }
    }

    fn expire_references_at(&mut self, now: Instant) -> bool {
        let mut removed = false;
        for index in (0..self.reference_expirations.len()).rev() {
            if now >= self.reference_expirations[index] {
                self.reference_expirations.remove(index);
                self.references.remove(index);
                removed = true;
            }
        }
        removed
    }
}

fn normalize_master_server_url(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.contains("://") || matches!(value, "http:" | "https:") {
        value.to_string()
    } else {
        format!("http://{value}/")
    }
}

pub fn direct_reference_endpoint(
    address: &str,
    default_port: u16,
) -> Result<ReferenceEndpoint, String> {
    let address = address.trim();
    if address.is_empty() {
        return Err("direct reference address is empty".to_string());
    }
    if let Ok(address) = address.parse::<SocketAddr>() {
        return Ok(ReferenceEndpoint::Address(address));
    }
    if let Ok(address) = address.parse::<std::net::IpAddr>() {
        return Ok(ReferenceEndpoint::Address(SocketAddr::new(
            address,
            default_port,
        )));
    }

    let has_http_scheme = address
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"));
    let has_https_scheme = address
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
    if address.contains("://") && !has_http_scheme && !has_https_scheme {
        return Err(format!(
            "unsupported direct reference address scheme in `{address}`"
        ));
    }
    let candidate = if has_http_scheme || has_https_scheme {
        address.to_string()
    } else {
        format!("http://{address}")
    };
    let explicit_port = direct_url_has_explicit_port(&candidate);
    let mut url = reqwest::Url::parse(&candidate)
        .map_err(|error| format!("invalid direct reference address `{address}`: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(format!("invalid direct reference address `{address}`"));
    }
    if !explicit_port {
        url.set_port(Some(default_port)).map_err(|()| {
            format!("invalid direct reference port {default_port} for `{address}`")
        })?;
    }
    Ok(ReferenceEndpoint::Url(url.into()))
}

fn direct_url_has_explicit_port(url: &str) -> bool {
    let authority = url
        .split_once("://")
        .map_or(url, |(_, remainder)| remainder)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    if let Some(close) = authority.find(']') {
        return authority[close + 1..]
            .strip_prefix(':')
            .is_some_and(|port| {
                !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
            });
    }
    authority.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty() && !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit())
    })
}

#[derive(Clone, Debug)]
pub enum StartupGameSearchEvent {
    Cleared,
    ReferencesUpdated(Vec<NetworkGameReference>),
    GameDiscoveryQueryStarted {
        address: SocketAddr,
    },
    GameDiscoveryQueryResolved {
        address: SocketAddr,
        references: Vec<NetworkGameReference>,
        selected_index: Option<usize>,
    },
    GameDiscoveryQueryFailed {
        address: SocketAddr,
        message: String,
    },
    DirectQueryResolved {
        request_id: u64,
        references: Vec<NetworkGameReference>,
        selected_index: Option<usize>,
    },
    DirectQueryFailed {
        request_id: u64,
        message: String,
    },
    MasterserverReply(MasterserverReplyInfo),
    SearchError {
        source: Option<ReferenceQuerySource>,
        message: String,
    },
}

/// Which half of the discovery path a refresh has to report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LanProbeFailure {
    /// The socket could not be built, so no datagram was ever attempted.
    Unavailable,
    /// `sendto` itself failed — the only failure C++ carries into its refresh
    /// modal (pinned oracle src/C4NetIO.cpp:1784).
    Send,
}

fn lan_probe_error_event(
    trigger: LanProbeTrigger,
    failure: LanProbeFailure,
    error: io::Error,
) -> Option<StartupGameSearchEvent> {
    (trigger == LanProbeTrigger::ExplicitRefresh).then(|| StartupGameSearchEvent::SearchError {
        source: Some(ReferenceQuerySource::GameDiscovery),
        message: match failure {
            LanProbeFailure::Unavailable => format!("LAN discovery is unavailable: {error}"),
            LanProbeFailure::Send => format!("unable to send LAN discovery probe: {error}"),
        },
    })
}

fn masterserver_failure_allows_fast_retry(failures: &mut u8) -> bool {
    *failures = failures.saturating_add(1);
    *failures <= MASTERSERVER_FAST_RETRIES
}

#[derive(Clone, Debug)]
enum StartupGameSearchCommand {
    InitialRefresh,
    Refresh,
    SetInternetEnabled(bool),
    QueryDirect {
        request_id: u64,
        address: String,
        default_port: u16,
    },
    Stop,
}

pub struct StartupGameSearch {
    commands: mpsc::Sender<StartupGameSearchCommand>,
    events: mpsc::Receiver<StartupGameSearchEvent>,
    worker: Option<thread::JoinHandle<()>>,
}

impl StartupGameSearch {
    pub fn start(config: NetworkGameSearchConfig) -> io::Result<Self> {
        Self::start_with_reference_config(config, ReferenceQueryConfig::default())
    }

    pub fn start_with_reference_config(
        config: NetworkGameSearchConfig,
        reference_config: ReferenceQueryConfig,
    ) -> io::Result<Self> {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("clonk-game-search".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = event_tx.send(StartupGameSearchEvent::SearchError {
                            source: None,
                            message: format!("unable to start search runtime: {error}"),
                        });
                        return;
                    }
                };
                runtime.block_on(run_game_search(
                    config,
                    reference_config,
                    command_rx,
                    event_tx,
                ));
            })?;
        Ok(Self {
            commands: command_tx,
            events: event_rx,
            worker: Some(worker),
        })
    }

    pub fn refresh(&self) -> Result<(), mpsc::SendError<()>> {
        self.commands
            .send(StartupGameSearchCommand::Refresh)
            .map_err(|_| mpsc::SendError(()))
    }

    pub fn initial_refresh(&self) -> Result<(), mpsc::SendError<()>> {
        self.commands
            .send(StartupGameSearchCommand::InitialRefresh)
            .map_err(|_| mpsc::SendError(()))
    }

    pub fn set_internet_enabled(&self, enabled: bool) -> Result<(), mpsc::SendError<()>> {
        self.commands
            .send(StartupGameSearchCommand::SetInternetEnabled(enabled))
            .map_err(|_| mpsc::SendError(()))
    }

    pub fn query_direct(
        &self,
        request_id: u64,
        address: String,
        default_port: u16,
    ) -> Result<(), mpsc::SendError<()>> {
        self.commands
            .send(StartupGameSearchCommand::QueryDirect {
                request_id,
                address,
                default_port,
            })
            .map_err(|_| mpsc::SendError(()))
    }

    pub fn events(&self) -> &mpsc::Receiver<StartupGameSearchEvent> {
        &self.events
    }
}

impl Drop for StartupGameSearch {
    fn drop(&mut self) {
        let _ = self.commands.send(StartupGameSearchCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct QueryResult {
    generation: u64,
    masterserver_generation: u64,
    source: ReferenceQuerySource,
    endpoint: ReferenceEndpoint,
    direct_request_id: Option<u64>,
    result: Result<ReferenceQueryResponse, ReferenceFetchError>,
}

struct DiscoverySocket {
    socket: UdpSocket,
    multicast_interfaces: Vec<u32>,
}

impl DiscoverySocket {
    async fn send_probe(&self, payload: &[u8], target: SocketAddrV6) -> io::Result<()> {
        send_discovery_datagram(&self.socket, payload, target, &self.multicast_interfaces).await
    }
}

/// Sends one discovery datagram to every target the joined interface list
/// expands to, succeeding when any of them left the host.
pub(crate) async fn send_discovery_datagram(
    socket: &UdpSocket,
    payload: &[u8],
    target: SocketAddrV6,
    interfaces: &[u32],
) -> io::Result<()> {
    let mut last_error = None;
    let mut sent = false;
    for target in multicast_targets(target, interfaces) {
        if let Some(interface) = multicast_send_interface(&target) {
            if let Err(error) = SockRef::from(socket).set_multicast_if_v6(interface) {
                last_error = Some(error);
                continue;
            }
        }
        match socket.send_to(payload, target).await {
            Ok(_) => sent = true,
            Err(error) => last_error = Some(error),
        }
    }
    if sent {
        Ok(())
    } else {
        Err(last_error.unwrap_or_else(|| io::Error::from(io::ErrorKind::AddrNotAvailable)))
    }
}

/// Whether an explicit refresh should rebuild the discovery socket.
///
/// C4StartupNetDlg never re-inits its DiscoverClient (pristine 9ffa0a5d
/// src/C4StartupNetDlg.cpp:737, 1093-1105); the port rebuilds only a socket
/// that reaches no group at all, so that joining a network after the dialog
/// opened recovers without reopening it.
fn discovery_needs_rebuild(discovery: &io::Result<DiscoverySocket>) -> bool {
    discovery
        .as_ref()
        .is_ok_and(|socket| !socket.multicast_interfaces.is_empty())
        .not()
}

async fn run_game_search(
    config: NetworkGameSearchConfig,
    reference_config: ReferenceQueryConfig,
    commands: mpsc::Receiver<StartupGameSearchCommand>,
    events: mpsc::Sender<StartupGameSearchEvent>,
) {
    let mut search = NetworkGameSearch::new(config.clone());
    let mut discovery = discovery_socket(config.discovery_port);
    let (query_tx, mut query_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut generation = 0_u64;
    let mut masterserver_generation = 0_u64;
    let mut masterserver_failures = 0_u8;
    let mut masterserver_query: Option<tokio::task::JoinHandle<()>> = None;
    let mut discovery_queries = GameDiscoveryQueryGate::default();
    let mut stopped = false;
    let mut datagram = [0_u8; 512];
    let mut deadlines = SearchDeadlines::armed_at(Instant::now());

    while !stopped {
        while let Ok(command) = commands.try_recv() {
            match command {
                command @ (StartupGameSearchCommand::InitialRefresh
                | StartupGameSearchCommand::Refresh) => {
                    if let Some(query) = masterserver_query.take() {
                        query.abort();
                    }
                    generation = generation.wrapping_add(1);
                    masterserver_failures = 0;
                    discovery_queries.clear();
                    deadlines = SearchDeadlines::armed_at(Instant::now());
                    let _ = events.send(StartupGameSearchEvent::Cleared);
                    if matches!(command, StartupGameSearchCommand::Refresh)
                        && discovery_needs_rebuild(&discovery)
                    {
                        discovery = discovery_socket(config.discovery_port);
                    }
                    let commands = match command {
                        StartupGameSearchCommand::InitialRefresh => search.initial_commands(),
                        _ => search.refresh(),
                    };
                    for command in commands {
                        execute_search_command(
                            command,
                            (generation, masterserver_generation),
                            None,
                            &mut masterserver_query,
                            discovery.as_ref(),
                            &query_tx,
                            &events,
                            &reference_config,
                        )
                        .await;
                    }
                }
                StartupGameSearchCommand::SetInternetEnabled(enabled) => {
                    let changed = search.config.internet_enabled != enabled;
                    if changed {
                        masterserver_failures = 0;
                        deadlines.defer_masterserver_query_at(Instant::now());
                    }
                    if let Some(command) = search.set_internet_enabled(enabled) {
                        masterserver_generation = masterserver_generation.wrapping_add(1);
                        execute_search_command(
                            command,
                            (generation, masterserver_generation),
                            None,
                            &mut masterserver_query,
                            discovery.as_ref(),
                            &query_tx,
                            &events,
                            &reference_config,
                        )
                        .await;
                    } else if changed {
                        masterserver_generation = masterserver_generation.wrapping_add(1);
                        if let Some(query) = masterserver_query.take() {
                            query.abort();
                        }
                    }
                }
                StartupGameSearchCommand::QueryDirect {
                    request_id,
                    address,
                    default_port,
                } => match direct_reference_endpoint(&address, default_port) {
                    Ok(endpoint) => {
                        execute_search_command(
                            SearchCommand::QueryReferences {
                                endpoint,
                                source: ReferenceQuerySource::DirectJoin,
                                timeout: REFERENCE_QUERY_TIMEOUT,
                            },
                            (generation, masterserver_generation),
                            Some(request_id),
                            &mut masterserver_query,
                            discovery.as_ref(),
                            &query_tx,
                            &events,
                            &reference_config,
                        )
                        .await;
                    }
                    Err(message) => {
                        let _ = events.send(StartupGameSearchEvent::DirectQueryFailed {
                            request_id,
                            message,
                        });
                    }
                },
                StartupGameSearchCommand::Stop => {
                    if let Some(query) = masterserver_query.take() {
                        query.abort();
                    }
                    stopped = true;
                }
            }
        }
        while let Ok(query) = query_rx.try_recv() {
            // C4StartupNetDlg::DoRefresh deletes direct-query rows together
            // with discovered rows, so every query belongs to its generation.
            if query.generation != generation
                || (query.source == ReferenceQuerySource::Masterserver
                    && (query.masterserver_generation != masterserver_generation
                        || !search.config.internet_enabled))
            {
                continue;
            }
            let discovery_address = if query.source == ReferenceQuerySource::GameDiscovery {
                match query.endpoint {
                    ReferenceEndpoint::Address(address) => Some(address),
                    ReferenceEndpoint::Url(_) => None,
                }
            } else {
                None
            };
            if query.source == ReferenceQuerySource::Masterserver {
                masterserver_query.take();
                deadlines.defer_masterserver_query_at(Instant::now());
            }
            match query.result {
                Ok(response) => {
                    let now = Instant::now();
                    if let Some(address) = discovery_address {
                        discovery_queries.finish_at(
                            now,
                            address,
                            if response.references.is_empty() {
                                GameDiscoveryQueryOutcome::NoReferences
                            } else {
                                GameDiscoveryQueryOutcome::References
                            },
                        );
                    }
                    let ReferenceQueryResponse {
                        references,
                        masterserver,
                    } = response;
                    let selected_reference = (query.direct_request_id.is_some()
                        || discovery_address.is_some())
                    .then(|| references.first().cloned())
                    .flatten();
                    search.merge_references_at(now, references);
                    search.expire_references_at(now);
                    let selected_index = selected_reference.as_ref().and_then(|selected| {
                        search
                            .references()
                            .iter()
                            .position(|reference| reference == selected)
                            .or_else(|| {
                                search.references().iter().position(|reference| {
                                    reference.is_same_host_and_address(selected)
                                })
                            })
                    });
                    if let Some(request_id) = query.direct_request_id {
                        let _ = events.send(StartupGameSearchEvent::DirectQueryResolved {
                            request_id,
                            references: search.references().to_vec(),
                            selected_index,
                        });
                    } else if let Some(address) = discovery_address {
                        let _ = events.send(StartupGameSearchEvent::GameDiscoveryQueryResolved {
                            address,
                            references: search.references().to_vec(),
                            selected_index,
                        });
                    } else {
                        let _ = events.send(StartupGameSearchEvent::ReferencesUpdated(
                            search.references().to_vec(),
                        ));
                    }
                    if query.direct_request_id.is_none()
                        && query.source == ReferenceQuerySource::Masterserver
                    {
                        let _ =
                            events.send(StartupGameSearchEvent::MasterserverReply(masterserver));
                    }
                }
                Err(error) => {
                    let retry_masterserver = query.direct_request_id.is_none()
                        && query.source == ReferenceQuerySource::Masterserver
                        && masterserver_failure_allows_fast_retry(&mut masterserver_failures);
                    if let Some(address) = discovery_address {
                        discovery_queries.finish_at(
                            Instant::now(),
                            address,
                            GameDiscoveryQueryOutcome::Failed,
                        );
                    }
                    let message = error.to_string();
                    if let Some(request_id) = query.direct_request_id {
                        let _ = events.send(StartupGameSearchEvent::DirectQueryFailed {
                            request_id,
                            message,
                        });
                    } else if let Some(address) = discovery_address {
                        let _ = events.send(StartupGameSearchEvent::GameDiscoveryQueryFailed {
                            address,
                            message,
                        });
                    } else if !retry_masterserver {
                        let _ = events.send(StartupGameSearchEvent::SearchError {
                            source: Some(query.source),
                            message,
                        });
                    }
                    if retry_masterserver {
                        execute_search_command(
                            search.masterserver_query(),
                            (generation, masterserver_generation),
                            None,
                            &mut masterserver_query,
                            discovery.as_ref(),
                            &query_tx,
                            &events,
                            &reference_config,
                        )
                        .await;
                    }
                }
            }
        }
        if search.expire_references_at(Instant::now()) {
            let _ = events.send(StartupGameSearchEvent::ReferencesUpdated(
                search.references().to_vec(),
            ));
        }
        if stopped {
            break;
        }
        let now = Instant::now();
        if deadlines.take_due_lan_probe_at(now) {
            execute_search_command(
                search.periodic_lan_command(),
                (generation, masterserver_generation),
                None,
                &mut masterserver_query,
                discovery.as_ref(),
                &query_tx,
                &events,
                &reference_config,
            )
            .await;
        }
        if search.config.internet_enabled
            && masterserver_query.is_none()
            && deadlines.take_due_masterserver_query_at(now)
        {
            execute_search_command(
                search.masterserver_query(),
                (generation, masterserver_generation),
                None,
                &mut masterserver_query,
                discovery.as_ref(),
                &query_tx,
                &events,
                &reference_config,
            )
            .await;
        }
        if let Ok(socket) = discovery.as_ref() {
            if let Ok(Ok((size, source))) = tokio::time::timeout(
                Duration::from_millis(20),
                socket.socket.recv_from(&mut datagram),
            )
            .await
            {
                if let Some(command) = search.handle_lan_datagram(source, &datagram[..size]) {
                    if discovery_queries.begin_at(Instant::now(), &command) {
                        execute_search_command(
                            command,
                            (generation, masterserver_generation),
                            None,
                            &mut masterserver_query,
                            discovery.as_ref(),
                            &query_tx,
                            &events,
                            &reference_config,
                        )
                        .await;
                    }
                }
            }
        } else {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// The two periodic deadlines the search worker re-arms by hand.
///
/// `C4StartupNetDlg` drives both from `OnSec1Timer`: a per-second countdown
/// re-sends the discovery probe, while each masterserver row re-queries itself
/// once its own `iTimeout` passes (pinned oracle src/C4StartupNetDlg.cpp:
/// 1116-1131,186-210).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SearchDeadlines {
    lan_probe_at: Instant,
    masterserver_query_at: Instant,
}

impl SearchDeadlines {
    fn armed_at(now: Instant) -> Self {
        Self {
            lan_probe_at: now + LAN_DISCOVERY_INTERVAL,
            masterserver_query_at: now + GAME_SEARCH_INTERVAL,
        }
    }

    fn take_due_lan_probe_at(&mut self, now: Instant) -> bool {
        if now < self.lan_probe_at {
            return false;
        }
        self.lan_probe_at = now + LAN_DISCOVERY_INTERVAL;
        true
    }

    fn take_due_masterserver_query_at(&mut self, now: Instant) -> bool {
        if now < self.masterserver_query_at {
            return false;
        }
        self.defer_masterserver_query_at(now);
        true
    }

    fn defer_masterserver_query_at(&mut self, now: Instant) {
        self.masterserver_query_at = now + GAME_SEARCH_INTERVAL;
    }
}

/// How a LAN reference query ended, which is what decides when its address may
/// be queried again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GameDiscoveryQueryOutcome {
    References,
    NoReferences,
    Failed,
}

/// Per-host gate for LAN reference queries.
///
/// `C4StartupNetDlg` keeps one list entry per discovered address and
/// `AddReferenceQuery` refuses a second query while that entry is still waiting
/// for its answer (pinned oracle src/C4StartupNetDlg.cpp:1133-1154,590-600), so
/// how often a host is re-queried follows the lifetime of the row its last
/// answer produced rather than anything the probe decides.
#[derive(Debug, Default)]
struct GameDiscoveryQueryGate {
    active: HashSet<SocketAddr>,
    next_allowed: HashMap<SocketAddr, Instant>,
}

impl GameDiscoveryQueryGate {
    fn clear(&mut self) {
        self.active.clear();
        self.next_allowed.clear();
    }

    fn begin_at(&mut self, now: Instant, command: &SearchCommand) -> bool {
        let SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Address(address),
            source: ReferenceQuerySource::GameDiscovery,
            ..
        } = command
        else {
            return true;
        };
        if self
            .next_allowed
            .get(address)
            .is_some_and(|allowed_at| now < *allowed_at)
        {
            return false;
        }
        self.next_allowed.remove(address);
        self.active.insert(*address)
    }

    fn finish_at(&mut self, now: Instant, address: SocketAddr, outcome: GameDiscoveryQueryOutcome) {
        self.active.remove(&address);
        let backoff = match outcome {
            GameDiscoveryQueryOutcome::References => GAME_SEARCH_INTERVAL,
            GameDiscoveryQueryOutcome::NoReferences | GameDiscoveryQueryOutcome::Failed => {
                EMPTY_REFERENCE_LIFETIME
            }
        };
        let allowed_at = now.checked_add(backoff).unwrap_or(now);
        self.next_allowed.insert(address, allowed_at);
    }
}

// Search execution receives independent generation, transport, result, and
// presentation channels whose ownership differs across command variants.
#[allow(clippy::too_many_arguments)]
async fn execute_search_command(
    command: SearchCommand,
    query_generation: (u64, u64),
    direct_request_id: Option<u64>,
    masterserver_query: &mut Option<tokio::task::JoinHandle<()>>,
    discovery: Result<&DiscoverySocket, &io::Error>,
    query_tx: &tokio::sync::mpsc::UnboundedSender<QueryResult>,
    events: &mpsc::Sender<StartupGameSearchEvent>,
    reference_config: &ReferenceQueryConfig,
) {
    let (generation, masterserver_generation) = query_generation;
    match command {
        SearchCommand::SendLanProbe {
            target,
            payload,
            trigger,
        } => {
            let failure = match discovery {
                Ok(socket) => socket
                    .send_probe(&payload, target)
                    .await
                    .err()
                    .map(|error| (LanProbeFailure::Send, error)),
                Err(error) => Some((
                    LanProbeFailure::Unavailable,
                    io::Error::new(error.kind(), error.to_string()),
                )),
            };
            if let Some(event) =
                failure.and_then(|(failure, error)| lan_probe_error_event(trigger, failure, error))
            {
                let _ = events.send(event);
            }
        }
        SearchCommand::QueryReferences {
            endpoint,
            source,
            timeout,
        } => {
            if source == ReferenceQuerySource::GameDiscovery {
                if let ReferenceEndpoint::Address(address) = &endpoint {
                    let _ = events.send(StartupGameSearchEvent::GameDiscoveryQueryStarted {
                        address: *address,
                    });
                }
            }
            let query_tx = query_tx.clone();
            let reference_config = reference_config.clone();
            let result_endpoint = endpoint.clone();
            let query = tokio::spawn(async move {
                let result = fetch_reference_query_endpoint_with_config(
                    endpoint,
                    timeout,
                    &reference_config,
                )
                .await;
                let _ = query_tx.send(QueryResult {
                    generation,
                    masterserver_generation,
                    source,
                    endpoint: result_endpoint,
                    direct_request_id,
                    result,
                });
            });
            if source == ReferenceQuerySource::Masterserver {
                if let Some(previous) = masterserver_query.replace(query) {
                    previous.abort();
                }
            }
        }
    }
}

fn discovery_socket(port: u16) -> io::Result<DiscoverySocket> {
    let requested = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0));
    let (socket, address) =
        crate::dual_stack::create_bound_socket(requested, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    // A host without an IPv6 stack cannot reach the C++ discovery group at all.
    // Keeping the degraded socket leaves masterserver and direct queries
    // working instead of failing the whole search.
    let dual_stack = crate::dual_stack::bound_socket_family(address)
        == crate::dual_stack::SocketFamily::DualStack;
    if dual_stack {
        socket.set_multicast_hops_v6(16)?;
        socket.set_multicast_loop_v6(true)?;
    }
    socket.bind(&address.into())?;
    let multicast_interfaces = if dual_stack {
        join_discovery_multicast(&socket)
    } else {
        Vec::new()
    };
    socket.set_nonblocking(true)?;
    Ok(DiscoverySocket {
        socket: UdpSocket::from_std(socket.into())?,
        multicast_interfaces,
    })
}

/// Expands the discovery group into one destination per joined interface.
///
/// The C++ default-interface join yields the single unscoped destination
/// C4NetIOSimpleUDP sends to (pinned oracle src/C4NetIO.cpp:1624, :1793-1796),
/// and so does a host that joined nothing at all.
pub(crate) fn multicast_targets(target: SocketAddrV6, interfaces: &[u32]) -> Vec<SocketAddrV6> {
    if interfaces
        .iter()
        .all(|interface| *interface == DEFAULT_MULTICAST_INTERFACE)
    {
        return vec![target];
    }
    interfaces
        .iter()
        .map(|interface| {
            SocketAddrV6::new(*target.ip(), target.port(), target.flowinfo(), *interface)
        })
        .collect()
}

/// The interface `IPV6_MULTICAST_IF` must name before sending to `target`, or
/// `None` where C++ leaves the option untouched.
fn multicast_send_interface(target: &SocketAddrV6) -> Option<u32> {
    (target.scope_id() != DEFAULT_MULTICAST_INTERFACE).then(|| target.scope_id())
}

/// Joins the C++ discovery group, preferring the platform default interface and
/// falling back to every interface that accepts the join when it refuses.
///
/// Never fails: `C4NetIOSimpleUDP::InitBroadcast` returns false on a refused
/// join without closing anything (pinned oracle src/C4NetIO.cpp:1626-1632), and
/// both callers keep going — the client discards the result outright
/// (src/C4StartupNetDlg.cpp:737) and the host merely logs and drops its
/// discovery object, building the reference server afterwards
/// (src/C4Network2IO.cpp:86-89, :151-161).
pub(crate) fn join_discovery_multicast(socket: &Socket) -> Vec<u32> {
    joined_discovery_interfaces(&multicast_interface_indices, &|interface| {
        socket.join_multicast_v6(&DISCOVERY_MULTICAST, interface)
    })
}

/// `candidates` stays unevaluated until the default interface has refused the
/// join, so a host that behaves like C++ never enumerates anything.
fn joined_discovery_interfaces(
    candidates: &dyn Fn() -> Vec<u32>,
    join: &dyn Fn(u32) -> io::Result<()>,
) -> Vec<u32> {
    if join(DEFAULT_MULTICAST_INTERFACE).is_ok() {
        return vec![DEFAULT_MULTICAST_INTERFACE];
    }
    candidates()
        .into_iter()
        .filter(|interface| *interface != DEFAULT_MULTICAST_INTERFACE && join(*interface).is_ok())
        .collect()
}

/// Interface indices to try once the platform default has refused the join,
/// ascending so the joined set does not depend on kernel enumeration order.
#[cfg(unix)]
pub(crate) fn multicast_interface_indices() -> Vec<u32> {
    // SAFETY: `if_nameindex` returns a caller-owned array terminated by an
    // entry whose index is zero, released exactly once by `if_freenameindex`
    // below. Every read stays inside that terminator.
    let list = unsafe { libc::if_nameindex() };
    if list.is_null() {
        return Vec::new();
    }
    let mut indices = BTreeSet::new();
    let mut entry = list;
    while let index @ 1.. = unsafe { (*entry).if_index } {
        indices.insert(index);
        entry = unsafe { entry.add(1) };
    }
    unsafe { libc::if_freenameindex(list) };
    indices.into_iter().collect()
}

#[cfg(not(unix))]
pub(crate) fn multicast_interface_indices() -> Vec<u32> {
    // Enumerating interfaces needs `GetAdaptersAddresses` here, which no
    // required gate compiles for this crate. Leaving the fallback empty keeps
    // the C++ default-interface join as the only attempt, unchanged.
    Vec::new()
}

pub fn parse_reference_response(
    bytes: &[u8],
) -> Result<Vec<NetworkGameReference>, ReferenceParseError> {
    Ok(parse_reference_query_response(bytes)?.references)
}

pub fn parse_reference_query_response(
    bytes: &[u8],
) -> Result<ReferenceQueryResponse, ReferenceParseError> {
    parse_reference_query_response_with_config(bytes, &ReferenceQueryConfig::default())
}

pub fn parse_reference_query_response_with_config(
    bytes: &[u8],
    config: &ReferenceQueryConfig,
) -> Result<ReferenceQueryResponse, ReferenceParseError> {
    let mut chunks = Vec::new();
    let mut current = None::<Vec<&[u8]>>;
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some((indent, section)) = reference_ini_section(line) {
            if indent != 0 {
                if let Some(chunk) = current.as_mut() {
                    chunk.push(line);
                }
                continue;
            }
            if let Some(chunk) = current.take() {
                chunks.push(chunk);
            }
            if section == b"Reference" {
                current = Some(Vec::new());
            }
        } else if let Some(chunk) = current.as_mut() {
            chunk.push(line);
        }
    }
    if let Some(chunk) = current {
        chunks.push(chunk);
    }
    let references: Vec<NetworkGameReference> = chunks
        .into_iter()
        .map(|lines| parse_reference_chunk(lines, config))
        .collect::<Result<_, _>>()?;
    let mut masterserver = parse_masterserver_reply_info(bytes, config)?;
    masterserver.game_count = references.len();
    masterserver.player_count = references
        .iter()
        .map(|reference| reference.player_names.len())
        .sum();
    Ok(ReferenceQueryResponse {
        references,
        masterserver,
    })
}

fn parse_masterserver_reply_info(
    bytes: &[u8],
    config: &ReferenceQueryConfig,
) -> Result<MasterserverReplyInfo, ReferenceParseError> {
    let mut info = MasterserverReplyInfo::default();
    let mut found_engine_section = false;
    let mut in_engine_section = false;
    let mut nested_section_indents = Vec::new();
    let mut saw_version = false;
    let mut saw_motd = false;
    let mut saw_motd_url = false;
    let mut saw_redirect = false;

    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some((indent, section)) = reference_ini_section(line) {
            if indent == 0 {
                if in_engine_section {
                    break;
                }
                if !found_engine_section && section == b"LegacyClonk" {
                    found_engine_section = true;
                    in_engine_section = true;
                    nested_section_indents.clear();
                }
            } else if in_engine_section {
                while nested_section_indents
                    .last()
                    .is_some_and(|parent| *parent >= indent)
                {
                    nested_section_indents.pop();
                }
                nested_section_indents.push(indent);
            }
            continue;
        }
        if !in_engine_section {
            continue;
        }

        let content = trim_reference_horizontal_start(line);
        if content.is_empty() {
            continue;
        }
        let indent = line.len() - content.len();
        let value_indent = indent.saturating_add(1);
        while nested_section_indents
            .last()
            .is_some_and(|section| *section >= value_indent)
        {
            nested_section_indents.pop();
        }
        if !nested_section_indents.is_empty() {
            continue;
        }
        let Some(equal) = content.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = &content[..equal];
        let value = decode_masterserver_raw_value(&content[equal + 1..], config)?;
        match key {
            b"Version" if !saw_version => {
                saw_version = true;
                let version = parse_masterserver_version(&value);
                info.version = (version.version[0] != 0).then_some(version);
            }
            b"MOTD" if !saw_motd => {
                saw_motd = true;
                info.motd = value;
            }
            b"MOTDURL" if !saw_motd_url => {
                saw_motd_url = true;
                info.motd_url = value;
            }
            b"LeagueServerRedirect" if !saw_redirect => {
                saw_redirect = true;
                info.league_server_redirect = value;
            }
            _ => {}
        }
    }

    Ok(info)
}

fn parse_masterserver_version(value: &str) -> MasterserverVersion {
    let mut parts = [0_i32; 5];
    for (index, part) in value.split(',').take(parts.len()).enumerate() {
        let part = part.trim();
        if !part.is_empty() {
            parts[index] = part.parse().unwrap_or_default();
        }
    }
    MasterserverVersion {
        version: [parts[0], parts[1], parts[2], parts[3]],
        build: parts[4],
    }
}

fn decode_masterserver_raw_value(
    value: &[u8],
    config: &ReferenceQueryConfig,
) -> Result<String, ReferenceParseError> {
    let value = trim_reference_horizontal_start(value);
    let end = value
        .iter()
        .position(|byte| matches!(*byte, b'\r' | 0))
        .unwrap_or(value.len());
    config.decode(&value[..end])
}

fn reference_ini_section(line: &[u8]) -> Option<(usize, &[u8])> {
    let content = trim_reference_horizontal_start(line);
    let indent = line.len() - content.len();
    let content = content.strip_prefix(b"[")?;
    let close = content.iter().position(|byte| *byte == b']')?;
    let name = &content[..close];
    if !name.first().is_some_and(|byte| byte.is_ascii_alphabetic())
        || !name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
    {
        return None;
    }
    Some((indent, name))
}

fn trim_reference_horizontal_start(mut bytes: &[u8]) -> &[u8] {
    while bytes
        .first()
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        bytes = &bytes[1..];
    }
    bytes
}

pub async fn fetch_reference_endpoint(
    endpoint: ReferenceEndpoint,
    timeout: Duration,
) -> Result<Vec<NetworkGameReference>, ReferenceFetchError> {
    Ok(fetch_reference_query_endpoint(endpoint, timeout)
        .await?
        .references)
}

pub async fn fetch_reference_query_endpoint(
    endpoint: ReferenceEndpoint,
    timeout: Duration,
) -> Result<ReferenceQueryResponse, ReferenceFetchError> {
    fetch_reference_query_endpoint_with_config(endpoint, timeout, &ReferenceQueryConfig::default())
        .await
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReferenceRequestPlan {
    url: String,
    connect_address: Option<SocketAddr>,
    host_header: Option<String>,
}

impl ReferenceRequestPlan {
    fn for_endpoint(endpoint: ReferenceEndpoint) -> Self {
        match endpoint {
            ReferenceEndpoint::Address(SocketAddr::V6(address)) if address.scope_id() != 0 => {
                Self {
                    url: format!("http://{SCOPED_IPV6_REQUEST_HOST}:{}/", address.port()),
                    connect_address: Some(SocketAddr::V6(address)),
                    host_header: Some(format!("[{}]:{}", address.ip(), address.port())),
                }
            }
            ReferenceEndpoint::Address(address) => Self {
                url: reference_url(address),
                connect_address: None,
                host_header: None,
            },
            ReferenceEndpoint::Url(url) => Self {
                url,
                connect_address: None,
                host_header: None,
            },
        }
    }

    fn client_builder(
        &self,
        backend: crate::HttpBackend,
    ) -> Result<reqwest::ClientBuilder, reqwest::Error> {
        let builder = crate::http_backend::bundled_root_client_builder()?;
        let builder = match self.connect_address {
            Some(address) => builder
                .no_proxy()
                .resolve(SCOPED_IPV6_REQUEST_HOST, address),
            None => builder,
        };
        Ok(backend.apply(builder))
    }

    fn get(&self, client: &reqwest::Client) -> reqwest::RequestBuilder {
        let request = client.get(&self.url);
        match self.host_header.as_ref() {
            Some(host) => request.header(reqwest::header::HOST, host),
            None => request,
        }
    }
}

pub async fn fetch_reference_endpoint_with_config(
    endpoint: ReferenceEndpoint,
    timeout: Duration,
    config: &ReferenceQueryConfig,
) -> Result<Vec<NetworkGameReference>, ReferenceFetchError> {
    Ok(
        fetch_reference_query_endpoint_with_config(endpoint, timeout, config)
            .await?
            .references,
    )
}

pub async fn fetch_reference_query_endpoint_with_config(
    endpoint: ReferenceEndpoint,
    timeout: Duration,
    config: &ReferenceQueryConfig,
) -> Result<ReferenceQueryResponse, ReferenceFetchError> {
    let plan = ReferenceRequestPlan::for_endpoint(endpoint);
    let client = plan
        .client_builder(config.http_backend)?
        .user_agent(crate::league::LEAGUE_HTTP_USER_AGENT)
        .gzip(true)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let response = plan
        .get(&client)
        .header("Accept-Charset", config.charset_code_name())
        .header("Accept-Language", &config.language_sequence)
        .send()
        .await?
        .error_for_status()?;
    let source = response.remote_addr();
    let bytes = response.bytes().await?;
    let mut response = parse_reference_query_response_with_config(&bytes, config)?;
    if let Some(source) = source {
        fill_reference_source_addresses(&mut response.references, source);
    }
    Ok(response)
}

fn fill_reference_source_addresses(references: &mut [NetworkGameReference], source: SocketAddr) {
    for reference in references {
        reference.source_address = source;
        for address in &mut reference.addresses {
            if address.endpoint.ip().is_unspecified() {
                let port = address.endpoint.port();
                address.endpoint = match source {
                    SocketAddr::V4(source) => SocketAddr::new((*source.ip()).into(), port),
                    SocketAddr::V6(source) => SocketAddr::V6(SocketAddrV6::new(
                        *source.ip(),
                        port,
                        source.flowinfo(),
                        source.scope_id(),
                    )),
                };
            }
        }
        reference.tcp_addresses = reference
            .addresses
            .iter()
            .filter(|address| address.protocol == NetworkProtocol::Tcp)
            .map(|address| address.endpoint)
            .collect();
    }
}

fn reference_url(address: SocketAddr) -> String {
    match address {
        SocketAddr::V4(address) => format!("http://{address}/"),
        SocketAddr::V6(address) if address.scope_id() == 0 => format!("http://{address}/"),
        SocketAddr::V6(address) => format!(
            "http://[{}%25{}]:{}/",
            address.ip(),
            address.scope_id(),
            address.port()
        ),
    }
}

fn parse_reference_chunk(
    lines: Vec<&[u8]>,
    config: &ReferenceQueryConfig,
) -> Result<NetworkGameReference, ReferenceParseError> {
    let mut reference = NetworkGameReference::default();
    let mut direct_client = false;
    let mut netpuncher_id = false;
    let mut in_player_infos = false;
    let mut in_player = false;
    let mut player = ParsedReferencePlayer::default();
    let mut direct_client_id = None;
    let mut direct_client_name = None;
    let mut direct_client_nick = None;

    for line in lines {
        let trimmed_start = trim_reference_ascii_start(line);
        let indent = line.len() - trimmed_start.len();
        let trimmed = trim_reference_ascii_end(trimmed_start);
        if trimmed.starts_with(b"[") && trimmed.ends_with(b"]") {
            if in_player {
                player.finish(&mut reference.player_names);
                player = ParsedReferencePlayer::default();
                in_player = false;
            }
            if direct_client {
                flush_direct_client(
                    &mut reference,
                    &mut direct_client_id,
                    &mut direct_client_name,
                    &mut direct_client_nick,
                );
            }
            direct_client = indent == 2 && trimmed == b"[Client]";
            netpuncher_id = indent == 2 && trimmed == b"[NetpuncherID]";
            if indent == 2 {
                in_player_infos = trimmed == b"[PlayerInfos]";
            } else if in_player_infos && indent == 6 && trimmed == b"[Player]" {
                in_player = true;
            }
            continue;
        }
        let Some(equal) = trimmed.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let Ok(key) = std::str::from_utf8(&trimmed[..equal]) else {
            continue;
        };
        let raw_value = trim_reference_ascii(&trimmed[equal + 1..]);
        let value = decode_reference_value(raw_value, config)?;
        if in_player && indent == 6 {
            match key {
                "Name" => player.name = value,
                "ForcedName" => player.forced_name = value,
                "LeagueAccount" => player.league_account = value,
                "Flags" => player.set_flags(&value),
                "Type" => player.is_script = value.eq_ignore_ascii_case("script"),
                _ => {}
            }
            continue;
        }
        if direct_client && indent == 2 {
            match key {
                "ID" => direct_client_id = Some(parse_i32(key, &value)?),
                "Name" => direct_client_name = Some(value),
                "Nick" => direct_client_nick = Some(value),
                _ => {}
            }
            continue;
        }
        if netpuncher_id && indent == 2 {
            match key {
                "IPv4" => reference.netpuncher_ipv4 = parse_u32(key, &value)?,
                "IPv6" => reference.netpuncher_ipv6 = parse_u32(key, &value)?,
                _ => {}
            }
            continue;
        }
        if indent != 0 {
            continue;
        }
        match key {
            "Icon" => reference.icon = parse_i32(key, &value)?,
            "State" => reference.state = value,
            "CtrlMode" => reference.control_mode = parse_i32(key, &value)?,
            "Time" => reference.time = parse_i32(key, &value)?,
            "StartTime" => reference.start_time = parse_i64(key, &value)?,
            "Comment" => reference.comment = value,
            "JoinAllowed" => reference.join_allowed = parse_bool(&value),
            "PasswordNeeded" => reference.password_needed = parse_bool(&value),
            "OfficialServer" => reference.official_server = parse_bool(&value),
            "UseFairCrew" => reference.use_fair_crew = parse_bool(&value),
            "Goals" => reference.goals = parse_reference_goal_ids(&value),
            "League" => reference.league = value,
            "LeagueAddress" => reference.league_address = value,
            "MaxPlayers" => reference.max_players = parse_i32(key, &value)?,
            "Address" => {
                let addresses = parse_reference_addresses(&value);
                reference.tcp_addresses = addresses
                    .iter()
                    .filter(|address| address.protocol == NetworkProtocol::Tcp)
                    .map(|address| address.endpoint)
                    .collect();
                reference.addresses = addresses;
            }
            "Game" => reference.game = value,
            "Version" => {
                for (index, part) in value.split(',').take(4).enumerate() {
                    reference.version[index] = parse_i32(key, part.trim())?;
                }
            }
            "Build" => reference.build = parse_i32(key, &value)?,
            "Title" => reference.title = value,
            "NetpuncherAddr" => reference.netpuncher_address = value,
            _ => {}
        }
    }
    if direct_client {
        flush_direct_client(
            &mut reference,
            &mut direct_client_id,
            &mut direct_client_name,
            &mut direct_client_nick,
        );
    }
    if in_player {
        player.finish(&mut reference.player_names);
    }
    Ok(reference)
}

#[derive(Default)]
struct ParsedReferencePlayer {
    name: String,
    forced_name: String,
    league_account: String,
    removed: bool,
    invisible: bool,
    is_script: bool,
}

impl ParsedReferencePlayer {
    fn set_flags(&mut self, flags: &str) {
        for flag in flags.split(['|', ',', ' ']).filter(|flag| !flag.is_empty()) {
            self.removed |= flag.eq_ignore_ascii_case("removed");
            self.invisible |= flag.eq_ignore_ascii_case("invisible");
        }
    }

    fn finish(self, names: &mut Vec<String>) {
        // C4PlayerInfo::CompileFunc clears Invisible for ordinary users.
        if self.removed || (self.is_script && self.invisible) {
            return;
        }
        let name = if !self.league_account.is_empty() {
            self.league_account
        } else if !self.forced_name.is_empty() {
            self.forced_name
        } else {
            self.name
        };
        names.push(name);
    }
}

fn parse_reference_goal_ids(value: &str) -> Vec<String> {
    value
        .split(';')
        .filter_map(|entry| {
            let id = entry
                .trim()
                .split_once('=')
                .map_or(entry.trim(), |(id, _)| id.trim());
            (!id.is_empty()).then(|| id.to_string())
        })
        .collect()
}

fn trim_reference_ascii_start(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[1..];
    }
    bytes
}

fn trim_reference_ascii_end(mut bytes: &[u8]) -> &[u8] {
    while bytes.last().is_some_and(|byte| byte.is_ascii_whitespace()) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn trim_reference_ascii(bytes: &[u8]) -> &[u8] {
    trim_reference_ascii_end(trim_reference_ascii_start(bytes))
}

fn decode_reference_value(
    value: &[u8],
    config: &ReferenceQueryConfig,
) -> Result<String, ReferenceParseError> {
    if value.starts_with(b"\"") {
        let (mut decoded, _, _) = crate::league::parse_escaped_value(value);
        if let Some(nul) = decoded.iter().position(|byte| *byte == 0) {
            decoded.truncate(nul);
        }
        config.decode(&decoded)
    } else {
        config.decode(value)
    }
}

fn flush_direct_client(
    reference: &mut NetworkGameReference,
    client_id: &mut Option<i32>,
    client_name: &mut Option<String>,
    client_nick: &mut Option<String>,
) {
    if *client_id == Some(0) {
        if let Some(name) = client_name.take() {
            reference.host_name = name;
        }
        if let Some(nick) = client_nick.take() {
            reference.host_nick = nick;
        }
    }
    *client_id = None;
    *client_name = None;
    *client_nick = None;
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn parse_i32(key: &str, value: &str) -> Result<i32, ReferenceParseError> {
    value.parse().map_err(|_| invalid_integer(key, value))
}

fn parse_i64(key: &str, value: &str) -> Result<i64, ReferenceParseError> {
    value.parse().map_err(|_| invalid_integer(key, value))
}

fn parse_u32(key: &str, value: &str) -> Result<u32, ReferenceParseError> {
    value.parse().map_err(|_| invalid_integer(key, value))
}

fn invalid_integer(key: &str, value: &str) -> ReferenceParseError {
    ReferenceParseError::InvalidInteger {
        key: key.to_string(),
        value: value.to_string(),
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "true" | "1")
}

fn parse_reference_addresses(value: &str) -> Vec<NetworkAddress> {
    value
        .split(',')
        .filter_map(|entry| {
            let (protocol, address) = entry.trim().split_once(':')?;
            let protocol = match protocol {
                "UDP" => Some(NetworkProtocol::Udp),
                "TCP" => Some(NetworkProtocol::Tcp),
                protocol => protocol.parse().ok().map(NetworkProtocol::from_wire),
            }?;
            Some((protocol, unquote(address)))
        })
        .map(|(protocol, address)| {
            NetworkAddress::new(protocol, decode_cpp_endpoint(address.as_bytes()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_reference_server(responses: Vec<Vec<u8>>) -> (SocketAddr, thread::JoinHandle<()>) {
        spawn_reference_status_server(responses.into_iter().map(|body| (200, body)).collect())
    }

    fn spawn_reference_status_server(
        responses: Vec<(u16, Vec<u8>)>,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request).unwrap();
                let reason = if status == 200 {
                    "OK"
                } else {
                    "Service Unavailable"
                };
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
                stream.flush().unwrap();
            }
        });
        (address, worker)
    }

    #[test]
    fn masterserver_failures_retry_twice_before_periodic_interval() {
        // C4StartupNetListEntry::OnRequestFailed immediately replaces the
        // failed masterserver query while iNumFails <= 2; only the third
        // entry-lifetime failure falls through to the periodic timer
        // (C4StartupNetDlg.cpp:186-238).
        let body = b"[LegacyClonk]\n\
MOTD=Recovered\n\
[Reference]\n\
Title=Recovered game\n"
            .to_vec();
        let (address, server) =
            spawn_reference_status_server(vec![(503, Vec::new()), (503, Vec::new()), (200, body)]);
        let search = StartupGameSearch::start(NetworkGameSearchConfig {
            master_server_url: format!("http://{address}/"),
            discovery_port: 0,
            ..NetworkGameSearchConfig::default()
        })
        .unwrap();
        let started = Instant::now();
        search.initial_refresh().unwrap();

        let deadline = started + Duration::from_secs(5);
        let mut terminal_failure_count = 0;
        let mut recovered_references = false;
        let reply = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = search
                .events()
                .recv_timeout(remaining)
                .expect("fast masterserver retries recover before the periodic interval");
            match event {
                StartupGameSearchEvent::SearchError {
                    source: Some(ReferenceQuerySource::Masterserver),
                    ..
                } => terminal_failure_count += 1,
                StartupGameSearchEvent::ReferencesUpdated(references) => {
                    recovered_references = references
                        .iter()
                        .any(|reference| reference.title == "Recovered game");
                }
                StartupGameSearchEvent::MasterserverReply(reply) => break reply,
                _ => {}
            }
        };

        assert_eq!(terminal_failure_count, 0);
        assert!(recovered_references);
        assert_eq!(reply.motd, "Recovered");
        assert!(started.elapsed() < GAME_SEARCH_INTERVAL);
        drop(search);
        server.join().unwrap();
    }

    #[test]
    fn masterserver_fast_retry_budget_is_bounded_to_two_entry_lifetime_failures() {
        let mut failures = 0;

        assert!(masterserver_failure_allows_fast_retry(&mut failures));
        assert!(masterserver_failure_allows_fast_retry(&mut failures));
        assert!(!masterserver_failure_allows_fast_retry(&mut failures));
        assert!(!masterserver_failure_allows_fast_retry(&mut failures));
        assert_eq!(failures, 4);
    }

    #[test]
    fn cpp_join_address_preparation_applies_source_scope_and_stable_rank() {
        // InitClient copies the complete reference set, applies the source
        // scope through SetScopeId, and stable-sorts by C4NetIO address rank
        // before attempting connections (pristine 9ffa0a5d
        // src/C4Network2.cpp:296-303;
        // src/C4Network2Address.cpp:123-128;
        // src/C4NetIO.cpp:232-275, 382-386).
        let global_v6 =
            NetworkAddress::new(NetworkProtocol::Tcp, "[2001:db8::1]:11112".parse().unwrap());
        let private_v4 =
            NetworkAddress::new(NetworkProtocol::Udp, "10.0.0.1:11113".parse().unwrap());
        let global_v4 =
            NetworkAddress::new(NetworkProtocol::Tcp, "203.0.113.1:11112".parse().unwrap());
        let link_local_v6 = NetworkAddress::new(
            NetworkProtocol::Udp,
            SocketAddr::V6(SocketAddrV6::new(
                "fe80::beef".parse().unwrap(),
                11_113,
                4,
                0,
            )),
        );
        let private_v6 =
            NetworkAddress::new(NetworkProtocol::Tcp, "[fd00::1]:11112".parse().unwrap());
        let reference = NetworkGameReference {
            addresses: vec![global_v6, private_v4, global_v4, link_local_v6, private_v6],
            source_address: SocketAddr::V6(SocketAddrV6::new(
                "fe80::1234".parse().unwrap(),
                11_111,
                0,
                9,
            )),
            ..NetworkGameReference::default()
        };
        let scoped_link_local_v6 = NetworkAddress::new(
            NetworkProtocol::Udp,
            SocketAddr::V6(SocketAddrV6::new(
                "fe80::beef".parse().unwrap(),
                11_113,
                4,
                9,
            )),
        );

        let without_global_ipv6 = reference.join_addresses(false);
        assert_eq!(
            without_global_ipv6,
            [
                global_v4,
                private_v4,
                private_v6,
                scoped_link_local_v6,
                global_v6,
            ]
        );
        assert_eq!(
            without_global_ipv6[3].endpoint,
            SocketAddr::V6(SocketAddrV6::new(
                "fe80::beef".parse().unwrap(),
                11_113,
                4,
                9,
            ))
        );
        assert_eq!(
            reference.join_addresses(true),
            [
                global_v6,
                global_v4,
                private_v4,
                private_v6,
                scoped_link_local_v6,
            ]
        );
    }

    #[test]
    fn cpp_join_attempts_skip_null_and_expand_local_addresses_by_interface() {
        // InitClient skips null endpoints, attempts each non-local address
        // once, and expands every local address across the local interface ID
        // list in order (pristine 9ffa0a5d src/C4Network2.cpp:375-405;
        // src/C4Network2Address.cpp:92-101, 123-128).
        let global_v4 =
            NetworkAddress::new(NetworkProtocol::Tcp, "203.0.113.1:11112".parse().unwrap());
        let link_local_v6 =
            NetworkAddress::new(NetworkProtocol::Udp, "[fe80::beef]:11113".parse().unwrap());
        let link_local_v4 =
            NetworkAddress::new(NetworkProtocol::Tcp, "169.254.1.2:11112".parse().unwrap());
        let reference = NetworkGameReference {
            addresses: vec![
                NetworkAddress::new(NetworkProtocol::Tcp, "[::]:0".parse().unwrap()),
                link_local_v6,
                global_v4,
                link_local_v4,
            ],
            source_address: SocketAddr::V6(SocketAddrV6::new(
                "fe80::1234".parse().unwrap(),
                11_111,
                0,
                9,
            )),
            ..NetworkGameReference::default()
        };

        let expected_attempts = [
            global_v4,
            NetworkAddress::new(
                NetworkProtocol::Udp,
                SocketAddr::V6(SocketAddrV6::new(
                    "fe80::beef".parse().unwrap(),
                    11_113,
                    0,
                    3,
                )),
            ),
            NetworkAddress::new(
                NetworkProtocol::Udp,
                SocketAddr::V6(SocketAddrV6::new(
                    "fe80::beef".parse().unwrap(),
                    11_113,
                    0,
                    7,
                )),
            ),
            link_local_v4,
            link_local_v4,
        ];
        let route_plan = reference.join_route_plan(false, &[3, 7]);
        assert_eq!(
            route_plan.logical_addresses,
            [
                global_v4,
                NetworkAddress::new(
                    NetworkProtocol::Udp,
                    SocketAddr::V6(SocketAddrV6::new(
                        "fe80::beef".parse().unwrap(),
                        11_113,
                        0,
                        9,
                    )),
                ),
                link_local_v4,
            ],
            "the progress routes retain one source-scoped logical address"
        );
        assert_eq!(route_plan.dial_attempts, expected_attempts);
        assert_eq!(reference.join_attempts(false, &[3, 7]), expected_attempts);
    }

    #[test]
    fn reference_source_fills_only_null_hosts_and_retains_the_complete_endpoint() {
        // SetSourceAddress retains the response endpoint and copies its host,
        // flowinfo, and scope into only advertised null hosts while preserving
        // their ports (pristine 9ffa0a5d src/C4Network2Reference.cpp:37-47;
        // src/C4Network2Address.cpp:187-205).
        let source = SocketAddr::V6(SocketAddrV6::new(
            "fe80::1234".parse().unwrap(),
            11_111,
            0x55,
            7,
        ));
        let null_tcp = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 11_112, 0, 0));
        let advertised_udp = SocketAddr::V6(SocketAddrV6::new(
            "fe80::beef".parse().unwrap(),
            11_113,
            0,
            0,
        ));
        let mut references = [NetworkGameReference {
            addresses: vec![
                NetworkAddress::new(NetworkProtocol::Tcp, null_tcp),
                NetworkAddress::new(NetworkProtocol::Udp, advertised_udp),
            ],
            tcp_addresses: vec![null_tcp],
            ..NetworkGameReference::default()
        }];

        fill_reference_source_addresses(&mut references, source);

        assert_eq!(references[0].source_address, source);
        assert_eq!(
            references[0].addresses[0].endpoint,
            SocketAddr::V6(SocketAddrV6::new(
                "fe80::1234".parse().unwrap(),
                11_112,
                0x55,
                7,
            ))
        );
        assert_eq!(references[0].addresses[1].endpoint, advertised_udp);
        assert_eq!(
            references[0].tcp_addresses,
            vec![references[0].addresses[0].endpoint]
        );
    }

    #[test]
    fn reference_addresses_resolve_hostnames_and_keep_malformed_entries() {
        // C4Network2EndpointAddress resolves the first getaddrinfo result and
        // leaves a failed endpoint cleared. C4Network2Reference keeps that
        // address and every following reference, then fills only its null host
        // from the query source (pristine 9ffa0a5d
        // src/C4Network2Address.cpp:263-325,489-505;
        // src/C4Network2Reference.cpp:37-47,994-1039).
        let mut references = parse_reference_response(
            br#"[Reference]
Title="R\344uber"
Address=UDP:"localhost:31113",TCP:"127.0.0.1:31112",UDP:"[::1]:31114",TCP:"not-an-addr:"

[Reference]
Title="Still visible"
Address=TCP:"192.0.2.8:41112"
"#,
        )
        .expect("hostname and malformed endpoint compatibility");

        assert_eq!(references.len(), 2);
        assert_eq!(references[0].title, "Räuber");
        assert_eq!(references[1].title, "Still visible");
        assert_eq!(
            references[0]
                .addresses
                .iter()
                .map(|address| address.protocol)
                .collect::<Vec<_>>(),
            [
                NetworkProtocol::Udp,
                NetworkProtocol::Tcp,
                NetworkProtocol::Udp,
                NetworkProtocol::Tcp,
            ]
        );
        assert!(references[0].addresses[0].endpoint.ip().is_loopback());
        assert_eq!(references[0].addresses[0].endpoint.port(), 31_113);
        assert_eq!(
            references[0].addresses[1].endpoint,
            "127.0.0.1:31112".parse().unwrap()
        );
        assert_eq!(
            references[0].addresses[2].endpoint,
            "[::1]:31114".parse().unwrap()
        );
        assert_eq!(
            references[0].addresses[3].endpoint,
            "[::]:0".parse().unwrap()
        );
        assert_eq!(
            references[1].addresses,
            [NetworkAddress::new(
                NetworkProtocol::Tcp,
                "192.0.2.8:41112".parse().unwrap(),
            )]
        );

        let source = "203.0.113.9:51111".parse().unwrap();
        fill_reference_source_addresses(&mut references, source);

        assert_eq!(
            references[0].addresses[3].endpoint,
            "203.0.113.9:0".parse().unwrap()
        );
        assert_eq!(
            references[0].tcp_addresses,
            [
                "127.0.0.1:31112".parse().unwrap(),
                "203.0.113.9:0".parse().unwrap(),
            ]
        );
        assert_eq!(
            references[1].addresses[0].endpoint,
            "192.0.2.8:41112".parse().unwrap()
        );
    }

    #[test]
    fn scoped_ipv6_reference_plan_keeps_the_zone_out_of_http() {
        // Discovery keeps the datagram sender and replaces only its port before
        // passing the address to the reference client. The HTTP client then
        // parses that endpoint as its server (pristine 9ffa0a5d
        // src/C4Network2Discover.cpp:76-87;
        // src/C4StartupNetDlg.cpp:903-908;
        // src/C4Network2Reference.cpp:532-537).
        let address = SocketAddr::V6(SocketAddrV6::new(
            "fe80::1234".parse().unwrap(),
            DEFAULT_REFERENCE_PORT,
            0,
            7,
        ));

        let plan = ReferenceRequestPlan::for_endpoint(ReferenceEndpoint::Address(address));
        assert_eq!(
            plan,
            ReferenceRequestPlan {
                url: format!("http://{SCOPED_IPV6_REQUEST_HOST}:{DEFAULT_REFERENCE_PORT}/"),
                connect_address: Some(address),
                host_header: Some(format!("[fe80::1234]:{DEFAULT_REFERENCE_PORT}")),
            }
        );

        let client = plan
            .client_builder(crate::HttpBackend::default())
            .expect("bundled roots parse")
            .build()
            .unwrap();
        let request = plan.get(&client).build().unwrap();
        assert_eq!(
            request.url().as_str(),
            format!("http://{SCOPED_IPV6_REQUEST_HOST}:{DEFAULT_REFERENCE_PORT}/")
        );
        assert_eq!(
            request.headers().get(reqwest::header::HOST).unwrap(),
            format!("[fe80::1234]:{DEFAULT_REFERENCE_PORT}").as_str()
        );
    }

    #[test]
    fn direct_reference_endpoints_apply_the_configured_default_port() {
        let default_port = 12_345;

        assert_eq!(
            direct_reference_endpoint(" 127.0.0.1:23456 ", default_port).unwrap(),
            ReferenceEndpoint::Address("127.0.0.1:23456".parse().unwrap())
        );
        assert_eq!(
            direct_reference_endpoint("2001:db8::1", default_port).unwrap(),
            ReferenceEndpoint::Address("[2001:db8::1]:12345".parse().unwrap())
        );
        assert_eq!(
            direct_reference_endpoint("games.example.test", default_port).unwrap(),
            ReferenceEndpoint::Url("http://games.example.test:12345/".to_string())
        );
        assert_eq!(
            direct_reference_endpoint("games.example.test:23456", default_port).unwrap(),
            ReferenceEndpoint::Url("http://games.example.test:23456/".to_string())
        );
        assert_eq!(
            direct_reference_endpoint("http://games.example.test/reference", default_port).unwrap(),
            ReferenceEndpoint::Url("http://games.example.test:12345/reference".to_string())
        );
        assert_eq!(
            direct_reference_endpoint("https://games.example.test:23456", default_port).unwrap(),
            ReferenceEndpoint::Url("https://games.example.test:23456/".to_string())
        );

        assert!(direct_reference_endpoint("  ", default_port).is_err());
        assert!(direct_reference_endpoint("ftp://games.example.test", default_port).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_reference_url_reports_the_parse_cause() {
        // C4StartupNetListEntry forwards C4Network2RefClient::SetServer's
        // detailed URI parse error through GetError (pristine 9ffa0a5d
        // src/C4StartupNetDlg.cpp:139-156;
        // src/C4Network2Reference.cpp:532-543).
        let error = fetch_reference_endpoint(
            ReferenceEndpoint::Url("https:".to_string()),
            Duration::from_secs(1),
        )
        .await
        .expect_err("the malformed URI must fail before a request is sent");

        assert!(error.to_string().contains("empty host"), "{error:?}");
    }

    #[test]
    fn reference_rows_retain_classic_display_fields_and_master_counts() {
        let response = parse_reference_query_response(
            b"[LegacyClonk]\n\
MOTD=Welcome\n\
MOTDURL=https://example.test/news\n\
LeagueServerRedirect=https://example.test/league\n\
[Reference]\n\
Icon=7\n\
State=Running\n\
Time=3723\n\
Comment=Host comment\n\
PasswordNeeded=true\n\
OfficialServer=true\n\
UseFairCrew=true\n\
Goals=GOAL=2;ZERO=0;MELE=1\n\
League=Cup\n\
LeagueAddress=https://example.test/cup\n\
MaxPlayers=8\n\
Game=LegacyClonk\n\
Version=4,9,11,0\n\
Build=362\n\
Title=Fixture\n\
\x20\x20[PlayerInfos]\n\
\x20\x20\x20\x20[Client]\n\
\x20\x20\x20\x20\x20\x20[Player]\n\
\x20\x20\x20\x20\x20\x20Name=Alice\n\
\x20\x20\x20\x20\x20\x20Flags=Joined\n\
\x20\x20\x20\x20\x20\x20[Player]\n\
\x20\x20\x20\x20\x20\x20Name=Removed\n\
\x20\x20\x20\x20\x20\x20Flags=Joined|Removed\n\
\x20\x20\x20\x20\x20\x20[Player]\n\
\x20\x20\x20\x20\x20\x20Name=Hidden bot\n\
\x20\x20\x20\x20\x20\x20Flags=Invisible\n\
\x20\x20\x20\x20\x20\x20Type=Script\n\
\x20\x20\x20\x20\x20\x20[Player]\n\
\x20\x20\x20\x20\x20\x20Name=Original\n\
\x20\x20\x20\x20\x20\x20ForcedName=Forced\n\
\x20\x20\x20\x20\x20\x20LeagueAccount=League Alice\n\
\x20\x20\x20\x20\x20\x20Flags=Invisible\n\
\x20\x20\x20\x20\x20\x20Type=User\n\
[Reference]\n\
Title=Empty\n",
        )
        .expect("parse display-complete reference response");

        assert_eq!(response.references.len(), 2);
        let reference = &response.references[0];
        assert_eq!((reference.icon, reference.time), (7, 3723));
        assert_eq!(reference.comment, "Host comment");
        assert_eq!(reference.goals, ["GOAL", "ZERO", "MELE"]);
        assert!(reference.use_fair_crew);
        assert_eq!(reference.league, "Cup");
        assert_eq!(reference.player_names, ["Alice", "League Alice"]);
        assert_eq!(response.masterserver.game_count, 2);
        assert_eq!(response.masterserver.player_count, 2);
        assert_eq!(response.masterserver.motd, "Welcome");
        assert_eq!(
            response.masterserver.league_server_redirect,
            "https://example.test/league"
        );
    }

    #[test]
    fn references_expire_at_cpp_deadline_and_updates_refresh_it() {
        // C++ gives every received reference a 42-second timeout, removes it
        // when `now >= timeout`, and resets that timeout when a matching newer
        // reference replaces it (pristine 9ffa0a5d
        // src/C4StartupNetDlg.h:27-30;
        // src/C4StartupNetDlg.cpp:186-209, 441-502, 520-550).
        let observed_at = std::time::Instant::now();
        let reference = |host: &str, start_time| NetworkGameReference {
            host_name: host.to_string(),
            start_time,
            tcp_addresses: vec!["203.0.113.1:11112".parse().unwrap()],
            ..NetworkGameReference::default()
        };
        let mut search = NetworkGameSearch::new(NetworkGameSearchConfig::default());
        search.merge_references_at(
            observed_at,
            [reference("Stale", 1), reference("Refreshed", 1)],
        );
        search.merge_references_at(
            observed_at + Duration::from_secs(30),
            [reference("Refreshed", 2)],
        );

        assert!(!search.expire_references_at(observed_at + Duration::from_secs(41)));
        assert!(search.expire_references_at(observed_at + Duration::from_secs(42)));
        assert_eq!(search.references()[0].host_name, "Refreshed");
        assert!(!search.expire_references_at(observed_at + Duration::from_secs(71)));
        assert!(search.expire_references_at(observed_at + Duration::from_secs(72)));
        assert!(search.references().is_empty());
    }

    #[test]
    fn query_charset_names_match_cpp_config_mapping() {
        // C4Config::GetCharsetCodeName maps these language resource charset
        // names to the HTTP code-page names and defaults every other value to
        // CP1252 (pristine 9ffa0a5d src/C4Config.cpp:875-893).
        for (configured, expected) in [
            ("SHIFTJIS", "CP932"),
            ("hangul", "CP949"),
            ("JOHAB", "CP1361"),
            ("CHINESEBIG5", "CP950"),
            ("GREEK", "CP1253"),
            ("TURKISH", "CP1254"),
            ("VIETNAMESE", "CP1258"),
            ("HEBREW", "CP1255"),
            ("ARABIC", "CP1256"),
            ("BALTIC", "CP1257"),
            ("RUSSIAN", "CP1251"),
            ("THAI", "CP874"),
            ("EASTEUROPE", "CP1250"),
            ("UTF-8", "UTF-8"),
            ("", "CP1252"),
            ("CP1252", "CP1252"),
        ] {
            let config = ReferenceQueryConfig {
                language_charset: configured.to_string(),
                language_sequence: String::new(),
                http_backend: Default::default(),
            };
            assert_eq!(config.charset_code_name(), expected, "{configured}");
        }
    }

    #[test]
    fn reference_text_decodes_with_each_cpp_language_code_page() {
        // The C++ frontend converts its internal reference strings through the
        // converter created from GetCharsetCodeName(LanguageCharset)
        // (pristine 9ffa0a5d src/C4Language.cpp:310-316;
        // src/C4TextEncoding.cpp:24-36).
        for (configured, encoded, expected) in [
            ("SHIFTJIS", &[0x93, 0xfa][..], "日"),
            ("HANGUL", &[0xc7, 0xd1][..], "한"),
            ("JOHAB", &[0xd0, 0x65][..], "한"),
            ("CHINESEBIG5", &[0xba, 0x7e][..], "漢"),
            ("GREEK", &[0xc1][..], "Α"),
            ("TURKISH", &[0xd0][..], "Ğ"),
            ("VIETNAMESE", &[0xd0][..], "Đ"),
            ("HEBREW", &[0xe0][..], "א"),
            ("ARABIC", &[0xc7][..], "ا"),
            ("BALTIC", &[0xc0][..], "Ą"),
            ("RUSSIAN", &[0xc0][..], "А"),
            ("THAI", &[0xa1][..], "ก"),
            ("EASTEUROPE", &[0xa5][..], "Ą"),
            ("UTF-8", &[0xe2, 0x82, 0xac][..], "€"),
            ("", &[0x80][..], "€"),
        ] {
            let config = ReferenceQueryConfig {
                language_charset: configured.to_string(),
                language_sequence: String::new(),
                http_backend: Default::default(),
            };
            let mut body = b"[Reference]\nTitle=\"".to_vec();
            body.extend_from_slice(encoded);
            body.extend_from_slice(b"\"\n");
            let reference = parse_reference_query_response_with_config(&body, &config)
                .unwrap()
                .references
                .remove(0);
            assert_eq!(reference.title, expected, "{configured}");
        }
    }

    #[test]
    fn discovery_multicast_target_uses_cpp_default_interface() {
        // C4NetIOSimpleUDP::InitBroadcast joins ff02::1 with
        // ipv6mr_interface=0 and leaves the destination scope unset; it does
        // not enumerate or fan out over interfaces (pristine 9ffa0a5d
        // src/C4NetIO.cpp:1587-1633, under its own `// TODO: do multicast on
        // all interfaces?` at :1623). Wherever that join succeeds the port
        // still sends exactly that one datagram; the fan-out below is reached
        // only once the kernel has refused it, because on a host whose default
        // multicast route has no IPv6-capable interface -- a Mac with IPv6
        // switched off on its only LAN NIC is enough -- the C++ join returns
        // EADDRNOTAVAIL and every send EHOSTUNREACH, so LAN discovery is dead
        // in both directions (clonk-org/clonk-rs#107). Enumerating cannot
        // desync: discovery only selects which game to join, before any
        // control is exchanged.
        let target = SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0);

        assert_eq!(
            multicast_targets(target, &[DEFAULT_MULTICAST_INTERFACE]),
            vec![target]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_an_unusable_discovery_socket_is_rebuilt_on_refresh() {
        // C4StartupNetDlg builds DiscoverClient once in its constructor and
        // never re-inits it (pristine 9ffa0a5d src/C4StartupNetDlg.cpp:737,
        // 1093-1105). The port rebuilds only what cannot work at all, so a
        // socket that joined a group keeps its buffered replies across a
        // refresh exactly as C++ does.
        let unbuilt = Err::<DiscoverySocket, _>(io::Error::from(io::ErrorKind::AddrNotAvailable));
        let joined = discovery_socket(0).expect("an ephemeral discovery socket binds");

        assert!(discovery_needs_rebuild(&unbuilt));
        assert_eq!(
            discovery_needs_rebuild(&Ok(joined)),
            joined_nothing_on_this_host(),
        );
    }

    /// Whether this host refused every multicast join, which decides what
    /// `only_an_unusable_discovery_socket_is_rebuilt_on_refresh` may expect
    /// without baking one kernel's answer into the assertion.
    fn joined_nothing_on_this_host() -> bool {
        discovery_socket(0)
            .expect("an ephemeral discovery socket binds")
            .multicast_interfaces
            .is_empty()
    }

    #[test]
    fn a_scoped_join_set_sends_one_probe_per_joined_interface() {
        // The unscoped destination only reaches the interface the kernel picks
        // by default. Once that interface has refused the join there is nothing
        // left to reach it through, so each joined interface gets its own
        // destination scope.
        let target = SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0);

        assert_eq!(
            multicast_targets(target, &[3, 11]),
            vec![
                SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 3),
                SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 11),
            ]
        );
    }

    #[test]
    fn an_unjoinable_host_still_probes_the_cpp_default_interface() {
        // C4NetIOSimpleUDP::Send keeps sending to the unscoped group after a
        // refused join, because InitBroadcast leaves the socket usable
        // (pinned oracle src/C4NetIO.cpp:1626-1632, :1773-1791).
        let target = SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0);

        assert_eq!(multicast_targets(target, &[]), vec![target]);
    }

    #[test]
    fn the_cpp_default_interface_never_sets_ipv6_multicast_if() {
        // C++ sets IPV6_MULTICAST_HOPS, IPV6_ADD_MEMBERSHIP and
        // IPV6_MULTICAST_LOOP and no other multicast option; IPV6_MULTICAST_IF
        // appears nowhere in the oracle tree (pinned oracle
        // src/C4NetIO.cpp:1614, :1627, :1886). macOS rejects a request for
        // interface 0 outright, so only the scoped fan-out may ask for one.
        let target = SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0);
        let scoped = SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 7);

        assert_eq!(multicast_send_interface(&target), None);
        assert_eq!(multicast_send_interface(&scoped), Some(7));
    }

    #[test]
    fn enumerated_multicast_interfaces_do_not_depend_on_kernel_listing_order() {
        // `if_nameindex` reports interfaces in kernel-list order, which differs
        // between hosts and across reboots. The fallback drives probe send
        // order, so it is sorted and deduplicated; index 0 is excluded because
        // it is the attempt that already failed.
        let interfaces = multicast_interface_indices();

        assert!(interfaces.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(!interfaces.contains(&DEFAULT_MULTICAST_INTERFACE));
    }

    #[test]
    fn a_refused_default_multicast_join_keeps_every_joinable_interface() {
        // C4NetIOSimpleUDP::InitBroadcast joins on ipv6mr_interface=0 alone and
        // gives up when the kernel refuses it (pinned oracle
        // src/C4NetIO.cpp:1620-1631). Where the default interface has no IPv6
        // route the port keeps searching instead, so discovery survives.
        let refuses_the_default = |interface: u32| {
            (interface == DEFAULT_MULTICAST_INTERFACE || interface == 7)
                .then(|| io::Error::from(io::ErrorKind::AddrNotAvailable))
                .map_or(Ok(()), Err)
        };

        assert_eq!(
            joined_discovery_interfaces(&|| vec![3, 7, 11], &refuses_the_default),
            vec![3, 11]
        );
    }

    #[test]
    fn an_accepted_default_multicast_join_enumerates_no_interfaces() {
        // The whole fallback is invisible wherever C++ works: the port must not
        // even ask the kernel for an interface list, so a host that behaves
        // like the oracle issues exactly the one join the oracle issues
        // (pinned oracle src/C4NetIO.cpp:1627-1631).
        let enumerated = std::cell::Cell::new(false);
        let joined = joined_discovery_interfaces(
            &|| {
                enumerated.set(true);
                vec![3, 11]
            },
            &|_| Ok(()),
        );

        assert_eq!(joined, vec![DEFAULT_MULTICAST_INTERFACE]);
        assert!(
            !enumerated.get(),
            "the C++ path must not enumerate interfaces"
        );
    }

    #[test]
    fn duplicate_live_lan_reference_queries_are_suppressed_by_address() {
        let address: SocketAddr = "127.0.0.1:31112".parse().unwrap();
        let command = SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Address(address),
            source: ReferenceQuerySource::GameDiscovery,
            timeout: REFERENCE_QUERY_TIMEOUT,
        };
        let mut gate = GameDiscoveryQueryGate::default();
        let now = Instant::now();

        assert!(gate.begin_at(now, &command));
        assert!(!gate.begin_at(now, &command));
        gate.finish_at(now, address, GameDiscoveryQueryOutcome::NoReferences);
        let empty_until = now + EMPTY_REFERENCE_LIFETIME;
        assert!(!gate.begin_at(empty_until - Duration::from_millis(1), &command));
        assert!(gate.begin_at(empty_until, &command));
    }

    #[test]
    fn lan_probes_run_more_often_than_the_masterserver_query() {
        // Deliberate divergence. C4StartupNetDlg counts one countdown down per
        // second and re-probes the LAN when it passes zero, so its game list
        // and its masterserver row both refresh every thirty seconds (pinned
        // oracle src/C4StartupNetDlg.cpp:1116-1131; src/C4StartupNetDlg.h:27,31).
        // The port keeps the masterserver on the oracle's interval - that is a
        // shared public server - and probes the LAN on its own.
        assert!(LAN_DISCOVERY_INTERVAL < GAME_SEARCH_INTERVAL);
        let now = Instant::now();
        let mut deadlines = SearchDeadlines::armed_at(now);

        assert!(!deadlines
            .take_due_lan_probe_at(now + LAN_DISCOVERY_INTERVAL - Duration::from_millis(1)));
        assert!(deadlines.take_due_lan_probe_at(now + LAN_DISCOVERY_INTERVAL));
        assert!(
            !deadlines.take_due_masterserver_query_at(now + LAN_DISCOVERY_INTERVAL),
            "a LAN probe must not drag the masterserver query forward with it"
        );
        assert!(deadlines.take_due_masterserver_query_at(now + GAME_SEARCH_INTERVAL));
    }

    #[test]
    fn a_stalled_search_worker_sends_one_catch_up_lan_probe() {
        // A worker that missed its deadline owes one probe, not one per
        // interval it slept through: C4StartupNetDlg reloads its countdown from
        // the tick that fired it (pinned oracle src/C4StartupNetDlg.cpp:
        // 1123-1127), so a stalled second never queues a burst.
        let now = Instant::now();
        let mut deadlines = SearchDeadlines::armed_at(now);
        let stalled_until = now + GAME_SEARCH_INTERVAL * 2;

        assert!(deadlines.take_due_lan_probe_at(stalled_until));

        assert!(
            !deadlines.take_due_lan_probe_at(
                stalled_until + LAN_DISCOVERY_INTERVAL - Duration::from_millis(1)
            ),
            "the missed ticks are dropped rather than fired back to back"
        );
        assert!(deadlines.take_due_lan_probe_at(stalled_until + LAN_DISCOVERY_INTERVAL));
    }

    #[test]
    fn a_resolved_lan_host_is_requeried_on_the_cpp_discovery_interval() {
        // C4StartupNetDlg deletes a LAN query row the moment its answer is
        // converted into references (pinned oracle src/C4StartupNetDlg.cpp:
        // 329-334), and IsSameRefQueryAddress matches unretrieved rows only
        // (:590-600), so the host is queried again on the dialog's next probe -
        // once per C4NetGameDiscoveryInterval (:1116-1131; C4StartupNetDlg.h:31).
        // This port probes far more often than that, so it holds the per-host
        // interval here instead of inheriting it from the probe cadence.
        let address: SocketAddr = "127.0.0.1:31113".parse().unwrap();
        let command = SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Address(address),
            source: ReferenceQuerySource::GameDiscovery,
            timeout: REFERENCE_QUERY_TIMEOUT,
        };
        let mut gate = GameDiscoveryQueryGate::default();
        let now = Instant::now();

        assert!(gate.begin_at(now, &command));
        gate.finish_at(now, address, GameDiscoveryQueryOutcome::References);

        assert!(
            !gate.begin_at(
                now + GAME_SEARCH_INTERVAL - Duration::from_millis(1),
                &command
            ),
            "a host already on the list keeps the oracle's re-query interval"
        );
        assert!(gate.begin_at(now + GAME_SEARCH_INTERVAL, &command));
    }

    #[test]
    fn a_failed_lan_reference_query_backs_off_for_the_cpp_error_row_lifetime() {
        // Deliberate divergence. IsSameRefQueryAddress refuses to match a failed
        // non-masterserver row - "if request failed, create a duplicate anyway
        // in case the game is opened now" (pinned oracle
        // src/C4StartupNetDlg.cpp:590-600) - so C++ retries a refusing host on
        // every probe. At this port's probe rate that would stack an error row
        // and a connection attempt several times over the ten seconds one such
        // row is displayed, so the retry waits out C4NetErrorRefTimeout
        // (src/C4StartupNetDlg.h:30), the lifetime C++ gives the row itself
        // (:506-531).
        let address: SocketAddr = "127.0.0.1:31114".parse().unwrap();
        let command = SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Address(address),
            source: ReferenceQuerySource::GameDiscovery,
            timeout: REFERENCE_QUERY_TIMEOUT,
        };
        let mut gate = GameDiscoveryQueryGate::default();
        let now = Instant::now();

        assert!(gate.begin_at(now, &command));
        gate.finish_at(now, address, GameDiscoveryQueryOutcome::Failed);

        assert!(
            !gate.begin_at(
                now + EMPTY_REFERENCE_LIFETIME - Duration::from_millis(1),
                &command
            ),
            "a refusing host is not retried on every probe"
        );
        assert!(gate.begin_at(now + EMPTY_REFERENCE_LIFETIME, &command));
    }

    #[test]
    fn an_explicit_refresh_readmits_every_host_the_gate_is_holding() {
        // DoRefresh deletes every row and restarts discovery from nothing
        // (pinned oracle src/C4StartupNetDlg.cpp:1078-1109), so no backoff this
        // gate is holding may outlive it - the button has to mean now.
        let address: SocketAddr = "127.0.0.1:31115".parse().unwrap();
        let command = SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Address(address),
            source: ReferenceQuerySource::GameDiscovery,
            timeout: REFERENCE_QUERY_TIMEOUT,
        };
        let mut gate = GameDiscoveryQueryGate::default();
        let now = Instant::now();

        assert!(gate.begin_at(now, &command));
        gate.finish_at(now, address, GameDiscoveryQueryOutcome::References);
        assert!(!gate.begin_at(now, &command));

        gate.clear();

        assert!(gate.begin_at(now, &command));
    }

    #[test]
    fn lan_probe_send_failure_reporting_matches_cpp_call_sites() {
        // C4StartupNetDlg ignores the initial and timer StartDiscovery results,
        // but checks the explicit refresh result before continuing with the
        // master query (pristine 9ffa0a5d src/C4StartupNetDlg.cpp:736-739,
        // 1093-1105, 1122-1128).
        let failure = || io::Error::new(io::ErrorKind::HostUnreachable, "no route");
        let send = LanProbeFailure::Send;

        assert!(lan_probe_error_event(LanProbeTrigger::Initial, send, failure()).is_none());
        assert!(lan_probe_error_event(LanProbeTrigger::Periodic, send, failure()).is_none());

        let event = lan_probe_error_event(LanProbeTrigger::ExplicitRefresh, send, failure())
            .expect("explicit refresh reports the discovery send failure");
        match event {
            StartupGameSearchEvent::SearchError { source, message } => {
                assert_eq!(source, Some(ReferenceQuerySource::GameDiscovery));
                assert_eq!(message, "unable to send LAN discovery probe: no route");
            }
            _ => panic!("expected LAN discovery error"),
        }
    }

    #[test]
    fn an_unbuilt_discovery_socket_is_not_reported_as_a_failed_send() {
        // The only error C4StartupNetDlg's refresh modal can carry comes from
        // C4NetIOSimpleUDP::Send, because InitBroadcast's failure never reaches
        // GetError() there (pinned oracle src/C4NetIO.cpp:1784,
        // src/C4StartupNetDlg.cpp:1094-1102). A socket that was never built
        // sent nothing, so it must not claim a send was attempted.
        let error = || io::Error::new(io::ErrorKind::AddrNotAvailable, "no multicast interface");

        let event = lan_probe_error_event(
            LanProbeTrigger::ExplicitRefresh,
            LanProbeFailure::Unavailable,
            error(),
        )
        .expect("explicit refresh reports the unusable socket");
        match event {
            StartupGameSearchEvent::SearchError { source, message } => {
                assert_eq!(source, Some(ReferenceQuerySource::GameDiscovery));
                assert_eq!(
                    message,
                    "LAN discovery is unavailable: no multicast interface"
                );
            }
            _ => panic!("expected LAN discovery error"),
        }
    }

    #[test]
    fn startup_direct_query_tags_merged_and_empty_results() {
        let body = b"[Reference]\n\
Title=First returned\n\
State=Running\n\
StartTime=1\n\
JoinAllowed=1\n\
Address=TCP:127.0.0.1:31112\n\
Game=LegacyClonk\n\
Version=4,9,10,0\n\
Build=361\n\
[Reference]\n\
Title=Higher sorted\n\
State=Lobby\n\
StartTime=2\n\
JoinAllowed=1\n\
Address=TCP:127.0.0.1:31113\n\
Game=LegacyClonk\n\
Version=4,9,11,0\n\
Build=362\n"
            .to_vec();
        let (address, server) = spawn_reference_server(vec![body, Vec::new()]);
        let search = StartupGameSearch::start(NetworkGameSearchConfig {
            internet_enabled: false,
            discovery_port: 0,
            ..NetworkGameSearchConfig::default()
        })
        .unwrap();

        search
            .query_direct(41, format!("  {address}  "), DEFAULT_REFERENCE_PORT)
            .unwrap();
        let first_references = match search
            .events()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            StartupGameSearchEvent::DirectQueryResolved {
                request_id,
                references,
                selected_index,
            } => {
                assert_eq!(request_id, 41);
                assert_eq!(selected_index, Some(1));
                assert_eq!(references.len(), 2);
                assert_eq!(references[0].title, "Higher sorted");
                assert_eq!(references[1].title, "First returned");
                references
            }
            event => panic!("expected tagged direct-query result, got {event:?}"),
        };

        search
            .query_direct(42, address.to_string(), DEFAULT_REFERENCE_PORT)
            .unwrap();
        match search
            .events()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            StartupGameSearchEvent::DirectQueryResolved {
                request_id,
                references,
                selected_index,
            } => {
                assert_eq!(request_id, 42);
                assert_eq!(selected_index, None);
                assert_eq!(references, first_references);
            }
            event => panic!("expected tagged empty direct-query result, got {event:?}"),
        }

        server.join().unwrap();
    }

    #[test]
    fn startup_lan_reference_query_reports_address_lifecycle() {
        let discovery_port = std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let unavailable_reference_port = std::net::TcpListener::bind((Ipv6Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let search = StartupGameSearch::start(NetworkGameSearchConfig {
            internet_enabled: false,
            discovery_port,
            ..NetworkGameSearchConfig::default()
        })
        .unwrap();
        search.initial_refresh().unwrap();
        loop {
            if matches!(
                search
                    .events()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap(),
                StartupGameSearchEvent::Cleared
            ) {
                break;
            }
        }

        let sender = std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
        let port = unavailable_reference_port.to_ne_bytes();
        sender
            .send_to(
                &[DISCOVERY_REPLY, 0, port[0], port[1]],
                (Ipv6Addr::LOCALHOST, discovery_port),
            )
            .unwrap();
        let expected = SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::LOCALHOST,
            unavailable_reference_port,
            0,
            0,
        ));

        assert!(matches!(
            search.events().recv_timeout(Duration::from_secs(5)).unwrap(),
            StartupGameSearchEvent::GameDiscoveryQueryStarted { address }
                if address == expected
        ));
        match search
            .events()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            StartupGameSearchEvent::GameDiscoveryQueryFailed { address, message } => {
                assert_eq!(address, expected);
                assert!(!message.is_empty());
            }
            event => panic!("expected LAN reference failure, got {event:?}"),
        }
    }

    #[test]
    fn startup_refresh_discards_inflight_direct_query_with_deleted_row() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (request_started_tx, request_started_rx) = mpsc::channel();
        let (release_response_tx, release_response_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            request_started_tx.send(()).unwrap();
            release_response_rx.recv().unwrap();
            let body = b"[Reference]\n\
Title=Delayed direct\n\
State=Lobby\n\
StartTime=1\n\
JoinAllowed=1\n\
Address=TCP:127.0.0.1:31112\n\
Game=LegacyClonk\n\
Version=4,9,11,0\n\
Build=362\n";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        });
        let search = StartupGameSearch::start(NetworkGameSearchConfig {
            internet_enabled: false,
            discovery_port: 0,
            ..NetworkGameSearchConfig::default()
        })
        .unwrap();

        search
            .query_direct(59, address.to_string(), DEFAULT_REFERENCE_PORT)
            .unwrap();
        request_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("direct request reached the delayed server");
        search.refresh().unwrap();
        loop {
            if matches!(
                search
                    .events()
                    .recv_timeout(Duration::from_secs(5))
                    .expect("refresh emits its cleared event"),
                StartupGameSearchEvent::Cleared
            ) {
                break;
            }
        }
        release_response_tx.send(()).unwrap();
        server.join().unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match search.events().recv_timeout(Duration::from_millis(100)) {
                Ok(StartupGameSearchEvent::DirectQueryResolved { request_id, .. })
                | Ok(StartupGameSearchEvent::DirectQueryFailed { request_id, .. }) => {
                    panic!("refresh deleted direct-query row {request_id}, but it completed")
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    #[test]
    fn startup_direct_query_reports_tagged_endpoint_failures() {
        let search = StartupGameSearch::start(NetworkGameSearchConfig {
            internet_enabled: false,
            discovery_port: 0,
            ..NetworkGameSearchConfig::default()
        })
        .unwrap();

        search
            .query_direct(
                73,
                "ftp://games.example.test".to_string(),
                DEFAULT_REFERENCE_PORT,
            )
            .unwrap();
        match search
            .events()
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
        {
            StartupGameSearchEvent::DirectQueryFailed {
                request_id,
                message,
            } => {
                assert_eq!(request_id, 73);
                assert!(message.contains("unsupported"), "{message}");
            }
            event => panic!("expected tagged direct-query failure, got {event:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn discovery_initialization_failure_waits_for_explicit_refresh() {
        // C4StartupNetDlg ignores discovery initialization and its first send,
        // then reports StartDiscovery failure only from DoRefresh (pristine
        // 9ffa0a5d src/C4StartupNetDlg.cpp:736-739, 1093-1105).
        let discovery = Err::<DiscoverySocket, _>(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no multicast interface",
        ));
        let (query_tx, _query_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel();
        let mut masterserver_query = None;
        let command = |trigger| SearchCommand::SendLanProbe {
            target: SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0),
            payload: vec![DISCOVERY_PROBE],
            trigger,
        };

        for trigger in [LanProbeTrigger::Initial, LanProbeTrigger::Periodic] {
            execute_search_command(
                command(trigger),
                (0, 0),
                None,
                &mut masterserver_query,
                discovery.as_ref(),
                &query_tx,
                &event_tx,
                &ReferenceQueryConfig::default(),
            )
            .await;
            assert!(matches!(
                event_rx.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
        }

        execute_search_command(
            command(LanProbeTrigger::ExplicitRefresh),
            (0, 0),
            None,
            &mut masterserver_query,
            discovery.as_ref(),
            &query_tx,
            &event_tx,
            &ReferenceQueryConfig::default(),
        )
        .await;
        match event_rx
            .try_recv()
            .expect("explicit refresh reports failure")
        {
            StartupGameSearchEvent::SearchError { source, message } => {
                assert_eq!(source, Some(ReferenceQuerySource::GameDiscovery));
                assert_eq!(
                    message,
                    "LAN discovery is unavailable: no multicast interface"
                );
            }
            _ => panic!("expected LAN discovery error"),
        }
    }
}
