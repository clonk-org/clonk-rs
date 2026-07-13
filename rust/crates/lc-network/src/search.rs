//! Startup network-game discovery matching `C4StartupNetDlg` and
//! `C4Network2IODiscoverClient`.

use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use thiserror::Error;
use tokio::net::UdpSocket;

pub const DEFAULT_MASTER_SERVER_URL: &str = "https://league.clonkspot.org/";
pub const DEFAULT_REFERENCE_PORT: u16 = 11_111;
pub const DEFAULT_DISCOVERY_PORT: u16 = 11_114;
pub const MAX_LAN_DISCOVERS: usize = 64;
pub const REFERENCE_QUERY_TIMEOUT: Duration = Duration::from_secs(12);
pub const GAME_SEARCH_INTERVAL: Duration = Duration::from_secs(30);

const DISCOVERY_PROBE: u8 = 0x03;
const DISCOVERY_REPLY: u8 = 0x04;
pub(crate) const DISCOVERY_MULTICAST: Ipv6Addr =
    Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

pub const CURRENT_GAME_VERSION: [i32; 4] = [4, 9, 11, 0];
pub const CURRENT_GAME_BUILD: i32 = 362;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkGameReference {
    pub title: String,
    pub host_name: String,
    pub host_nick: String,
    pub state: String,
    pub control_mode: i32,
    pub start_time: i64,
    pub join_allowed: bool,
    pub password_needed: bool,
    pub official_server: bool,
    pub max_players: i32,
    pub game: String,
    pub version: [i32; 4],
    pub build: i32,
    pub tcp_addresses: Vec<SocketAddr>,
}

impl Default for NetworkGameReference {
    fn default() -> Self {
        Self {
            title: String::new(),
            host_name: String::new(),
            host_nick: String::new(),
            state: "None".to_string(),
            control_mode: -1,
            start_time: 0,
            join_allowed: true,
            password_needed: false,
            official_server: false,
            max_players: 0,
            game: "None".to_string(),
            version: [0; 4],
            build: -1,
            tcp_addresses: Vec::new(),
        }
    }
}

impl NetworkGameReference {
    pub fn is_joinable(&self) -> bool {
        self.join_allowed
            && self.version == CURRENT_GAME_VERSION
            && self.build == CURRENT_GAME_BUILD
    }

