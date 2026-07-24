//! Connection establishment: host binding & start entry points, client dial races, mesh routes.
//!
//! Moved byte-verbatim from `session.rs` (wave 2 of the decomposition
//! campaign, see REFACTOR_PLAN.md). Structural only.

use super::*;

#[cfg(test)]
type PostJoinBootstrapPause = (oneshot::Sender<()>, oneshot::Receiver<()>);

#[cfg(test)]
fn post_join_bootstrap_pauses() -> &'static Mutex<BTreeMap<Vec<u8>, PostJoinBootstrapPause>> {
    static PAUSES: std::sync::OnceLock<Mutex<BTreeMap<Vec<u8>, PostJoinBootstrapPause>>> =
        std::sync::OnceLock::new();
    PAUSES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
pub(crate) fn pause_client_post_join_bootstrap(
    client_name: &[u8],
) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
    let (reached_tx, reached_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    let replaced = post_join_bootstrap_pauses()
        .lock()
        .expect("post-JoinData bootstrap pause lock poisoned")
        .insert(client_name.to_vec(), (reached_tx, resume_rx));
    assert!(
        replaced.is_none(),
        "post-JoinData bootstrap pause already installed for this client"
    );
    (reached_rx, resume_tx)
}

#[cfg(test)]
async fn wait_at_client_post_join_bootstrap_pause(client_name: &[u8]) {
    let pause = post_join_bootstrap_pauses()
        .lock()
        .expect("post-JoinData bootstrap pause lock poisoned")
        .remove(client_name);
    let Some((reached, resume)) = pause else {
        return;
    };
    let _ = reached.send(());
    let _ = resume.await;
}

#[cfg(test)]
type ResourceBootstrapProbePause = (oneshot::Sender<()>, std::sync::mpsc::Receiver<()>);

#[cfg(test)]
fn resource_bootstrap_probe_pauses(
) -> &'static Mutex<BTreeMap<Vec<u8>, ResourceBootstrapProbePause>> {
    static PAUSES: std::sync::OnceLock<Mutex<BTreeMap<Vec<u8>, ResourceBootstrapProbePause>>> =
        std::sync::OnceLock::new();
    PAUSES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
pub(crate) fn pause_client_resource_bootstrap_probe(
    client_name: &[u8],
) -> (oneshot::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (reached_tx, reached_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let replaced = resource_bootstrap_probe_pauses()
        .lock()
        .expect("resource-bootstrap probe pause lock poisoned")
        .insert(client_name.to_vec(), (reached_tx, resume_rx));
    assert!(
        replaced.is_none(),
        "resource-bootstrap probe pause already installed for this client"
    );
    (reached_rx, resume_tx)
}

#[cfg(test)]
fn wait_at_client_resource_bootstrap_probe_pause(client_name: &[u8]) {
    let pause = resource_bootstrap_probe_pauses()
        .lock()
        .expect("resource-bootstrap probe pause lock poisoned")
        .remove(client_name);
    let Some((reached, resume)) = pause else {
        return;
    };
    let _ = reached.send(());
    let _ = resume.recv();
}

struct ClientPostJoinResourceBootstrap {
    resource_state: ClientResourceState,
    resolver: crate::client_bootstrap::ClientBootstrapResolver,
    join_data: JoinDataEnvelope,
    initialized_game_resources: usize,
}

struct ClientPostJoinResourceConfig {
    local_candidates: crate::ClientBootstrapLocalCandidates,
    local_resource_roots: Vec<PathBuf>,
    local_system_path: Option<PathBuf>,
    trusted_local_system_path: Option<PathBuf>,
    resource_directory: Option<PathBuf>,
    group_maker: clonk_engine::LegacyCString,
    #[cfg(test)]
    client_name: Vec<u8>,
}

impl ClientPostJoinResourceBootstrap {
    fn resolve_before_addresses(
        mut resource_state: ClientResourceState,
        mut join_data: JoinDataEnvelope,
        mut config: ClientPostJoinResourceConfig,
    ) -> Result<Self, ClientError> {
        #[cfg(test)]
        wait_at_client_resource_bootstrap_probe_pause(&config.client_name);

        resource_state.next_control_request_at =
            tokio::time::Instant::now() + CONTROL_REQUEST_INTERVAL;
        config
            .local_candidates
            .extend_from_roots(&join_data, &config.local_resource_roots);
        if let Some(system_path) = config.local_system_path {
            for system in join_data
                .parameters
                .game_resources
                .iter()
                .filter(|core| core.resource_type == crate::HostResourceType::System as u8)
            {
                config
                    .local_candidates
                    .prioritize(system.id, system_path.clone());
            }
        }
        let standalone_directory = config
            .resource_directory
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("Network"));
        let mut resolver = crate::client_bootstrap::ClientBootstrapResolver::new_with_group_maker(
            &config.local_candidates,
            standalone_directory.to_path_buf(),
            config.group_maker,
        );
        if let Some(path) = config.trusted_local_system_path {
            resolver = resolver.with_trusted_local_system_path(path);
        }

        let mut initialized_game_resources = 0;
        for core in &join_data.parameters.game_resources {
            if resource_state
                .resolve_and_add_bootstrap_resource(
                    &resolver,
                    crate::ClientBootstrapResourceRole::GameResource,
                    core,
                )
                .is_err()
            {
                break;
            }
            initialized_game_resources += 1;
        }
        resource_state
            .resolve_and_add_bootstrap_resource(
                &resolver,
                crate::ClientBootstrapResourceRole::Dynamic,
                &join_data.dynamic,
            )
            .map_err(ClientError::Handshake)?;
        for player in join_data
            .parameters
            .player_infos
            .clients
            .iter_mut()
            .flat_map(|client| &mut client.players)
        {
            let flags = player.flags;
            if flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0
                || flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE == 0
            {
                continue;
            }
            if flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0 {
                crate::client_bootstrap::clear_player_resource(player);
                continue;
            }
            let Some(core) = player.resource.clone() else {
                crate::client_bootstrap::clear_player_resource(player);
                continue;
            };
            match resource_state.resolve_and_add_bootstrap_resource(
                &resolver,
                crate::ClientBootstrapResourceRole::Player,
                &core,
            ) {
                Ok(
                    ClientBootstrapRegistration::AlreadyPresent
                    | ClientBootstrapRegistration::Registered,
                ) => {}
                Ok(ClientBootstrapRegistration::UnavailableNonLoadable) | Err(_) => {
                    crate::client_bootstrap::clear_player_resource(player);
                }
            }
        }

        Ok(Self {
            resource_state,
            resolver,
            join_data,
            initialized_game_resources,
        })
    }

    fn resolve_after_addresses(
        mut self,
    ) -> Result<(ClientResourceState, JoinDataEnvelope), ClientError> {
        self.resource_state
            .resolve_and_add_bootstrap_resource(
                &self.resolver,
                crate::ClientBootstrapResourceRole::Scenario,
                &self.join_data.parameters.scenario,
            )
            .map_err(ClientError::Handshake)?;
        for core in self
            .join_data
            .parameters
            .game_resources
            .iter()
            .skip(self.initialized_game_resources)
        {
            self.resource_state
                .resolve_and_add_bootstrap_resource(
                    &self.resolver,
                    crate::ClientBootstrapResourceRole::GameResource,
                    core,
                )
                .map_err(ClientError::Handshake)?;
        }
        self.resource_state.retain_resource_resolver(self.resolver);
        Ok((self.resource_state, self.join_data))
    }
}

async fn run_client_resource_bootstrap<T, F>(operation: F) -> Result<T, ClientError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ClientError> + Send + 'static,
{
    // Local group probing recursively walks, opens, and hashes files. Keep it
    // off the async executor so the accepted route task remains equivalent to
    // C++'s dedicated C4InteractiveThread even on a current-thread runtime
    // (oracle-src-pinned src/C4Network2.cpp:1628-1638;
    // src/C4Network2IO.cpp:117-197).
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            ClientError::Handshake(format!("resource bootstrap worker failed: {error}"))
        })?
}

async fn complete_client_post_join_step<T>(
    routes: &mut ClientRouteManager,
    result: Result<T, ClientError>,
) -> Result<T, ClientAttemptError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            // HandleJoinData failures call Clear before returning, which
            // immediately removes the admitted C4Network2IO connection
            // (oracle-src-pinned src/C4Network2.cpp:1590-1639).
            routes.shutdown().await;
            Err(error.into())
        }
    }
}

pub(crate) fn selected_puncher_addresses(addresses: &[SocketAddr]) -> Vec<SocketAddr> {
    let mut have_ipv4 = false;
    let mut have_ipv6 = false;
    addresses
        .iter()
        .copied()
        .map(crate::canonical_reliable_udp_peer_address)
        .filter(|address| match address {
            SocketAddr::V4(_) if !have_ipv4 => {
                have_ipv4 = true;
                true
            }
            SocketAddr::V6(_) if !have_ipv6 => {
                have_ipv6 = true;
                true
            }
            _ => false,
        })
        .collect()
}

