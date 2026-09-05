//! Host session loop: accepts, run_host, transport spawning, admission & join-data flow.
//!
//! This child module shares the parent session's private protocol machinery;
//! `session.rs` re-exports its crate-facing surface under the original paths.

use super::*;

async fn accept_udp_session(
    hub: &mut Option<crate::ReliableUdpSessionHub>,
) -> io::Result<crate::ReliableUdpPeerStream> {
    match hub {
        Some(hub) => hub.accept().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
fn tcp_accept_failure_injections() -> &'static (Mutex<BTreeSet<SocketAddr>>, tokio::sync::Notify) {
    static INJECTIONS: std::sync::OnceLock<(Mutex<BTreeSet<SocketAddr>>, tokio::sync::Notify)> =
        std::sync::OnceLock::new();
    INJECTIONS.get_or_init(|| (Mutex::new(BTreeSet::new()), tokio::sync::Notify::new()))
}

#[cfg(test)]
pub(crate) fn inject_tcp_accept_failure(address: SocketAddr) {
    let (injections, notify) = tcp_accept_failure_injections();
    assert!(
        injections.lock().unwrap().insert(address),
        "TCP accept failure already injected for {address}"
    );
    notify.notify_waiters();
}

#[cfg(test)]
async fn next_injected_tcp_accept_failure(address: SocketAddr) -> io::Error {
    let (injections, notify) = tcp_accept_failure_injections();
    loop {
        let notified = notify.notified();
        if injections.lock().unwrap().remove(&address) {
            return io::Error::other("injected TCP accept failure");
        }
        notified.await;
    }
}

async fn accept_tcp_connection(
    listener: &mut Option<TcpListener>,
) -> io::Result<(TcpStream, SocketAddr)> {
    match listener {
        Some(listener) => {
            #[cfg(test)]
            {
                let address = listener.local_addr()?;
                tokio::select! {
                    result = listener.accept() => result,
                    error = next_injected_tcp_accept_failure(address) => Err(error),
                }
            }
            #[cfg(not(test))]
            listener.accept().await
        }
        None => std::future::pending().await,
    }
}

async fn next_host_puncher_event(
    events: &mut Option<mpsc::Receiver<NetpuncherIoEvent>>,
) -> NetpuncherIoEvent {
    if let Some(events) = events.as_mut() {
        if let Some(event) = events.recv().await {
            return event;
        }
    }
    std::future::pending().await
}

async fn next_host_voice_media(
    events: &mut Option<mpsc::Receiver<crate::udp_session::ReliableUdpVoiceDatagram>>,
) -> crate::udp_session::ReliableUdpVoiceDatagram {
    if let Some(events) = events.as_mut() {
        if let Some(event) = events.recv().await {
            return event;
        }
    }
    std::future::pending().await
}

fn host_voice_routes(
    state: &HostState,
) -> Vec<(ClientId, SocketAddr, crate::voice::VoiceMediaCipher)> {
    if !state.config.voice_enabled {
        return Vec::new();
    }
    let mut selected = BTreeSet::new();
    state
        .accepted_routes
        .values()
        .filter_map(|route| {
            if !route.voice_auth.is_negotiated() {
                return None;
            }
            let cipher = route.voice_auth.send_cipher()?.clone();
            (route.protocol == crate::NetworkProtocol::Udp && selected.insert(route.client_id))
                .then_some((
                    route.client_id,
                    crate::canonical_reliable_udp_peer_address(route.peer_addr),
                    cipher,
                ))
        })
        .collect()
}

fn host_voice_ingress(
    state: &HostState,
    source: SocketAddr,
) -> Option<(ClientId, crate::voice::VoiceMediaCipher)> {
    if !state.config.voice_enabled {
        return None;
    }
    let source = crate::canonical_reliable_udp_peer_address(source);
    state.accepted_routes.values().find_map(|route| {
        if !route.voice_auth.is_negotiated() {
            return None;
        }
        let peer = crate::canonical_reliable_udp_peer_address(route.peer_addr);
        (route.protocol == crate::NetworkProtocol::Udp && peer == source)
            .then(|| {
                route
                    .voice_auth
                    .receive_cipher()
                    .map(|cipher| (route.client_id, cipher.clone()))
            })
            .flatten()
    })
}

fn send_host_voice_frame(
    frame: crate::VoiceFrame,
    udp_handle: Option<&crate::ReliableUdpSessionHandle>,
    state: &HostState,
) {
    let Some(udp_handle) = udp_handle else {
        return;
    };
    let frame = frame.with_authenticated_source(HOST_CLIENT_ID);
    for (_, peer, cipher) in host_voice_routes(state) {
        if let Ok(wire) = crate::voice::encode_authenticated_voice_packet(
            &cipher,
            &crate::voice::VoicePacket::Relayed(frame.clone()),
        ) {
            let _ = udp_handle.try_send_voice_media(peer, wire);
        }
    }
}

fn handle_host_voice_media(
    media: crate::udp_session::ReliableUdpVoiceDatagram,
    udp_handle: Option<&crate::ReliableUdpSessionHandle>,
    voice_events: &mpsc::Sender<crate::VoiceFrame>,
    state: &HostState,
    limiter: &mut crate::voice::VoiceIngressLimiter,
) {
    let Some((source_client_id, receive_cipher)) = host_voice_ingress(state, media.peer) else {
        return;
    };
    let Some(packet) = crate::voice::admit_voice_ingress(
        &media.payload,
        &receive_cipher,
        source_client_id,
        limiter,
        Instant::now(),
    ) else {
        return;
    };
    let Some((frame, direct_recipients)) =
        crate::voice::authenticate_host_ingress(source_client_id, packet)
    else {
        return;
    };
    let _ = voice_events.try_send(frame.clone());
    let Some(udp_handle) = udp_handle else {
        return;
    };
    for (client_id, peer, cipher) in host_voice_routes(state) {
        if crate::voice::host_relay_selects(client_id, source_client_id, &direct_recipients) {
            if let Ok(wire) = crate::voice::encode_authenticated_voice_packet(
                &cipher,
                &crate::voice::VoicePacket::Relayed(frame.clone()),
            ) {
                let _ = udp_handle.try_send_voice_media(peer, wire);
            }
        }
    }
}

async fn emit_host_netpuncher_state(state: &HostState) {
    let local_addresses = state
        .client_addresses
        .get(&(HOST_CLIENT_ID as i32))
        .cloned()
        .unwrap_or_default();
    let _ = state
        .event_tx
        .send(HostEvent::NetpuncherStateChanged {
            game_ids: state.netpuncher_game_ids,
            local_addresses,
        })
        .await;
}

async fn emit_host_local_addresses(state: &HostState) {
    let local_addresses = state
        .client_addresses
        .get(&(HOST_CLIENT_ID as i32))
        .cloned()
        .unwrap_or_default();
    let _ = state
        .event_tx
        .send(HostEvent::LocalAddressesChanged { local_addresses })
        .await;
}

async fn handle_host_puncher_event(
    event: NetpuncherIoEvent,
    udp_handle: Option<&crate::ReliableUdpSessionHandle>,
    configured_tcp_port: u16,
    configured_udp_port: u16,
    state: &mut HostState,
) {
    let Some(udp_handle) = udp_handle else {
        return;
    };
    match event {
        NetpuncherIoEvent::Connected {
            family,
            observed_address,
            ..
        } => {
            let added = crate::address_packet::add_addresses_from_puncher(
                state
                    .client_addresses
                    .entry(HOST_CLIENT_ID as i32)
                    .or_default(),
                observed_address,
                configured_udp_port,
                configured_tcp_port,
            );
            for address in &added {
                let packet = crate::AddressPacket {
                    client_id: HOST_CLIENT_ID as i32,
                    address: *address,
                };
                let _ = broadcast_host_message(
                    state,
                    ConnectionTrafficClass::Message,
                    ControlMessage::Address(packet),
                    None,
                );
            }
            if !added.is_empty() {
                emit_host_local_addresses(state).await;
            }
            if let Some(packet) = crate::reduce_puncher_connect(
                NetpuncherRole::Host,
                NetpuncherRuntimeState::Other,
                family,
                state.netpuncher_game_ids,
            ) {
                if let Err(error) = udp_handle.send_puncher_packet(family, packet).await {
                    let _ = state
                        .event_tx
                        .send(HostEvent::TransportError {
                            client_id: None,
                            error: format!("failed to send netpuncher ID request: {error}"),
                        })
                        .await;
                }
            }
        }
        NetpuncherIoEvent::Packet {
            family,
            packet: NetpuncherPacket::AssignId { id },
            ..
        } => {
            match family {
                NetpuncherAddressFamily::Ipv4 => state.netpuncher_game_ids.ipv4 = id,
                NetpuncherAddressFamily::Ipv6 => state.netpuncher_game_ids.ipv6 = id,
            }
            emit_host_netpuncher_state(state).await;
        }
        NetpuncherIoEvent::Packet {
            puncher_address, ..
        } => {
            let _ = udp_handle.close_puncher(puncher_address).await;
        }
    }
}

// The host loop owns each listener, channel, resource service, and statistics
// handle for its full lifetime; explicit arguments document that ownership.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_host(
    mut listener: Option<TcpListener>,
    mut udp_hub: Option<crate::ReliableUdpSessionHub>,
    udp_start_error: Option<String>,
    config: HostConfig,
    resource_backend: Option<crate::ResourceTransferBackend>,
    io_statistics: crate::NetworkIoStatistics,
    mut commands: mpsc::Receiver<HostCommand>,
    control_send_time: ControlSendTimeSnapshot,
    event_tx: mpsc::Sender<HostEvent>,
    mut voice_commands: mpsc::Receiver<crate::VoiceFrame>,
    voice_events: mpsc::Sender<crate::VoiceFrame>,
    voice_available: Arc<std::sync::atomic::AtomicBool>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let listener_addr = listener
        .as_ref()
        .and_then(|listener| listener.local_addr().ok());
    let udp_listener_addr = udp_hub
        .as_ref()
        .map(crate::ReliableUdpSessionHub::local_addr);
    let mut puncher_events = udp_hub
        .as_mut()
        .map(crate::ReliableUdpSessionHub::take_puncher_event_receiver);
    let mut voice_media = udp_hub
        .as_mut()
        .map(crate::ReliableUdpSessionHub::take_voice_media_receiver);
    let mut puncher_start_errors = Vec::new();
    if let Some(hub) = udp_hub.as_ref() {
        for address in selected_puncher_addresses(&config.netpuncher_addresses) {
            if let Err(error) = hub.init_puncher(address, NetpuncherRole::Host).await {
                puncher_start_errors.push(format!(
                    "failed to initialize netpuncher at {address}: {error}"
                ));
            }
        }
    }
    let udp_handle = udp_hub.as_ref().map(crate::ReliableUdpSessionHub::handle);
    let puncher_tcp_port = listener_addr
        .map(|address| config.configured_tcp_port.unwrap_or(address.port()))
        .unwrap_or(0);
    let puncher_udp_port = udp_listener_addr
        .map(|address| config.configured_udp_port.unwrap_or(address.port()))
        .unwrap_or(0);
    let backlog_limit = config.backlog_limit;
    let mut coordinator = ControlCoordinator::with_start_tick(backlog_limit, config.start_tick);
    // The host is an active lockstep participant from session start. C++ keeps
    // it in the ordered control-client list used by PackCompleteCtrl, which
    // waits for every client at the requested tick
    // (src/C4GameControlNetwork.cpp:741-769). Registering lazily on first input
    // lets a client-first packet incorrectly complete a tick by itself.
    coordinator
        .register_client(HOST_CLIENT_ID)
        .expect("new host coordinator must accept client ID 0");
    let required_password = (!config.password.is_empty()).then(|| config.password.clone());
    let admission = HostAdmission::new(
        1,
        config.allow_join,
        required_password,
        [config.local_core.name.clone()],
    );
    let client_cores = BTreeMap::from([(0, config.local_core.clone())]);
    let mut host_addresses = Vec::new();
    if let Some(listener_addr) = listener_addr {
        let port = config.configured_tcp_port.unwrap_or(listener_addr.port());
        if port != 0 {
            host_addresses.push(crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                SocketAddr::from(([0, 0, 0, 0], port)),
            ));
            if !listener_addr.ip().is_unspecified() {
                crate::append_received_address(
                    &mut host_addresses,
                    crate::NetworkAddress::new(
                        crate::NetworkProtocol::Tcp,
                        SocketAddr::new(listener_addr.ip(), port),
                    ),
                );
            }
        }
    }
    if let Some(listener_addr) = udp_listener_addr {
        let port = config.configured_udp_port.unwrap_or(listener_addr.port());
        if port != 0 {
            host_addresses.push(crate::NetworkAddress::new(
                crate::NetworkProtocol::Udp,
                SocketAddr::from(([0, 0, 0, 0], port)),
            ));
            if !listener_addr.ip().is_unspecified() {
                crate::append_received_address(
                    &mut host_addresses,
                    crate::NetworkAddress::new(
                        crate::NetworkProtocol::Udp,
                        SocketAddr::new(listener_addr.ip(), port),
                    ),
                );
            }
        }
    }
    let client_addresses = BTreeMap::from([(0, host_addresses)]);
    let mut resource_catalog = crate::ResourceCatalog::new(0);
    config
        .resource_registrations
        .iter()
        .copied()
        .for_each(|registration| {
            resource_catalog.register(registration);
        });
    let mut local_candidates = crate::ClientBootstrapLocalCandidates::default();
    local_candidates.extend_search_roots(&config.local_resource_roots);
    let resource_resolver = crate::client_bootstrap::ClientBootstrapResolver::new_with_group_maker(
        &local_candidates,
        config
            .resource_directory
            .clone()
            .unwrap_or_else(|| PathBuf::from("Network")),
        config.group_maker.clone(),
    );
    let published_player_sources = config.player_resource_sources.iter().cloned().collect();
    let published_player_local_paths = config
        .player_resource_sources
        .iter()
        .map(|(source_path, core)| {
            (
                source_path.clone(),
                super::host_state::configured_player_resource_local_path(
                    source_path,
                    core,
                    &config.resource_files,
                ),
            )
        })
        .collect();
    let mut state = HostState {
        coordinator,
        game_control_tick: config.start_tick,
        pending_complete: BTreeMap::new(),
        backlog: ControlBacklog::new(backlog_limit),
        client_performance: ClientPerformanceStats::new(backlog_limit),
        local_control_backlog: ControlBacklog::new(backlog_limit),
        scheduler: ResyncScheduler::new(config.resync_cooldown),
        clients: BTreeMap::new(),
        accepted_routes: BTreeMap::new(),
        #[cfg(test)]
        accepted_route_waiters: Vec::new(),
        #[cfg(test)]
        peer_capability_waiters: Vec::new(),
        control_send_time_epoch: 0,
        closed_routes: crate::post_mortem::ClosedConnectionRouter::default(),
        pending_sync: Vec::new(),
        status_barrier: StatusBarrier::stable(config.initial_status),
        last_chase_target_update: None,
        game_started: matches!(
            config.initial_status.state,
            NETWORK_STATE_GO | NETWORK_STATE_PAUSE
        ),
        control_mode: config.initial_status.control_mode,
        control_waiting_clients: BTreeMap::new(),
        control_discarded_clients: BTreeMap::new(),
        straggler_late: Default::default(),
        peer_capabilities: Default::default(),
        async_control_wait: None,
        admission,
        client_cores,
        client_addresses,
        netpuncher_game_ids: NetpuncherGameIds { ipv4: 0, ipv6: 0 },
        pending_kinds: BTreeMap::new(),
        join_snapshot: config.initial_join_snapshot.clone(),
        dynamic_required_clients: BTreeSet::new(),
        resource_catalog,
        resource_backend,
        published_player_sources,
        published_player_local_paths,
        resource_resolver,
        resource_epoch: Instant::now(),
        next_connection_id: 0,
        pending_route_peers: BTreeMap::new(),
        pending_route_clients: BTreeMap::new(),
        pending_admissions: BTreeMap::new(),
        pending_post_mortems: BTreeMap::new(),
        removing_clients: BTreeSet::new(),
        round_restart_pending_clients: BTreeMap::new(),
        round_restart_routes: BTreeMap::new(),
        round_restart_nonce: 0,
        lobby_chat_history: VecDeque::new(),
        event_tx: event_tx.clone(),
        config,
    };

    // C4InteractiveThread::PushEvent appends accepted network events to an
    // uncapped FIFO; a delayed main-thread consumer never suspends socket
    // reads or drops an already accepted event
    // (oracle-src-pinned src/C4InteractiveThread.cpp:70-100).
    let (client_tx, mut client_rx) = mpsc::unbounded_channel::<HostLoopMessage>();
    let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(32);
    let mut route_tasks = tokio::task::JoinSet::<()>::new();
    let mut resync_timer = interval(state.config.resync_interval);
    let mut resource_timer = interval(Duration::from_millis(crate::NETWORK_TIMER_INTERVAL_MS));
    // Runtime dynamics age out from C4Network2::OnSec1Timer, not from the
    // simulation/control clock (oracle-src-pinned src/C4Network2.cpp:674-697).
    let mut runtime_dynamic_timer = interval(Duration::from_secs(1));
    let mut published_control_send_time_epoch = None;
    let mut voice_ingress_limiter = crate::voice::VoiceIngressLimiter::default();

    if let Some(error) = udp_start_error {
        let _ = state
            .event_tx
            .send(HostEvent::TransportError {
                client_id: None,
                error,
            })
            .await;
    }
    for error in puncher_start_errors {
        let _ = state
            .event_tx
            .send(HostEvent::TransportError {
                client_id: None,
                error,
            })
            .await;
    }

    loop {
        voice_ingress_limiter.retain_sources(
            state
                .accepted_routes
                .values()
                .filter(|route| route.protocol == crate::NetworkProtocol::Udp)
                .map(|route| route.client_id),
        );
        voice_available.store(
            !host_voice_routes(&state).is_empty(),
            std::sync::atomic::Ordering::Release,
        );
        if published_control_send_time_epoch != Some(state.control_send_time_epoch) {
            publish_host_control_send_time(&state, &control_send_time);
            published_control_send_time_epoch = Some(state.control_send_time_epoch);
        }
        if !state
            .status_barrier
            .remotes
            .values()
            .any(|remote| *remote == RemoteBarrierState::Chasing)
        {
            state.last_chase_target_update = None;
        }
        let chase_target_update_deadline = state
            .last_chase_target_update
            .map(|last_update| last_update + CHASE_TARGET_UPDATE_INTERVAL);
        let async_control_deadline = state.async_control_deadline();
        // Socket tasks feed `client_rx` through an unbounded FIFO, just like
        // C4InteractiveThread. Do not let that always-ready FIFO starve game
        // commands: a command already queued disables every earlier network
        // arm. A command racing this check can be delayed by at most one
        // network operation before the next pass observes it.
        let command_pending = !commands.is_empty();
        let voice_media_ready = state.config.voice_enabled
            && crate::voice::voice_media_may_run(command_pending, !client_rx.is_empty());
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                break;
            }
            _ = wait_for_async_control_deadline(async_control_deadline), if !command_pending => {
                force_expired_async_control(&mut state).await;
            }
            event = next_host_puncher_event(&mut puncher_events), if !command_pending => {
                handle_host_puncher_event(
                    event,
                    udp_handle.as_ref(),
                    puncher_tcp_port,
                    puncher_udp_port,
                    &mut state,
                )
                .await;
            }
            accept_result = accept_tcp_connection(&mut listener), if !command_pending => {
                match accept_result {
                    Ok((stream, addr)) => {
                        let connection_id = state.next_connection_id;
                        state.next_connection_id = state.next_connection_id.wrapping_add(1);
                        state.pending_route_peers.insert(connection_id, addr);
                        spawn_host_accept(
                            &mut route_tasks,
                            stream,
                            addr,
                            state.config.local_core.clone(),
                            connection_id,
                            io_statistics.clone(),
                            admission_tx.clone(),
                            client_tx.clone(),
                        );
                    }
                    Err(error) => {
                        let _ = state.event_tx
                            .send(HostEvent::TransportError {
                                client_id: None,
                                error: format!("failed to accept connection: {error}"),
                            })
                            .await;
                        // C4NetIOTCP::Accept reports this scheduler pass as
                        // failed, but the scheduler keeps the TCP proc
                        // installed and its worker immediately runs another
                        // pass (src/C4NetIO.cpp:610-625,1038-1053;
                        // src/StdScheduler.cpp:160-191,229-244).
                        tokio::task::yield_now().await;
                    }
                }
            }
            accept_result = accept_udp_session(&mut udp_hub), if !command_pending => {
                match accept_result {
                    Ok(stream) => {
                        let addr = stream.peer_addr();
                        let udp_outbox = stream.outbox_registration();
                        let connection_id = state.next_connection_id;
                        state.next_connection_id = state.next_connection_id.wrapping_add(1);
                        if let Err(error) = stream.bind_statistics_connection(connection_id).await {
                            let _ = state.event_tx
                                .send(HostEvent::TransportError {
                                    client_id: None,
                                    error: format!(
                                        "failed to bind reliable-UDP connection statistics: {error}"
                                    ),
                                })
                                .await;
                            continue;
                        }
                        state.pending_route_peers.insert(connection_id, addr);
                        spawn_host_transport(
                            &mut route_tasks,
                            stream,
                            addr,
                            crate::NetworkProtocol::Udp,
                            state.config.local_core.clone(),
                            connection_id,
                            io_statistics.clone(),
                            admission_tx.clone(),
                            client_tx.clone(),
                            Some(udp_outbox),
                        );
                    }
                    Err(error) => {
                        let _ = state.event_tx
                            .send(HostEvent::TransportError {
                                client_id: None,
                                error: format!("failed to accept reliable-UDP connection: {error}"),
                            })
                            .await;
                        // A failed UDP socket does not take down the healthy
                        // TCP fallback. Dropping the hub disables this select
                        // branch without letting a terminal error busy-loop.
                        udp_hub.take();
                    }
                }
            }
            completed = route_tasks.join_next(), if !command_pending && !route_tasks.is_empty() => {
                let _ = completed;
            }
            Some(request) = admission_rx.recv(), if !command_pending => {
                handle_host_admission_request(request, &mut state).await;
            }
            Some(message) = client_rx.recv(), if !command_pending => {
                match message {
                    HostLoopMessage::ClientAccepted {
                        connection_id,
                        remote_connection_id,
                        core,
                        peer_is_port,
                        peer_addr,
                        protocol,
                        outbound,
                        setup_tx,
                    } => {
                        handle_client_accepted(
                            connection_id,
                            remote_connection_id,
                            core,
                            peer_is_port,
                            peer_addr,
                            protocol,
                            outbound,
                            setup_tx,
                            &mut state,
                        )
                        .await;
                    }
                    HostLoopMessage::ClientMessage {
                        connection_id,
                        client_id,
                        message,
                        ping_ms,
                    } => {
                        if state
                            .accepted_routes
                            .get(&connection_id)
                            .is_some_and(|route| route.client_id == client_id)
                        {
                            handle_client_message_with_restart_fence(
                                connection_id,
                                client_id,
                                message,
                                ping_ms,
                                &mut state,
                            )
                            .await;
                        }
                    }
                    HostLoopMessage::ConnectionPing {
                        connection_id,
                        client_id,
                        update,
                    } => {
                        if let Some(route) = state
                            .accepted_routes
                            .get_mut(&connection_id)
                            .filter(|route| route.client_id == client_id)
                        {
                            route.ping.apply(update, Instant::now());
                            state.invalidate_control_send_time();
                        }
                    }
                    HostLoopMessage::ClientDisconnected {
                        connection_id,
                        client_id,
                        next_inbound_packet,
                        next_outbound_packet,
                        post_mortem,
                        reason,
                    } => {
                        handle_client_disconnected(
                            connection_id,
                            client_id,
                            next_inbound_packet,
                            next_outbound_packet,
                            post_mortem,
                            reason,
                            &mut state,
                        )
                        .await;
                    }
                    HostLoopMessage::AdmissionFailed { connection_id, error } => {
                        handle_admission_failed(connection_id, error, &mut state).await;
                    }
                    HostLoopMessage::UnhandledPacket {
                        client_id,
                        packet_type,
                    } => {
                        let _ = state
                            .event_tx
                            .send(HostEvent::UnhandledPacket {
                                client_id,
                                packet_type,
                            })
                            .await;
                    }
                }
            }
            Some(command) = commands.recv() => {
                match command {
                    HostCommand::ChangeStatus(status) => {
                        let effects = state.status_barrier.change_status(status);
                        apply_barrier_effects(effects, &mut state).await;
                    }
                    HostCommand::BeginGo {
                        status,
                        join_allowed,
                        completion,
                    } => {
                        let effects = state.status_barrier.change_status(status);
                        apply_barrier_effects(effects, &mut state).await;
                        state.admission.set_allow_join(join_allowed);
                        let _ = completion.send(());
                    }
                    HostCommand::BroadcastStatusAck(status) => {
                        broadcast_status(status, true, &mut state).await;
                    }
                    HostCommand::ControlTickReached {
                        tick,
                        control_rate,
                        target_fps,
                        reached_at,
                    } => {
                        state.control_tick_reached(tick, control_rate, target_fps, reached_at);
                    }
                    HostCommand::ControlTickConsumed {
                        tick,
                        consumed_at,
                        client_ids,
                        reset_performance,
                    } => {
                        if reset_performance {
                            state.client_performance.reset_accumulators();
                        }
                        state
                            .client_performance
                            .mark_consumed(tick, consumed_at, client_ids);
                    }
                    HostCommand::Execute {
                        current_control_tick,
                        completion,
                    } => {
                        state.game_control_tick = state.game_control_tick.max(current_control_tick);
                        let removed = remove_stale_host_runtime_dynamic(&mut state);
                        let _ = completion.send(removed);
                    }
                    HostCommand::StatusReachedCurrent => {
                        let effects = state.status_barrier.local_reached();
                        apply_barrier_effects(effects, &mut state).await;
                    }
                    HostCommand::StatusReached {
                        status,
                        actual_control_tick,
                    } => {
                        let effects = state
                            .status_barrier
                            .local_reached_for(status, actual_control_tick);
                        apply_barrier_effects(effects, &mut state).await;
                    }
                    HostCommand::SubmitLocal(packet) => {
                        ingest_control(packet, ControlIngress::Local, &mut state).await
                    }
                    HostCommand::SubmitLobbyCountdown(packet) => {
                        let _ = state.event_tx.send(HostEvent::LobbyCountdown { packet }).await;
                        broadcast_lobby_countdown(packet, &mut state).await;
                    }
                    HostCommand::SubmitReadyCheck(packet) => {
                        apply_ready_check_to_host_state(packet, &mut state);
                        broadcast_ready_check(packet, None, &mut state).await;
                    }
                    HostCommand::BroadcastLeagueRoundResults(packet) => {
                        broadcast_league_round_results(packet, &mut state).await;
                    }
                    HostCommand::BroadcastHostRestarting { rejoin_seconds, completion } => {
                        broadcast_host_restarting(rejoin_seconds, &mut state).await;
                        let _ = completion.send(());
                    }
                    HostCommand::RestartRoundInLobby { config, completion } => {
                        let prepared = (|| {
                            let retained_client_ids = state
                                .clients
                                .keys()
                                .filter(|client_id| !state.removing_clients.contains(client_id))
                                .map(|client_id| *client_id as i32)
                                .collect::<Vec<_>>();
                            let restart_supported = retained_client_ids.is_empty()
                                || state.peer_capabilities.session_supports(
                                    retained_client_ids,
                                    crate::PortCapabilities::ROUND_RESTART_V2,
                                );
                            if !restart_supported {
                                return Err(
                                    "a retained client does not support atomic round restart"
                                        .to_string(),
                                );
                            }
                            let retained_routes = prepare_host_restart_routes(&state)?;
                            let prepared = prepare_host_round_config(*config, &state)?;
                            Ok((retained_routes, prepared))
                        })();
                        let result = match prepared {
                            Ok((retained_routes, prepared)) => {
                                finish_host_restart_removals(&mut state).await;
                                install_prepared_host_round_config(prepared, &mut state);
                                retain_host_restart_routes(&retained_routes, &mut state);
                                state.round_restart_routes = retained_routes.clone();
                                queue_host_restart_lobby(&retained_routes, &mut state)
                            }
                            Err(error) => Err(error),
                        };
                        if result.is_ok() {
                            #[cfg(test)]
                            notify_accepted_route_waiters(&mut state);
                            let _ = state.event_tx.send(HostEvent::RoundRestarted).await;
                            publish_pending_join_data(&mut state).await;
                        }
                        let _ = completion.send(result);
                    }
                    HostCommand::SubmitPacket { delivery, data } => broadcast_packet(delivery, data, None, &mut state).await,
                    HostCommand::ExecSync { control_tick } => broadcast_exec_sync(control_tick, &mut state).await,
                    HostCommand::PublishJoinSnapshot(snapshot) => {
                        if state
                            .join_snapshot
                            .as_ref()
                            .is_none_or(|current| current.dynamic != snapshot.dynamic)
                        {
                            state.dynamic_required_clients.clear();
                        }
                        state.join_snapshot = Some(*snapshot);
                        publish_pending_join_data(&mut state).await;
                    }
                    HostCommand::PublishRuntimeDynamic {
                        dynamic,
                        synchronized_control_tick,
                        parameters,
                        completion,
                    } => {
                        match publish_host_runtime_dynamic(
                            *dynamic,
                            synchronized_control_tick,
                            *parameters,
                            &mut state,
                        ) {
                            Ok(publication) => {
                                // A waiting joiner must receive the new core
                                // before the superseded resource is hidden.
                                publish_pending_join_data(&mut state).await;
                                if let Some(resource_id) = publication.previous_dynamic_id {
                                    mark_host_resource_removed(resource_id, &mut state);
                                }
                                let _ = completion.send(Ok(publication.core));
                            }
                            Err(error) => {
                                let _ = completion.send(Err(error));
                            }
                        }
                    }
                    HostCommand::RemoveRuntimeDynamic { completion } => {
                        let result = remove_host_runtime_dynamic(&mut state);
                        let _ = completion.send(result);
                    }
                    HostCommand::FailPendingJoinData { reason, completion } => {
                        let removed = fail_host_pending_join_data(reason, &mut state).await;
                        let _ = completion.send(removed);
                    }
                    HostCommand::PublishPlayerResource {
                        request,
                        completion,
                    } => {
                        let result = publish_host_player_resource_with_path(request, &mut state);
                        let _ = completion.send(result);
                    }
                    HostCommand::BeginResourceDerive {
                        resource_id,
                        source_path,
                        ownership,
                        completion,
                    } => {
                        let result = begin_host_resource_derive(
                            resource_id,
                            source_path,
                            ownership,
                            &mut state,
                        );
                        let _ = completion.send(result);
                    }
                    HostCommand::FinishResourceDerive {
                        derivation,
                        completion,
                    } => match finish_host_resource_derive(derivation, &mut state) {
                        Ok((core, events)) => {
                            dispatch_host_resource_events(events, true, &mut state).await;
                            let _ = completion.send(Ok(core));
                        }
                        Err(error) => {
                            let _ = completion.send(Err(error));
                        }
                    },
                    HostCommand::SetJoinAllowed {
                        allowed,
                        completion,
                    } => {
                        state.admission.set_allow_join(allowed);
                        let _ = completion.send(());
                    }
                    HostCommand::InspectRuntimeClientStates {
                        tick,
                        reset_performance,
                        completion,
                    } => {
                        if reset_performance {
                            state.client_performance.reset_accumulators();
                        }
                        let _ = completion.send(host_runtime_client_states(&state, tick));
                    }
                    HostCommand::SetPassword {
                        password,
                        completion,
                    } => {
                        state.admission.set_password(password);
                        let _ = completion.send(());
                    }
                    HostCommand::InitNetpunchers {
                        addresses,
                        completion,
                    } => {
                        if let Some(udp_handle) = udp_handle.as_ref() {
                            for address in selected_puncher_addresses(&addresses) {
                                if let Err(error) = udp_handle
                                    .init_puncher(address, NetpuncherRole::Host)
                                    .await
                                {
                                    let _ = state
                                        .event_tx
                                        .send(HostEvent::TransportError {
                                            client_id: None,
                                            error: format!(
                                                "failed to initialize netpuncher at {address}: {error}"
                                            ),
                                        })
                                        .await;
                                }
                            }
                        }
                        let _ = completion.send(());
                    }
                    HostCommand::InspectRuntimeConnections { completion } => {
                        let _ = completion.send(host_runtime_connections(&state));
                    }
                    HostCommand::InspectLobbyClientTelemetry {
                        client_ids,
                        completion,
                    } => {
                        let catalog = state
                            .resource_backend
                            .as_ref()
                            .map(crate::ResourceTransferBackend::catalog)
                            .unwrap_or(&state.resource_catalog);
                        let telemetry = runtime_lobby_client_telemetry(
                            host_runtime_connections(&state),
                            catalog,
                            client_ids,
                        );
                        let _ = completion.send(telemetry);
                    }
                    HostCommand::DisconnectRuntimeConnection {
                        connection_id,
                        completion,
                    } => {
                        let disconnected =
                            disconnect_host_runtime_connection(connection_id, &mut state);
                        let _ = completion.send(disconnected);
                    }
                    #[cfg(test)]
                    HostCommand::InspectAcceptedRoutes { completion } => {
                        let _ = completion.send(accepted_route_snapshot(&state));
                    }
                    #[cfg(test)]
                    HostCommand::WaitForAcceptedRoutesChange {
                        initial_ids,
                        expected_count,
                        completion,
                    } => {
                        let routes = accepted_route_snapshot(&state);
                        if accepted_routes_changed(&routes, &initial_ids, expected_count) {
                            let _ = completion.send(routes);
                        } else {
                            state.accepted_route_waiters.push(AcceptedRouteWaiter {
                                initial_ids,
                                expected_count,
                                completion,
                            });
                        }
                    }
                    #[cfg(test)]
                    HostCommand::WaitForPeerCapability {
                        client_id,
                        capability,
                        completion,
                        registered,
                    } => {
                        if state
                            .peer_capabilities
                            .peer_supports(client_id as i32, capability)
                        {
                            let _ = completion.send(());
                        } else {
                            state.peer_capability_waiters.push(PeerCapabilityWaiter {
                                client_id,
                                capability,
                                completion,
                            });
                        }
                        if let Some(registered) = registered {
                            let _ = registered.send(());
                        }
                    }
                    #[cfg(test)]
                    HostCommand::InspectConnectedClients { completion } => {
                        let _ = completion.send(state.clients.keys().copied().collect());
                    }
                    HostCommand::Shutdown => break,
                }
            }
            // Media is deliberately below admitted route/control input and
            // game commands in this biased select. Its handler is synchronous
            // and bounded, so an event racing the readiness snapshot waits for
            // at most one media transition.
            media = next_host_voice_media(&mut voice_media), if voice_media_ready => {
                handle_host_voice_media(
                    media,
                    udp_handle.as_ref(),
                    &voice_events,
                    &state,
                    &mut voice_ingress_limiter,
                );
            }
            Some(frame) = voice_commands.recv(), if voice_media_ready => {
                send_host_voice_frame(frame, udp_handle.as_ref(), &state);
            }
            _ = wait_for_chase_target_update(chase_target_update_deadline) => {
                update_chase_targets(&mut state).await;
            }
            _ = resync_timer.tick() => {
                request_missing_controls(&mut state).await;
            }
            _ = resource_timer.tick() => {
                io_statistics.generate_statistics(network_statistics_now_ms());
                // C4Network2IO::CheckTimeout removes closed routes once their
                // ten-second post-mortem recovery window has elapsed.
                state.closed_routes.expire();
                let now_seconds = state.resource_epoch.elapsed().as_secs();
                if let Some(backend) = state.resource_backend.as_mut() {
                    let mut random = resource_safe_random;
                    match backend.on_timer(now_seconds, &mut random) {
                        Ok(events) => {
                            dispatch_host_resource_events(events, false, &mut state).await
                        }
                        Err(error) => report_host_resource_error(error, &state).await,
                    }
                    advance_shadow_resource_catalog_timer(
                        &mut state.resource_catalog,
                        now_seconds,
                    );
                } else {
                    let actions = state.resource_catalog.on_timer(now_seconds);
                    dispatch_host_resource_actions(actions, &mut state).await;
                }
            }
            _ = runtime_dynamic_timer.tick() => {
                remove_stale_host_runtime_dynamic(&mut state);
            }
        }
    }

    voice_available.store(false, std::sync::atomic::Ordering::Release);
    client_rx.close();
    admission_rx.close();
    for route in state.accepted_routes.values() {
        route.outbound.retire();
    }
    route_tasks.shutdown().await;
    for client_id in state.clients.keys() {
        let _ = state.event_tx.try_send(HostEvent::ClientLeft {
            client_id: *client_id,
        });
    }
    if let Some(hub) = udp_hub {
        let _ = hub.shutdown().await;
    }
}

