//! Startup network-game discovery matching `C4StartupNetDlg` and
//! `C4Network2IODiscoverClient`.

use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use thiserror::Error;
use tokio::net::UdpSocket;

use crate::{NetworkAddress, NetworkProtocol};

pub const DEFAULT_MASTER_SERVER_URL: &str = "https://league.clonkspot.org/";
pub const DEFAULT_REFERENCE_PORT: u16 = 11_111;
pub const DEFAULT_DISCOVERY_PORT: u16 = 11_114;
pub const MAX_LAN_DISCOVERS: usize = 64;
pub const REFERENCE_QUERY_TIMEOUT: Duration = Duration::from_secs(12);
pub const GAME_SEARCH_INTERVAL: Duration = Duration::from_secs(30);
const REFERENCE_LIFETIME: Duration = Duration::from_secs(42);

const DISCOVERY_PROBE: u8 = 0x03;
const DISCOVERY_REPLY: u8 = 0x04;
const SCOPED_IPV6_REQUEST_HOST: &str = "legacyclonk-lan.invalid";
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
    pub league_address: String,
    pub max_players: i32,
    pub game: String,
    pub version: [i32; 4],
    pub build: i32,
    /// Complete ordered `C4Network2Reference::Addrs` transport set.
    pub addresses: Vec<NetworkAddress>,
    /// Server endpoint retained by `C4Network2Reference::SetSourceAddress`.
    pub source_address: SocketAddr,
    /// Transitional TCP display projection retained for existing consumers.
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
            league_address: String::new(),
            max_players: 0,
            game: "None".to_string(),
            version: [0; 4],
            build: -1,
            addresses: Vec::new(),
            source_address: SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::UNSPECIFIED,
                0,
                0,
                0,
            )),
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

    fn sort_order(&self, use_alternate_server: bool) -> i32 {
        i32::from(self.official_server && !use_alternate_server) * 50
            + i32::from(self.is_joinable()) * 25
            + i32::from(!self.league_address.is_empty()) * 5
            + i32::from(self.state == "Lobby") * 3
            + i32::from(!self.password_needed)
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
    masterserver_generation: u64,
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
    let mut masterserver_generation = 0_u64;
    let mut masterserver_query: Option<tokio::task::JoinHandle<()>> = None;
    let mut stopped = false;
    let mut datagram = [0_u8; 512];
    let mut next_periodic_search = tokio::time::Instant::now() + GAME_SEARCH_INTERVAL;

    while !stopped {
        while let Ok(command) = commands.try_recv() {
            match command {
                command @ (StartupGameSearchCommand::InitialRefresh
                | StartupGameSearchCommand::Refresh) => {
                    if let Some(query) = masterserver_query.take() {
                        query.abort();
                    }
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
                            (generation, masterserver_generation),
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
                    if let Some(command) = search.set_internet_enabled(enabled) {
                        masterserver_generation = masterserver_generation.wrapping_add(1);
                        execute_search_command(
                            command,
                            (generation, masterserver_generation),
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
                StartupGameSearchCommand::Stop => {
                    if let Some(query) = masterserver_query.take() {
                        query.abort();
                    }
                    stopped = true;
                }
            }
        }
        while let Ok(query) = query_rx.try_recv() {
            if query.generation != generation
                || (query.source == ReferenceQuerySource::Masterserver
                    && (query.masterserver_generation != masterserver_generation
                        || !search.config.internet_enabled))
            {
                continue;
            }
            match query.result {
                Ok(references) => {
                    let now = Instant::now();
                    search.merge_references_at(now, references);
                    search.expire_references_at(now);
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
        if search.expire_references_at(Instant::now()) {
            let _ = events.send(StartupGameSearchEvent::ReferencesUpdated(
                search.references().to_vec(),
            ));
        }
        if stopped {
            break;
        }
        if tokio::time::Instant::now() >= next_periodic_search {
            next_periodic_search += GAME_SEARCH_INTERVAL;
            for command in search.periodic_commands() {
                execute_search_command(
                    command,
                    (generation, masterserver_generation),
                    &mut masterserver_query,
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
                        (generation, masterserver_generation),
                        &mut masterserver_query,
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
    query_generation: (u64, u64),
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
            let query = tokio::spawn(async move {
                let result =
                    fetch_reference_endpoint_with_config(endpoint, timeout, &reference_config)
                        .await;
                let _ = query_tx.send(QueryResult {
                    generation,
                    masterserver_generation,
                    source,
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
    _interfaces: &[u32],
) -> Vec<SocketAddrV6> {
    vec![target]
}

pub(crate) fn multicast_interface_indices() -> Vec<u32> {
    // C4NetIOSimpleUDP::InitBroadcast uses ipv6mr_interface=0 and relies on
    // the platform's default interface (C4NetIO.cpp:1617-1633).
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

    fn client_builder(&self) -> reqwest::ClientBuilder {
        match self.connect_address {
            Some(address) => reqwest::Client::builder()
                .no_proxy()
                .resolve(SCOPED_IPV6_REQUEST_HOST, address),
            None => reqwest::Client::builder(),
        }
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
    let plan = ReferenceRequestPlan::for_endpoint(endpoint);
    let client = plan
        .client_builder()
        .user_agent("LegacyClonk/4.9.11.0 [362]")
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
    let mut references = parse_reference_response_with_config(&bytes, config)?;
    if let Some(source) = source {
        fill_reference_source_addresses(&mut references, source);
    }
    Ok(references)
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
            "LeagueAddress" => reference.league_address = value.to_string(),
            "MaxPlayers" => reference.max_players = parse_i32(key, value)?,
            "Address" => {
                let addresses = parse_reference_addresses(value)?;
                reference.tcp_addresses = addresses
                    .iter()
                    .filter(|address| address.protocol == NetworkProtocol::Tcp)
                    .map(|address| address.endpoint)
                    .collect();
                reference.addresses = addresses;
            }
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

fn parse_reference_addresses(value: &str) -> Result<Vec<NetworkAddress>, ReferenceParseError> {
    value
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            entry
                .strip_prefix("UDP:")
                .map(|address| (NetworkProtocol::Udp, unquote(address)))
                .or_else(|| {
                    entry
                        .strip_prefix("TCP:")
                        .map(|address| (NetworkProtocol::Tcp, unquote(address)))
                })
        })
        .map(|(protocol, address)| {
            address
                .parse()
                .map_err(|_| ReferenceParseError::InvalidAddress(address.to_string()))
                .map(|endpoint| NetworkAddress::new(protocol, endpoint))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let client = plan.client_builder().build().unwrap();
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
    fn discovery_multicast_target_uses_cpp_default_interface() {
        // C4NetIOSimpleUDP::InitBroadcast joins ff02::1 with
        // ipv6mr_interface=0 and leaves the destination scope unset; it does
        // not enumerate or fan out over interfaces (pristine 9ffa0a5d
        // src/C4NetIO.cpp:1587-1633).
        let target = SocketAddrV6::new(DISCOVERY_MULTICAST, DEFAULT_DISCOVERY_PORT, 0, 0);

        assert_eq!(multicast_targets(target, &[2, 7]), vec![target]);
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
                    "unable to send LAN discovery probe: no multicast interface"
                );
            }
            _ => panic!("expected LAN discovery error"),
        }
    }
}