enum ClientDialStream {
    Tcp(TcpStream),
    Udp(crate::ReliableUdpPeerStream),
}

type ClientConnectFuture =
    Pin<Box<dyn Future<Output = Result<ClientDialStream, io::Error>> + Send + 'static>>;

pub(crate) struct ClientDialAttempt {
    pub(crate) index: usize,
    future: Option<ClientConnectFuture>,
    result: Option<Result<ClientDialStream, io::Error>>,
}

pub(crate) struct ClientDialRace {
    pub(crate) attempts: Vec<ClientDialAttempt>,
    deadline: tokio::time::Instant,
}

impl ClientDialRace {
    pub(crate) fn new(
        addresses: impl IntoIterator<Item = crate::NetworkAddress>,
        tcp_available: bool,
        udp_handle: Option<crate::ReliableUdpSessionHandle>,
    ) -> Self {
        let mut seen_addresses = HashSet::new();
        Self {
            attempts: addresses
                .into_iter()
                .enumerate()
                .filter(|(_, address)| seen_addresses.insert(*address))
                .filter_map(|(index, address)| {
                    if address.is_ip_null() {
                        return None;
                    }
                    let endpoint = address.endpoint;
                    let future: ClientConnectFuture = match address.protocol {
                        crate::NetworkProtocol::Tcp => {
                            if !tcp_available {
                                return None;
                            }
                            Box::pin(async move {
                                TcpStream::connect(endpoint)
                                    .await
                                    .map(ClientDialStream::Tcp)
                            })
                        }
                        crate::NetworkProtocol::Udp => {
                            let handle = udp_handle.clone()?;
                            Box::pin(async move {
                                handle.connect(endpoint).await.map(ClientDialStream::Udp)
                            })
                        }
                        crate::NetworkProtocol::Unknown(_) => return None,
                    };
                    Some(ClientDialAttempt {
                        index,
                        future: Some(future),
                        result: None,
                    })
                })
                .collect(),
            deadline: tokio::time::Instant::now() + HANDSHAKE_TIMEOUT,
        }
    }

    fn is_empty(&self) -> bool {
        self.attempts.is_empty()
    }

    async fn next(&mut self) -> Option<(usize, Result<ClientDialStream, io::Error>)> {
        if self.attempts.is_empty() {
            return None;
        }
        let ready = tokio::time::timeout_at(
            self.deadline,
            poll_fn(|context| {
                // Poll every fresh attempt before selecting a winner. This
                // starts the transports together while retaining stable input
                // order when multiple attempts are already ready.
                for attempt in &mut self.attempts {
                    let ready = attempt.future.as_mut().and_then(|future| {
                        match future.as_mut().poll(context) {
                            Poll::Ready(ready) => Some(ready),
                            Poll::Pending => None,
                        }
                    });
                    if let Some(ready) = ready {
                        attempt.result = Some(ready);
                        attempt.future = None;
                    }
                }
                self.attempts
                    .iter_mut()
                    .enumerate()
                    .find_map(|(position, attempt)| {
                        attempt.result.take().map(|result| (position, result))
                    })
                    .map_or(Poll::Pending, Poll::Ready)
            }),
        )
        .await;
        match ready {
            Ok((position, result)) => {
                let attempt = self.attempts.remove(position);
                Some((attempt.index, result))
            }
            Err(_) => {
                let index = self.attempts[0].index;
                self.attempts.clear();
                Some((
                    index,
                    Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "connection attempt timed out",
                    )),
                ))
            }
        }
    }
}

pub(crate) struct PreparedClientMesh {
    tcp_listener: Option<TcpListener>,
    pub(crate) udp_hub: Option<crate::ReliableUdpSessionHub>,
    pub(crate) puncher_events: Option<mpsc::Receiver<crate::NetpuncherIoEvent>>,
    pub(crate) puncher_init: Arc<Mutex<ClientPuncherInitState>>,
    io_statistics: crate::NetworkIoStatistics,
}

pub(crate) struct ClientPuncherInitState {
    pub(crate) initializing: bool,
    pub(crate) observations: Vec<SocketAddr>,
}

impl PreparedClientMesh {
    fn udp_handle(&self) -> Option<crate::ReliableUdpSessionHandle> {
        self.udp_hub
            .as_ref()
            .map(crate::ReliableUdpSessionHub::handle)
    }
}

pub(crate) async fn prepare_client_mesh(
    config: &ClientConfig,
    require_udp: bool,
) -> Result<PreparedClientMesh, ClientError> {
    let io_statistics = crate::NetworkIoStatistics::new(network_statistics_now_ms());
    let mut first_bind_error = None;
    let tcp_listener = match config.mesh_tcp_bind_address {
        Some(bind_address) => match bind_client_mesh_tcp_listener(bind_address).await {
            Ok(listener) => Some(listener),
            Err(error) => {
                first_bind_error = Some(io::Error::new(
                    error.kind(),
                    format!("failed to bind client mesh TCP at {bind_address}: {error}"),
                ));
                None
            }
        },
        None => None,
    };
    let udp_bind_address = config.mesh_udp_bind_address.or_else(|| {
        (require_udp || !config.mesh_punchers.is_empty())
            .then_some(SocketAddr::from(([0_u16; 8], 0)))
    });
    let mut udp_hub = match udp_bind_address {
        Some(bind_address) => match crate::ReliableUdpSessionHub::bind_with_statistics(
            bind_address,
            io_statistics.clone(),
        ) {
            Ok(hub) => Some(hub),
            Err(error) => {
                first_bind_error.get_or_insert_with(|| {
                    io::Error::new(
                        error.kind(),
                        format!("failed to bind client mesh UDP at {bind_address}: {error}"),
                    )
                });
                None
            }
        },
        None => None,
    };
    let requested_transport = config.mesh_tcp_bind_address.is_some() || udp_bind_address.is_some();
    if requested_transport && tcp_listener.is_none() && udp_hub.is_none() {
        return Err(ClientError::Connect(first_bind_error.unwrap_or_else(
            || {
                io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    "no configured client network transport could be bound",
                )
            },
        )));
    }
    let source_puncher_events = udp_hub
        .as_mut()
        .map(crate::ReliableUdpSessionHub::take_puncher_event_receiver);
    let udp_handle = udp_hub.as_ref().map(crate::ReliableUdpSessionHub::handle);
    if let Some(handle) = udp_handle.as_ref() {
        for puncher in &config.mesh_punchers {
            let _ = handle
                .init_puncher(puncher.address, crate::NetpuncherRole::Client)
                .await;
        }
    }
    let puncher_init = Arc::new(Mutex::new(ClientPuncherInitState {
        initializing: true,
        observations: Vec::new(),
    }));
    let puncher_events = match (source_puncher_events, udp_handle) {
        (Some(mut source), Some(handle)) => {
            let (forward_tx, forward_rx) = mpsc::channel(64);
            let game_ids = crate::NetpuncherGameIds {
                ipv4: config
                    .mesh_punchers
                    .iter()
                    .find(|puncher| puncher.address.is_ipv4())
                    .map_or(0, |puncher| puncher.game_id),
                ipv6: config
                    .mesh_punchers
                    .iter()
                    .find(|puncher| puncher.address.is_ipv6())
                    .map_or(0, |puncher| puncher.game_id),
            };
            let pump_init = puncher_init.clone();
            tokio::spawn(async move {
                while let Some(event) = source.recv().await {
                    match &event {
                        crate::NetpuncherIoEvent::Connected {
                            family,
                            puncher_address,
                            ..
                        } => {
                            let family = *family;
                            let puncher_address = *puncher_address;
                            let initializing = {
                                let mut init = pump_init
                                    .lock()
                                    .expect("client puncher initialization lock poisoned");
                                if init.initializing {
                                    if let crate::NetpuncherIoEvent::Connected {
                                        observed_address,
                                        ..
                                    } = &event
                                    {
                                        init.observations.push(*observed_address);
                                    }
                                }
                                init.initializing
                            };
                            if initializing {
                                if let Some(packet) = crate::reduce_puncher_connect(
                                    crate::NetpuncherRole::Client,
                                    crate::NetpuncherRuntimeState::Initializing,
                                    family,
                                    game_ids,
                                ) {
                                    let _ = handle.send_puncher_packet(family, packet).await;
                                }
                            }
                            if forward_tx.send(event).await.is_err() {
                                let _ = handle.close_puncher(puncher_address).await;
                                break;
                            }
                        }
                        crate::NetpuncherIoEvent::Packet {
                            puncher_address,
                            packet: crate::NetpuncherPacket::AssignId { .. },
                            ..
                        } => {
                            let _ = puncher_address;
                        }
                        crate::NetpuncherIoEvent::Packet {
                            puncher_address,
                            packet: crate::NetpuncherPacket::ClientRequest { .. },
                            ..
                        } if pump_init
                            .lock()
                            .expect("client puncher initialization lock poisoned")
                            .initializing =>
                        {
                            let _ = puncher_address;
                        }
                        crate::NetpuncherIoEvent::Packet {
                            puncher_address, ..
                        } => {
                            let _ = handle.close_puncher(*puncher_address).await;
                        }
                    }
                }
            });
            Some(forward_rx)
        }
        (Some(source), None) => Some(source),
        (None, _) => None,
    };
    Ok(PreparedClientMesh {
        tcp_listener,
        udp_hub,
        puncher_events,
        puncher_init,
        io_statistics,
    })
}