// Admission hands distinct classic connection identity and task channels to
// the transport task; keeping them explicit prevents accidental ID mixing.
#[allow(clippy::too_many_arguments)]
fn spawn_host_accept(
    route_tasks: &mut tokio::task::JoinSet<()>,
    stream: TcpStream,
    addr: SocketAddr,
    local_core: clonk_engine::ClientCoreControlData,
    connection_id: u32,
    io_statistics: crate::NetworkIoStatistics,
    admission_tx: mpsc::Sender<HostAdmissionRequest>,
    host_tx: mpsc::UnboundedSender<HostLoopMessage>,
) {
    if let Err(error) = stream.set_nodelay(true) {
        let _ = host_tx.send(HostLoopMessage::AdmissionFailed {
            connection_id,
            error: format!("failed to configure connection {addr}: {error}"),
        });
        return;
    }
    spawn_host_transport(
        route_tasks,
        stream,
        addr,
        crate::NetworkProtocol::Tcp,
        local_core,
        connection_id,
        io_statistics,
        admission_tx,
        host_tx,
        None,
    );
}

// Admission hands distinct classic connection identity and task channels to
// the transport task; keeping them explicit prevents accidental ID mixing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_host_transport<S>(
    route_tasks: &mut tokio::task::JoinSet<()>,
    stream: S,
    addr: SocketAddr,
    protocol: crate::NetworkProtocol,
    local_core: clonk_engine::ClientCoreControlData,
    connection_id: u32,
    io_statistics: crate::NetworkIoStatistics,
    admission_tx: mpsc::Sender<HostAdmissionRequest>,
    host_tx: mpsc::UnboundedSender<HostLoopMessage>,
    udp_outbox: Option<crate::udp_session::ReliableUdpOutboxRegistration>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    route_tasks.spawn(async move {
        let request = crate::ConnectionRequest {
            core: local_core,
            build: CURRENT_GAME_BUILD,
            password: clonk_engine::LegacyCString::default(),
            connection_id,
            port_protocol: true,
        };
        let mut transport = crate::ControlTransport::new(stream);
        if matches!(protocol, crate::NetworkProtocol::Tcp) {
            transport.set_statistics(
                io_statistics.open_connection(connection_id, crate::NetworkProtocol::Tcp),
            );
        }
        let handshake =
            match run_host_connection_handshake(&mut transport, request, &admission_tx).await {
                Ok(handshake) => handshake,
                Err(error) => {
                    let _ = host_tx.send(HostLoopMessage::AdmissionFailed {
                        connection_id,
                        error: format!("connection admission from {addr} failed: {error}"),
                    });
                    return;
                }
            };
        let crate::HostConnectionHandshake {
            local_connection_id,
            remote_connection_id,
            peer_core,
            peer_is_port,
            liveness,
        } = handshake;
        debug_assert_eq!(local_connection_id, connection_id);
        let Ok(client_id) = ClientId::try_from(peer_core.client_id) else {
            let _ = host_tx.send(HostLoopMessage::AdmissionFailed {
                connection_id,
                error: "accepted peer has a negative client id".to_string(),
            });
            return;
        };
        let udp_outbound = match udp_outbox {
            Some(registration) => match registration.promote(transport.outbound_packet_log()).await
            {
                Ok(outbound) => Some(outbound),
                Err(error) => {
                    let _ = host_tx.send(HostLoopMessage::AdmissionFailed {
                        connection_id,
                        error: format!("failed to promote UDP route {addr}: {error}"),
                    });
                    return;
                }
            },
            None => None,
        };
        let (outbound, mut client_task): (
            HostOutboundSender,
            Pin<Box<dyn Future<Output = ()> + Send>>,
        ) = if let Some(udp_outbound) = udp_outbound {
            let outbound = HostOutboundSender::from_udp(udp_outbound);
            let retire_rx = outbound.subscribe_retire();
            let task = UdpClientTask {
                local_connection_id: connection_id,
                remote_connection_id,
                client_id,
                transport,
                outbound: outbound.clone(),
                retire_rx,
                host_tx: host_tx.clone(),
                liveness,
            }
            .run();
            (outbound, Box::pin(task))
        } else {
            let (outbound, outbound_rx) = HostOutboundSender::channel();
            let retire_rx = outbound.subscribe_retire();
            let task = ClientTask {
                local_connection_id: connection_id,
                remote_connection_id,
                client_id,
                transport,
                outbound_rx,
                retire_rx,
                host_tx: host_tx.clone(),
                liveness,
            }
            .run();
            (outbound, Box::pin(task))
        };
        let (setup_tx, setup_rx) = oneshot::channel();
        if host_tx
            .send(HostLoopMessage::ClientAccepted {
                connection_id,
                remote_connection_id,
                core: peer_core,
                peer_is_port,
                peer_addr: addr,
                protocol,
                outbound: outbound.clone(),
                setup_tx,
            })
            .is_err()
        {
            return;
        }
        // C4Network2IO keeps every mutually accepted socket live while the
        // main thread prepares SendJoinData
        // (src/C4Network2IO.cpp:611-623,1155-1191;
        // src/C4Network2.cpp:1107-1133,1836-1865).
        // The owning host loop performs C++'s synchronous SendJoinData work
        // and queues the complete JoinData/address prefix before releasing
        // this transport gate. The accepted route still services inbound
        // Ping/Pong through `client_task` while that main-thread work waits.
        tokio::select! {
            setup = setup_rx => match setup {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    outbound.retire();
                    client_task.await;
                    return;
                }
            },
            () = &mut client_task => return,
        }
        client_task.await;
    });
}

