//! Client session loop: route driving, control replay/recovery, resource dispatch, membership.
//!
//! Moved byte-verbatim from `session.rs` (wave 2 of the decomposition
//! campaign, see REFACTOR_PLAN.md). Structural only.

use super::*;

struct ReceivedControlDeduplicator {
    entries: BTreeSet<(Tick, ClientId)>,
    highest_tick: Option<Tick>,
    last_pruned_highest: Option<Tick>,
    backlog_limit: Tick,
    #[cfg(test)]
    prune_passes: usize,
}

impl ReceivedControlDeduplicator {
    fn new(backlog_limit: usize) -> Self {
        Self {
            entries: BTreeSet::new(),
            highest_tick: None,
            last_pruned_highest: None,
            backlog_limit: Tick::try_from(backlog_limit).unwrap_or(Tick::MAX),
            #[cfg(test)]
            prune_passes: 0,
        }
    }

    fn seed(&mut self, client_id: ClientId, tick: Tick) -> bool {
        if !self.entries.insert((tick, client_id)) {
            return false;
        }
        self.highest_tick = Some(self.highest_tick.map_or(tick, |highest| highest.max(tick)));
        true
    }

    fn insert(&mut self, client_id: ClientId, tick: Tick) -> bool {
        if !self.entries.insert((tick, client_id)) {
            return false;
        }

        let highest = self.highest_tick.map_or(tick, |highest| highest.max(tick));
        self.highest_tick = Some(highest);
        let threshold = highest.saturating_sub(self.backlog_limit);
        if tick < threshold {
            self.entries.remove(&(tick, client_id));
        }
        if self.last_pruned_highest != Some(highest) {
            self.entries = self.entries.split_off(&(threshold, ClientId::MIN));
            self.last_pruned_highest = Some(highest);
            #[cfg(test)]
            {
                self.prune_passes += 1;
            }
        }
        true
    }

    #[cfg(test)]
    fn prune_passes(&self) -> usize {
        self.prune_passes
    }

    #[cfg(test)]
    fn retained_len(&self) -> usize {
        self.entries.len()
    }
}

enum ClientRouteWriterExit {
    Cancelled,
    OutboundClosed,
    Failed(String),
}

async fn run_client_route_writer<W>(
    mut transport: crate::ControlTransport<W>,
    mut outbound_rx: mpsc::UnboundedReceiver<ClientRouteCommand>,
    mut cancel_rx: watch::Receiver<bool>,
) -> ClientRouteWriterExit
where
    W: AsyncWrite + Unpin,
{
    let exit = loop {
        let command = tokio::select! {
            biased;
            command = outbound_rx.recv() => {
                let Some(command) = command else {
                    break ClientRouteWriterExit::OutboundClosed;
                };
                command
            }
            _ = wait_for_route_retirement(&mut cancel_rx) => {
                break ClientRouteWriterExit::Cancelled;
            }
        };
        let message = match command {
            ClientRouteCommand::Message(message) => message,
            ClientRouteCommand::Flush(completion) => {
                let _ = completion.send(());
                continue;
            }
        };
        // Log before the cancellable socket write, matching
        // C4Network2IOConnection::Send (oracle-src-pinned
        // src/C4Network2IO.cpp:1451-1491).
        let frame = match transport.prepare_message_frame(message) {
            Ok(frame) => frame,
            Err(error) => break ClientRouteWriterExit::Failed(format!("send failed: {error}")),
        };
        let result = tokio::select! {
            biased;
            _ = wait_for_route_retirement(&mut cancel_rx) => {
                break ClientRouteWriterExit::Cancelled;
            }
            result = transport.send_prepared_frame(&frame) => result,
        };
        if let Err(error) = result {
            break ClientRouteWriterExit::Failed(format!("send failed: {error}"));
        }
    };

    // Every logical send accepted before route failure belongs in the one
    // PostMortem suffix, even if its frame never reached the socket.
    outbound_rx.close();
    while let Ok(command) = outbound_rx.try_recv() {
        if let ClientRouteCommand::Message(message) = command {
            let _ = transport.retain_unsent_message(message);
        }
    }
    exit
}