/// UDP socket prepared before a host publishes its league Start reference.
/// A failed optional bind is retained so the host can surface its transport
/// error while continuing with TCP.
pub struct HostUdpBinding {
    hub: Option<crate::ReliableUdpSessionHub>,
    start_error: Option<String>,
    io_statistics: crate::NetworkIoStatistics,
}

impl HostUdpBinding {
    pub fn bind(config: &HostConfig) -> Self {
        let io_statistics = crate::NetworkIoStatistics::new(network_statistics_now_ms());
        let udp_bind_address = (config.configured_udp_port != Some(0))
            .then(|| {
                config.udp_bind_address.or_else(|| {
                    (!config.netpuncher_addresses.is_empty())
                        .then_some(SocketAddr::from(([0_u16; 8], 0)))
                })
            })
            .flatten();
        match udp_bind_address {
            Some(bind_address) => match crate::ReliableUdpSessionHub::bind_with_statistics(
                bind_address,
                io_statistics.clone(),
            ) {
                Ok(hub) => Self {
                    hub: Some(hub),
                    start_error: None,
                    io_statistics,
                },
                Err(error) => Self {
                    hub: None,
                    start_error: Some(format!(
                        "failed to start reliable-UDP listener at {bind_address}: {error}"
                    )),
                    io_statistics,
                },
            },
            None => Self {
                hub: None,
                start_error: None,
                io_statistics,
            },
        }
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.hub
            .as_ref()
            .map(crate::ReliableUdpSessionHub::local_addr)
    }

    /// Retains the optional UDP bind diagnostic until another transport has
    /// been checked, so callers can fail only when both are unavailable.
    pub fn bind_error(&self) -> Option<&str> {
        self.start_error.as_deref()
    }
}

/// Starts the multiplayer host loop after binding its optional UDP socket.
pub async fn start_host_with_udp_binding(
    listener: TcpListener,
    config: HostConfig,
    udp_binding: HostUdpBinding,
) -> Result<HostHandle, HostError> {
    start_host_with_bindings(Some(listener), config, udp_binding).await
}

/// Starts the multiplayer host loop after independently preparing its TCP and
/// UDP transports. At least one binding must be live.
pub async fn start_host_with_bindings(
    listener: Option<TcpListener>,
    config: HostConfig,
    udp_binding: HostUdpBinding,
) -> Result<HostHandle, HostError> {
    start_host_with_udp_binding_and_backend(
        listener,
        config,
        udp_binding,
        &crate::upnp::RealPortMappingBackend,
    )
    .await
}

pub(crate) async fn start_host_with_udp_binding_and_backend(
    listener: Option<TcpListener>,
    config: HostConfig,
    udp_binding: HostUdpBinding,
    port_mapping_backend: &dyn crate::upnp::PortMappingBackend,
) -> Result<HostHandle, HostError> {
    if listener.is_none() && udp_binding.local_addr().is_none() {
        return Err(HostError::NoTransport);
    }
    let resource_backend = build_host_resource_backend(&config)?;
    let HostUdpBinding {
        hub: udp_hub,
        start_error: udp_start_error,
        io_statistics,
    } = udp_binding;
    let tcp_local_addr = listener
        .as_ref()
        .and_then(|listener| listener.local_addr().ok());
    let udp_local_addr = udp_hub
        .as_ref()
        .map(crate::ReliableUdpSessionHub::local_addr);
    let mapping_requests = host_port_mapping_requests(&config, tcp_local_addr, udp_local_addr);
    let active_port_mappings =
        (!mapping_requests.is_empty()).then(|| port_mapping_backend.start(&mapping_requests));
    let (command_tx, command_rx) = mpsc::channel::<HostCommand>(64);
    let control_send_time = ControlSendTimeSnapshot::default();
    let worker_control_send_time = control_send_time.clone();
    let (event_tx, event_rx) = mpsc::channel::<HostEvent>(64);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task_io_statistics = io_statistics.clone();
    let join_handle = tokio::spawn(async move {
        run_host(
            listener,
            udp_hub,
            udp_start_error,
            config,
            resource_backend,
            task_io_statistics,
            command_rx,
            worker_control_send_time,
            event_tx.clone(),
            shutdown_rx,
        )
        .await;
        if let Some(active_port_mappings) = active_port_mappings {
            active_port_mappings.shutdown().await;
        }
    });
    Ok(HostHandle {
        command_tx,
        control_send_time,
        event_rx: Some(event_rx),
        shutdown_tx: Some(shutdown_tx),
        join_handle,
        udp_local_addr,
        io_statistics,
    })
}

pub(crate) fn host_port_mapping_requests(
    config: &HostConfig,
    tcp_local_addr: Option<SocketAddr>,
    udp_local_addr: Option<SocketAddr>,
) -> Vec<crate::upnp::PortMappingRequest> {
    if !config.enable_upnp {
        return Vec::new();
    }

    let mut requests = Vec::with_capacity(2);
    let tcp_port = tcp_local_addr
        .map(|address| config.configured_tcp_port.unwrap_or(address.port()))
        .filter(|port| *port != 0);
    if let Some(internal_port) = tcp_port {
        requests.push(crate::upnp::PortMappingRequest {
            protocol: crate::upnp::PortMappingProtocol::Tcp,
            internal_port,
            external_port: 0,
        });
    }
    let udp_port = udp_local_addr
        .map(|address| config.configured_udp_port.unwrap_or(address.port()))
        .filter(|port| *port != 0);
    if let Some(internal_port) = udp_port {
        requests.push(crate::upnp::PortMappingRequest {
            protocol: crate::upnp::PortMappingProtocol::Udp,
            internal_port,
            external_port: 0,
        });
    }
    requests
}

/// Starts the multiplayer host loop.
pub async fn start_host(
    listener: TcpListener,
    config: HostConfig,
) -> Result<HostHandle, HostError> {
    let udp_binding = HostUdpBinding::bind(&config);
    start_host_with_udp_binding(listener, config, udp_binding).await
}

fn build_host_resource_backend(
    config: &HostConfig,
) -> Result<Option<crate::ResourceTransferBackend>, HostError> {
    let Some(directory) = config.resource_directory.as_ref() else {
        if config.resource_files.is_empty() {
            return Ok(None);
        }
        return Err(HostError::Resource(
            "host resource files require a network working directory".to_string(),
        ));
    };
    let mut backend = crate::ResourceTransferBackend::new(0, directory)
        .map_err(|error| HostError::Resource(error.to_string()))?;
    for resource in &config.resource_files {
        backend
            .register_hosted_resource(
                resource.core.clone(),
                &resource.path,
                resource.ownership,
                resource.binary_compatible,
            )
            .map_err(|error| HostError::Resource(error.to_string()))?;
    }
    Ok(Some(backend))
}

/// Connects to an existing host and returns a handle for interaction.
pub async fn connect_client(
    addr: SocketAddr,
    config: ClientConfig,
) -> Result<ClientHandle, ClientError> {
    connect_client_addresses(
        [crate::NetworkAddress::new(
            crate::NetworkProtocol::Tcp,
            addr,
        )],
        config,
    )
    .await
}

/// Connects one logical client over both session transports. Reliable UDP is
/// preferred for message traffic, while TCP is preferred for resource data;
/// either accepted route remains a fallback if the other one closes.
pub async fn connect_dual_client(
    tcp_addr: SocketAddr,
    udp_addr: SocketAddr,
    config: ClientConfig,
) -> Result<ClientHandle, ClientError> {
    connect_client_from_inner_with_udp(TcpStream::connect(tcp_addr), config, None, Some(udp_addr))
        .await
}

/// Connects to an existing host through C4NetIOUDP's reliable packet stream.
pub async fn connect_udp_client(
    addr: SocketAddr,
    config: ClientConfig,
) -> Result<ClientHandle, ClientError> {
    connect_client_addresses(
        [crate::NetworkAddress::new(
            crate::NetworkProtocol::Udp,
            addr,
        )],
        config,
    )
    .await
}