pub(crate) async fn handle_host_admission_request(
    request: HostAdmissionRequest,
    state: &mut HostState,
) {
    if ClientId::try_from(request.request.core.client_id)
        .is_ok_and(|client_id| state.removing_clients.contains(&client_id))
    {
        let _ = request.decision_tx.send(AdmissionDecision::Reject {
            message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec())
                .unwrap_or_default(),
            wrong_password: false,
        });
        return;
    }
    let requested_kind = if request.request.core.observer {
        ParticipantKind::Observer
    } else {
        ParticipantKind::Player
    };
    let canonical_peer = state
        .client_cores
        .get(&request.request.core.client_id)
        .filter(|core| {
            core.client_id != HOST_CLIENT_ID as i32
                && core.name == request.request.core.name
                && core.nick == request.request.core.nick
        })
        .cloned();
    let mut decision = match canonical_peer.as_ref() {
        Some(core)
            if ClientId::try_from(core.client_id).is_ok_and(|client_id| {
                !secondary_route_matches_existing_host(state, client_id, request.connection_id)
            }) =>
        {
            AdmissionDecision::Reject {
                message: clonk_engine::LegacyCString::from_bytes(
                    b"secondary connection came from a different peer host".to_vec(),
                )
                .unwrap_or_default(),
                wrong_password: false,
            }
        }
        Some(core) => crate::KnownPeerAdmission::admit(&request.request, core, false),
        None => state.admission.admit_new_peer(&request.request),
    };
    if let AdmissionDecision::Accept {
        before_reply,
        peer_core,
        ..
    } = &mut decision
    {
        for action in std::mem::take(before_reply) {
            let ConnectionAction::EmitDirectClientJoin(join) = action else {
                let _ = request.decision_tx.send(AdmissionDecision::Reject {
                    message: clonk_engine::LegacyCString::from_bytes(
                        b"invalid host admission action".to_vec(),
                    )
                    .unwrap_or_default(),
                    wrong_password: false,
                });
                return;
            };
            state
                .client_cores
                .insert(join.core.client_id, join.core.clone());
            state.invalidate_control_send_time();
            state
                .pending_kinds
                .insert(join.core.client_id, requested_kind);
            if let Ok(data) =
                crate::encode_control_entry_payload(&clonk_engine::ControlPacket::ClientJoin(join))
            {
                let _ = broadcast_host_message(
                    state,
                    ConnectionTrafficClass::Message,
                    ControlMessage::Packet {
                        delivery: ControlDelivery::Direct,
                        data: data.clone(),
                    },
                    None,
                );
                let _ = state
                    .event_tx
                    .send(HostEvent::Direct {
                        client_id: HOST_CLIENT_ID,
                        delivery: ControlDelivery::Direct,
                        data,
                    })
                    .await;
            }
        }
        debug_assert_eq!(
            state.client_cores.get(&peer_core.client_id),
            Some(&*peer_core)
        );
        if let Ok(client_id) = ClientId::try_from(peer_core.client_id) {
            state
                .pending_route_clients
                .insert(request.connection_id, client_id);
        }
        if canonical_peer.is_none() {
            state
                .pending_admissions
                .insert(request.connection_id, peer_core.client_id);
        }
    }
    let _ = request.decision_tx.send(decision);
}