// A route task owns each endpoint identifier, channel, transport, and liveness
// handle independently; retaining those arguments makes the ownership transfer
// at task spawn explicit.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_client_route<S>(
    local_connection_id: u32,
    remote_connection_id: u32,
    peer_addr: Option<SocketAddr>,
    transport: crate::ControlTransport<S>,
    route_tx: mpsc::UnboundedSender<ClientRouteCommand>,
    outbound_rx: mpsc::UnboundedReceiver<ClientRouteCommand>,
    mut retire_rx: watch::Receiver<bool>,
    event_tx: mpsc::UnboundedSender<ClientRouteEvent>,
    mut liveness: ConnectionLivenessState,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut transport, writer) = transport.into_split();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let mut writer_task = tokio::spawn(run_client_route_writer(writer, outbound_rx, cancel_rx));
    let mut writer_finished = false;
    let mut publish_disconnect = true;
    let mut liveness_timer = new_liveness_timer(liveness.next_timer_at());
    let reason = loop {
        let liveness_deadline = liveness.next_timer_at();
        if liveness_timer.deadline() != liveness_deadline {
            liveness_timer.as_mut().reset(liveness_deadline);
        }
        tokio::select! {
            _ = wait_for_route_retirement(&mut retire_rx) => break None,
            writer_result = &mut writer_task => {
                writer_finished = true;
                match writer_result {
                    Ok(ClientRouteWriterExit::Cancelled) => break None,
                    Ok(ClientRouteWriterExit::OutboundClosed) => {
                        publish_disconnect = false;
                        break None;
                    }
                    Ok(ClientRouteWriterExit::Failed(reason)) => break Some(reason),
                    Err(error) => break Some(format!("route writer task failed: {error}")),
                }
            }
            packet = transport.read_packet() => {
                let packet = match packet {
                    Ok(packet) => packet,
                    Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                        break None;
                    }
                    Err(error) => break Some(format!("read failed: {error}")),
                };
                match packet {
                    crate::transport::InboundPacket::Message(ControlMessage::Ping(packet)) => {
                        liveness.record_inbound_message(&ControlMessage::Ping(packet));
                        if route_tx
                            .send(ClientRouteCommand::Message(ControlMessage::Pong(packet)))
                            .is_err()
                        {
                            break Some("pong send failed: route writer closed".to_string());
                        }
                    }
                    crate::transport::InboundPacket::Message(ControlMessage::Pong(packet)) => {
                        liveness.record_inbound_message(&ControlMessage::Pong(packet));
                        let round_trip_ms = liveness.record_pong(packet);
                        if event_tx
                            .send(ClientRouteEvent::PingMeasured {
                                route_id: local_connection_id,
                                round_trip_ms,
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                    crate::transport::InboundPacket::Message(message) => {
                        liveness.record_inbound_message(&message);
                        if event_tx
                            .send(ClientRouteEvent::Packet {
                                route_id: local_connection_id,
                                peer_addr,
                                packet: crate::transport::InboundPacket::Message(message),
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                    crate::transport::InboundPacket::Ignored(packet_type) => {
                        liveness.record_inbound_packet(packet_type);
                        if event_tx
                            .send(ClientRouteEvent::Packet {
                                route_id: local_connection_id,
                                peer_addr,
                                packet: crate::transport::InboundPacket::Ignored(packet_type),
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                    crate::transport::InboundPacket::Empty => {}
                    crate::transport::InboundPacket::Invalid { packet_type, error } => {
                        liveness.record_inbound_packet(packet_type);
                        if event_tx
                            .send(ClientRouteEvent::Packet {
                                route_id: local_connection_id,
                                peer_addr,
                                packet: crate::transport::InboundPacket::Invalid {
                                    packet_type,
                                    error,
                                },
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                }
            }
            _ = liveness_timer.as_mut() => {
                let ping = match liveness.timer_tick() {
                    Ok(ping) => ping,
                    Err(timeout) => {
                        break Some(format!("connection {timeout:?} timeout"));
                    }
                };
                if let Some(ping) = ping {
                    let sent = route_tx
                        .send(ClientRouteCommand::Message(ControlMessage::Ping(ping)));
                    liveness.record_ping_dispatched();
                    if sent.is_err() {
                        break Some("ping send failed: route writer closed".to_string());
                    }
                    if event_tx
                        .send(ClientRouteEvent::PingDispatched {
                            route_id: local_connection_id,
                        })
                        .is_err()
                    {
                        publish_disconnect = false;
                        break None;
                    }
                }
            }
        }
    };
    cancel_tx.send_replace(true);
    drop(route_tx);
    if !writer_finished {
        let _ = writer_task.await;
    }
    if publish_disconnect {
        let next_outbound_packet = transport.outbound_packet_counter();
        let post_mortem = transport.create_post_mortem(remote_connection_id);
        let _ = event_tx.send(ClientRouteEvent::Disconnected {
            route_id: local_connection_id,
            next_inbound_packet: liveness.connection().inbound_packet_counter(),
            next_outbound_packet,
            post_mortem,
            reason,
        });
    }
}

// Established UDP routes enqueue directly into their endpoint's logical
// outbox. The route task keeps ownership of reads and liveness, but no longer
// needs a second writer task or per-route command wake.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_udp_client_route<S>(
    local_connection_id: u32,
    remote_connection_id: u32,
    peer_addr: Option<SocketAddr>,
    transport: crate::ControlTransport<S>,
    outbound: crate::udp_session::ReliableUdpRouteSender,
    mut retire_rx: watch::Receiver<bool>,
    event_tx: mpsc::UnboundedSender<ClientRouteEvent>,
    mut liveness: ConnectionLivenessState,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut transport, writer) = transport.into_split();
    drop(writer);
    let mut publish_disconnect = true;
    let mut liveness_timer = new_liveness_timer(liveness.next_timer_at());
    let reason = loop {
        let liveness_deadline = liveness.next_timer_at();
        if liveness_timer.deadline() != liveness_deadline {
            liveness_timer.as_mut().reset(liveness_deadline);
        }
        tokio::select! {
            _ = wait_for_route_retirement(&mut retire_rx) => break None,
            packet = transport.read_packet() => {
                let packet = match packet {
                    Ok(packet) => packet,
                    Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                        break None;
                    }
                    Err(error) => break Some(format!("read failed: {error}")),
                };
                match packet {
                    crate::transport::InboundPacket::Message(ControlMessage::Ping(packet)) => {
                        liveness.record_inbound_message(&ControlMessage::Ping(packet));
                        if outbound.try_send(ControlMessage::Pong(packet)).is_err() {
                            break Some("pong send failed: UDP outbox closed".to_string());
                        }
                    }
                    crate::transport::InboundPacket::Message(ControlMessage::Pong(packet)) => {
                        liveness.record_inbound_message(&ControlMessage::Pong(packet));
                        let round_trip_ms = liveness.record_pong(packet);
                        if event_tx
                            .send(ClientRouteEvent::PingMeasured {
                                route_id: local_connection_id,
                                round_trip_ms,
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                    crate::transport::InboundPacket::Message(message) => {
                        liveness.record_inbound_message(&message);
                        if event_tx
                            .send(ClientRouteEvent::Packet {
                                route_id: local_connection_id,
                                peer_addr,
                                packet: crate::transport::InboundPacket::Message(message),
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                    crate::transport::InboundPacket::Ignored(packet_type) => {
                        liveness.record_inbound_packet(packet_type);
                        if event_tx
                            .send(ClientRouteEvent::Packet {
                                route_id: local_connection_id,
                                peer_addr,
                                packet: crate::transport::InboundPacket::Ignored(packet_type),
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                    crate::transport::InboundPacket::Empty => {}
                    crate::transport::InboundPacket::Invalid { packet_type, error } => {
                        liveness.record_inbound_packet(packet_type);
                        if event_tx
                            .send(ClientRouteEvent::Packet {
                                route_id: local_connection_id,
                                peer_addr,
                                packet: crate::transport::InboundPacket::Invalid {
                                    packet_type,
                                    error,
                                },
                            })
                            .is_err()
                        {
                            publish_disconnect = false;
                            break None;
                        }
                    }
                }
            }
            _ = liveness_timer.as_mut() => {
                let ping = match liveness.timer_tick() {
                    Ok(ping) => ping,
                    Err(timeout) => break Some(format!("connection {timeout:?} timeout")),
                };
                if let Some(ping) = ping {
                    let sent = outbound.try_send(ControlMessage::Ping(ping));
                    liveness.record_ping_dispatched();
                    if sent.is_err() {
                        break Some("ping send failed: UDP outbox closed".to_string());
                    }
                    if event_tx
                        .send(ClientRouteEvent::PingDispatched {
                            route_id: local_connection_id,
                        })
                        .is_err()
                    {
                        publish_disconnect = false;
                        break None;
                    }
                }
            }
        }
    };
    outbound.retire();
    outbound.wait_drained().await;
    if publish_disconnect {
        let next_outbound_packet = transport.outbound_packet_counter();
        let post_mortem = transport.create_post_mortem(remote_connection_id);
        let _ = event_tx.send(ClientRouteEvent::Disconnected {
            route_id: local_connection_id,
            next_inbound_packet: liveness.connection().inbound_packet_counter(),
            next_outbound_packet,
            post_mortem,
            reason,
        });
    }
}

async fn replay_client_controls(
    transport: &mut ClientRouteManager,
    backlog: &ControlBacklog,
    control: &mut ClientControlState,
    local_client_id: ClientId,
    from_tick: Tick,
) -> Result<Vec<ControlPacket>, String> {
    let mut ready = Vec::new();
    for packet in contiguous_client_controls(backlog, local_client_id, from_tick) {
        let message = decentral_control_message(&packet).map_err(|error| error.to_string())?;
        transport
            .send_message(message)
            .await
            .map_err(|error| error.to_string())?;
        ready.extend(control.ingest_contribution(packet)?);
    }
    Ok(ready)
}

pub(crate) fn eligible_client_recovery_tick(
    resource_state: &ClientResourceState,
    backlog: &ControlBacklog,
) -> Option<Tick> {
    let request_tick = resource_state.control.recovery_tick()?;
    let local_client_id = ClientId::try_from(resource_state.catalog.local_client_id()).ok();
    let local_activated =
        local_client_id.is_some_and(|client_id| resource_state.control.is_registered(client_id));
    (!local_activated
        || local_client_id
            .is_some_and(|client_id| backlog.contains_packet(client_id, request_tick)))
    .then_some(request_tick)
}

pub(crate) async fn send_client_recovery_request(
    transport: &mut ClientRouteManager,
    control_mode: i32,
    from_tick: Tick,
) -> Result<(), TransportError> {
    let request = ControlMessage::Request { from_tick };
    if control_mode != 1 {
        transport.send_to_connected_peers(request.clone());
    }
    transport.send_message(request).await
}

async fn publish_client_ready(
    ready: Vec<ControlPacket>,
    postpone_control_request: bool,
    backlog: &mut ControlBacklog,
    next_control_request_at: &mut tokio::time::Instant,
    event_tx: &mpsc::Sender<ClientEvent>,
) {
    if postpone_control_request && !ready.is_empty() {
        let postponed = tokio::time::Instant::now() + CONTROL_REQUEST_INTERVAL;
        if *next_control_request_at < postponed {
            *next_control_request_at = postponed;
        }
    }
    for packet in ready {
        backlog.record_packet(&packet);
        let _ = event_tx.send(ClientEvent::Ready { packet }).await;
    }
}

async fn receive_optional_voice_media(
    events: &mut Option<mpsc::Receiver<crate::udp_session::ReliableUdpVoiceDatagram>>,
) -> crate::udp_session::ReliableUdpVoiceDatagram {
    if let Some(events) = events.as_mut() {
        if let Some(event) = events.recv().await {
            return event;
        }
    }
    std::future::pending().await
}

fn client_voice_available(transport: &ClientRouteManager) -> bool {
    !transport.authenticated_voice_send_routes().is_empty()
}

fn send_client_voice_frame(
    frame: crate::VoiceFrame,
    local_client_id: ClientId,
    transport: &ClientRouteManager,
    udp_handle: Option<&crate::ReliableUdpSessionHandle>,
) {
    let Some(udp_handle) = udp_handle else {
        return;
    };
    let frame = frame.with_authenticated_source(local_client_id);
    let routes = transport.authenticated_voice_send_routes();
    let mut direct_recipients = Vec::new();
    for (peer_id, peer, cookie) in routes
        .iter()
        .copied()
        .filter(|(peer_id, _, _)| *peer_id != HOST_CLIENT_ID)
    {
        if direct_recipients.len() == crate::voice::MAX_VOICE_DIRECT_RECIPIENTS {
            break;
        }
        let Ok(wire) = crate::voice::encode_authenticated_voice_packet(
            cookie,
            &crate::voice::VoicePacket::Direct(frame.clone()),
        ) else {
            continue;
        };
        if udp_handle.try_send_voice_media(peer, wire) {
            direct_recipients.push(peer_id);
        }
    }
    let Some((_, host_peer, host_cookie)) = routes
        .into_iter()
        .find(|(peer_id, _, _)| *peer_id == HOST_CLIENT_ID)
    else {
        return;
    };
    let relay = crate::voice::VoicePacket::RelayRequest {
        frame,
        direct_recipients,
    };
    if let Ok(wire) = crate::voice::encode_authenticated_voice_packet(host_cookie, &relay) {
        let _ = udp_handle.try_send_voice_media(host_peer, wire);
    }
}

fn handle_client_voice_media(
    media: crate::udp_session::ReliableUdpVoiceDatagram,
    transport: &ClientRouteManager,
    voice_events: &mpsc::Sender<crate::VoiceFrame>,
    known_clients: &BTreeMap<i32, clonk_engine::ClientCoreControlData>,
    limiter: &mut crate::voice::VoiceIngressLimiter,
) {
    let Some((ingress_peer_id, receive_cookie)) = transport.authenticated_voice_ingress(media.peer)
    else {
        return;
    };
    let Ok(packet) =
        crate::voice::decode_authenticated_voice_packet(&media.payload, receive_cookie)
    else {
        return;
    };
    let Some(frame) = crate::voice::authenticate_client_ingress(
        ingress_peer_id,
        ingress_peer_id == HOST_CLIENT_ID,
        packet,
    ) else {
        return;
    };
    if !i32::try_from(frame.client_id)
        .ok()
        .is_some_and(|client_id| known_clients.contains_key(&client_id))
    {
        return;
    }
    if !limiter.allow(frame.client_id, Instant::now()) {
        return;
    }
    let _ = voice_events.try_send(frame);
}

#[cfg(test)]
pub(crate) async fn run_client_loop_with_addresses<S>(
    transport: crate::ControlTransport<S>,
    commands: mpsc::Receiver<ClientCommand>,
    event_tx: mpsc::Sender<ClientEvent>,
    shutdown_rx: oneshot::Receiver<()>,
    host_peer_addr: Option<SocketAddr>,
    client_addresses: BTreeMap<i32, Vec<crate::NetworkAddress>>,
    resource_state: ClientResourceState,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let liveness = resource_state.liveness.clone();
    let mut routes = ClientRouteManager::new();
    routes.add_route(
        0,
        0,
        crate::NetworkProtocol::Tcp,
        host_peer_addr,
        transport,
        liveness,
    );
    let (_voice_command_tx, voice_commands) =
        mpsc::channel::<crate::VoiceFrame>(VOICE_APP_CHANNEL_CAPACITY);
    let (voice_events, _voice_event_rx) =
        mpsc::channel::<crate::VoiceFrame>(VOICE_APP_CHANNEL_CAPACITY);
    let voice_available = Arc::new(std::sync::atomic::AtomicBool::new(false));
    run_client_loop_with_routes(
        routes,
        crate::NetworkIoStatistics::new(network_statistics_now_ms()),
        commands,
        ControlSendTimeSnapshot::default(),
        crate::ControlWaitAttributionSnapshot::default(),
        event_tx,
        voice_commands,
        voice_events,
        voice_available,
        shutdown_rx,
        host_peer_addr,
        client_addresses,
        BTreeMap::new(),
        BTreeMap::new(),
        ClientHandshakeRequestTemplate::new(
            clonk_engine::ClientCoreControlData::default(),
            CURRENT_GAME_BUILD,
            clonk_engine::LegacyCString::default(),
        ),
        Arc::new(AtomicU32::new(1)),
        Vec::new(),
        None,
        None,
        None,
        0,
        0,
        resource_state,
        None,
        None,
        None,
        None,
    )
    .await;
}

// This long-lived task is the ownership boundary for independent network
// routes, recovery state, resource state, and application channels. A wrapper
// struct would merely move this one-time destructuring into the function body.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_client_loop_with_routes(
    mut transport: ClientRouteManager,
    io_statistics: crate::NetworkIoStatistics,
    mut commands: mpsc::Receiver<ClientCommand>,
    control_send_time: ControlSendTimeSnapshot,
    control_wait_attribution: crate::ControlWaitAttributionSnapshot,
    event_tx: mpsc::Sender<ClientEvent>,
    mut voice_commands: mpsc::Receiver<crate::VoiceFrame>,
    voice_events: mpsc::Sender<crate::VoiceFrame>,
    voice_available: Arc<std::sync::atomic::AtomicBool>,
    mut shutdown_rx: oneshot::Receiver<()>,
    host_peer_addr: Option<SocketAddr>,
    mut client_addresses: BTreeMap<i32, Vec<crate::NetworkAddress>>,
    mut client_cores: BTreeMap<i32, clonk_engine::ClientCoreControlData>,
    mut mesh_peers: BTreeMap<i32, crate::ClientMeshPeerState>,
    mesh_request_template: ClientHandshakeRequestTemplate,
    connection_ids: Arc<AtomicU32>,
    mesh_interface_ids: Vec<u32>,
    mut mesh_tcp_listener: Option<TcpListener>,
    mut mesh_udp_hub: Option<crate::ReliableUdpSessionHub>,
    mut mesh_puncher_events: Option<mpsc::Receiver<crate::NetpuncherIoEvent>>,
    mesh_tcp_port: u16,
    mesh_udp_port: u16,
    mut resource_state: ClientResourceState,
    mut udp_reconnect: Option<ClientUdpReconnect>,
    mut pending_secondary: Option<PendingClientRoute>,
    mut tcp_reconnect: Option<ClientTcpReconnect>,
    mut pending_tcp: Option<PendingTcpClientRoute>,
) {
    let local_core = mesh_request_template.local_core.clone();
    // What the host announced it can do beyond the C++ protocol. Empty until it
    // says so, which is exactly where a stock C++ host leaves it.
    let mut backlog = ControlBacklog::new(CLIENT_BACKLOG_LIMIT);
    let mut client_performance = ClientPerformanceStats::new(CLIENT_BACKLOG_LIMIT);
    let mut next_control_request_at = resource_state.next_control_request_at;
    let mut peer_recovery_from_tick = None::<Tick>;
    let mut pending_sync = Vec::<clonk_engine::ControlPacket>::new();
    let mut received_controls = ReceivedControlDeduplicator::new(CLIENT_BACKLOG_LIMIT);
    let mut resource_timer = interval(Duration::from_millis(crate::NETWORK_TIMER_INTERVAL_MS));
    let mut udp_retry_at = None::<tokio::time::Instant>;
    let mut tcp_retry_at = None::<tokio::time::Instant>;
    let mesh_epoch = Instant::now();
    let mut pending_mesh_routes = tokio::task::JoinSet::<MeshRouteCompletion>::new();
    let mut active_mesh_dials = BTreeSet::<MeshDialKey>::new();
    let mut pending_tcp_sim_open = BTreeMap::<i32, tokio::net::TcpSocket>::new();
    let mesh_udp_handle = mesh_udp_hub
        .as_ref()
        .map(crate::ReliableUdpSessionHub::handle);
    let mut voice_media = mesh_udp_hub
        .as_mut()
        .map(crate::ReliableUdpSessionHub::take_voice_media_receiver);
    let mut voice_ingress_limiter = crate::voice::VoiceIngressLimiter::default();
    let mut mesh_udp_accept_enabled = mesh_udp_hub.is_some();

    if let Some(local_addresses) = client_addresses
        .get(&local_core.client_id)
        .filter(|addresses| !addresses.is_empty())
    {
        let _ = event_tx
            .send(ClientEvent::LocalAddressesChanged {
                local_addresses: local_addresses.clone(),
            })
            .await;
    }

    for (core, path) in std::mem::take(&mut resource_state.initial_complete_resources) {
        let _ = event_tx
            .send(ClientEvent::ResourceComplete {
                resource_id: core.id,
                core,
                path,
                local: true,
            })
            .await;
    }

    for packet in std::mem::take(&mut resource_state.initial_controls) {
        let key = (packet.client_id(), packet.tick());
        if received_controls.seed(key.0, key.1) {
            client_performance.record_arrival(key.0, key.1, tokio::time::Instant::now());
            let backlog_packet = packet.clone();
            let ready = match resource_state.control.accept_network(packet) {
                Ok(ready) => ready,
                Err(error) => {
                    let _ = event_tx
                        .send(ClientEvent::Disconnected {
                            reason: Some(format!("invalid initial control packet: {error}")),
                        })
                        .await;
                    return;
                }
            };
            backlog.record_packet(&backlog_packet);
            publish_client_ready(
                ready,
                resource_state.control.mode == 0,
                &mut backlog,
                &mut next_control_request_at,
                &event_tx,
            )
            .await;
        }
    }

    for packet in std::mem::take(&mut resource_state.initial_ready_checks) {
        if packet.data.vote_requested() && packet.client_id != 0 {
            continue;
        }
        let _ = event_tx.send(ClientEvent::ReadyCheck { packet }).await;
    }

    for packet in std::mem::take(&mut resource_state.initial_lobby_countdowns) {
        let _ = event_tx.send(ClientEvent::LobbyCountdown { packet }).await;
    }

    for packet in std::mem::take(&mut resource_state.initial_packets) {
        let now_seconds = resource_state.resource_epoch.elapsed().as_secs();
        let result = if let Some(backend) = resource_state.backend.as_mut() {
            let mut random = resource_safe_random;
            match backend.on_packet(
                resource_state.host_peer_id,
                &packet,
                now_seconds,
                &mut random,
            ) {
                Ok(events) => dispatch_client_resource_events(
                    events,
                    &mut resource_state,
                    &mut transport,
                    &event_tx,
                )
                .await
                .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            }
        } else {
            let actions = resource_state
                .catalog
                .on_packet(resource_state.host_peer_id, &packet);
            dispatch_client_resource_actions(
                actions,
                &mut resource_state,
                &mut transport,
                &event_tx,
            )
            .await
            .map_err(|error| error.to_string())
        };
        if let Err(error) = result {
            let _ = event_tx
                .send(ClientEvent::Disconnected {
                    reason: Some(format!("resource bootstrap failed: {error}")),
                })
                .await;
            return;
        }
    }

    'outer: loop {
        voice_ingress_limiter.retain_sources(
            client_cores
                .keys()
                .filter_map(|client_id| ClientId::try_from(*client_id).ok()),
        );
        voice_available.store(
            client_voice_available(&transport),
            std::sync::atomic::Ordering::Release,
        );
        if transport.control_send_time_needs_publish() {
            let local_client_id = ClientId::try_from(resource_state.catalog.local_client_id())
                .unwrap_or(HOST_CLIENT_ID);
            let known_clients = client_cores
                .keys()
                .filter_map(|client_id| ClientId::try_from(*client_id).ok())
                .collect();
            transport.publish_control_send_time(
                &control_send_time,
                resource_state.control.mode,
                local_client_id,
                known_clients,
            );
        }
        let has_pending_secondary = pending_secondary.is_some();
        let has_pending_tcp = pending_tcp.is_some();
        let has_pending_mesh_route = !pending_mesh_routes.is_empty();
        let mesh_tcp_available = mesh_tcp_listener.is_some();
        let mesh_udp_available = mesh_udp_accept_enabled;
        let can_accept_mesh_tcp =
            mesh_tcp_available && pending_mesh_routes.len() < CLIENT_MESH_PENDING_LIMIT;
        let can_accept_mesh_udp =
            mesh_udp_available && pending_mesh_routes.len() < CLIENT_MESH_PENDING_LIMIT;
        let udp_retry_deadline = udp_retry_at.unwrap_or_else(tokio::time::Instant::now);
        let tcp_retry_deadline = tcp_retry_at.unwrap_or_else(tokio::time::Instant::now);
        let control_recovery_tick = eligible_client_recovery_tick(&resource_state, &backlog);
        let control_request_deadline = next_control_request_at;
        // Mesh setup and retry sources can all be ready during a join storm.
        // Give an already-queued game command deterministic service; a
        // command racing this snapshot waits for at most one network branch.
        let command_pending = !commands.is_empty();
        let voice_media_ready = transport.voice_enabled()
            && crate::voice::voice_media_may_run(command_pending, transport.has_pending_input());
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => break,
            _ = tokio::time::sleep_until(control_request_deadline), if !command_pending && control_recovery_tick.is_some() => {
                let from_tick = control_recovery_tick.expect("guarded recovery tick exists");
                let control_mode = resource_state.control.mode;
                let _ = send_client_recovery_request(
                    &mut transport,
                    control_mode,
                    from_tick,
                )
                .await;
                if control_mode != 1 {
                    // A repeated request can race ahead of replies to the
                    // prior request. Keep the earliest outstanding tick so
                    // those trailing partial or complete replies stay valid.
                    extend_peer_recovery_window(&mut peer_recovery_from_tick, from_tick);
                }
                next_control_request_at =
                    tokio::time::Instant::now() + CONTROL_REQUEST_INTERVAL;
            }
            completed = pending_mesh_routes.join_next(), if !command_pending && has_pending_mesh_route => {
                match completed {
                    Some(Ok(completion)) => {
                        if let Some(dial_key) = completion.dial_key {
                            active_mesh_dials.remove(&dial_key);
                        }
                        if let Ok(route) = completion.result {
                            if connected_mesh_route_matches_registry(&route, &client_cores) {
                                if let Some(peer_id) =
                                    add_connected_mesh_route(route, &mut transport)
                                {
                                    if let Err(error) = dispatch_client_resource_peer_connected(
                                        peer_id,
                                        &mut resource_state,
                                        &mut transport,
                                        &event_tx,
                                    )
                                    .await
                                    {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!(
                                                    "peer resource discovery failed: {error}"
                                                )),
                                            })
                                            .await;
                                        break 'outer;
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(_)) => {}
                    None => {}
                }
            }
            puncher_event = receive_optional_puncher_event(&mut mesh_puncher_events), if !command_pending => {
                let Some(puncher_event) = puncher_event else {
                    mesh_puncher_events = None;
                    continue;
                };
                if let crate::NetpuncherIoEvent::Connected {
                    observed_address,
                    ..
                } = puncher_event
                {
                    let update = mesh_peers
                        .entry(local_core.client_id)
                        .or_default()
                        .add_address_from_puncher(
                            observed_address,
                            mesh_udp_port,
                            mesh_tcp_port,
                            mesh_epoch.elapsed(),
                        );
                    let addresses_changed = !update.announcements.is_empty();
                    for address in update.announcements {
                        let addresses = client_addresses
                            .entry(local_core.client_id)
                            .or_default();
                        if !addresses.contains(&address) {
                            addresses.insert(0, address);
                        }
                        let packet = crate::AddressPacket {
                            client_id: local_core.client_id,
                            address,
                        };
                        let _ = transport
                            .send_to_connected_peers(ControlMessage::Address(packet));
                        let _ = transport
                            .send_message(ControlMessage::Address(packet))
                            .await;
                    }
                    if addresses_changed {
                        if let Some(local_addresses) = client_addresses.get(&local_core.client_id) {
                            let _ = event_tx
                                .send(ClientEvent::LocalAddressesChanged {
                                    local_addresses: local_addresses.clone(),
                                })
                                .await;
                        }
                    }
                }
            }
            incoming = accept_optional_mesh_tcp(&mut mesh_tcp_listener), if !command_pending && can_accept_mesh_tcp => {
                if let Some(Ok((stream, peer_addr))) = incoming {
                    let connection_id = connection_ids.fetch_add(1, AtomicOrdering::Relaxed);
                    let mut known_peers = client_cores.clone();
                    known_peers.remove(&local_core.client_id);
                    let request_template = mesh_request_template.clone();
                    let io_statistics = io_statistics.clone();
                    pending_mesh_routes.spawn(async move {
                        let result = accept_mesh_tcp_route(
                            stream,
                            peer_addr,
                            request_template,
                            known_peers,
                            connection_id,
                            io_statistics,
                        )
                        .await;
                        MeshRouteCompletion {
                            dial_key: None,
                            result,
                        }
                    });
                }
            }
            incoming = accept_optional_mesh_udp(&mut mesh_udp_hub), if !command_pending && can_accept_mesh_udp => {
                match incoming {
                    Some(Ok(stream)) => {
                        let connection_id = connection_ids.fetch_add(1, AtomicOrdering::Relaxed);
                        let mut known_peers = client_cores.clone();
                        known_peers.remove(&local_core.client_id);
                        let request_template = mesh_request_template.clone();
                        pending_mesh_routes.spawn(async move {
                            let result = accept_mesh_udp_route(
                                stream,
                                request_template,
                                known_peers,
                                connection_id,
                            )
                            .await;
                            MeshRouteCompletion {
                                dial_key: None,
                                result,
                            }
                        });
                    }
                    Some(Err(_)) => {
                        mesh_udp_accept_enabled = false;
                        udp_reconnect = None;
                        if let Some(task) = pending_secondary.take() {
                            task.abort();
                        }
                    }
                    None => {}
                }
            }
            route = await_pending_client_route(&mut pending_secondary), if !command_pending && has_pending_secondary => {
                pending_secondary.take();
                if let Some(mut route) = route {
                    udp_retry_at = None;
                    let outbound = route
                        .udp_outbound
                        .take()
                        .expect("established secondary UDP route has an outbox sender");
                    transport.add_udp_route(
                        route.local_connection_id,
                        route.remote_connection_id,
                        Some(route.peer_addr),
                        route.transport,
                        route.liveness,
                        outbound,
                    );
                } else if udp_reconnect.is_some() {
                    udp_retry_at = Some(tokio::time::Instant::now() + CLIENT_ROUTE_RETRY_INTERVAL);
                }
            }
            _ = tokio::time::sleep_until(udp_retry_deadline), if !command_pending && udp_retry_at.is_some() => {
                udp_retry_at = None;
                if pending_secondary.is_none() {
                    if let Some(reconnect) = udp_reconnect.as_mut() {
                        pending_secondary = Some(reconnect.start());
                    }
                }
            }
            route = await_pending_tcp_client_route(&mut pending_tcp), if !command_pending && has_pending_tcp => {
                pending_tcp.take();
                if let Some(route) = route {
                    tcp_retry_at = None;
                    transport.add_route(
                        route.local_connection_id,
                        route.remote_connection_id,
                        crate::NetworkProtocol::Tcp,
                        Some(route.peer_addr),
                        route.transport,
                        route.liveness,
                    );
                } else if tcp_reconnect.is_some() {
                    tcp_retry_at = Some(tokio::time::Instant::now() + CLIENT_ROUTE_RETRY_INTERVAL);
                }
            }
            _ = tokio::time::sleep_until(tcp_retry_deadline), if !command_pending && tcp_retry_at.is_some() => {
                tcp_retry_at = None;
                if pending_tcp.is_none() {
                    if let Some(reconnect) = tcp_reconnect.as_mut() {
                        pending_tcp = Some(reconnect.start());
                    }
                }
            }
            Some(command) = commands.recv() => {
                match command {
                    ClientCommand::SubmitStatusAck(status) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::StatusAck(status))
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                        resource_state.control.clear_target();
                    }
                    ClientCommand::SubmitReadyCheck(packet) => {
                        let raw_result = transport
                            .send_message(ControlMessage::ReadyCheck(packet))
                            .await;
                        let forward_result = match raw_result {
                            Ok(()) => {
                                let direct_peers = transport
                                    .send_to_connected_peers(ControlMessage::ReadyCheck(packet));
                                let nested_packet =
                                    crate::transport::encode_complete_ready_check_packet(packet);
                                let mut excluded = vec![resource_state.host_peer_id];
                                excluded.extend(
                                    direct_peers
                                        .into_iter()
                                        .filter_map(|peer_id| i32::try_from(peer_id).ok()),
                                );
                                transport
                                    .send_message(ControlMessage::ForwardRequest(
                                        crate::ForwardPacket {
                                            negative_list: true,
                                            clients: excluded,
                                            nested_packet,
                                        },
                                    ))
                                    .await
                            }
                            Err(error) => Err(error),
                        };
                        if let Err(error) = forward_result {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::RequestActivation(tick) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::ActivationRequest { tick })
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::SubmitPlayerInfoUpdate(request) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::PlayerInfoUpdate(request))
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::SubmitControl(packet) => {
                        let clone = packet.clone();
                        let message = if resource_state.control.mode == 0 {
                            let direct_peers = transport
                                .send_to_connected_peers(ControlMessage::Control(packet.clone()));
                            match decentral_control_message_to_unconnected(&packet, direct_peers) {
                                Ok(message) => message,
                                Err(error) => {
                                    let _ = event_tx
                                        .send(ClientEvent::Disconnected {
                                            reason: Some(format!("send failed: {error}")),
                                        })
                                        .await;
                                    break;
                                }
                            }
                        } else {
                            ControlMessage::Control(packet)
                        };
                        match transport.send_message(message).await {
                            Ok(()) => {
                                client_performance.record_arrival(
                                    clone.client_id(),
                                    clone.tick(),
                                    tokio::time::Instant::now(),
                                );
                                backlog.record_packet(&clone);
                                match resource_state.control.ingest_contribution(clone) {
                                    Ok(ready) => {
                                        publish_client_ready(
                                            ready,
                                            resource_state.control.mode == 0,
                                            &mut backlog,
                                            &mut next_control_request_at,
                                            &event_tx,
                                        )
                                        .await;
                                    }
                                    Err(error) => {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!(
                                                    "invalid local control packet: {error}"
                                                )),
                                            })
                                            .await;
                                        break;
                                    }
                                }
                            }
                            Err(error) => {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!("send failed: {error}")),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                    ClientCommand::SubmitPacket { delivery, data } => {
                        let message = if matches!(
                            delivery,
                            ControlDelivery::Direct | ControlDelivery::Private
                        ) {
                            let direct_peers = transport
                                .send_to_connected_peers(ControlMessage::Packet {
                                    delivery,
                                    data: data.clone(),
                                });
                            ControlMessage::ForwardRequest(crate::ForwardPacket {
                                negative_list: true,
                                clients: direct_peers
                                    .into_iter()
                                    .filter_map(|peer_id| i32::try_from(peer_id).ok())
                                    .collect(),
                                nested_packet:
                                    crate::transport::encode_complete_control_delivery_packet(
                                        delivery, &data,
                                    ),
                            })
                        } else {
                            ControlMessage::Packet { delivery, data }
                        };
                        if let Err(error) = transport.send_message(message).await {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::ExecSync { control_tick } => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::ExecSync { control_tick })
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::SubmitResource(packet) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::Resource(packet))
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("resource send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::RemoveResource {
                        resource_id,
                        completion,
                    } => {
                        let _ = completion.send(resource_state.remove_resource(resource_id));
                    }
                    ClientCommand::PublishPlayerResource {
                        request,
                        completion,
                    } => {
                        let result = resource_state.publish_player_resource(request);
                        let _ = completion.send(result);
                    }
                    ClientCommand::BeginResourceDerive {
                        resource_id,
                        source_path,
                        ownership,
                        completion,
                    } => {
                        let result = resource_state.begin_resource_derive(
                            resource_id,
                            source_path,
                            ownership,
                        );
                        let _ = completion.send(result);
                    }
                    ClientCommand::GracefulPart { completion } => {
                        let result = match transport
                            .send_message(ControlMessage::ConnectionReply(
                                crate::ConnectionReply {
                                    ok: false,
                                    message: clonk_engine::LegacyCString::from_bytes(
                                        b"removing client".to_vec(),
                                    )
                                    .unwrap_or_default(),
                                    wrong_password: false,
                                },
                            ))
                            .await
                        {
                            Ok(()) => transport.flush_to(HOST_CLIENT_ID).await,
                            Err(error) => Err(error),
                        }
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                        break;
                    }
                    ClientCommand::ControlTickReached { tick, reached_at } => {
                        resource_state
                            .control
                            .note_runtime_control_tick_reached(tick);
                        client_performance.record_cadence(tick, reached_at);
                    }
                    ClientCommand::ControlTickConsumed {
                        tick,
                        consumed_at,
                        client_ids,
                        reset_performance,
                    } => {
                        if reset_performance {
                            client_performance.reset_accumulators();
                        }
                        client_performance.mark_consumed(
                            tick,
                            consumed_at,
                            client_ids,
                        );
                    }
                    ClientCommand::InspectRuntimeClientStates {
                        tick,
                        reset_performance,
                        completion,
                    } => {
                        if reset_performance {
                            client_performance.reset_accumulators();
                        }
                        let client_ids = client_cores
                            .keys()
                            .filter_map(|client_id| ClientId::try_from(*client_id).ok());
                        let states = transport.runtime_client_states(
                            resource_state.control.mode,
                            tick,
                            client_ids,
                            &backlog,
                            &client_performance,
                        );
                        let _ = completion.send(states);
                    }
                    ClientCommand::InspectRuntimeConnections { completion } => {
                        let _ = completion.send(transport.runtime_connections());
                    }
                    ClientCommand::InspectLobbyClientTelemetry {
                        client_ids,
                        completion,
                    } => {
                        let catalog = resource_state
                            .backend
                            .as_ref()
                            .map(crate::ResourceTransferBackend::catalog)
                            .unwrap_or(&resource_state.catalog);
                        let telemetry = runtime_lobby_client_telemetry(
                            transport.runtime_connections(),
                            catalog,
                            client_ids,
                        );
                        let _ = completion.send(telemetry);
                    }
                    ClientCommand::DisconnectRuntimeConnection {
                        connection_id,
                        completion,
                    } => {
                        let disconnected =
                            transport.disconnect_runtime_connection(connection_id);
                        let _ = completion.send(disconnected);
                    }
                    #[cfg(test)]
                    ClientCommand::InspectMeshPeers { completion } => {
                        let peers = transport
                            .connected_peer_ids()
                            .into_iter()
                            .filter(|peer_id| *peer_id != HOST_CLIENT_ID)
                            .collect();
                        let _ = completion.send(peers);
                    }
                    #[cfg(test)]
                    ClientCommand::InspectMeshAddressCount {
                        peer_id,
                        completion,
                    } => {
                        let count = mesh_peers
                            .get(&peer_id)
                            .map_or(0, |peer| peer.addresses().len());
                        let _ = completion.send(count);
                    }
                    #[cfg(test)]
                    ClientCommand::ForceMeshAttempt {
                        peer_id,
                        completion,
                    } => {
                        if let (Ok(peer_id_wire), Some(peer)) =
                            (ClientId::try_from(peer_id), mesh_peers.get_mut(&peer_id))
                        {
                            let connectivity = transport.mesh_connectivity(
                                peer_id_wire,
                                mesh_tcp_available,
                                mesh_udp_available,
                            );
                            let now = peer.next_attempt_at().unwrap_or_else(|| mesh_epoch.elapsed());
                            if let crate::ClientMeshConnectDecision::Dial(attempt) =
                                peer.do_connect_attempt(now, connectivity)
                            {
                                spawn_mesh_dial(
                                    &mut pending_mesh_routes,
                                    &mut active_mesh_dials,
                                    peer_id,
                                    attempt,
                                    &client_cores,
                                    &mesh_request_template,
                                    &connection_ids,
                                    &mesh_interface_ids,
                                    mesh_udp_handle.as_ref(),
                                    &io_statistics,
                                );
                            }
                        }
                        let _ = completion.send(());
                    }
                    ClientCommand::Shutdown => break,
                }
            }
            _ = resource_timer.tick() => {
                io_statistics.generate_statistics(network_statistics_now_ms());
                transport.expire_closed_routes();
                let mesh_now = mesh_epoch.elapsed();
                let due_peers = mesh_peers
                    .iter()
                    .filter_map(|(peer_id, peer)| {
                        (*peer_id != local_core.client_id
                            && *peer_id != resource_state.host_peer_id
                            && peer.scheduled_attempt_due(mesh_now))
                        .then_some(*peer_id)
                    })
                    .collect::<Vec<_>>();
                for peer_id in due_peers {
                    let Ok(peer_id_wire) = ClientId::try_from(peer_id) else {
                        continue;
                    };
                    let connectivity = transport.mesh_connectivity(
                        peer_id_wire,
                        mesh_tcp_available,
                        mesh_udp_available,
                    );
                    let attempt = mesh_peers.get_mut(&peer_id).and_then(|peer| {
                        match peer.do_connect_attempt(mesh_now, connectivity) {
                            crate::ClientMeshConnectDecision::Dial(attempt) => Some(attempt),
                            crate::ClientMeshConnectDecision::NotDue { .. }
                            | crate::ClientMeshConnectDecision::Backoff { .. } => None,
                        }
                    });
                    if let Some(attempt) = attempt {
                        let local_puncher_address = mesh_peers
                            .get(&local_core.client_id)
                            .and_then(crate::ClientMeshPeerState::ipv6_address_from_puncher);
                        maybe_initiate_tcp_simultaneous_open(
                            &mut pending_tcp_sim_open,
                            pending_mesh_routes.len(),
                            &mut transport,
                            &local_core,
                            peer_id,
                            attempt,
                            local_puncher_address,
                        );
                        spawn_mesh_dial(
                            &mut pending_mesh_routes,
                            &mut active_mesh_dials,
                            peer_id,
                            attempt,
                            &client_cores,
                            &mesh_request_template,
                            &connection_ids,
                            &mesh_interface_ids,
                            mesh_udp_handle.as_ref(),
                            &io_statistics,
                        );
                    }
                }
                let now_seconds = resource_state.resource_epoch.elapsed().as_secs();
                if let Some(backend) = resource_state.backend.as_mut() {
                    let mut random = resource_safe_random;
                    match backend.on_timer(now_seconds, &mut random) {
                        Ok(events) => {
                            if let Err(error) = dispatch_client_resource_events(
                                events,
                                &mut resource_state,
                                &mut transport,
                                &event_tx,
                            )
                            .await
                            {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!("resource timer send failed: {error}")),
                                    })
                                    .await;
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("resource timer failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                } else {
                    let actions = resource_state.catalog.on_timer(now_seconds);
                    if let Err(error) = dispatch_client_resource_actions(
                        actions,
                        &mut resource_state,
                        &mut transport,
                        &event_tx,
                    )
                    .await
                    {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(format!("resource timer send failed: {error}")),
                            })
                            .await;
                        break;
                    }
                }
            }
            route_event = transport.read_event() => {
                let (ingress_peer_id, packet, ingress_peer_addr) = match route_event {
                    Ok(ClientRouteRead::Packet {
                        peer_id,
                        packet,
                        peer_addr,
                    }) => (peer_id, packet, peer_addr),
                    Ok(ClientRouteRead::PingMeasured {
                        peer_id,
                        round_trip_ms,
                    }) => {
                        if peer_id == HOST_CLIENT_ID {
                            let _ = event_tx
                                .send(ClientEvent::PingMeasured { round_trip_ms })
                                .await;
                        }
                        continue;
                    }
                    Ok(ClientRouteRead::Disconnected {
                        peer_id,
                        protocol,
                        routes_remaining,
                        post_mortem,
                        reason,
                    }) => {
                        if peer_id == HOST_CLIENT_ID && !routes_remaining {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(reason.unwrap_or_else(|| {
                                        "all client transport routes closed".to_string()
                                    })),
                                })
                                .await;
                            break;
                        }
                        if let Some(post_mortem) = post_mortem {
                            if peer_id != HOST_CLIENT_ID {
                                if let Ok(peer_id) = i32::try_from(peer_id) {
                                    if let Ok(nested_packet) =
                                        crate::transport::encode_complete_post_mortem_packet(
                                            &post_mortem,
                                        )
                                    {
                                        let _ = transport.try_send_to(
                                            HOST_CLIENT_ID,
                                            ControlMessage::ForwardRequest(
                                                crate::ForwardPacket {
                                                    negative_list: false,
                                                    clients: vec![peer_id],
                                                    nested_packet,
                                                },
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                        match (peer_id, protocol) {
                            (HOST_CLIENT_ID, crate::NetworkProtocol::Udp)
                                if pending_secondary.is_none() => {
                                if let Some(reconnect) = udp_reconnect.as_mut() {
                                    pending_secondary = Some(reconnect.start());
                                    udp_retry_at = None;
                                }
                            }
                            (HOST_CLIENT_ID, crate::NetworkProtocol::Tcp)
                                if pending_tcp.is_none() => {
                                if let Some(reconnect) = tcp_reconnect.as_mut() {
                                    pending_tcp = Some(reconnect.start());
                                    tcp_retry_at = None;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    Err(error) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(format!("read failed: {error}")),
                            })
                            .await;
                        break;
                    }
                };
                let result = match packet {
                    crate::transport::InboundPacket::Message(message) => Ok(message),
                    crate::transport::InboundPacket::Ignored(packet_type) => {
                        let _ = event_tx
                            .send(ClientEvent::UnhandledPacket { packet_type })
                            .await;
                        continue;
                    }
                    crate::transport::InboundPacket::Empty => continue,
                    crate::transport::InboundPacket::Invalid {
                        packet_type,
                        error,
                    } => {
                        let _ = packet_type;
                        if ingress_peer_id != HOST_CLIENT_ID {
                            transport.retire_peer(ingress_peer_id);
                            continue;
                        }
                        Err(error)
                    }
                };
                let result = match result {
                    Ok(ControlMessage::Forward(packet)) if ingress_peer_id == HOST_CLIENT_ID => {
                        let local_client_id = resource_state.catalog.local_client_id();
                        if !forward_selects(&packet, local_client_id) {
                            continue;
                        }
                        match crate::transport::parse_complete_packet(&packet.nested_packet) {
                            Ok(Some(message)) => Ok(message),
                            Ok(None) => {
                                let packet_type =
                                    packet.nested_packet.first().copied().unwrap_or_default();
                                let _ = event_tx
                                    .send(ClientEvent::UnhandledPacket { packet_type })
                                    .await;
                                continue;
                            }
                            Err(error) => Err(error),
                        }
                    }
                    other => other,
                };
                match result {
                    Ok(ControlMessage::PortCapabilities(_)) => {}
                    Ok(ControlMessage::ControlWaitAttribution(attribution)) => {
                        record_host_control_wait_attribution(
                            ingress_peer_id,
                            attribution,
                            &control_wait_attribution,
                        );
                    }
                    // Only the host restarts the session, so a notice relayed
                    // by a peer client says nothing about the host's intent.
                    Ok(ControlMessage::HostRestarting { .. })
                        if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::HostRestarting { rejoin_seconds }) => {
                        let _ = event_tx
                            .send(ClientEvent::HostRestarting { rejoin_seconds })
                            .await;
                    }
                    // Same authority rule as the reconnect notice above: only
                    // the host can end its own round.
                    Ok(ControlMessage::HostRestartLobby)
                        if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::HostRestartLobby) => {
                        let _ = event_tx.send(ClientEvent::HostRestartLobby).await;
                    }
                    Ok(ControlMessage::Ping(packet)) => {
                        if let Err(error) = transport.send_message(ControlMessage::Pong(packet)).await {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("pong send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    // Route workers own their liveness state and consume direct
                    // Pong packets. A nested forwarded Pong has no route-local
                    // round trip to measure here.
                    Ok(ControlMessage::Pong(_)) => {}
                    Ok(ControlMessage::ConnectionRequest(_))
                        if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::ConnectionRequest(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some("host sent a duplicate connection request".to_string()),
                            })
                            .await;
                        break;
                    }
                    Ok(ControlMessage::ConnectionReply(reply))
                        if ingress_peer_id != HOST_CLIENT_ID && !reply.ok =>
                    {
                        transport.retire_peer(ingress_peer_id);
                    }
                    Ok(ControlMessage::ConnectionReply(_))
                        if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::ConnectionReply(reply)) if !reply.ok => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(clonk_resources::decode_legacy_script_text(
                                    reply.message.as_bytes(),
                                )),
                            })
                            .await;
                        break;
                    }
                    Ok(ControlMessage::ConnectionReply(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some("host sent a duplicate connection reply".to_string()),
                            })
                            .await;
                        break;
                    }
                    Ok(ControlMessage::Forward(_)) if ingress_peer_id != HOST_CLIENT_ID => {
                        // PID_Fwd is a host routing envelope. A direct peer
                        // cannot nominate another route's source.
                    }
                    Ok(ControlMessage::ForwardRequest(_))
                        if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::ForwardRequest(_)) | Ok(ControlMessage::Forward(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(
                                    "recursive forwarding packet is not accepted".to_string(),
                                ),
                            })
                            .await;
                        break;
                    }
                    // ClientRouteManager consumes and replays every matching
                    // post-mortem envelope before this logical packet loop.
                    Ok(ControlMessage::PostMortem(packet)) => {
                        transport.handle_post_mortem(HOST_CLIENT_ID, packet);
                    }
                    // Admission consumes the only valid GS_Init JoinData.
                    // C++ merely logs/ignores later packets rather than
                    // disconnecting (src/C4Network2.cpp:1574-1580).
                    Ok(ControlMessage::JoinData(_)) => {}
                    Ok(ControlMessage::LeagueRoundResults(_))
                        if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::LeagueRoundResults(packet)) => {
                        let _ = event_tx
                            .send(ClientEvent::LeagueRoundResults { packet })
                            .await;
                    }
                    Ok(ControlMessage::Address(packet)) => {
                        if !client_addresses.contains_key(&packet.client_id) {
                            continue;
                        }
                        let packet = ingress_peer_addr
                            .or(host_peer_addr)
                            .map(|peer| packet.announcement_for_peer(peer))
                            .unwrap_or(packet);
                        let mesh_now = mesh_epoch.elapsed();
                        let Ok(peer_id_wire) = ClientId::try_from(packet.client_id) else {
                            continue;
                        };
                        let connectivity = transport.mesh_connectivity(
                            peer_id_wire,
                            mesh_tcp_available,
                            mesh_udp_available,
                        );
                        let mesh_peer = mesh_peers.entry(packet.client_id).or_default();
                        mesh_peer.add_address(packet.address, mesh_now);
                        let attempt = if packet.client_id != local_core.client_id
                            && packet.client_id != resource_state.host_peer_id
                        {
                            match mesh_peer.do_connect_attempt(mesh_now, connectivity) {
                                crate::ClientMeshConnectDecision::Dial(attempt) => Some(attempt),
                                crate::ClientMeshConnectDecision::NotDue { .. }
                                | crate::ClientMeshConnectDecision::Backoff { .. } => None,
                            }
                        } else {
                            None
                        };
                        if let Some(attempt) = attempt {
                            let local_puncher_address = mesh_peers
                                .get(&local_core.client_id)
                                .and_then(crate::ClientMeshPeerState::ipv6_address_from_puncher);
                            maybe_initiate_tcp_simultaneous_open(
                                &mut pending_tcp_sim_open,
                                pending_mesh_routes.len(),
                                &mut transport,
                                &local_core,
                                packet.client_id,
                                attempt,
                                local_puncher_address,
                            );
                            spawn_mesh_dial(
                                &mut pending_mesh_routes,
                                &mut active_mesh_dials,
                                packet.client_id,
                                attempt,
                                &client_cores,
                                &mesh_request_template,
                                &connection_ids,
                                &mesh_interface_ids,
                                mesh_udp_handle.as_ref(),
                                &io_statistics,
                            );
                        }
                        let insertion = crate::append_received_address(
                            client_addresses.entry(packet.client_id).or_default(),
                            packet.address,
                        );
                        if matches!(insertion, crate::AddressInsertion::Added { .. }) {
                            // C++ re-announces a newly learned address to each
                            // directly connected logical client. Duplicate
                            // suppression keeps every ordered set stable.
                            let _ = transport
                                .send_to_connected_peers(ControlMessage::Address(packet));
                            if let Err(error) = transport
                                .send_message(ControlMessage::Address(packet))
                                .await
                            {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!(
                                            "address announcement failed: {error}"
                                        )),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                    Ok(ControlMessage::TcpSimOpen(packet)) => {
                        if packet.client_id == local_core.client_id
                            || !matches!(packet.address.protocol, crate::NetworkProtocol::Tcp)
                        {
                            continue;
                        }
                        let Ok(peer_id) = ClientId::try_from(packet.client_id) else {
                            continue;
                        };
                        if ingress_peer_id != HOST_CLIENT_ID && ingress_peer_id != peer_id {
                            transport.retire_peer(ingress_peer_id);
                            continue;
                        }
                        let Some(peer_core) = client_cores.get(&packet.client_id).cloned() else {
                            continue;
                        };
                        let has_bound_socket =
                            pending_tcp_sim_open.contains_key(&packet.client_id);
                        if pending_mesh_routes.len() + pending_tcp_sim_open.len()
                            >= CLIENT_MESH_PENDING_LIMIT
                            && !has_bound_socket
                        {
                            continue;
                        }

                        let (socket, initiator_id, delay) = if let Some(socket) =
                            pending_tcp_sim_open.remove(&packet.client_id)
                        {
                            (
                                socket,
                                ClientId::try_from(local_core.client_id)
                                    .unwrap_or(ClientId::MAX),
                                Duration::ZERO,
                            )
                        } else {
                            let local_puncher_address = mesh_peers
                                .get(&local_core.client_id)
                                .and_then(crate::ClientMeshPeerState::ipv6_address_from_puncher);
                            let Some(local_puncher_address) = local_puncher_address else {
                                continue;
                            };
                            let Ok((socket, bound_address)) =
                                bind_tcp_sim_open_socket(local_puncher_address)
                            else {
                                continue;
                            };
                            pending_tcp_sim_open.insert(packet.client_id, socket);
                            let response = crate::TcpSimOpenPacket {
                                client_id: local_core.client_id,
                                address: crate::NetworkAddress::new(
                                    crate::NetworkProtocol::Tcp,
                                    bound_address,
                                ),
                            };
                            if transport
                                .try_send_to(peer_id, ControlMessage::TcpSimOpen(response))
                                .is_err()
                            {
                                continue;
                            }
                            let socket = pending_tcp_sim_open
                                .remove(&packet.client_id)
                                .expect("successful response retains bound socket");
                            let delay_ms = (transport.peer_ping_ms(peer_id) / 2).clamp(0, 10);
                            (
                                socket,
                                peer_id,
                                Duration::from_millis(delay_ms as u64),
                            )
                        };
                        let connection_id =
                            connection_ids.fetch_add(1, AtomicOrdering::Relaxed);
                        let endpoint = packet.address.endpoint;
                        let request_template = mesh_request_template.clone();
                        let io_statistics = io_statistics.clone();
                        pending_mesh_routes.spawn(async move {
                            let result = connect_mesh_tcp_socket_route(
                                peer_id,
                                initiator_id,
                                socket,
                                endpoint,
                                request_template,
                                peer_core,
                                connection_id,
                                delay,
                                io_statistics,
                            )
                            .await;
                            MeshRouteCompletion {
                                dial_key: None,
                                result,
                            }
                        });
                    }
                    Ok(ControlMessage::Resource(packet)) => {
                        let Ok(resource_peer_id) = i32::try_from(ingress_peer_id) else {
                            if ingress_peer_id != HOST_CLIENT_ID {
                                transport.retire_peer(ingress_peer_id);
                            }
                            continue;
                        };
                        if let Err(error) = dispatch_client_resource_packet(
                            resource_peer_id,
                            &packet,
                            &mut resource_state,
                            &mut transport,
                            &event_tx,
                        )
                        .await
                        {
                            if ingress_peer_id != HOST_CLIENT_ID {
                                // Native resource handlers reject malformed or
                                // unusable peer work locally; one mesh source
                                // cannot tear down the independent host link.
                                continue;
                            }
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("resource response failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    Ok(ControlMessage::Status(_)) if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::Status(status)) => {
                        resource_state.control.set_status_target(status);
                        let _ = event_tx.send(ClientEvent::Status(status)).await;
                    }
                    Ok(ControlMessage::StatusAck(_)) if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::StatusAck(status)) => {
                        resource_state.control.clear_target();
                        if status.state == NETWORK_STATE_GO {
                            // This is the side that matters most. A client
                            // downloading a resource while the game runs -- a
                            // runtime join -- carries the chunk fragments and
                            // its own control on the same strictly-ordered
                            // stream, so bulk sitting ahead of control blocks
                            // its own ticks and, through lockstep, everybody
                            // else's. Narrow the window now that there is
                            // control to protect.
                            // The backend's catalog is the one that schedules
                            // whenever there is a backend, so narrowing only the
                            // bare fallback would leave the lobby window in
                            // force for the entire runtime join.
                            resource_state.catalog.set_max_loads_per_peer(
                                crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME,
                            );
                            if let Some(backend) = resource_state.backend.as_mut() {
                                backend.set_max_loads_per_peer(
                                    crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME,
                                );
                            }
                            let current_tick = Tick::try_from(status.target_tick).unwrap_or_else(
                                |_| resource_state.control.coordinator.current_tick(),
                            );
                            let mode_change = resource_state
                                .control
                                .change_mode(status.control_mode, current_tick);
                            let (changed, mut ready) = match mode_change {
                                Ok(result) => result,
                                Err(error) => {
                                    let _ = event_tx
                                        .send(ClientEvent::Disconnected {
                                            reason: Some(format!(
                                                "control-mode change failed: {error}"
                                            )),
                                        })
                                        .await;
                                    break;
                                }
                            };
                            if changed {
                                transport.invalidate_control_send_time();
                            }
                            if changed && status.control_mode == 0 {
                                let has_current_control = backlog
                                    .packets_from(current_tick)
                                    .first()
                                    .is_some_and(|(tick, _)| *tick == current_tick);
                                if has_current_control {
                                    let local_client_id = match ClientId::try_from(
                                        resource_state.catalog.local_client_id(),
                                    ) {
                                        Ok(client_id) => client_id,
                                        Err(_) => {
                                            let _ = event_tx
                                                .send(ClientEvent::Disconnected {
                                                    reason: Some(
                                                        "control-mode change has no local client ID"
                                                            .to_string(),
                                                    ),
                                                })
                                                .await;
                                            break;
                                        }
                                    };
                                    match replay_client_controls(
                                        &mut transport,
                                        &backlog,
                                        &mut resource_state.control,
                                        local_client_id,
                                        current_tick,
                                    )
                                    .await
                                    {
                                        Ok(replayed_ready) => ready.extend(replayed_ready),
                                        Err(error) => {
                                            let _ = event_tx
                                                .send(ClientEvent::Disconnected {
                                                    reason: Some(format!(
                                                        "control-mode replay failed: {error}"
                                                    )),
                                                })
                                                .await;
                                            break;
                                        }
                                    }
                                }
                            }
                            publish_client_ready(
                                ready,
                                resource_state.control.mode == 0,
                                &mut backlog,
                                &mut next_control_request_at,
                                &event_tx,
                            )
                            .await;
                        }
                        let _ = event_tx.send(ClientEvent::StatusAck(status)).await;
                    }
                    Ok(ControlMessage::LobbyCountdown(_)) if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::LobbyCountdown(packet)) => {
                        let _ = event_tx
                            .send(ClientEvent::LobbyCountdown { packet })
                            .await;
                    }
                    // A client accepts a Request only when packet.Client is
                    // the host. Other ReadyCheck values keep their claimed
                    // client unchanged (src/C4Network2.cpp:1625-1646).
                    Ok(ControlMessage::ReadyCheck(packet))
                        if packet.data.vote_requested() && packet.client_id != 0 => {}
                    Ok(ControlMessage::ReadyCheck(packet)) => {
                        let _ = event_tx.send(ClientEvent::ReadyCheck { packet }).await;
                    }
                    Ok(ControlMessage::ActivationRequest { .. }) => {
                        // PID_ClientActReq is accepted by the host only
                        // (src/C4Network2.cpp:982-991).
                    }
                    Ok(ControlMessage::Control(packet)) => {
                        if ingress_peer_id != HOST_CLIENT_ID
                            && validate_peer_control_or_recovery(
                                &packet,
                                ingress_peer_id,
                                peer_recovery_from_tick,
                            )
                            .is_err()
                        {
                            transport.retire_peer(ingress_peer_id);
                            continue;
                        }
                        let key = (packet.client_id(), packet.tick());
                        if !received_controls.insert(key.0, key.1) {
                            continue;
                        }
                        client_performance.record_arrival(
                            key.0,
                            key.1,
                            tokio::time::Instant::now(),
                        );
                        let backlog_packet = packet.clone();
                        match resource_state.control.accept_network(packet) {
                            Ok(ready) => {
                                backlog.record_packet(&backlog_packet);
                                publish_client_ready(
                                    ready,
                                    resource_state.control.mode == 0,
                                    &mut backlog,
                                    &mut next_control_request_at,
                                    &event_tx,
                                )
                                .await;
                            }
                            Err(error) => {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!(
                                            "invalid synchronized control packet: {error}"
                                        )),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                    Ok(ControlMessage::PlayerInfoUpdate(_)) => {
                        // PID_PlayerInfoUpdReq is accepted by the host only
                        // (src/C4Network2Players.cpp:405-411).
                    }
                    Ok(ControlMessage::Packet { delivery, data }) => {
                        let authenticated_peer_control = if ingress_peer_id != HOST_CLIENT_ID {
                            let Ok(expected_author) = i32::try_from(ingress_peer_id) else {
                                continue;
                            };
                            match authenticated_single_control(&data, expected_author) {
                                Ok(control) if !control_requires_host_ingress(&control) => {
                                    Some(control)
                                }
                                Err(_) => continue,
                                Ok(_) => continue,
                            }
                        } else {
                            None
                        };
                        match delivery {
                            ControlDelivery::Direct | ControlDelivery::Private => {
                                let mut local_data = data;
                                let decoded = authenticated_peer_control
                                    .map(Ok)
                                    .unwrap_or_else(|| decode_control_entry_payload(&local_data));
                                if let Ok(mut control) = decoded {
                                    let local_sources =
                                        if let clonk_engine::ControlPacket::PlayerInfo(info) =
                                            &mut control
                                        {
                                            let local_sources = resource_state
                                                .load_authoritative_player_resources(info);
                                            if let Ok(normalized) =
                                                crate::encode_control_entry_payload(&control)
                                            {
                                                local_data = normalized;
                                            }
                                            local_sources
                                        } else {
                                            Vec::new()
                                        };
                                    for (path, core) in local_sources {
                                        let _ = event_tx
                                            .send(ClientEvent::ResourceComplete {
                                                resource_id: core.id,
                                                core,
                                                path,
                                                local: true,
                                            })
                                            .await;
                                    }
                                    let ready = match resource_state
                                        .control
                                        .apply_membership(&control)
                                    {
                                        Ok(ready) => ready,
                                        Err(error) => {
                                            let _ = event_tx
                                                .send(ClientEvent::Disconnected {
                                                    reason: Some(format!(
                                                        "control membership update failed: {error}"
                                                    )),
                                                })
                                                .await;
                                            break;
                                        }
                                    };
                                    let removed_peer = apply_client_membership(
                                        &mut client_addresses,
                                        &mut client_cores,
                                        &mut mesh_peers,
                                        &mut resource_state.catalog,
                                        resource_state.backend.as_mut(),
                                        &control,
                                    );
                                    transport.invalidate_control_send_time();
                                    if let Some(peer_id) = removed_peer {
                                        transport.retire_peer(peer_id);
                                        if let Ok(peer_id) = i32::try_from(peer_id) {
                                            pending_tcp_sim_open.remove(&peer_id);
                                            active_mesh_dials.retain(|(active_peer, _, _)| {
                                                *active_peer != peer_id
                                            });
                                        }
                                    }
                                    publish_client_ready(
                                        ready,
                                        resource_state.control.mode == 0,
                                        &mut backlog,
                                        &mut next_control_request_at,
                                        &event_tx,
                                    )
                                    .await;
                                }
                                let _ = event_tx
                                    .send(ClientEvent::Direct {
                                        delivery,
                                        data: local_data,
                                    })
                                    .await;
                            }
                            ControlDelivery::Queue
                            | ControlDelivery::Sync
                            | ControlDelivery::Decide => {
                                let decoded = authenticated_peer_control
                                    .map(Ok)
                                    .unwrap_or_else(|| decode_control_entry_payload(&data));
                                match decoded {
                                    Ok(control) => pending_sync.push(control),
                                    Err(error) => {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!(
                                                    "invalid synchronized control packet: {error}"
                                                )),
                                            })
                                            .await;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Ok(ControlMessage::ExecSync { .. }) if ingress_peer_id != HOST_CLIENT_ID => {}
                    Ok(ControlMessage::ExecSync { control_tick }) => {
                        if pending_sync.is_empty() {
                            // Temporary compatibility for the session's
                            // pre-status startup marker. Real empty releases
                            // are suppressed by the host.
                            let _ = event_tx.send(ClientEvent::ExecSync { control_tick }).await;
                        } else {
                            let controls = std::mem::take(&mut pending_sync);
                            for control in &controls {
                                let ready = match resource_state
                                    .control
                                    .apply_membership(control)
                                {
                                    Ok(ready) => ready,
                                    Err(error) => {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!(
                                                    "control membership update failed: {error}"
                                                )),
                                            })
                                            .await;
                                        break 'outer;
                                    }
                                };
                                let removed_peer = apply_client_membership(
                                    &mut client_addresses,
                                    &mut client_cores,
                                    &mut mesh_peers,
                                    &mut resource_state.catalog,
                                    resource_state.backend.as_mut(),
                                    control,
                                );
                                transport.invalidate_control_send_time();
                                if let Some(peer_id) = removed_peer {
                                    transport.retire_peer(peer_id);
                                    if let Ok(peer_id) = i32::try_from(peer_id) {
                                        pending_tcp_sim_open.remove(&peer_id);
                                        active_mesh_dials.retain(|(active_peer, _, _)| {
                                            *active_peer != peer_id
                                        });
                                    }
                                }
                                publish_client_ready(
                                    ready,
                                    resource_state.control.mode == 0,
                                    &mut backlog,
                                    &mut next_control_request_at,
                                    &event_tx,
                                )
                                .await;
                            }
                            let _ = event_tx
                                .send(ClientEvent::SyncScheduled {
                                    control_tick,
                                    controls,
                                })
                                .await;
                        }
                    }
                    Ok(ControlMessage::Request { from_tick }) => {
                        let resend = backlog.fulfill_request(from_tick);
                        if ingress_peer_id != HOST_CLIENT_ID {
                            let mut failed = false;
                            for packet in resend {
                                if transport
                                    .try_send_to(
                                        ingress_peer_id,
                                        ControlMessage::Control(packet),
                                    )
                                    .is_err()
                                {
                                    failed = true;
                                    break;
                                }
                            }
                            if failed {
                                transport.retire_peer_gracefully(ingress_peer_id);
                            }
                            continue;
                        }
                        for packet in resend {
                            if let Err(error) = transport
                                .try_send_to(ingress_peer_id, ControlMessage::Control(packet))
                            {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!("send failed: {error}")),
                                    })
                                    .await;
                                break 'outer;
                            }
                        }
                    }
                    Err(_) if ingress_peer_id != HOST_CLIENT_ID => {
                        transport.retire_peer(ingress_peer_id);
                    }
                    Err(error) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(format!("read failed: {error}")),
                            })
                            .await;
                        break;
                    }
                }
            }
            media = receive_optional_voice_media(&mut voice_media), if voice_media_ready => {
                handle_client_voice_media(
                    media,
                    &transport,
                    &voice_events,
                    &client_cores,
                    &mut voice_ingress_limiter,
                );
            }
            Some(frame) = voice_commands.recv(), if voice_media_ready => {
                if let Ok(local_client_id) = ClientId::try_from(local_core.client_id) {
                    send_client_voice_frame(
                        frame,
                        local_client_id,
                        &transport,
                        mesh_udp_handle.as_ref(),
                    );
                }
            }
        }
    }
    voice_available.store(false, std::sync::atomic::Ordering::Release);
    if let Some(task) = pending_secondary.take() {
        task.abort();
        let _ = task.await;
    }
    if let Some(task) = pending_tcp.take() {
        task.abort();
        let _ = task.await;
    }
    pending_mesh_routes.shutdown().await;
    transport.shutdown().await;
    drop(mesh_puncher_events.take());
    if let Some(hub) = mesh_udp_hub.take() {
        let _ = hub.shutdown().await;
    }
}

pub(crate) async fn dispatch_client_resource_peer_connected(
    peer_id: ClientId,
    resource_state: &mut ClientResourceState,
    transport: &mut ClientRouteManager,
    event_tx: &mpsc::Sender<ClientEvent>,
) -> Result<(), String> {
    let peer_id = i32::try_from(peer_id)
        .map_err(|_| "connected resource peer ID exceeds the C++ signed range".to_string())?;
    let now_seconds = resource_state.resource_epoch.elapsed().as_secs();
    if let Some(backend) = resource_state.backend.as_mut() {
        let mut random = resource_safe_random;
        let events = backend
            .on_peer_connected(peer_id, now_seconds, &mut random)
            .map_err(|error| error.to_string())?;
        dispatch_client_resource_events(events, resource_state, transport, event_tx)
            .await
            .map_err(|error| error.to_string())
    } else {
        let actions = resource_state.catalog.on_peer_connected(peer_id);
        dispatch_client_resource_actions(actions, resource_state, transport, event_tx)
            .await
            .map_err(|error| error.to_string())
    }
}

pub(crate) async fn dispatch_client_resource_packet(
    peer_id: i32,
    packet: &ResourcePacket,
    resource_state: &mut ClientResourceState,
    transport: &mut ClientRouteManager,
    event_tx: &mpsc::Sender<ClientEvent>,
) -> Result<(), String> {
    let now_seconds = resource_state.resource_epoch.elapsed().as_secs();
    if let Some(backend) = resource_state.backend.as_mut() {
        let mut random = resource_safe_random;
        let events = backend
            .on_packet(peer_id, packet, now_seconds, &mut random)
            .map_err(|error| error.to_string())?;
        if matches!(packet, ResourcePacket::Derive(_)) {
            let _ = resource_state.catalog.on_packet(peer_id, packet);
        }
        update_derived_resource_sources(&mut resource_state.local_resource_sources, &events);
        dispatch_client_resource_events(events, resource_state, transport, event_tx)
            .await
            .map_err(|error| error.to_string())
    } else {
        let actions = resource_state.catalog.on_packet(peer_id, packet);
        dispatch_client_resource_actions(actions, resource_state, transport, event_tx)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClientResourceSendOutcome {
    Sent,
    PeerUnavailable,
}

fn try_dispatch_client_resource_to_peer(
    transport: &mut ClientRouteManager,
    host_peer_id: i32,
    peer_id: i32,
    packet: ResourcePacket,
) -> Result<ClientResourceSendOutcome, TransportError> {
    let Ok(peer_id_wire) = ClientId::try_from(peer_id) else {
        return Ok(ClientResourceSendOutcome::PeerUnavailable);
    };
    match transport.try_send_to(peer_id_wire, ControlMessage::Resource(packet)) {
        Ok(()) => Ok(ClientResourceSendOutcome::Sent),
        Err(error) if peer_id == host_peer_id => Err(error),
        Err(_) => {
            // C++ fails a peer-specific resource send without tearing down the
            // independent host session. Request accounting is reconciled by
            // the action dispatcher before it continues the native refill.
            transport.retire_peer_gracefully(peer_id_wire);
            Ok(ClientResourceSendOutcome::PeerUnavailable)
        }
    }
}

pub(crate) async fn dispatch_client_resource_actions(
    actions: Vec<crate::ResourceCatalogAction>,
    resource_state: &mut ClientResourceState,
    transport: &mut ClientRouteManager,
    event_tx: &mpsc::Sender<ClientEvent>,
) -> Result<(), TransportError> {
    let mut unavailable_peers = BTreeSet::new();
    dispatch_client_resource_actions_with_unavailable(
        actions,
        resource_state,
        transport,
        event_tx,
        &mut unavailable_peers,
    )
    .await
}

async fn dispatch_client_resource_actions_with_unavailable(
    actions: Vec<crate::ResourceCatalogAction>,
    resource_state: &mut ClientResourceState,
    transport: &mut ClientRouteManager,
    event_tx: &mpsc::Sender<ClientEvent>,
    unavailable_peers: &mut BTreeSet<i32>,
) -> Result<(), TransportError> {
    let host_peer_id = resource_state.host_peer_id;
    let mut pending = VecDeque::from(actions);
    while let Some(action) = pending.pop_front() {
        match action {
            crate::ResourceCatalogAction::SendToPeer { peer_id, packet } => {
                let request = match &packet {
                    ResourcePacket::Request(request) => Some(*request),
                    _ => None,
                };
                let outcome =
                    try_dispatch_client_resource_to_peer(transport, host_peer_id, peer_id, packet)?;
                if outcome == ClientResourceSendOutcome::PeerUnavailable {
                    unavailable_peers.insert(peer_id);
                    if let Some(request) = request {
                        pending.extend(resource_state.on_request_send_failed(
                            peer_id,
                            &request,
                            unavailable_peers,
                        ));
                    }
                }
            }
            crate::ResourceCatalogAction::Broadcast { packet } => {
                // BroadcastMsg selects one message route per connected
                // logical client, preferring UDP and falling back to TCP.
                for peer_id in transport.connected_peer_ids() {
                    let Ok(peer_id) = i32::try_from(peer_id) else {
                        continue;
                    };
                    let outcome = try_dispatch_client_resource_to_peer(
                        transport,
                        host_peer_id,
                        peer_id,
                        packet.clone(),
                    )?;
                    if outcome == ClientResourceSendOutcome::PeerUnavailable {
                        unavailable_peers.insert(peer_id);
                    }
                }
            }
            external => {
                let _ = event_tx.send(ClientEvent::ResourceAction(external)).await;
            }
        }
    }
    Ok(())
}

async fn dispatch_client_resource_events(
    events: Vec<crate::ResourceTransferEvent>,
    resource_state: &mut ClientResourceState,
    transport: &mut ClientRouteManager,
    event_tx: &mpsc::Sender<ClientEvent>,
) -> Result<(), TransportError> {
    let mut unavailable_peers = BTreeSet::new();
    for event in events {
        match event {
            crate::ResourceTransferEvent::Transport(action) => {
                dispatch_client_resource_actions_with_unavailable(
                    vec![action],
                    resource_state,
                    transport,
                    event_tx,
                    &mut unavailable_peers,
                )
                .await?;
            }
            crate::ResourceTransferEvent::Progress {
                resource_id,
                present_percent,
            } => {
                let _ = event_tx
                    .send(ClientEvent::ResourceProgress {
                        resource_id,
                        present_percent,
                    })
                    .await;
            }
            crate::ResourceTransferEvent::Completed {
                resource_id,
                core,
                path,
            } => {
                let _ = event_tx
                    .send(ClientEvent::ResourceComplete {
                        resource_id,
                        core,
                        path,
                        local: false,
                    })
                    .await;
            }
            crate::ResourceTransferEvent::LoadFailed { resource_id } => {
                let _ = event_tx
                    .send(ClientEvent::ResourceLoadFailed { resource_id })
                    .await;
            }
        }
    }
    Ok(())
}

fn apply_client_membership(
    client_addresses: &mut BTreeMap<i32, Vec<crate::NetworkAddress>>,
    client_cores: &mut BTreeMap<i32, clonk_engine::ClientCoreControlData>,
    mesh_peers: &mut BTreeMap<i32, crate::ClientMeshPeerState>,
    resource_catalog: &mut crate::ResourceCatalog,
    resource_backend: Option<&mut crate::ResourceTransferBackend>,
    control: &clonk_engine::ControlPacket,
) -> Option<ClientId> {
    match control {
        clonk_engine::ControlPacket::ClientJoin(join)
            if join.by_client == HOST_CLIENT_ID as i32 =>
        {
            client_addresses.entry(join.core.client_id).or_default();
            mesh_peers.entry(join.core.client_id).or_default();
            client_cores.insert(join.core.client_id, join.core.clone());
            None
        }
        clonk_engine::ControlPacket::ClientRemove(remove)
            if remove.by_client == HOST_CLIENT_ID as i32 =>
        {
            client_addresses.remove(&remove.client_id);
            client_cores.remove(&remove.client_id);
            mesh_peers.remove(&remove.client_id);
            resource_catalog.remove_at_client(remove.client_id);
            if let Some(backend) = resource_backend {
                backend.remove_at_client(remove.client_id);
            }
            ClientId::try_from(remove.client_id).ok()
        }
        _ => None,
    }
}

fn record_host_control_wait_attribution(
    ingress_peer_id: ClientId,
    attribution: crate::ControlWaitAttribution,
    snapshot: &crate::ControlWaitAttributionSnapshot,
) {
    if ingress_peer_id == HOST_CLIENT_ID {
        snapshot.publish(attribution);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    #[test]
    fn only_the_host_can_publish_control_wait_attribution() {
        let snapshot = crate::ControlWaitAttributionSnapshot::default();
        let attribution = crate::ControlWaitAttribution {
            tick: 73,
            waited_for_recipient: false,
            waited_for_other: true,
        };

        record_host_control_wait_attribution(7, attribution, &snapshot);
        assert_eq!(snapshot.sample(73), None);

        record_host_control_wait_attribution(HOST_CLIENT_ID, attribution, &snapshot);
        assert_eq!(snapshot.sample(73), Some(attribution));
    }

    #[test]
    fn received_control_deduplication_prunes_once_per_advancing_tick() {
        let mut received = ReceivedControlDeduplicator::new(2);

        for client_id in 1..=24 {
            assert!(received.insert(client_id, 10));
        }
        assert_eq!(received.prune_passes(), 1);
        assert!(!received.insert(7, 10), "live duplicates stay suppressed");
        assert_eq!(received.prune_passes(), 1);

        for client_id in 1..=24 {
            assert!(received.insert(client_id, 11));
        }
        assert_eq!(received.prune_passes(), 2);

        assert!(received.insert(1, 13));
        assert_eq!(received.prune_passes(), 3);
        assert_eq!(received.retained_len(), 25);
        assert!(
            received.insert(7, 10),
            "controls older than the deduplication window remain replayable"
        );
        assert!(received.insert(7, 10));
        assert_eq!(received.retained_len(), 25);
        assert_eq!(received.prune_passes(), 3);
        assert!(
            !received.insert(7, 11),
            "retained duplicates stay suppressed"
        );
    }

    struct RetireAfterFlushWriter {
        retire: watch::Sender<bool>,
    }

    impl AsyncWrite for RetireAfterFlushWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.retire.send_replace(true);
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_writer_completes_flushed_part_before_racing_route_retirement() {
        // C++ handles a negative PID_ConnRe and immediately closes the route;
        // a successful preceding send remains successful
        // (oracle-src-pinned src/C4Network2.cpp:1459-1469;
        // src/C4Network2Client.cpp:104-116).
        let (retire, retire_rx) = watch::channel(false);
        let (outbound, outbound_rx) = mpsc::unbounded_channel();
        let (completion, completed) = oneshot::channel();
        outbound
            .send(ClientRouteCommand::Message(
                ControlMessage::ConnectionReply(crate::ConnectionReply {
                    ok: false,
                    message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec())
                        .unwrap_or_default(),
                    wrong_password: false,
                }),
            ))
            .expect("queue graceful part");
        outbound
            .send(ClientRouteCommand::Flush(completion))
            .expect("queue graceful-part flush");

        let exit = run_client_route_writer(
            crate::ControlTransport::new(RetireAfterFlushWriter { retire }),
            outbound_rx,
            retire_rx,
        )
        .await;

        assert!(matches!(exit, ClientRouteWriterExit::Cancelled));
        completed
            .await
            .expect("flushed graceful part must complete before route retirement");
    }
}