/// Connects through the first admitted route from an already prepared C++
/// join-attempt list.
///
/// Callers joining from a game reference prepare that list with
/// [`crate::NetworkGameReference::join_attempts`]. TCP and reliable-UDP
/// transports are established concurrently, while their admission handshakes
/// are serialized so only one route can create the logical client.
pub async fn connect_client_addresses(
    addresses: impl IntoIterator<Item = crate::NetworkAddress>,
    config: ClientConfig,
) -> Result<ClientHandle, ClientError> {
    let addresses = addresses.into_iter().collect::<Vec<_>>();
    let mut secondary_tcp_addr = addresses
        .iter()
        .find(|address| {
            matches!(address.protocol, crate::NetworkProtocol::Tcp) && !address.is_ip_null()
        })
        .map(|address| address.endpoint);
    let mut secondary_udp_addr = addresses
        .iter()
        .find(|address| {
            matches!(address.protocol, crate::NetworkProtocol::Udp) && !address.is_ip_null()
        })
        .map(|address| address.endpoint);
    let mut client_mesh = prepare_client_mesh(&config, secondary_udp_addr.is_some()).await?;
    let tcp_available =
        config.mesh_tcp_bind_address.is_none() || client_mesh.tcp_listener.is_some();
    if !tcp_available {
        secondary_tcp_addr = None;
    }
    let udp_handle = client_mesh.udp_handle();
    if udp_handle.is_none() {
        secondary_udp_addr = None;
    }
    let mut dials = ClientDialRace::new(addresses, tcp_available, udp_handle);
    if dials.is_empty() {
        return Err(ClientError::Connect(io::Error::new(
            io::ErrorKind::Unsupported,
            "join address list contains no supported TCP or reliable-UDP endpoint",
        )));
    }
    let mut failures = Vec::new();
    let mut wrong_password: Option<(usize, ClientError)> = None;
    while let Some((index, result)) = dials.next().await {
        let stream = match result {
            Ok(stream) => stream,
            Err(error) => {
                failures.push((index, ClientError::Connect(error)));
                continue;
            }
        };
        let admission = match stream {
            ClientDialStream::Tcp(stream) => {
                stream.set_nodelay(true).ok();
                let peer_addr = stream.peer_addr().ok();
                connect_client_stream_attempt(
                    stream,
                    peer_addr,
                    crate::NetworkProtocol::Tcp,
                    config.clone(),
                    None,
                    secondary_udp_addr,
                    None,
                    &mut client_mesh,
                )
                .await
            }
            ClientDialStream::Udp(stream) => {
                let peer_addr = stream.peer_addr();
                match stream.bind_statistics_connection(0).await {
                    Ok(()) => {
                        connect_client_stream_attempt(
                            stream,
                            Some(peer_addr),
                            crate::NetworkProtocol::Udp,
                            config.clone(),
                            None,
                            None,
                            secondary_tcp_addr,
                            &mut client_mesh,
                        )
                        .await
                    }
                    Err(error) => Err(ClientAttemptError::Retryable(ClientError::Connect(error))),
                }
            }
        };
        match admission {
            Ok(client) => return Ok(client),
            Err(ClientAttemptError::Retryable(error)) => failures.push((index, error)),
            Err(ClientAttemptError::WrongPassword(error)) => {
                if wrong_password
                    .as_ref()
                    .is_none_or(|(previous_index, _)| index < *previous_index)
                {
                    wrong_password = Some((index, error));
                }
            }
            Err(ClientAttemptError::Terminal(error)) => return Err(error),
        }
    }
    if let Some((_, error)) = wrong_password {
        return Err(error);
    }
    failures.sort_by_key(|(index, _)| *index);
    Err(failures
        .into_iter()
        .next()
        .expect("a completed dial race retains its failure")
        .1)
}

#[cfg(test)]
pub(crate) async fn connect_client_from<F>(
    connection: F,
    config: ClientConfig,
) -> Result<ClientHandle, ClientError>
where
    F: Future<Output = Result<TcpStream, io::Error>>,
{
    connect_client_from_inner(connection, config, None).await
}

#[cfg(test)]
pub(crate) async fn connect_client_from_inner<F>(
    connection: F,
    config: ClientConfig,
    liveness: Option<ConnectionLivenessState>,
) -> Result<ClientHandle, ClientError>
where
    F: Future<Output = Result<TcpStream, io::Error>>,
{
    connect_client_from_inner_with_udp(connection, config, liveness, None).await
}

async fn connect_client_from_inner_with_udp<F>(
    connection: F,
    config: ClientConfig,
    liveness: Option<ConnectionLivenessState>,
    secondary_udp_addr: Option<SocketAddr>,
) -> Result<ClientHandle, ClientError>
where
    F: Future<Output = Result<TcpStream, io::Error>>,
{
    let mut client_mesh = prepare_client_mesh(&config, secondary_udp_addr.is_some()).await?;
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connection)
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream.set_nodelay(true).ok();
    let host_peer_addr = stream.peer_addr().ok();

    connect_client_stream_attempt(
        stream,
        host_peer_addr,
        crate::NetworkProtocol::Tcp,
        config,
        liveness,
        secondary_udp_addr,
        None,
        &mut client_mesh,
    )
    .await
    .map_err(ClientAttemptError::into_error)
}