// Acceptance reconciles independent route IDs, protocol metadata, channels,
// and host state. These are intentionally explicit at the reducer boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_client_accepted(
    connection_id: u32,
    remote_connection_id: u32,
    core: clonk_engine::ClientCoreControlData,
    peer_is_port: bool,
    peer_addr: SocketAddr,
    protocol: crate::NetworkProtocol,
    outbound: HostOutboundSender,
    setup_tx: oneshot::Sender<Result<(), String>>,
    state: &mut HostState,
) {
    state.pending_admissions.remove(&connection_id);
    state.pending_route_peers.remove(&connection_id);
    let pending_client_id = state.pending_route_clients.remove(&connection_id);
    let Ok(client_id) = ClientId::try_from(core.client_id) else {
        let _ = setup_tx.send(Err("accepted peer has a negative client id".to_string()));
        return;
    };
    if pending_client_id != Some(client_id)
        || state.removing_clients.contains(&client_id)
        || !state.client_cores.contains_key(&core.client_id)
    {
        let _ = setup_tx.send(Err(
            "client was removed before route setup completed".to_string()
        ));
        return;
    }
    let voice_auth = if state.config.voice_enabled && protocol == crate::NetworkProtocol::Udp {
        crate::voice::VoiceRouteAuthentication::new_udp()
    } else {
        crate::voice::VoiceRouteAuthentication::default()
    };
    let capability_announcement = voice_auth
        .announcement()
        .unwrap_or_else(crate::PortCapabilities::supported_without_voice);
    let replaced_route = state.accepted_routes.insert(
        connection_id,
        AcceptedConnectionRoute {
            client_id,
            remote_connection_id,
            peer_addr,
            protocol,
            ping: RoutePingLag::default(),
            outbound: outbound.clone(),
            voice_auth,
            peer_is_port,
        },
    );
    state.invalidate_control_send_time();
    debug_assert!(replaced_route.is_none());
    if peer_is_port {
        if let Some(cookie) = state
            .accepted_routes
            .get(&connection_id)
            .and_then(|route| route.voice_auth.receive_cookie())
        {
            outbound.set_voice_receive_cookie(cookie);
        }
        let _ = outbound.try_send(ControlMessage::PortCapabilities(capability_announcement));
    }
    if state.clients.contains_key(&client_id) {
        if setup_tx.send(Ok(())).is_err() {
            state.accepted_routes.remove(&connection_id);
            state.invalidate_control_send_time();
            return;
        }
        #[cfg(test)]
        notify_accepted_route_waiters(state);
        let preferred = preferred_host_route(state, client_id, ConnectionTrafficClass::Message)
            .map(|route| (route.outbound.clone(), route.peer_addr));
        if let (Some(client), Some((outbound, peer_addr))) =
            (state.clients.get_mut(&client_id), preferred)
        {
            client.outbound = outbound;
            client.peer_addr = peer_addr;
        }
        return;
    }
    let kind = state
        .pending_kinds
        .remove(&core.client_id)
        .unwrap_or(ParticipantKind::Player);
    state.clients.insert(
        client_id,
        ClientConnection {
            outbound: outbound.clone(),
            core: core.clone(),
            peer_addr,
            join_data_sent: false,
            join_data_needed_emitted: false,
        },
    );
    state.client_addresses.entry(core.client_id).or_default();
    let _ = state
        .event_tx
        .send(HostEvent::ClientJoined {
            client_id,
            name: clonk_resources::decode_legacy_script_text(core.name.as_bytes()),
            kind,
        })
        .await;

    let setup_result = match build_client_setup(client_id, state) {
        Ok(Some(setup)) => match enqueue_client_setup_prefix(client_id, &outbound, setup, state) {
            Ok(()) => {
                mark_join_data_sent(client_id, state);
                Ok(())
            }
            Err(error) => Err(error),
        },
        Ok(None) => {
            emit_join_data_needed(client_id, state).await;
            Ok(())
        }
        Err(error) => Err(error),
    };
    let setup_error = setup_result.as_ref().err().cloned();
    let setup_delivered = setup_tx.send(setup_result).is_ok();
    if setup_error.is_some() || !setup_delivered {
        handle_client_disconnected(
            connection_id,
            client_id,
            0,
            0,
            None,
            setup_error.or_else(|| Some("accepted connection setup was dropped".to_string())),
            state,
        )
        .await;
        return;
    }
    #[cfg(test)]
    notify_accepted_route_waiters(state);
    let now_seconds = state.resource_epoch.elapsed().as_secs();
    if let Some(backend) = state.resource_backend.as_mut() {
        let mut random = resource_safe_random;
        match backend.on_peer_connected(core.client_id, now_seconds, &mut random) {
            Ok(events) => dispatch_host_resource_events(events, false, state).await,
            Err(error) => report_host_resource_error(error, state).await,
        }
    } else {
        let actions = state.resource_catalog.on_peer_connected(core.client_id);
        dispatch_host_resource_actions(actions, state).await;
    }
}