    fn is_same_host_and_address(&self, other: &Self) -> bool {
        self.host_name == other.host_name
            && self
                .tcp_addresses
                .iter()
                .any(|address| other.tcp_addresses.contains(address))
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
    #[error("reference request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Parse(#[from] ReferenceParseError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkGameSearchConfig {
    pub internet_enabled: bool,
    pub master_server_url: String,
    pub discovery_port: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceQueryConfig {
    /// `Config.General.LanguageCharset`, before C++ code-page canonicalization.
    pub language_charset: String,
    /// `Config.General.LanguageEx`, preserved as the HTTP language preference.
    pub language_sequence: String,
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
}

impl NetworkGameSearch {
    pub fn new(mut config: NetworkGameSearchConfig) -> Self {
        config.master_server_url = normalize_master_server_url(&config.master_server_url);
        Self {
            config,
            lan_discover_count: 0,
            references: Vec::new(),
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
        self.config.internet_enabled = enabled;
        enabled.then(|| self.masterserver_query())
    }

    pub fn periodic_commands(&mut self) -> Vec<SearchCommand> {
        self.lan_discover_count = 0;
        let mut commands = vec![SearchCommand::SendLanProbe {
            target: SocketAddrV6::new(DISCOVERY_MULTICAST, self.config.discovery_port, 0, 0),
            payload: vec![DISCOVERY_PROBE],
            trigger: LanProbeTrigger::Periodic,
        }];
        if self.config.internet_enabled {
            commands.push(self.masterserver_query());
        }
        commands
    }

    fn masterserver_query(&self) -> SearchCommand {
        SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Url(self.config.master_server_url.clone()),
            source: ReferenceQuerySource::Masterserver,
            timeout: REFERENCE_QUERY_TIMEOUT,
        }
    }

    pub fn merge_references(&mut self, references: impl IntoIterator<Item = NetworkGameReference>) {
        for incoming in references {
            if let Some(existing) = self.references.iter_mut().find(|existing| {
                existing.is_same_host_and_address(&incoming)
                    && existing.start_time <= incoming.start_time
            }) {
                *existing = incoming;
            } else {
                self.references.push(incoming);
            }
        }
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

#[derive(Clone, Debug)]
pub enum StartupGameSearchEvent {
    Cleared,
    ReferencesUpdated(Vec<NetworkGameReference>),
    SearchError {
        source: Option<ReferenceQuerySource>,
        message: String,
    },
}

fn lan_probe_error_event(
    trigger: LanProbeTrigger,
    error: io::Error,
) -> Option<StartupGameSearchEvent> {
    (trigger == LanProbeTrigger::ExplicitRefresh).then(|| StartupGameSearchEvent::SearchError {
        source: Some(ReferenceQuerySource::GameDiscovery),
        message: format!("unable to send LAN discovery probe: {error}"),
    })
}

#[derive(Clone, Copy, Debug)]
enum StartupGameSearchCommand {
    InitialRefresh,
    Refresh,
    SetInternetEnabled(bool),
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
            .name("lc-game-search".to_string())
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
    source: ReferenceQuerySource,
    result: Result<Vec<NetworkGameReference>, ReferenceFetchError>,
}

struct DiscoverySocket {
    socket: UdpSocket,
    multicast_interfaces: Vec<u32>,
}

impl DiscoverySocket {
    async fn send_probe(&self, payload: &[u8], target: SocketAddrV6) -> io::Result<()> {
        let mut last_error = None;
        let mut sent = false;
        for target in multicast_targets(target, &self.multicast_interfaces) {
            if let Err(error) = SockRef::from(&self.socket).set_multicast_if_v6(target.scope_id()) {
                last_error = Some(error);
                continue;
            }
            match self.socket.send_to(payload, target).await {
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
}

async fn run_game_search(
    config: NetworkGameSearchConfig,
    reference_config: ReferenceQueryConfig,
    commands: mpsc::Receiver<StartupGameSearchCommand>,
    events: mpsc::Sender<StartupGameSearchEvent>,
) {
    let mut search = NetworkGameSearch::new(config.clone());
    let discovery = discovery_socket(config.discovery_port);
    let (query_tx, mut query_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut generation = 0_u64;
    let mut stopped = false;
    let mut datagram = [0_u8; 512];
    let mut next_periodic_search = tokio::time::Instant::now() + GAME_SEARCH_INTERVAL;

    while !stopped {
        while let Ok(command) = commands.try_recv() {
            match command {
                command @ (StartupGameSearchCommand::InitialRefresh
                | StartupGameSearchCommand::Refresh) => {
                    generation = generation.wrapping_add(1);
                    next_periodic_search = tokio::time::Instant::now() + GAME_SEARCH_INTERVAL;
                    let _ = events.send(StartupGameSearchEvent::Cleared);
                    let commands = match command {
                        StartupGameSearchCommand::InitialRefresh => search.initial_commands(),
                        _ => search.refresh(),
                    };
                    for command in commands {
                        execute_search_command(
                            command,
                            generation,
                            discovery.as_ref(),
                            &query_tx,
                            &events,
                            &reference_config,
                        )
                        .await;
                    }
                }
                StartupGameSearchCommand::SetInternetEnabled(enabled) => {
                    if let Some(command) = search.set_internet_enabled(enabled) {
                        execute_search_command(
                            command,
                            generation,
                            discovery.as_ref(),
                            &query_tx,
                            &events,
                            &reference_config,
                        )
                        .await;
                    }
                }
                StartupGameSearchCommand::Stop => stopped = true,
            }
        }
        while let Ok(query) = query_rx.try_recv() {
            if query.generation != generation {
                continue;
            }
            match query.result {
                Ok(references) => {
                    search.merge_references(references);
                    let _ = events.send(StartupGameSearchEvent::ReferencesUpdated(
                        search.references().to_vec(),
                    ));
                }
                Err(error) => {
                    let _ = events.send(StartupGameSearchEvent::SearchError {
                        source: Some(query.source),
                        message: error.to_string(),
                    });
                }
            }
        }
        if stopped {
            break;
        }
        if tokio::time::Instant::now() >= next_periodic_search {
            next_periodic_search += GAME_SEARCH_INTERVAL;
            for command in search.periodic_commands() {
                execute_search_command(
                    command,
                    generation,
                    discovery.as_ref(),
                    &query_tx,
                    &events,
                    &reference_config,
                )
                .await;
            }
        }
        if let Ok(socket) = discovery.as_ref() {
            if let Ok(Ok((size, source))) =
                tokio::time::timeout(
                    Duration::from_millis(20),
                    socket.socket.recv_from(&mut datagram),
                )
                    .await
            {
                if let Some(command) = search.handle_lan_datagram(source, &datagram[..size]) {
                    execute_search_command(
                        command,
                        generation,
                        discovery.as_ref(),
                        &query_tx,
                        &events,
                        &reference_config,
                    )
                    .await;
                }
            }
        } else {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn execute_search_command(
    command: SearchCommand,
    generation: u64,
    discovery: Result<&DiscoverySocket, &io::Error>,
    query_tx: &tokio::sync::mpsc::UnboundedSender<QueryResult>,
    events: &mpsc::Sender<StartupGameSearchEvent>,
    reference_config: &ReferenceQueryConfig,
) {
    match command {
        SearchCommand::SendLanProbe {
            target,
            payload,
            trigger,
        } => {
            let result = match discovery {
                Ok(socket) => socket.send_probe(&payload, target).await,
                Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
            };
            if let Err(error) = result {
                if let Some(event) = lan_probe_error_event(trigger, error) {
                    let _ = events.send(event);
                }
            }
        }
        SearchCommand::QueryReferences {
            endpoint,
            source,
            timeout,
        } => {
            let query_tx = query_tx.clone();
            let reference_config = reference_config.clone();
            tokio::spawn(async move {
                let result =
                    fetch_reference_endpoint_with_config(endpoint, timeout, &reference_config)
                        .await;
                let _ = query_tx.send(QueryResult {
                    generation,
                    source,
                    result,
                });
            });
        }
    }
}

fn discovery_socket(port: u16) -> io::Result<DiscoverySocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(false)?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_multicast_hops_v6(16)?;
    socket.set_multicast_loop_v6(true)?;
    socket.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0).into())?;
    let mut multicast_interfaces = Vec::new();
    for interface in multicast_interface_indices() {
        if socket
            .join_multicast_v6(&DISCOVERY_MULTICAST, interface)
            .is_ok()
        {
            multicast_interfaces.push(interface);
        }
    }
    if multicast_interfaces.is_empty() {
        socket.join_multicast_v6(&DISCOVERY_MULTICAST, 0)?;
        multicast_interfaces.push(0);
    }
    socket.set_nonblocking(true)?;
    Ok(DiscoverySocket {
        socket: UdpSocket::from_std(socket.into())?,
        multicast_interfaces,
    })
}

pub(crate) fn multicast_targets(
    target: SocketAddrV6,
    interfaces: &[u32],
) -> Vec<SocketAddrV6> {
    if target.scope_id() != 0 || interfaces.is_empty() {
        return vec![target];
    }
    interfaces
        .iter()
        .map(|interface| {
            SocketAddrV6::new(
                *target.ip(),
                target.port(),
                target.flowinfo(),
                *interface,
            )
        })
        .collect()
}

#[cfg(unix)]
pub(crate) fn multicast_interface_indices() -> Vec<u32> {
    // Match C++'s getifaddrs enumeration, but select only usable IPv6 LAN
    // multicast interfaces. Numeric indices become the ff02::1 scope IDs.
    unsafe {
        let mut entries = std::ptr::null_mut();
        if libc::getifaddrs(&mut entries) != 0 || entries.is_null() {
            return Vec::new();
        }
        let mut indices = Vec::new();
        let mut entry = entries;
        while !entry.is_null() {
            let flags = (*entry).ifa_flags as libc::c_int;
            let address = (*entry).ifa_addr;
            if !(*entry).ifa_name.is_null()
                && !address.is_null()
                && (*address).sa_family as libc::c_int == libc::AF_INET6
                && flags & libc::IFF_UP != 0
                && flags & libc::IFF_MULTICAST != 0
                && flags & libc::IFF_LOOPBACK == 0
            {
                let index = libc::if_nametoindex((*entry).ifa_name);
                if index != 0 {
                    indices.push(index);
                }
            }
            entry = (*entry).ifa_next;
        }
        libc::freeifaddrs(entries);
        indices.sort_unstable();
        indices.dedup();
        indices
    }
}

#[cfg(not(unix))]
pub(crate) fn multicast_interface_indices() -> Vec<u32> {
    vec![0]
}

pub fn parse_reference_response(
    bytes: &[u8],
) -> Result<Vec<NetworkGameReference>, ReferenceParseError> {
    parse_reference_response_with_config(bytes, &ReferenceQueryConfig::default())
}

fn parse_reference_response_with_config(
    bytes: &[u8],
    config: &ReferenceQueryConfig,
) -> Result<Vec<NetworkGameReference>, ReferenceParseError> {
    let text = config.decode(bytes)?;
    let mut chunks = Vec::new();
    let mut current = None::<Vec<&str>>;
    for line in text.lines() {
        if line == "[Reference]" {
            if let Some(chunk) = current.take() {
                chunks.push(chunk);
            }
            current = Some(Vec::new());
        } else if let Some(chunk) = current.as_mut() {
            chunk.push(line);
        }
    }
    if let Some(chunk) = current {
        chunks.push(chunk);
    }
    chunks.into_iter().map(parse_reference_chunk).collect()
}

pub async fn fetch_reference_endpoint(
    endpoint: ReferenceEndpoint,
    timeout: Duration,
) -> Result<Vec<NetworkGameReference>, ReferenceFetchError> {
    fetch_reference_endpoint_with_config(endpoint, timeout, &ReferenceQueryConfig::default()).await
}

pub async fn fetch_reference_endpoint_with_config(
    endpoint: ReferenceEndpoint,
    timeout: Duration,
    config: &ReferenceQueryConfig,
) -> Result<Vec<NetworkGameReference>, ReferenceFetchError> {
    let url = match endpoint {
        ReferenceEndpoint::Url(url) => url,
        ReferenceEndpoint::Address(address) => reference_url(address),
    };
    let response = reqwest::Client::builder()
        .user_agent("LegacyClonk/4.9.11.0 [362]")
        .gzip(true)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?
        .get(url)
        .header("Accept-Charset", config.charset_code_name())
        .header("Accept-Language", &config.language_sequence)
        .send()
        .await?
        .error_for_status()?;
    let source = response.remote_addr();
    let bytes = response.bytes().await?;
    let mut references = parse_reference_response_with_config(&bytes, config)?;
    if let Some(source) = source {
        fill_reference_source_addresses(&mut references, source);
    }
    Ok(references)
}

fn fill_reference_source_addresses(references: &mut [NetworkGameReference], source: SocketAddr) {
    for reference in references {
        for address in &mut reference.tcp_addresses {
            let port = address.port();
            if address.ip().is_unspecified() {
                *address = SocketAddr::new(source.ip(), port);
                if let (SocketAddr::V6(address), SocketAddr::V6(source)) = (&mut *address, source) {
                    address.set_scope_id(source.scope_id());
                }
            } else if let (SocketAddr::V6(address), SocketAddr::V6(source)) =
                (&mut *address, source)
            {
                if address.scope_id() == 0 {
                    address.set_scope_id(source.scope_id());
                }
            }
        }
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

fn parse_reference_chunk(lines: Vec<&str>) -> Result<NetworkGameReference, ReferenceParseError> {
    let mut reference = NetworkGameReference::default();
    let mut direct_client = false;
    let mut direct_client_id = None;
    let mut direct_client_name = None;
    let mut direct_client_nick = None;

    for line in lines {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if direct_client {
                flush_direct_client(
                    &mut reference,
                    &mut direct_client_id,
                    &mut direct_client_name,
                    &mut direct_client_nick,
                );
            }
            direct_client = indent == 2 && trimmed == "[Client]";
            continue;
        }
        let Some((key, raw_value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = unquote(raw_value.trim());
        if direct_client && indent == 2 {
            match key {
                "ID" => direct_client_id = Some(parse_i32(key, value)?),
                "Name" => direct_client_name = Some(value.to_string()),
                "Nick" => direct_client_nick = Some(value.to_string()),
                _ => {}
            }
            continue;
        }
        if indent != 0 {
            continue;
        }
        match key {
            "State" => reference.state = value.to_string(),
            "CtrlMode" => reference.control_mode = parse_i32(key, value)?,
            "StartTime" => reference.start_time = parse_i64(key, value)?,
            "JoinAllowed" => reference.join_allowed = parse_bool(value),
            "PasswordNeeded" => reference.password_needed = parse_bool(value),
            "OfficialServer" => reference.official_server = parse_bool(value),
            "MaxPlayers" => reference.max_players = parse_i32(key, value)?,
            "Address" => reference.tcp_addresses = parse_tcp_addresses(value)?,
            "Game" => reference.game = value.to_string(),
            "Version" => {
                for (index, part) in value.split(',').take(4).enumerate() {
                    reference.version[index] = parse_i32(key, part.trim())?;
                }
            }
            "Build" => reference.build = parse_i32(key, value)?,
            "Title" => reference.title = value.to_string(),
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
    Ok(reference)
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

fn invalid_integer(key: &str, value: &str) -> ReferenceParseError {
    ReferenceParseError::InvalidInteger {
        key: key.to_string(),
        value: value.to_string(),
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "true" | "1")
}

fn parse_tcp_addresses(value: &str) -> Result<Vec<SocketAddr>, ReferenceParseError> {
    value
        .split(',')
        .filter_map(|entry| entry.trim().strip_prefix("TCP:").map(unquote))
        .map(|address| {
            address
                .parse()
                .map_err(|_| ReferenceParseError::InvalidAddress(address.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            };
            let mut body = b"[Reference]\nTitle=\"".to_vec();
            body.extend_from_slice(encoded);
            body.extend_from_slice(b"\"\n");
            let reference = parse_reference_response_with_config(&body, &config)
                .unwrap()
                .remove(0);
            assert_eq!(reference.title, expected, "{configured}");
        }
    }

    #[test]
    fn link_local_multicast_targets_are_scoped_to_each_interface() {
        let target = SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0);

        assert_eq!(
            multicast_targets(target, &[2, 7]),
            vec![
                SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 2),
                SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 7),
            ]
        );
    }

    #[test]
    fn lan_probe_send_failure_reporting_matches_cpp_call_sites() {
        // C4StartupNetDlg ignores the initial and timer StartDiscovery results,
        // but checks the explicit refresh result before continuing with the
        // master query (pristine 9ffa0a5d src/C4StartupNetDlg.cpp:736-739,
        // 1093-1105, 1122-1128).
        let failure = || io::Error::new(io::ErrorKind::HostUnreachable, "no route");

        assert!(lan_probe_error_event(LanProbeTrigger::Initial, failure()).is_none());
        assert!(lan_probe_error_event(LanProbeTrigger::Periodic, failure()).is_none());

        let event = lan_probe_error_event(LanProbeTrigger::ExplicitRefresh, failure())
            .expect("explicit refresh reports the discovery send failure");
        match event {
            StartupGameSearchEvent::SearchError { source, message } => {
                assert_eq!(source, Some(ReferenceQuerySource::GameDiscovery));
                assert_eq!(
                    message,
                    "unable to send LAN discovery probe: no route"
                );
            }
            _ => panic!("expected LAN discovery error"),
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
        let command = |trigger| SearchCommand::SendLanProbe {
            target: SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0),
            payload: vec![DISCOVERY_PROBE],
            trigger,
        };

        for trigger in [LanProbeTrigger::Initial, LanProbeTrigger::Periodic] {
            execute_search_command(
                command(trigger),
                0,
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
            0,
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
                    "unable to send LAN discovery probe: no multicast interface"
                );
            }
            _ => panic!("expected LAN discovery error"),
        }
    }
}