// The attempt combines independently optional secondary routes with the
// primary transport and prepared mesh state. Keeping those inputs explicit
// makes fallback behavior visible at each call site.
#[allow(clippy::too_many_arguments)]
async fn connect_client_stream_attempt<S>(
    stream: S,
    host_peer_addr: Option<SocketAddr>,
    host_protocol: crate::NetworkProtocol,
    config: ClientConfig,
    liveness: Option<ConnectionLivenessState>,
    secondary_udp_addr: Option<SocketAddr>,
    secondary_tcp_addr: Option<SocketAddr>,
    client_mesh: &mut PreparedClientMesh,
) -> Result<ClientHandle, ClientAttemptError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let io_statistics = client_mesh.io_statistics.clone();
    let ClientConfig {
        name,
        group_maker,
        kind,
        compatibility_build,
        password,
        resource_directory,
        bootstrap_local_candidates,
        local_system_path,
        trusted_local_system_path,
        local_resource_roots,
        mesh_tcp_bind_address: _,
        mesh_udp_bind_address: _,
        mesh_punchers: _,
    } = config;
    let wire_name =
        clonk_engine::LegacyCString::from_bytes(name.into_bytes()).ok_or_else(|| {
            ClientError::Handshake("client name contains an interior NUL".to_string())
        })?;
    let local_core = clonk_engine::ClientCoreControlData {
        client_id: -1,
        activated: matches!(kind, ParticipantKind::Player),
        observer: matches!(kind, ParticipantKind::Observer),
        name: wire_name.clone(),
        nick: wire_name,
        lobby_ready: false,
    };
    let primary_connection_id = 0;
    let request = crate::ConnectionRequest {
        core: local_core.clone(),
        build: compatibility_build,
        password: password.clone(),
        connection_id: primary_connection_id,
    };
    let mut transport = crate::ControlTransport::new(stream);
    if matches!(host_protocol, crate::NetworkProtocol::Tcp) {
        transport.set_statistics(
            io_statistics.open_connection(primary_connection_id, crate::NetworkProtocol::Tcp),
        );
    }
    let bootstrap = match liveness {
        Some(liveness) => {
            crate::connection_handshake::run_client_connection_handshake_with_liveness(
                &mut transport,
                request,
                liveness,
            )
            .await
        }
        None => run_client_connection_handshake(&mut transport, request).await,
    }
    .map_err(|error| match error {
        crate::ConnectionHandshakeError::PeerRejection {
            message,
            wrong_password: true,
        } => ClientAttemptError::WrongPassword(ClientError::WrongPassword { message }),
        error => {
            let retryable = matches!(
                &error,
                crate::ConnectionHandshakeError::Transport(_)
                    | crate::ConnectionHandshakeError::AdmissionTimeout
                    | crate::ConnectionHandshakeError::PingTimeout
            );
            let error = ClientError::Handshake(error.to_string());
            if retryable {
                ClientAttemptError::Retryable(error)
            } else {
                ClientAttemptError::Terminal(error)
            }
        }
    })?;
    let primary_local_connection_id = bootstrap.local_connection_id;
    let primary_remote_connection_id = bootstrap.remote_connection_id;
    debug_assert_eq!(primary_local_connection_id, primary_connection_id);
    let primary_liveness = bootstrap.liveness.clone();
    let host_core = bootstrap.peer_core.clone();
    let mut join_data = bootstrap.join_data;
    if join_data.client_id < 0 {
        return Err(ClientError::Handshake(
            "host did not assign a client id in JoinData".to_string(),
        )
        .into());
    }
    if !matches!(
        join_data.status.state,
        NETWORK_STATE_LOBBY | NETWORK_STATE_PAUSE | NETWORK_STATE_GO
    ) {
        return Err(ClientError::Handshake(format!(
            "host sent invalid JoinData status {}",
            join_data.status.state
        ))
        .into());
    }
    client_mesh
        .puncher_init
        .lock()
        .expect("client puncher initialization lock poisoned")
        .initializing = false;
    let existing_clients = JoinClientRegistrySnapshot {
        clients: vec![local_core],
        local_client_id: Some(-1),
    };
    join_data.parameters.clients = reconcile_join_client_registry(
        &existing_clients,
        join_data.parameters.clients.clone(),
        join_data.client_id,
    )
    .ok_or_else(|| {
        ClientError::Handshake("assigned local client is missing from JoinData".to_string())
    })?;
    let assigned_local_core = join_data
        .parameters
        .clients
        .clients
        .iter()
        .find(|core| core.client_id == join_data.client_id)
        .cloned()
        .ok_or_else(|| {
            ClientError::Handshake(
                "assigned local client core disappeared from JoinData".to_string(),
            )
        })?;
    let start_control_tick = Tick::try_from(join_data.start_control_tick).map_err(|_| {
        ClientError::Handshake(format!(
            "host sent negative JoinData control tick {}",
            join_data.start_control_tick
        ))
    })?;
    let mut resource_state = ClientResourceState::new(
        &join_data,
        bootstrap.peer_core.client_id,
        bootstrap.pending_resources,
        bootstrap.pending_controls,
        bootstrap.liveness,
        resource_directory.clone(),
    )
    .map_err(ClientError::Handshake)?;
    resource_state.initial_ready_checks = bootstrap.pending_ready_checks;
    resource_state.initial_lobby_countdowns = bootstrap.pending_lobby_countdowns;
    send_client_control_request(&mut transport, start_control_tick)
        .await
        .map_err(|error| {
            ClientError::Handshake(format!(
                "failed to initialize control after JoinData: {error}"
            ))
        })?;
    // JoinData is delivered to C++'s main thread, but the already accepted
    // transport remains registered with C4InteractiveThread throughout local
    // resource probing. Hand the route to its independent reader/writer and
    // liveness task before any synchronous bootstrap work
    // (oracle-src-pinned src/C4Network2.cpp:1590-1639;
    // src/C4Network2IO.cpp:117-197; src/C4Packet2.cpp:51-73).
    let mut routes = ClientRouteManager::new();
    routes.add_route(
        primary_local_connection_id,
        primary_remote_connection_id,
        host_protocol,
        host_peer_addr,
        transport,
        primary_liveness,
    );
    #[cfg(test)]
    wait_at_client_post_join_bootstrap_pause(assigned_local_core.name.as_bytes()).await;
    let resource_config = ClientPostJoinResourceConfig {
        local_candidates: bootstrap_local_candidates,
        local_resource_roots,
        local_system_path,
        trusted_local_system_path,
        resource_directory,
        group_maker,
        #[cfg(test)]
        client_name: assigned_local_core.name.as_bytes().to_vec(),
    };
    let resource_bootstrap = run_client_resource_bootstrap(move || {
        ClientPostJoinResourceBootstrap::resolve_before_addresses(
            resource_state,
            join_data,
            resource_config,
        )
    })
    .await;
    let ClientPostJoinResourceBootstrap {
        resource_state,
        resolver: bootstrap_resolver,
        join_data,
        initialized_game_resources,
    } = complete_client_post_join_step(&mut routes, resource_bootstrap).await?;
    let client_id = join_data.client_id as ClientId;
    let mesh_tcp_listener = client_mesh.tcp_listener.take();
    let mesh_udp_hub = client_mesh.udp_hub.take();
    let mesh_tcp_local_addr = mesh_tcp_listener
        .as_ref()
        .and_then(|listener| listener.local_addr().ok());
    let mesh_udp_local_addr = mesh_udp_hub
        .as_ref()
        .map(crate::ReliableUdpSessionHub::local_addr);
    let mesh_puncher_events = client_mesh.puncher_events.take();
    let mut client_addresses = join_data
        .parameters
        .clients
        .clients
        .iter()
        .map(|core| (core.client_id, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for packet in &bootstrap.pending_addresses {
        if !client_addresses.contains_key(&packet.client_id) {
            continue;
        }
        let packet = host_peer_addr
            .map(|peer| packet.announcement_for_peer(peer))
            .unwrap_or(*packet);
        crate::append_received_address(
            client_addresses.entry(packet.client_id).or_default(),
            packet.address,
        );
    }
    let mesh_interface_endpoints = crate::client_mesh::client_mesh_os_interface_endpoints();
    let mesh_interface_ids = mesh_interface_endpoints
        .iter()
        .filter_map(|endpoint| match endpoint {
            SocketAddr::V6(endpoint)
                if endpoint.ip().is_unicast_link_local() && endpoint.scope_id() != 0 =>
            {
                Some(endpoint.scope_id())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    for address in crate::client_mesh::client_mesh_local_addresses(
        mesh_tcp_local_addr,
        mesh_udp_local_addr,
        mesh_interface_endpoints,
    ) {
        crate::append_received_address(
            client_addresses.entry(join_data.client_id).or_default(),
            address,
        );
    }
    if let Some(host_peer_addr) = host_peer_addr {
        let host_address = crate::NetworkAddress::new(host_protocol, host_peer_addr);
        crate::append_received_address(
            client_addresses
                .entry(bootstrap.peer_core.client_id)
                .or_default(),
            host_address,
        );
    }
    let client_cores = join_data
        .parameters
        .clients
        .clients
        .iter()
        .cloned()
        .map(|core| (core.client_id, core))
        .collect::<BTreeMap<_, _>>();
    let mut mesh_peers = client_addresses
        .keys()
        .copied()
        .map(|peer_id| (peer_id, crate::ClientMeshPeerState::new()))
        .collect::<BTreeMap<_, _>>();
    for (peer_id, addresses) in &client_addresses {
        let peer = mesh_peers
            .get_mut(peer_id)
            .expect("address owner has mesh state");
        for address in addresses {
            peer.add_address(*address, Duration::ZERO);
        }
    }
    let initial_puncher_observations = {
        let mut observations = client_mesh
            .puncher_init
            .lock()
            .expect("client puncher initialization lock poisoned");
        std::mem::take(&mut observations.observations)
    };
    for observed_address in initial_puncher_observations {
        let update = mesh_peers
            .entry(join_data.client_id)
            .or_default()
            .add_address_from_puncher(
                observed_address,
                mesh_udp_local_addr.map_or(0, |address| address.port()),
                mesh_tcp_local_addr.map_or(0, |address| address.port()),
                Duration::ZERO,
            );
        for address in update.announcements {
            let addresses = client_addresses.entry(join_data.client_id).or_default();
            if !addresses.contains(&address) {
                addresses.insert(0, address);
            }
        }
    }
    let address_announcements = client_addresses
        .iter()
        .flat_map(|(client_id, addresses)| {
            addresses.iter().filter_map(move |address| {
                let address = host_peer_addr
                    .and_then(|peer| address_for_peer(*address, peer))
                    .or((host_peer_addr.is_none()).then_some(*address))?;
                Some(crate::AddressPacket {
                    client_id: *client_id,
                    address,
                })
            })
        })
        .collect();
    let announcement = send_client_route_address_announcements(&mut routes, address_announcements)
        .await
        .map_err(|error| {
            ClientError::Handshake(format!(
                "failed to announce addresses after JoinData: {error}"
            ))
        });
    complete_client_post_join_step(&mut routes, announcement).await?;
    let resource_bootstrap = run_client_resource_bootstrap(move || {
        ClientPostJoinResourceBootstrap {
            resource_state,
            resolver: bootstrap_resolver,
            join_data,
            initialized_game_resources,
        }
        .resolve_after_addresses()
    })
    .await;
    let (resource_state, join_data) =
        complete_client_post_join_step(&mut routes, resource_bootstrap).await?;

    let udp_reconnect_addr = secondary_udp_addr.or_else(|| {
        matches!(host_protocol, crate::NetworkProtocol::Udp)
            .then_some(host_peer_addr)
            .flatten()
    });
    let tcp_reconnect_addr = secondary_tcp_addr.or_else(|| {
        matches!(host_protocol, crate::NetworkProtocol::Tcp)
            .then_some(host_peer_addr)
            .flatten()
    });
    let connection_ids = Arc::new(AtomicU32::new(primary_local_connection_id.wrapping_add(1)));
    let mesh_udp_handle = mesh_udp_hub
        .as_ref()
        .map(crate::ReliableUdpSessionHub::handle);
    let host_request_template = ClientHandshakeRequestTemplate::new(
        assigned_local_core.clone(),
        compatibility_build,
        password,
    );
    let mut udp_reconnect = udp_reconnect_addr
        .zip(mesh_udp_handle)
        .map(|(addr, handle)| ClientUdpReconnect {
            addr,
            handle,
            request_template: host_request_template.clone(),
            expected_host_core: host_core.clone(),
            connection_ids: connection_ids.clone(),
        });
    let mut tcp_reconnect = tcp_reconnect_addr.map(|addr| ClientTcpReconnect {
        addr,
        request_template: host_request_template,
        expected_host_core: host_core.clone(),
        connection_ids: connection_ids.clone(),
        io_statistics: io_statistics.clone(),
    });
    let pending_secondary = if secondary_udp_addr.is_some() {
        udp_reconnect.as_mut().map(ClientUdpReconnect::start)
    } else {
        None
    };
    let pending_tcp = if secondary_tcp_addr.is_some() {
        tcp_reconnect.as_mut().map(ClientTcpReconnect::start)
    } else {
        None
    };

    let (command_tx, command_rx) = mpsc::channel::<ClientCommand>(64);
    let control_send_time = ControlSendTimeSnapshot::default();
    let worker_control_send_time = control_send_time.clone();
    let (event_tx, event_rx) = mpsc::channel::<ClientEvent>(64);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join_handle = tokio::spawn(run_client_loop_with_routes(
        routes,
        io_statistics.clone(),
        command_rx,
        worker_control_send_time,
        event_tx,
        shutdown_rx,
        host_peer_addr,
        client_addresses,
        client_cores,
        mesh_peers,
        ClientHandshakeRequestTemplate::new(
            assigned_local_core,
            compatibility_build,
            clonk_engine::LegacyCString::default(),
        ),
        connection_ids,
        mesh_interface_ids,
        mesh_tcp_listener,
        mesh_udp_hub,
        mesh_puncher_events,
        mesh_tcp_local_addr.map_or(0, |address| address.port()),
        mesh_udp_local_addr.map_or(0, |address| address.port()),
        resource_state,
        udp_reconnect,
        pending_secondary,
        tcp_reconnect,
        pending_tcp,
    ));
    Ok(ClientHandle {
        command_tx,
        control_send_time,
        event_rx: Some(event_rx),
        shutdown_tx: Some(shutdown_tx),
        join_handle,
        client_id,
        join_data: Some(join_data),
        io_statistics,
    })
}

pub(crate) struct ConnectedClientRoute<S> {
    pub(crate) local_connection_id: u32,
    pub(crate) remote_connection_id: u32,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) transport: crate::ControlTransport<S>,
    pub(crate) liveness: ConnectionLivenessState,
}

pub(crate) type PendingClientRoute = tokio::task::JoinHandle<
    Result<ConnectedClientRoute<crate::ReliableUdpPeerStream>, ClientError>,
>;
pub(crate) type PendingTcpClientRoute =
    tokio::task::JoinHandle<Result<ConnectedClientRoute<TcpStream>, ClientError>>;

#[derive(Clone)]
pub(crate) struct ClientHandshakeRequestTemplate {
    pub(crate) local_core: clonk_engine::ClientCoreControlData,
    compatibility_build: i32,
    password: clonk_engine::LegacyCString,
}

impl ClientHandshakeRequestTemplate {
    pub(crate) fn new(
        local_core: clonk_engine::ClientCoreControlData,
        compatibility_build: i32,
        password: clonk_engine::LegacyCString,
    ) -> Self {
        Self {
            local_core,
            compatibility_build,
            password,
        }
    }

    fn connection_request(&self, connection_id: u32) -> crate::ConnectionRequest {
        crate::ConnectionRequest {
            core: self.local_core.clone(),
            build: self.compatibility_build,
            password: self.password.clone(),
            connection_id,
        }
    }
}

pub(crate) enum ConnectedMeshRoute {
    Tcp {
        peer_id: ClientId,
        initiator_id: ClientId,
        peer_core: clonk_engine::ClientCoreControlData,
        route: ConnectedClientRoute<TcpStream>,
    },
    Udp {
        peer_id: ClientId,
        initiator_id: ClientId,
        peer_core: clonk_engine::ClientCoreControlData,
        route: ConnectedClientRoute<crate::ReliableUdpPeerStream>,
    },
}

pub(crate) type MeshDialKey = (i32, u8, SocketAddr);

pub(crate) struct MeshRouteCompletion {
    pub(crate) dial_key: Option<MeshDialKey>,
    pub(crate) result: Result<ConnectedMeshRoute, ClientError>,
}

pub(crate) async fn connect_mesh_tcp_route(
    peer_id: ClientId,
    addr: SocketAddr,
    request_template: ClientHandshakeRequestTemplate,
    expected_peer_core: clonk_engine::ClientCoreControlData,
    connection_id: u32,
    io_statistics: crate::NetworkIoStatistics,
) -> Result<ConnectedMeshRoute, ClientError> {
    let initiator_id =
        ClientId::try_from(request_template.local_core.client_id).unwrap_or(ClientId::MAX);
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "mesh TCP connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream.set_nodelay(true).ok();
    let peer_addr = stream.peer_addr().map_err(ClientError::Connect)?;
    let mut transport = crate::ControlTransport::with_statistics(
        stream,
        io_statistics.open_connection(connection_id, crate::NetworkProtocol::Tcp),
    );
    let handshake = crate::connection_handshake::run_known_peer_connection_handshake(
        &mut transport,
        request_template.connection_request(connection_id),
        &expected_peer_core,
    )
    .await
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    Ok(ConnectedMeshRoute::Tcp {
        peer_id,
        initiator_id,
        peer_core: handshake.peer_core,
        route: ConnectedClientRoute {
            local_connection_id: handshake.local_connection_id,
            remote_connection_id: handshake.remote_connection_id,
            peer_addr,
            transport,
            liveness: handshake.liveness,
        },
    })
}

pub(crate) async fn connect_mesh_udp_route(
    handle: crate::ReliableUdpSessionHandle,
    peer_id: ClientId,
    addr: SocketAddr,
    request_template: ClientHandshakeRequestTemplate,
    expected_peer_core: clonk_engine::ClientCoreControlData,
    connection_id: u32,
) -> Result<ConnectedMeshRoute, ClientError> {
    let initiator_id =
        ClientId::try_from(request_template.local_core.client_id).unwrap_or(ClientId::MAX);
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, handle.connect(addr))
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "mesh UDP connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream
        .bind_statistics_connection(connection_id)
        .await
        .map_err(ClientError::Connect)?;
    let peer_addr = stream.peer_addr();
    let mut transport = crate::ControlTransport::new(stream);
    let handshake = crate::connection_handshake::run_known_peer_connection_handshake(
        &mut transport,
        request_template.connection_request(connection_id),
        &expected_peer_core,
    )
    .await
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    Ok(ConnectedMeshRoute::Udp {
        peer_id,
        initiator_id,
        peer_core: handshake.peer_core,
        route: ConnectedClientRoute {
            local_connection_id: handshake.local_connection_id,
            remote_connection_id: handshake.remote_connection_id,
            peer_addr,
            transport,
            liveness: handshake.liveness,
        },
    })
}

// Classic mesh route identity contains two peer IDs and two connection IDs in
// addition to its transport inputs; spelling them out avoids ambiguous tuples.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn connect_mesh_tcp_socket_route(
    peer_id: ClientId,
    initiator_id: ClientId,
    socket: tokio::net::TcpSocket,
    addr: SocketAddr,
    request_template: ClientHandshakeRequestTemplate,
    expected_peer_core: clonk_engine::ClientCoreControlData,
    connection_id: u32,
    delay: Duration,
    io_statistics: crate::NetworkIoStatistics,
) -> Result<ConnectedMeshRoute, ClientError> {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, socket.connect(addr))
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "simultaneous-open TCP connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream.set_nodelay(true).ok();
    let peer_addr = stream.peer_addr().map_err(ClientError::Connect)?;
    let mut transport = crate::ControlTransport::with_statistics(
        stream,
        io_statistics.open_connection(connection_id, crate::NetworkProtocol::Tcp),
    );
    let handshake = crate::connection_handshake::run_known_peer_connection_handshake(
        &mut transport,
        request_template.connection_request(connection_id),
        &expected_peer_core,
    )
    .await
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    Ok(ConnectedMeshRoute::Tcp {
        peer_id,
        initiator_id,
        peer_core: handshake.peer_core,
        route: ConnectedClientRoute {
            local_connection_id: handshake.local_connection_id,
            remote_connection_id: handshake.remote_connection_id,
            peer_addr,
            transport,
            liveness: handshake.liveness,
        },
    })
}