#[cfg(test)]
pub(crate) fn accepted_route_snapshot(state: &HostState) -> Vec<(u32, ClientId, u32)> {
    state
        .accepted_routes
        .iter()
        .map(|(connection_id, route)| (*connection_id, route.client_id, route.remote_connection_id))
        .collect()
}

#[cfg(test)]
fn accepted_routes_changed(
    routes: &[(u32, ClientId, u32)],
    initial_ids: &BTreeSet<u32>,
    expected_count: usize,
) -> bool {
    routes.len() == expected_count
        && routes
            .iter()
            .map(|(connection_id, _, _)| *connection_id)
            .collect::<BTreeSet<_>>()
            != *initial_ids
}

#[cfg(test)]
pub(crate) fn notify_accepted_route_waiters(state: &mut HostState) {
    let routes = accepted_route_snapshot(state);
    let waiters = std::mem::take(&mut state.accepted_route_waiters);
    for waiter in waiters {
        if accepted_routes_changed(&routes, &waiter.initial_ids, waiter.expected_count) {
            let _ = waiter.completion.send(routes.clone());
        } else {
            state.accepted_route_waiters.push(waiter);
        }
    }
}

fn enqueue_client_setup_prefix(
    client_id: ClientId,
    outbound: &HostOutboundSender,
    setup: ClientSetup,
    state: &mut HostState,
) -> Result<(), String> {
    let ClientSetup {
        join_data,
        addresses,
        lobby_chat_history,
    } = setup;
    outbound
        .try_send(ControlMessage::JoinData(Box::new(join_data)))
        .map_err(|_| "accepted route closed while queueing JoinData".to_string())?;
    mark_join_data_dynamic_required(client_id, state);
    for address in addresses {
        outbound
            .try_send(ControlMessage::Address(address))
            .map_err(|_| "accepted route closed while queueing initial addresses".to_string())?;
    }
    for data in lobby_chat_history {
        outbound
            .try_send(ControlMessage::Packet {
                delivery: ControlDelivery::Private,
                data,
            })
            .map_err(|_| "accepted route closed while queueing lobby chat history".to_string())?;
    }
    Ok(())
}

