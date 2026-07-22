//! Client route registry: ClientTask, session liveness timer, route manager & route events.
//!
//! Moved byte-verbatim from `session.rs` (wave 2 of the decomposition
//! campaign, see REFACTOR_PLAN.md). Structural only.

use super::*;

pub(crate) struct ClientTask<S> {
    pub(crate) local_connection_id: u32,
    pub(crate) remote_connection_id: u32,
    pub(crate) client_id: ClientId,
    pub(crate) transport: crate::ControlTransport<S>,
    pub(crate) outbound_rx: mpsc::Receiver<HostOutboundMessage>,
    pub(crate) retire_rx: watch::Receiver<bool>,
    pub(crate) host_tx: mpsc::Sender<HostLoopMessage>,
    pub(crate) liveness: ConnectionLivenessState,
}

impl<S> ClientTask<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn retain_queued_post_mortem_packets(&mut self) {
        self.outbound_rx.close();
        while let Ok(message) = self.outbound_rx.try_recv() {
            let _ = match message {
                HostOutboundMessage::Message(message) => {
                    self.transport.retain_unsent_message(message)
                }
                HostOutboundMessage::Raw(packet) => {
                    self.transport.retain_unsent_complete_packet(packet)
                }
                HostOutboundMessage::Close(_) => Ok(()),
            };
        }
    }

    async fn notify_disconnected(&mut self, reason: Option<String>) {
        // A successful channel send is a successful logical send to callers.
        // Preserve commands that this dead route had accepted but had not yet
        // written so the surviving route's PostMortem replay cannot lose them.
        self.retain_queued_post_mortem_packets();
        let post_mortem = self.transport.create_post_mortem(self.remote_connection_id);
        let _ = self
            .host_tx
            .send(HostLoopMessage::ClientDisconnected {
                connection_id: self.local_connection_id,
                client_id: self.client_id,
                next_inbound_packet: self.liveness.connection().inbound_packet_counter(),
                post_mortem,
                reason,
            })
            .await;
    }

    pub(crate) async fn run(mut self) {
        loop {
            let liveness_deadline = self.liveness.next_timer_at();
            tokio::select! {
                biased;
                _ = wait_for_route_retirement(&mut self.retire_rx) => {
                    self.notify_disconnected(None).await;
                    break;
                }
                Some(message) = self.outbound_rx.recv() => {
                    let message = match message {
                        HostOutboundMessage::Close(reply) => {
                            let _ = self
                                .transport
                                .send_message(ControlMessage::ConnectionReply(reply))
                                .await;
                            break;
                        }
                        message => message,
                    };
                    let result = tokio::select! {
                        biased;
                        result = async {
                            match message {
                                HostOutboundMessage::Message(message) => {
                                    self.transport.send_message(message).await
                                }
                                HostOutboundMessage::Raw(packet) => {
                                    self.transport.send_complete_packet_bytes(&packet).await
                                }
                                HostOutboundMessage::Close(_) => {
                                    unreachable!("close handled before send selection")
                                }
                            }
                        } => Some(result),
                        _ = wait_for_route_retirement(&mut self.retire_rx) => None,
                    };
                    let Some(result) = result else {
                        self.notify_disconnected(None).await;
                        break;
                    };
                    if let Err(error) = result {
                        self.notify_disconnected(Some(format!("send failed: {error}")))
                            .await;
                        break;
                    }
                }
                packet = self.transport.read_packet() => {
                    let result = match packet {
                        Ok(crate::transport::InboundPacket::Message(message)) => {
                            self.liveness.record_inbound_message(&message);
                            Ok(message)
                        }
                        Ok(crate::transport::InboundPacket::Ignored(packet_type)) => {
                            self.liveness.record_inbound_packet(packet_type);
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::UnhandledPacket {
                                    client_id: Some(self.client_id),
                                    packet_type,
                                })
                                .await;
                            continue;
                        }
                        Ok(crate::transport::InboundPacket::Empty) => continue,
                        Ok(crate::transport::InboundPacket::Invalid {
                            packet_type,
                            error,
                        }) => {
                            self.liveness.record_inbound_packet(packet_type);
                            Err(error)
                        }
                        Err(error) => Err(error),
                    };
                    match result {
                        Ok(ControlMessage::Ping(packet)) => {
                            let result = tokio::select! {
                                biased;
                                result = self.transport.send_message(ControlMessage::Pong(packet)) => {
                                    Some(result)
                                },
                                _ = wait_for_route_retirement(&mut self.retire_rx) => None,
                            };
                            let Some(result) = result else {
                                self.notify_disconnected(None).await;
                                break;
                            };
                            if let Err(error) = result {
                                self.notify_disconnected(Some(format!("pong send failed: {error}")))
                                    .await;
                                break;
                            }
                        }
                        Ok(ControlMessage::Pong(packet)) => {
                            let round_trip_ms = self.liveness.record_pong(packet);
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::ConnectionPing {
                                    connection_id: self.local_connection_id,
                                    client_id: self.client_id,
                                    update: RoutePingUpdate::Measured(round_trip_ms),
                                })
                                .await;
                        }
                        Ok(ControlMessage::ConnectionReply(reply)) if !reply.ok => {
                            self.notify_disconnected(Some(
                                clonk_resources::decode_legacy_script_text(reply.message.as_bytes()),
                            ))
                            .await;
                            break;
                        }
                        Ok(message) => {
                            let ping_ms = self
                                .liveness
                                .connection()
                                .measured_ping_ms()
                                .unwrap_or(-1);
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::ClientMessage {
                                    connection_id: self.local_connection_id,
                                    client_id: self.client_id,
                                    message,
                                    ping_ms,
                                })
                                .await;
                        }
                        Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                            self.notify_disconnected(None).await;
                            break;
                        }
                        Err(error) => {
                            self.notify_disconnected(Some(format!("read failed: {error}"))).await;
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(liveness_deadline) => {
                    let result = tokio::select! {
                        biased;
                        result = drive_session_liveness_timer(
                            &mut self.transport,
                            &mut self.liveness,
                        ) => Some(result),
                        _ = wait_for_route_retirement(&mut self.retire_rx) => None,
                    };
                    let Some(result) = result else {
                        self.notify_disconnected(None).await;
                        break;
                    };
                    match result {
                        Ok(true) => {
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::ConnectionPing {
                                    connection_id: self.local_connection_id,
                                    client_id: self.client_id,
                                    update: RoutePingUpdate::Dispatched,
                                })
                                .await;
                        }
                        Ok(false) => {}
                        Err(reason) => {
                            self.notify_disconnected(Some(reason)).await;
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Returns whether this edge dispatched a ping probe, so the caller can
/// mirror the outstanding-ping timestamp onto its route registry.
pub(crate) async fn drive_session_liveness_timer<S>(
    transport: &mut crate::ControlTransport<S>,
    liveness: &mut ConnectionLivenessState,
) -> Result<bool, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ping = liveness
        .timer_tick()
        .map_err(|timeout| format!("connection {timeout:?} timeout"))?;
    let Some(ping) = ping else {
        return Ok(false);
    };
    let result = transport.send_message(ControlMessage::Ping(ping)).await;
    // C4Network2IO calls OnPing after the send attempt even on failure
    // (src/C4Network2IO.cpp:1141-1151).
    liveness.record_ping_dispatched();
    result.map_err(|error| format!("ping send failed: {error}"))?;
    Ok(true)
}

pub(crate) enum ClientRouteCommand {
    Message(ControlMessage),
    Flush(oneshot::Sender<()>),
}

#[derive(Clone)]
pub(crate) struct ClientRouteSender {
    pub(crate) sender: mpsc::Sender<ClientRouteCommand>,
    pub(crate) retire: watch::Sender<bool>,
}

impl ClientRouteSender {
    pub(crate) fn is_closed(&self) -> bool {
        self.sender.is_closed() || *self.retire.borrow()
    }

    fn retire(&self) {
        self.retire.send_replace(true);
    }
}

pub(crate) struct ClientRouteEntry {
    pub(crate) peer_id: ClientId,
    pub(crate) initiator_id: ClientId,
    pub(crate) remote_connection_id: u32,
    pub(crate) protocol: crate::NetworkProtocol,
    pub(crate) peer_addr: Option<SocketAddr>,
    pub(crate) ping: RoutePingLag,
    pub(crate) outbound: ClientRouteSender,
}

pub(crate) enum ClientRouteEvent {
    Packet {
        route_id: u32,
        peer_addr: Option<SocketAddr>,
        packet: crate::transport::InboundPacket,
    },
    PingMeasured {
        route_id: u32,
        round_trip_ms: i32,
    },
    /// The route's transport task dispatched a ping probe; the manager
    /// stamps the outstanding timestamp `getLag` grows from
    /// (src/C4Network2IO.cpp:1283-1295).
    PingDispatched {
        route_id: u32,
    },
    Disconnected {
        route_id: u32,
        next_inbound_packet: u32,
        post_mortem: Option<crate::PostMortemPacket>,
        reason: Option<String>,
    },
}

pub(crate) enum ClientRouteRead {
    Packet {
        peer_id: ClientId,
        packet: crate::transport::InboundPacket,
        peer_addr: Option<SocketAddr>,
    },
    PingMeasured {
        peer_id: ClientId,
        round_trip_ms: i32,
    },
    Disconnected {
        peer_id: ClientId,
        protocol: crate::NetworkProtocol,
        routes_remaining: bool,
        post_mortem: Option<crate::PostMortemPacket>,
        reason: Option<String>,
    },
}

pub(crate) struct ClientRouteManager {
    pub(crate) routes: BTreeMap<u32, ClientRouteEntry>,
    pub(crate) event_tx: mpsc::Sender<ClientRouteEvent>,
    event_rx: mpsc::Receiver<ClientRouteEvent>,
    tasks: BTreeMap<u32, tokio::task::JoinHandle<()>>,
    pub(crate) closed_routes: crate::post_mortem::ClosedConnectionRouter,
    closed_route_peers: BTreeMap<u32, Option<SocketAddr>>,
    pub(crate) pending_post_mortems: BTreeMap<u32, crate::PostMortemPacket>,
    peer_ping_ms: BTreeMap<ClientId, i32>,
    pub(crate) replay_packets:
        VecDeque<(ClientId, crate::transport::InboundPacket, Option<SocketAddr>)>,
}

impl ClientRouteManager {
    pub(crate) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel(64);
        Self {
            routes: BTreeMap::new(),
            event_tx,
            event_rx,
            tasks: BTreeMap::new(),
            closed_routes: crate::post_mortem::ClosedConnectionRouter::default(),
            closed_route_peers: BTreeMap::new(),
            pending_post_mortems: BTreeMap::new(),
            peer_ping_ms: BTreeMap::new(),
            replay_packets: VecDeque::new(),
        }
    }

    pub(crate) fn add_route<S>(
        &mut self,
        local_connection_id: u32,
        remote_connection_id: u32,
        protocol: crate::NetworkProtocol,
        peer_addr: Option<SocketAddr>,
        transport: crate::ControlTransport<S>,
        liveness: ConnectionLivenessState,
    ) where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.add_peer_route(
            HOST_CLIENT_ID,
            HOST_CLIENT_ID,
            local_connection_id,
            remote_connection_id,
            protocol,
            peer_addr,
            transport,
            liveness,
        );
    }

    pub(crate) fn add_peer_route<S>(
        &mut self,
        peer_id: ClientId,
        initiator_id: ClientId,
        local_connection_id: u32,
        remote_connection_id: u32,
        protocol: crate::NetworkProtocol,
        peer_addr: Option<SocketAddr>,
        transport: crate::ControlTransport<S>,
        liveness: ConnectionLivenessState,
    ) -> bool
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let peer_was_connected = self
            .routes
            .values()
            .any(|route| route.peer_id == peer_id && !route.outbound.is_closed());
        let route_rank = |initiator_id: ClientId, local_id: u32, remote_id: u32| {
            (
                initiator_id,
                local_id.min(remote_id),
                local_id.max(remote_id),
            )
        };
        let new_rank = route_rank(initiator_id, local_connection_id, remote_connection_id);
        let new_route_wins = !self.routes.iter().any(|(route_id, route)| {
            route.peer_id == peer_id
                && route.protocol == protocol
                && !route.outbound.is_closed()
                && route_rank(route.initiator_id, *route_id, route.remote_connection_id) <= new_rank
        });
        if new_route_wins {
            for route in self.routes.values() {
                if route.peer_id == peer_id
                    && route.protocol == protocol
                    && !route.outbound.is_closed()
                {
                    route.outbound.retire();
                }
            }
        }
        let (sender, outbound_rx) = mpsc::channel(64);
        let (retire, retire_rx) = watch::channel(false);
        let replaced = self.routes.insert(
            local_connection_id,
            ClientRouteEntry {
                peer_id,
                initiator_id,
                remote_connection_id,
                protocol,
                peer_addr,
                ping: RoutePingLag::default(),
                outbound: ClientRouteSender { sender, retire },
            },
        );
        debug_assert!(replaced.is_none());
        let events = self.event_tx.clone();
        let task = tokio::spawn(run_client_route(
            local_connection_id,
            remote_connection_id,
            peer_addr,
            transport,
            outbound_rx,
            retire_rx,
            events,
            liveness,
        ));
        let replaced = self.tasks.insert(local_connection_id, task);
        debug_assert!(replaced.is_none());
        if !new_route_wins {
            // Run the deterministic loser through normal retirement so its
            // inbound counter remains available for PostMortem recovery.
            self.routes
                .get(&local_connection_id)
                .expect("new client route exists")
                .outbound
                .retire();
        }
        !peer_was_connected && new_route_wins
    }

    fn preferred_route_id(
        &self,
        peer_id: ClientId,
        traffic: ConnectionTrafficClass,
    ) -> Option<u32> {
        self.routes
            .iter()
            .filter(|(_, route)| route.peer_id == peer_id && !route.outbound.is_closed())
            .min_by_key(|(route_id, route)| {
                let protocol_rank = match (traffic, route.protocol) {
                    (ConnectionTrafficClass::Message, crate::NetworkProtocol::Udp)
                    | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Tcp) => 0_u8,
                    (ConnectionTrafficClass::Message, crate::NetworkProtocol::Tcp)
                    | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Udp) => 1_u8,
                    _ => 2_u8,
                };
                (
                    protocol_rank,
                    route.initiator_id,
                    (**route_id).min(route.remote_connection_id),
                    (**route_id).max(route.remote_connection_id),
                )
            })
            .map(|(route_id, _)| *route_id)
    }

    pub(crate) fn expire_closed_routes(&mut self) {
        self.closed_routes.expire();
        let closed_routes = &self.closed_routes;
        self.closed_route_peers
            .retain(|route_id, _| closed_routes.contains(*route_id));
    }

    pub(crate) async fn send_message(&mut self, message: ControlMessage) -> Result<(), TransportError> {
        self.try_send_to(HOST_CLIENT_ID, message)
    }

    #[cfg(test)]
    pub(crate) async fn send_to(
        &mut self,
        peer_id: ClientId,
        mut message: ControlMessage,
    ) -> Result<(), TransportError> {
        let traffic = match &message {
            ControlMessage::Resource(packet) => resource_traffic_class(packet),
            _ => ConnectionTrafficClass::Message,
        };
        loop {
            let Some(route_id) = self.preferred_route_id(peer_id, traffic) else {
                return Err(TransportError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "client has no accepted transport route to peer",
                )));
            };
            let outbound = self
                .routes
                .get(&route_id)
                .expect("selected client route exists")
                .outbound
                .clone();
            match outbound
                .sender
                .send(ClientRouteCommand::Message(message))
                .await
            {
                Ok(()) => return Ok(()),
                Err(error) => {
                    message = match error.0 {
                        ClientRouteCommand::Message(message) => message,
                        ClientRouteCommand::Flush(_) => {
                            unreachable!("send_to only queues message commands")
                        }
                    };
                }
            }
        }
    }

    pub(crate) fn try_send_to(
        &mut self,
        peer_id: ClientId,
        mut message: ControlMessage,
    ) -> Result<(), TransportError> {
        let traffic = match &message {
            ControlMessage::Resource(packet) => resource_traffic_class(packet),
            _ => ConnectionTrafficClass::Message,
        };
        loop {
            let Some(route_id) = self.preferred_route_id(peer_id, traffic) else {
                return Err(TransportError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "client has no accepted transport route to peer",
                )));
            };
            let outbound = self
                .routes
                .get(&route_id)
                .expect("selected client route exists")
                .outbound
                .clone();
            match outbound
                .sender
                .try_send(ClientRouteCommand::Message(message))
            {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Closed(command)) => {
                    message = match command {
                        ClientRouteCommand::Message(message) => message,
                        ClientRouteCommand::Flush(_) => {
                            unreachable!("try_send_to only queues message commands")
                        }
                    };
                }
                Err(mpsc::error::TrySendError::Full(command)) => {
                    message = match command {
                        ClientRouteCommand::Message(message) => message,
                        ClientRouteCommand::Flush(_) => {
                            unreachable!("try_send_to only queues message commands")
                        }
                    };
                    outbound.retire();
                }
            }
        }
    }

    pub(crate) async fn flush_to(&mut self, peer_id: ClientId) -> Result<(), TransportError> {
        loop {
            let Some(route_id) =
                self.preferred_route_id(peer_id, ConnectionTrafficClass::Message)
            else {
                return Err(TransportError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "client has no accepted transport route to peer",
                )));
            };
            let outbound = self
                .routes
                .get(&route_id)
                .expect("selected client route exists")
                .outbound
                .clone();
            let (completion, completed) = oneshot::channel();
            match outbound
                .sender
                .try_send(ClientRouteCommand::Flush(completion))
            {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => continue,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    outbound.retire();
                    return Err(TransportError::Io(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "client route queue is full before graceful flush",
                    )));
                }
            }
            return match tokio::time::timeout(CLIENT_ROUTE_RETRY_INTERVAL, completed).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(TransportError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "client route closed before flushing queued messages",
                ))),
                Err(_) => {
                    outbound.retire();
                    Err(TransportError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "client route timed out while flushing graceful departure",
                    )))
                }
            };
        }
    }

    pub(crate) fn mesh_connectivity(
        &self,
        peer_id: ClientId,
        tcp_available: bool,
        udp_available: bool,
    ) -> crate::ClientMeshConnectivity {
        let message_route = self.preferred_route_id(peer_id, ConnectionTrafficClass::Message);
        let data_route = self.preferred_route_id(peer_id, ConnectionTrafficClass::Data);
        match (message_route, data_route) {
            (None, None) => {
                crate::ClientMeshConnectivity::disconnected(tcp_available, udp_available)
            }
            (Some(message_route), Some(data_route)) if message_route != data_route => {
                crate::ClientMeshConnectivity::distinct_routes(
                    self.routes[&message_route].protocol,
                    self.routes[&data_route].protocol,
                    tcp_available,
                    udp_available,
                )
            }
            (Some(route), _) | (_, Some(route)) => crate::ClientMeshConnectivity::single_route(
                self.routes[&route].protocol,
                tcp_available,
                udp_available,
            ),
        }
    }

    pub(crate) fn connected_peer_ids(&self) -> BTreeSet<ClientId> {
        self.routes
            .values()
            .filter(|route| !route.outbound.is_closed())
            .map(|route| route.peer_id)
            .collect()
    }

    pub(crate) fn runtime_connections(&self) -> Vec<RuntimeNetworkConnection> {
        let now = Instant::now();
        self.routes
            .iter()
            .filter(|(_, route)| !route.outbound.is_closed())
            .filter_map(|(connection_id, route)| {
                let is_message = self
                    .preferred_route_id(route.peer_id, ConnectionTrafficClass::Message)
                    == Some(*connection_id);
                let is_data = self.preferred_route_id(route.peer_id, ConnectionTrafficClass::Data)
                    == Some(*connection_id);
                Some(RuntimeNetworkConnection {
                    connection_id: *connection_id,
                    client_id: route.peer_id,
                    usage: runtime_connection_usage(is_message, is_data)?,
                    protocol: route.protocol,
                    peer_address: route.peer_addr,
                    packet_loss: 0,
                    ping_ms: route.ping.ping_ms(),
                    lag_ms: route.ping.lag_ms(now),
                })
            })
            .collect()
    }

    pub(crate) fn runtime_client_states(
        &self,
        control_mode: i32,
        tick: Tick,
        client_ids: impl IntoIterator<Item = ClientId>,
        backlog: &ControlBacklog,
        client_performance: &ClientPerformanceStats,
    ) -> Vec<RuntimeNetworkClientState> {
        let central_nonhost = control_mode == 1;
        client_ids
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|client_id| RuntimeNetworkClientState {
                client_id,
                status: RemoteBarrierState::Ready,
                // C4GameControlNetwork::ClientReady explicitly returns true
                // for a central non-host because only C4ClientIDAll packets
                // are delivered to it.
                control_ready: central_nonhost || backlog.contains_packet(client_id, tick),
                // ClientPerfStat has the same central/non-host special case
                // and returns the C++ integer value `true` (1), not route ping.
                wait_ms: if central_nonhost {
                    1
                } else {
                    client_performance.wait_ms(client_id)
                },
            })
            .collect()
    }

    pub(crate) fn disconnect_runtime_connection(&self, connection_id: u32) -> bool {
        let Some(route) = self
            .routes
            .get(&connection_id)
            .filter(|route| !route.outbound.is_closed())
        else {
            return false;
        };
        route.outbound.retire();
        true
    }

    pub(crate) fn peer_ping_ms(&self, peer_id: ClientId) -> i32 {
        self.peer_ping_ms.get(&peer_id).copied().unwrap_or(0)
    }

    pub(crate) fn retire_peer(&mut self, peer_id: ClientId) {
        let route_ids = self
            .routes
            .iter()
            .filter_map(|(route_id, route)| (route.peer_id == peer_id).then_some(*route_id))
            .collect::<Vec<_>>();
        for route_id in route_ids {
            self.pending_post_mortems.remove(&route_id);
            if let Some(route) = self.routes.remove(&route_id) {
                route.outbound.retire();
            }
        }
        self.peer_ping_ms.remove(&peer_id);
        self.closed_routes.remove_client(peer_id);
        self.closed_route_peers
            .retain(|route_id, _| self.closed_routes.contains(*route_id));
        self.replay_packets
            .retain(|(replay_peer_id, _, _)| *replay_peer_id != peer_id);
    }

    pub(crate) fn retire_peer_gracefully(&mut self, peer_id: ClientId) {
        for route in self.routes.values() {
            if route.peer_id == peer_id {
                route.outbound.retire();
            }
        }
        self.peer_ping_ms.remove(&peer_id);
    }

    pub(crate) fn send_to_connected_peers(&mut self, message: ControlMessage) -> Vec<ClientId> {
        let peer_ids = self
            .connected_peer_ids()
            .into_iter()
            .filter(|peer_id| *peer_id != HOST_CLIENT_ID)
            .collect::<Vec<_>>();
        let mut sent = Vec::new();
        for peer_id in peer_ids {
            if self.try_send_to(peer_id, message.clone()).is_ok() {
                sent.push(peer_id);
            } else {
                self.retire_peer_gracefully(peer_id);
            }
        }
        sent
    }

    fn recover_post_mortem(
        &mut self,
        source_peer_id: ClientId,
        packet: crate::PostMortemPacket,
    ) -> bool {
        self.expire_closed_routes();
        if source_peer_id != HOST_CLIENT_ID
            && self.closed_routes.client_id(packet.connection_id) != Some(source_peer_id)
        {
            return false;
        }
        let Some(replay) = self.closed_routes.recover(&packet) else {
            return false;
        };
        let peer_addr = self
            .closed_route_peers
            .remove(&replay.connection_id)
            .flatten();
        for nested_packet in replay.packets {
            let packet = match crate::transport::parse_complete_packet(&nested_packet) {
                Ok(Some(ControlMessage::PostMortem(post_mortem))) => {
                    self.handle_post_mortem(source_peer_id, post_mortem);
                    continue;
                }
                Ok(Some(message)) => crate::transport::InboundPacket::Message(message),
                Ok(None) => crate::transport::InboundPacket::Ignored(
                    nested_packet.first().copied().unwrap_or_default(),
                ),
                Err(error) => crate::transport::InboundPacket::Invalid {
                    packet_type: nested_packet.first().copied().unwrap_or_default(),
                    error,
                },
            };
            self.replay_packets
                .push_back((replay.client_id, packet, peer_addr));
        }
        true
    }

    pub(crate) fn handle_post_mortem(
        &mut self,
        source_peer_id: ClientId,
        post_mortem: crate::PostMortemPacket,
    ) {
        let connection_id = post_mortem.connection_id;
        if self.recover_post_mortem(source_peer_id, post_mortem.clone()) {
            return;
        }
        if let Some(outbound) = self
            .routes
            .get(&connection_id)
            .filter(|route| {
                source_peer_id == HOST_CLIENT_ID || route.peer_id == source_peer_id
            })
            .map(|route| route.outbound.clone())
        {
            self.pending_post_mortems.insert(connection_id, post_mortem);
            outbound.retire();
        }
    }

    pub(crate) async fn read_event(&mut self) -> Result<ClientRouteRead, TransportError> {
        loop {
            if let Some(packet) = self.replay_packets.pop_front() {
                return Ok(ClientRouteRead::Packet {
                    peer_id: packet.0,
                    packet: packet.1,
                    peer_addr: packet.2,
                });
            }
            let Some(event) = self.event_rx.recv().await else {
                return Err(TransportError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "all client transport routes stopped",
                )));
            };
            match event {
                ClientRouteEvent::Packet {
                    route_id,
                    peer_addr,
                    packet,
                } if self.routes.contains_key(&route_id) => {
                    if let crate::transport::InboundPacket::Message(ControlMessage::PostMortem(
                        post_mortem,
                    )) = packet
                    {
                        let source_peer_id = self
                            .routes
                            .get(&route_id)
                            .expect("checked route still exists")
                            .peer_id;
                        self.handle_post_mortem(source_peer_id, post_mortem);
                        continue;
                    }
                    if matches!(
                        &packet,
                        crate::transport::InboundPacket::Message(
                            ControlMessage::ConnectionRequest(_)
                        )
                    ) && self
                        .routes
                        .get(&route_id)
                        .is_some_and(|route| matches!(route.protocol, crate::NetworkProtocol::Udp))
                    {
                        // One side of reliable UDP can detect a reordered-Conn
                        // reconnect before the other. Retire the stale session
                        // route and let the surviving protocol start a fresh
                        // secondary handshake instead of treating it as a
                        // fatal duplicate host request.
                        if let Some(route) = self.routes.get(&route_id) {
                            route.outbound.retire();
                        }
                        continue;
                    }
                    let peer_id = self
                        .routes
                        .get(&route_id)
                        .expect("checked route still exists")
                        .peer_id;
                    return Ok(ClientRouteRead::Packet {
                        peer_id,
                        packet,
                        peer_addr,
                    });
                }
                ClientRouteEvent::Packet { .. } => {}
                ClientRouteEvent::PingMeasured {
                    route_id,
                    round_trip_ms,
                } if self.routes.contains_key(&route_id) => {
                    let peer_id = self.routes[&route_id].peer_id;
                    self.routes
                        .get_mut(&route_id)
                        .expect("checked route still exists")
                        .ping
                        .record_pong(round_trip_ms);
                    if self.preferred_route_id(peer_id, ConnectionTrafficClass::Message)
                        != Some(route_id)
                    {
                        continue;
                    }
                    self.peer_ping_ms.insert(peer_id, round_trip_ms);
                    return Ok(ClientRouteRead::PingMeasured {
                        peer_id,
                        round_trip_ms,
                    });
                }
                ClientRouteEvent::PingMeasured { .. } => {}
                ClientRouteEvent::PingDispatched { route_id } => {
                    if let Some(route) = self.routes.get_mut(&route_id) {
                        route.ping.record_ping_dispatched(Instant::now());
                    }
                }
                ClientRouteEvent::Disconnected {
                    route_id,
                    next_inbound_packet,
                    post_mortem,
                    reason,
                } => {
                    self.tasks.remove(&route_id);
                    let Some(route) = self.routes.remove(&route_id) else {
                        continue;
                    };
                    self.closed_routes
                        .retain(route_id, route.peer_id, next_inbound_packet);
                    self.closed_route_peers.insert(route_id, route.peer_addr);
                    if let Some(post_mortem) = self.pending_post_mortems.remove(&route_id) {
                        self.recover_post_mortem(route.peer_id, post_mortem);
                    }
                    let mut fallback_post_mortem = None;
                    if let Some(post_mortem) = post_mortem {
                        if self.preferred_route_id(
                            route.peer_id,
                            ConnectionTrafficClass::Message,
                        ).is_some()
                        {
                            if self
                                .try_send_to(
                                    route.peer_id,
                                    ControlMessage::PostMortem(post_mortem.clone()),
                                )
                                .is_err()
                            {
                                fallback_post_mortem = Some(post_mortem);
                            }
                        } else {
                            fallback_post_mortem = Some(post_mortem);
                        }
                    }
                    return Ok(ClientRouteRead::Disconnected {
                        peer_id: route.peer_id,
                        protocol: route.protocol,
                        routes_remaining: self
                            .routes
                            .values()
                            .any(|remaining| {
                                remaining.peer_id == route.peer_id
                                    && !remaining.outbound.is_closed()
                            }),
                        post_mortem: fallback_post_mortem,
                        reason,
                    });
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn read_packet(
        &mut self,
    ) -> Result<(crate::transport::InboundPacket, Option<SocketAddr>), TransportError> {
        loop {
            match self.read_event().await? {
                ClientRouteRead::Packet {
                    packet, peer_addr, ..
                } => return Ok((packet, peer_addr)),
                ClientRouteRead::PingMeasured { .. } => {}
                ClientRouteRead::Disconnected {
                    routes_remaining: true,
                    ..
                } => {}
                ClientRouteRead::Disconnected { reason, .. } => {
                    return Err(TransportError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        reason.unwrap_or_else(|| "all client transport routes closed".to_string()),
                    )));
                }
            }
        }
    }

    pub(crate) async fn shutdown(&mut self) {
        // Unblock any route worker waiting to publish into a full event queue
        // and interrupt a socket write before waiting for the task. Sending a
        // Shutdown command through the bounded outbound queue can deadlock
        // behind the write which shutdown is trying to cancel.
        self.event_rx.close();
        let senders = self
            .routes
            .values()
            .map(|route| route.outbound.clone())
            .collect::<Vec<_>>();
        self.routes.clear();
        for outbound in senders {
            outbound.retire();
        }
        for (_, task) in std::mem::take(&mut self.tasks) {
            let _ = task.await;
        }
    }
}