pub(crate) fn bind_tcp_sim_open_socket(
    observed_address: SocketAddr,
) -> io::Result<(tokio::net::TcpSocket, SocketAddr)> {
    let SocketAddr::V6(mut bind_address) = observed_address else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "TCP simultaneous open requires an IPv6 puncher address",
        ));
    };
    bind_address.set_port(0);
    let socket = tokio::net::TcpSocket::new_v6()?;
    socket.set_reuseaddr(true)?;
    socket.bind(SocketAddr::V6(bind_address))?;
    let bound_address = socket.local_addr()?;
    Ok((socket, bound_address))
}

pub(crate) async fn accept_mesh_tcp_route(
    stream: TcpStream,
    peer_addr: SocketAddr,
    request_template: ClientHandshakeRequestTemplate,
    canonical_peer_cores: BTreeMap<i32, clonk_engine::ClientCoreControlData>,
    connection_id: u32,
    io_statistics: crate::NetworkIoStatistics,
) -> Result<ConnectedMeshRoute, ClientError> {
    stream.set_nodelay(true).ok();
    let mut transport = crate::ControlTransport::with_statistics(
        stream,
        io_statistics.open_connection(connection_id, crate::NetworkProtocol::Tcp),
    );
    let handshake = crate::connection_handshake::run_registered_peer_connection_handshake(
        &mut transport,
        request_template.connection_request(connection_id),
        &canonical_peer_cores,
    )
    .await
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    let peer_id = ClientId::try_from(handshake.peer_core.client_id).map_err(|_| {
        ClientError::Handshake("mesh peer has a negative canonical client ID".to_string())
    })?;
    Ok(ConnectedMeshRoute::Tcp {
        peer_id,
        initiator_id: peer_id,
        peer_core: handshake.peer_core,
        route: ConnectedClientRoute {
            local_connection_id: handshake.local_connection_id,
            remote_connection_id: handshake.remote_connection_id,
            peer_addr,
            transport,
            liveness: handshake.liveness,
        },
    })
}