fn build_client_setup(
    client_id: ClientId,
    state: &HostState,
) -> Result<Option<ClientSetup>, String> {
    let Some(client) = state.clients.get(&client_id) else {
        return Err(format!("accepted client {client_id} is missing"));
    };
    if client.join_data_sent || state.removing_clients.contains(&client_id) {
        return Ok(None);
    }
    let Some(mut snapshot) = state.join_snapshot.clone() else {
        return Ok(None);
    };
    let current_tick = i32::try_from(state.game_control_tick).unwrap_or(i32::MAX);
    if snapshot.dynamic.resource_type == clonk_engine::NETWORK_RESOURCE_TYPE_NULL
        || snapshot.dynamic_tick < current_tick
    {
        return Ok(None);
    }

    snapshot.parameters.clients =
        JoinClientRegistrySnapshot::new(state.client_cores.values().cloned().collect());
    let join_data = JoinDataEnvelope {
        client_id: client.core.client_id,
        start_control_tick: snapshot.dynamic_tick,
        status: state.status_barrier.status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    let addresses = address_packets_for_peer(&state.client_addresses, client.peer_addr);
    let lobby_chat_history = if state.status_barrier.status.state == NETWORK_STATE_LOBBY {
        state.lobby_chat_history.iter().cloned().collect()
    } else {
        Vec::new()
    };
    Ok(Some(ClientSetup {
        join_data,
        addresses,
        lobby_chat_history,
    }))
}

fn mark_join_data_sent(client_id: ClientId, state: &mut HostState) {
    if let Some(client) = state.clients.get_mut(&client_id) {
        client.join_data_sent = true;
    }
    state
        .status_barrier
        .set_remote_state(client_id, RemoteBarrierState::Chasing);
    if state.last_chase_target_update.is_none() {
        state.last_chase_target_update = Some(tokio::time::Instant::now());
    }
}

fn mark_join_data_dynamic_required(client_id: ClientId, state: &mut HostState) {
    if state.join_snapshot.as_ref().is_some_and(|snapshot| {
        snapshot.dynamic.resource_type == crate::HostResourceType::Dynamic as u8
    }) {
        state.dynamic_required_clients.insert(client_id);
    }
}

pub(crate) async fn wait_for_chase_target_update(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn wait_for_async_control_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn force_expired_async_control(state: &mut HostState) {
    let Some(waiting) = state.async_control_wait else {
        return;
    };
    if state.control_mode != 2 || waiting.tick != state.coordinator.current_tick() {
        return;
    }
    let missing = state.coordinator.clients_missing(waiting.tick);
    // Waiting the full budget is right for a peer that hiccupped. It is wrong
    // for one that has missed every recent tick: the host would spend the whole
    // budget again on control that is not coming, and every other participant
    // waits with him. Once all the clients still outstanding are in that state,
    // pack immediately. They rejoin the waited-for set the moment they deliver.
    let patience = state.config.straggler_patience;
    let all_missing_are_stragglers = patience > 0
        && !missing.is_empty()
        && missing.iter().all(|client_id| {
            state
                .straggler_late
                .get(client_id)
                .is_some_and(|late| *late >= patience)
        });
    let deadline_expired =
        tokio::time::Instant::now() >= waiting.deadline(state.config.async_max_wait_frames);
    if !all_missing_are_stragglers && !deadline_expired {
        return;
    }
    for client_id in state.coordinator.client_ids().collect::<Vec<_>>() {
        let missed = missing.contains(&client_id);
        let late = state.straggler_late.entry(client_id).or_insert(0);
        if !missed {
            *late = 0;
        } else if deadline_expired {
            // Only a client that had the whole budget and still did not deliver
            // has earned a mark. On the fast path the host packs early *because*
            // of a known straggler, so anyone else merely still in flight has
            // not failed at anything and must not be written off for it.
            *late = late.saturating_add(1);
        }
    }
    // Whoever is still missing at this point loses the tick: `force_current_tick`
    // packs without them and `ControlCoordinator::ingest` rejects their packet
    // afterwards as stale. Record it so the aggregate carries the outcome back
    // to each affected client, which otherwise sees its input vanish with no
    // way to tell that from an engine bug.
    if !missing.is_empty() {
        state
            .control_discarded_clients
            .entry(waiting.tick)
            .or_default()
            .extend(missing.iter().copied());
    }
    let ready = state.coordinator.force_current_tick();
    resolve_host_ready(ready, state).await;
}

async fn update_chase_targets(state: &mut HostState) {
    let chasing_clients = state
        .status_barrier
        .remotes
        .iter()
        .filter_map(|(client_id, remote)| {
            (*remote == RemoteBarrierState::Chasing).then_some(*client_id)
        })
        .collect::<Vec<_>>();
    if chasing_clients.is_empty() {
        state.last_chase_target_update = None;
        return;
    }

    let status = state
        .status_barrier
        .status
        .with_target_tick(i32::try_from(state.coordinator.current_tick()).unwrap_or(i32::MAX));
    for client_id in chasing_clients {
        let _ = send_host_message(
            state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Status(status),
        )
        .await;
    }
    state.last_chase_target_update = Some(tokio::time::Instant::now());
}

async fn emit_join_data_needed(client_id: ClientId, state: &mut HostState) {
    let Some(client) = state.clients.get_mut(&client_id) else {
        return;
    };
    if client.join_data_needed_emitted {
        return;
    }
    client.join_data_needed_emitted = true;
    let _ = state
        .event_tx
        .send(HostEvent::JoinDataNeeded {
            client_id,
            current_control_tick: state.coordinator.current_tick(),
        })
        .await;
}

pub(crate) fn pending_join_data_client_ids(
    clients: &BTreeMap<ClientId, ClientConnection>,
    removing_clients: &BTreeSet<ClientId>,
) -> Vec<ClientId> {
    clients
        .iter()
        .filter_map(|(client_id, client)| {
            (!client.join_data_sent && !removing_clients.contains(client_id)).then_some(*client_id)
        })
        .collect()
}

pub(crate) async fn publish_pending_join_data(state: &mut HostState) {
    let pending = pending_join_data_client_ids(&state.clients, &state.removing_clients);
    for client_id in pending {
        let setup = match build_client_setup(client_id, state) {
            Ok(Some(setup)) => setup,
            Ok(None) => {
                emit_join_data_needed(client_id, state).await;
                continue;
            }
            Err(error) => {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: Some(client_id),
                        error,
                    })
                    .await;
                continue;
            }
        };
        if !send_host_message(
            state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::JoinData(Box::new(setup.join_data)),
        )
        .await
        {
            continue;
        }
        mark_join_data_dynamic_required(client_id, state);
        let mut failed = false;
        for address in setup.addresses {
            if !send_host_message(
                state,
                client_id,
                ConnectionTrafficClass::Message,
                ControlMessage::Address(address),
            )
            .await
            {
                failed = true;
                break;
            }
        }
        if failed {
            continue;
        }
        for data in setup.lobby_chat_history {
            if !send_host_message(
                state,
                client_id,
                ConnectionTrafficClass::Message,
                ControlMessage::Packet {
                    delivery: ControlDelivery::Private,
                    data,
                },
            )
            .await
            {
                failed = true;
                break;
            }
        }
        if failed {
            continue;
        }
        mark_join_data_sent(client_id, state);
    }
}

fn address_packets_for_peer(
    client_addresses: &BTreeMap<i32, Vec<crate::NetworkAddress>>,
    peer_addr: SocketAddr,
) -> Vec<crate::AddressPacket> {
    client_addresses
        .iter()
        .flat_map(|(client_id, addresses)| {
            addresses.iter().filter_map(move |address| {
                address_for_peer(*address, peer_addr).map(|address| crate::AddressPacket {
                    client_id: *client_id,
                    address,
                })
            })
        })
        .collect()
}

pub(crate) fn address_for_peer(
    mut address: crate::NetworkAddress,
    peer_addr: SocketAddr,
) -> Option<crate::NetworkAddress> {
    let SocketAddr::V6(mut endpoint) = address.endpoint else {
        return Some(address);
    };
    if endpoint.scope_id() == 0 {
        return Some(address);
    }
    let SocketAddr::V6(peer) = peer_addr else {
        return None;
    };
    if peer.scope_id() != endpoint.scope_id() {
        return None;
    }
    endpoint.set_scope_id(0);
    address.endpoint = SocketAddr::V6(endpoint);
    Some(address)
}

impl HostState {
    pub(crate) fn coordination_register(
        &mut self,
        client_id: ClientId,
    ) -> Result<(), crate::ControlError> {
        if !self.coordinator.client_ids().any(|id| id == client_id) {
            self.coordinator.register_client(client_id)?;
        }
        Ok(())
    }