pub(crate) async fn accept_mesh_udp_route(
    stream: crate::ReliableUdpPeerStream,
    request_template: ClientHandshakeRequestTemplate,
    canonical_peer_cores: BTreeMap<i32, clonk_engine::ClientCoreControlData>,
    connection_id: u32,
) -> Result<ConnectedMeshRoute, ClientError> {
    stream
        .bind_statistics_connection(connection_id)
        .await
        .map_err(ClientError::Connect)?;
    let peer_addr = stream.peer_addr();
    let mut transport = crate::ControlTransport::new(stream);
    let handshake = crate::connection_handshake::run_registered_peer_connection_handshake(
        &mut transport,
        request_template.connection_request(connection_id),
        &canonical_peer_cores,
    )
    .await
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    let peer_id = ClientId::try_from(handshake.peer_core.client_id).map_err(|_| {
        ClientError::Handshake("mesh peer has a negative canonical client ID".to_string())
    })?;
    Ok(ConnectedMeshRoute::Udp {
        peer_id,
        initiator_id: peer_id,
        peer_core: handshake.peer_core,
        route: ConnectedClientRoute {
            local_connection_id: handshake.local_connection_id,
            remote_connection_id: handshake.remote_connection_id,
            peer_addr,
            transport,
            liveness: handshake.liveness,
        },
    })
}

pub(crate) async fn accept_optional_mesh_tcp(
    listener: &mut Option<TcpListener>,
) -> Option<io::Result<(TcpStream, SocketAddr)>> {
    match listener {
        Some(listener) => Some(listener.accept().await),
        None => std::future::pending().await,
    }
}

pub(crate) async fn accept_optional_mesh_udp(
    hub: &mut Option<crate::ReliableUdpSessionHub>,
) -> Option<io::Result<crate::ReliableUdpPeerStream>> {
    match hub {
        Some(hub) => Some(hub.accept().await),
        None => std::future::pending().await,
    }
}

pub(crate) async fn receive_optional_puncher_event(
    events: &mut Option<mpsc::Receiver<crate::NetpuncherIoEvent>>,
) -> Option<crate::NetpuncherIoEvent> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

// Mesh dialing draws from separate peer, interface, handshake, UDP, and
// statistics state stores; this function is their task-spawn ownership seam.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_mesh_dial(
    pending: &mut tokio::task::JoinSet<MeshRouteCompletion>,
    active_dials: &mut BTreeSet<MeshDialKey>,
    peer_id: i32,
    attempt: crate::ClientMeshDialAttempt,
    client_cores: &BTreeMap<i32, clonk_engine::ClientCoreControlData>,
    request_template: &ClientHandshakeRequestTemplate,
    connection_ids: &Arc<AtomicU32>,
    interface_ids: &[u32],
    udp_handle: Option<&crate::ReliableUdpSessionHandle>,
    io_statistics: &crate::NetworkIoStatistics,
) {
    let Ok(peer_id_wire) = ClientId::try_from(peer_id) else {
        return;
    };
    let Some(peer_core) = client_cores.get(&peer_id).cloned() else {
        return;
    };
    let mut endpoints = vec![attempt.address.endpoint];
    if let SocketAddr::V6(endpoint) = attempt.address.endpoint {
        if endpoint.ip().is_unicast_link_local() {
            endpoints = interface_ids
                .iter()
                .map(|interface_id| {
                    let mut endpoint = endpoint;
                    endpoint.set_scope_id(*interface_id);
                    SocketAddr::V6(endpoint)
                })
                .collect();
        }
    }
    match attempt.address.protocol {
        crate::NetworkProtocol::Tcp => {
            if endpoints.is_empty() {
                return;
            }
            let dial_key = (
                peer_id,
                attempt.address.protocol.to_wire(),
                attempt.address.endpoint,
            );
            if !active_dials.insert(dial_key) {
                return;
            }
            let request_template = request_template.clone();
            let connection_ids = connection_ids.clone();
            let io_statistics = io_statistics.clone();
            pending.spawn(async move {
                let mut last_error = None;
                for endpoint in endpoints {
                    let connection_id = connection_ids.fetch_add(1, AtomicOrdering::Relaxed);
                    match connect_mesh_tcp_route(
                        peer_id_wire,
                        endpoint,
                        request_template.clone(),
                        peer_core.clone(),
                        connection_id,
                        io_statistics.clone(),
                    )
                    .await
                    {
                        Ok(route) => {
                            return MeshRouteCompletion {
                                dial_key: Some(dial_key),
                                result: Ok(route),
                            };
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                MeshRouteCompletion {
                    dial_key: Some(dial_key),
                    result: Err(last_error.expect("nonempty mesh endpoint list failed")),
                }
            });
        }
        crate::NetworkProtocol::Udp => {
            let Some(handle) = udp_handle.cloned() else {
                return;
            };
            if endpoints.is_empty() {
                return;
            }
            let dial_key = (
                peer_id,
                attempt.address.protocol.to_wire(),
                attempt.address.endpoint,
            );
            if !active_dials.insert(dial_key) {
                return;
            }
            let request_template = request_template.clone();
            let connection_ids = connection_ids.clone();
            pending.spawn(async move {
                let mut last_error = None;
                for endpoint in endpoints {
                    let connection_id = connection_ids.fetch_add(1, AtomicOrdering::Relaxed);
                    match connect_mesh_udp_route(
                        handle.clone(),
                        peer_id_wire,
                        endpoint,
                        request_template.clone(),
                        peer_core.clone(),
                        connection_id,
                    )
                    .await
                    {
                        Ok(route) => {
                            return MeshRouteCompletion {
                                dial_key: Some(dial_key),
                                result: Ok(route),
                            };
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                MeshRouteCompletion {
                    dial_key: Some(dial_key),
                    result: Err(last_error.expect("nonempty mesh endpoint list failed")),
                }
            });
        }
        crate::NetworkProtocol::Unknown(_) => {}
    }
}

pub(crate) fn maybe_initiate_tcp_simultaneous_open(
    pending_sockets: &mut BTreeMap<i32, tokio::net::TcpSocket>,
    pending_routes: usize,
    routes: &mut ClientRouteManager,
    local_core: &clonk_engine::ClientCoreControlData,
    peer_id: i32,
    attempt: crate::ClientMeshDialAttempt,
    local_puncher_address: Option<SocketAddr>,
) {
    if pending_routes + pending_sockets.len() >= CLIENT_MESH_PENDING_LIMIT {
        return;
    }
    if !crate::client_mesh_tcp_sim_open_eligible(
        local_core.client_id,
        peer_id,
        attempt.address,
        pending_sockets.contains_key(&peer_id),
    ) {
        return;
    }
    let Some(local_puncher_address) = local_puncher_address else {
        return;
    };
    let Ok((socket, bound_address)) = bind_tcp_sim_open_socket(local_puncher_address) else {
        return;
    };
    pending_sockets.insert(peer_id, socket);
    let Ok(peer_id_wire) = ClientId::try_from(peer_id) else {
        return;
    };
    let packet = crate::TcpSimOpenPacket {
        client_id: local_core.client_id,
        address: crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, bound_address),
    };
    // Native retains the bound socket if the request cannot be sent, blocking
    // repeated initiations until the peer object is destroyed.
    let _ = routes.try_send_to(peer_id_wire, ControlMessage::TcpSimOpen(packet));
}

pub(crate) fn add_connected_mesh_route(
    route: ConnectedMeshRoute,
    routes: &mut ClientRouteManager,
) -> Option<ClientId> {
    match route {
        ConnectedMeshRoute::Tcp {
            peer_id,
            initiator_id,
            peer_core: _,
            route,
        } => {
            let first_peer_route = routes.add_peer_route(
                peer_id,
                initiator_id,
                route.local_connection_id,
                route.remote_connection_id,
                crate::NetworkProtocol::Tcp,
                Some(route.peer_addr),
                route.transport,
                route.liveness,
            );
            first_peer_route.then_some(peer_id)
        }
        ConnectedMeshRoute::Udp {
            peer_id,
            initiator_id,
            peer_core: _,
            route,
        } => {
            let first_peer_route = routes.add_peer_route(
                peer_id,
                initiator_id,
                route.local_connection_id,
                route.remote_connection_id,
                crate::NetworkProtocol::Udp,
                Some(route.peer_addr),
                route.transport,
                route.liveness,
            );
            first_peer_route.then_some(peer_id)
        }
    }
}

pub(crate) fn connected_mesh_route_matches_registry(
    route: &ConnectedMeshRoute,
    registry: &BTreeMap<i32, clonk_engine::ClientCoreControlData>,
) -> bool {
    let (peer_id, peer_core) = match route {
        ConnectedMeshRoute::Tcp {
            peer_id, peer_core, ..
        }
        | ConnectedMeshRoute::Udp {
            peer_id, peer_core, ..
        } => (*peer_id, peer_core),
    };
    let Ok(peer_id) = i32::try_from(peer_id) else {
        return false;
    };
    registry.get(&peer_id).is_some_and(|canonical| {
        canonical.client_id == peer_core.client_id
            && canonical.name == peer_core.name
            && canonical.nick == peer_core.nick
    })
}

pub(crate) struct ClientUdpReconnect {
    addr: SocketAddr,
    handle: crate::ReliableUdpSessionHandle,
    request_template: ClientHandshakeRequestTemplate,
    expected_host_core: clonk_engine::ClientCoreControlData,
    connection_ids: Arc<AtomicU32>,
}

impl ClientUdpReconnect {
    pub(crate) fn start(&mut self) -> PendingClientRoute {
        let connection_id = self.connection_ids.fetch_add(1, AtomicOrdering::Relaxed);
        tokio::spawn(connect_secondary_udp_route(
            self.handle.clone(),
            self.addr,
            self.request_template.clone(),
            self.expected_host_core.clone(),
            connection_id,
        ))
    }
}

pub(crate) struct ClientTcpReconnect {
    addr: SocketAddr,
    request_template: ClientHandshakeRequestTemplate,
    expected_host_core: clonk_engine::ClientCoreControlData,
    connection_ids: Arc<AtomicU32>,
    io_statistics: crate::NetworkIoStatistics,
}

impl ClientTcpReconnect {
    pub(crate) fn start(&mut self) -> PendingTcpClientRoute {
        let connection_id = self.connection_ids.fetch_add(1, AtomicOrdering::Relaxed);
        tokio::spawn(connect_secondary_tcp_route(
            self.addr,
            self.request_template.clone(),
            self.expected_host_core.clone(),
            connection_id,
            self.io_statistics.clone(),
        ))
    }
}

pub(crate) async fn await_pending_client_route(
    pending: &mut Option<PendingClientRoute>,
) -> Option<ConnectedClientRoute<crate::ReliableUdpPeerStream>> {
    match pending.as_mut() {
        Some(task) => match task.await {
            Ok(Ok(route)) => Some(route),
            Ok(Err(_)) | Err(_) => None,
        },
        None => std::future::pending().await,
    }
}

pub(crate) async fn await_pending_tcp_client_route(
    pending: &mut Option<PendingTcpClientRoute>,
) -> Option<ConnectedClientRoute<TcpStream>> {
    match pending.as_mut() {
        Some(task) => match task.await {
            Ok(Ok(route)) => Some(route),
            Ok(Err(_)) | Err(_) => None,
        },
        None => std::future::pending().await,
    }
}

pub(crate) async fn connect_secondary_tcp_route(
    addr: SocketAddr,
    request_template: ClientHandshakeRequestTemplate,
    expected_host_core: clonk_engine::ClientCoreControlData,
    connection_id: u32,
    io_statistics: crate::NetworkIoStatistics,
) -> Result<ConnectedClientRoute<TcpStream>, ClientError> {
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "secondary TCP connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream.set_nodelay(true).ok();
    let peer_addr = stream.peer_addr().map_err(ClientError::Connect)?;
    let mut transport = crate::ControlTransport::with_statistics(
        stream,
        io_statistics.open_connection(connection_id, crate::NetworkProtocol::Tcp),
    );
    let handshake = crate::connection_handshake::run_client_route_handshake(
        &mut transport,
        request_template.connection_request(connection_id),
        &expected_host_core,
    )
    .await
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    debug_assert_eq!(handshake.local_connection_id, connection_id);
    debug_assert_eq!(handshake.peer_core, expected_host_core);
    Ok(ConnectedClientRoute {
        local_connection_id: handshake.local_connection_id,
        remote_connection_id: handshake.remote_connection_id,
        peer_addr,
        transport,
        liveness: handshake.liveness,
    })
}

pub(crate) async fn connect_secondary_udp_route(
    handle: crate::ReliableUdpSessionHandle,
    addr: SocketAddr,
    request_template: ClientHandshakeRequestTemplate,
    expected_host_core: clonk_engine::ClientCoreControlData,
    connection_id: u32,
) -> Result<ConnectedClientRoute<crate::ReliableUdpPeerStream>, ClientError> {
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, handle.connect(addr))
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "secondary reliable-UDP connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream
        .bind_statistics_connection(connection_id)
        .await
        .map_err(ClientError::Connect)?;
    let peer_addr = stream.peer_addr();
    let mut transport = crate::ControlTransport::new(stream);
    let handshake = crate::connection_handshake::run_client_route_handshake(
        &mut transport,
        request_template.connection_request(connection_id),
        &expected_host_core,
    )
    .await
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    debug_assert_eq!(handshake.local_connection_id, connection_id);
    debug_assert_eq!(handshake.peer_core, expected_host_core);
    Ok(ConnectedClientRoute {
        local_connection_id: handshake.local_connection_id,
        remote_connection_id: handshake.remote_connection_id,
        peer_addr,
        transport,
        liveness: handshake.liveness,
    })
}

#[cfg(test)]
pub(crate) async fn send_client_post_join_packets<S>(
    transport: &mut crate::ControlTransport<S>,
    start_control_tick: Tick,
    address_announcements: Vec<crate::AddressPacket>,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_client_control_request(transport, start_control_tick).await?;
    send_client_address_announcements(transport, address_announcements).await
}

async fn send_client_control_request<S>(
    transport: &mut crate::ControlTransport<S>,
    start_control_tick: Tick,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // C4GameControlNetwork::Init asks connected peers for the first control
    // tick before any JoinData resource initialization
    // (src/C4GameControlNetwork.cpp:46-62; src/C4Network2.cpp:1603-1613).
    transport
        .send_message(ControlMessage::Request {
            from_tick: start_control_tick,
        })
        .await
}

#[cfg(test)]
async fn send_client_address_announcements<S>(
    transport: &mut crate::ControlTransport<S>,
    address_announcements: Vec<crate::AddressPacket>,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // HandleJoinData announces addresses only after early GameRes, Dynamic,
    // and player-resource setup (src/C4Network2.cpp:1612-1622).
    for packet in address_announcements {
        transport
            .send_message(ControlMessage::Address(packet))
            .await?;
    }
    Ok(())
}

pub(crate) async fn send_client_route_address_announcements(
    routes: &mut ClientRouteManager,
    address_announcements: Vec<crate::AddressPacket>,
) -> Result<(), TransportError> {
    // SendAddresses appends each PID_Addr to C4NetIOTCP::Peer::OBuf and
    // returns without waiting for socket drainage. The route's lossless FIFO
    // is the equivalent acceptance boundary here
    // (oracle-src-pinned src/C4Network2Client.cpp:319-337,616-621;
    // src/C4NetIO.cpp:1345-1396).
    for packet in address_announcements {
        routes.send_message(ControlMessage::Address(packet)).await?;
    }
    Ok(())
}