    fn control_tick_reached(
        &mut self,
        tick: Tick,
        control_rate: i32,
        target_fps: i32,
        reached_at: tokio::time::Instant,
    ) {
        self.game_control_tick = self.game_control_tick.max(tick);
        self.client_performance.record_cadence(tick, reached_at);
        if tick != self.coordinator.current_tick() {
            return;
        }
        if self.control_mode != 0 {
            let waiting = self
                .coordinator
                .clients_missing(tick)
                .into_iter()
                .collect::<BTreeSet<_>>();
            if !waiting.is_empty() {
                self.control_waiting_clients.entry(tick).or_insert(waiting);
            }
        }
        if let Some(waiting) = self
            .async_control_wait
            .as_mut()
            .filter(|waiting| waiting.tick == tick)
        {
            // ExecQueuedSyncCtrl may change ControlRate or SetPreSend after
            // the first cadence stamp but before GetControl. Keep the first
            // wait instant while refreshing the two live deadline inputs.
            waiting.control_rate = control_rate;
            waiting.target_fps = target_fps;
            return;
        }
        self.async_control_wait = Some(AsyncControlWait {
            tick,
            reached_at,
            control_rate,
            target_fps,
        });
    }

    fn async_control_deadline(&self) -> Option<tokio::time::Instant> {
        let waiting = self.async_control_wait?;
        (self.control_mode == 2 && waiting.tick == self.coordinator.current_tick())
            .then(|| waiting.deadline(self.config.async_max_wait_frames))
    }
}
