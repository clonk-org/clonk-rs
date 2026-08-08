//! Client route registry: ClientTask, session liveness timer, route manager & route events.
//!
//! Moved byte-verbatim from `session.rs` (wave 2 of the decomposition
//! campaign, see REFACTOR_PLAN.md). Structural only.

use super::*;

const HOST_ROUTE_CLOSE_WRITE_GRACE: Duration = Duration::from_millis(25);

enum HostRouteWriterExit {
    Cancelled,
    OutboundClosed,
    ClosedByHost,
    Failed(String),
}

async fn wait_for_host_route_close(
    close_rx: &mut watch::Receiver<Option<crate::ConnectionReply>>,
) -> crate::ConnectionReply {
    loop {
        if let Some(reply) = close_rx.borrow_and_update().clone() {
            return reply;
        }
        if close_rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn udp_route_exit_notifies_disconnect(
    observed_disconnect: bool,
    close_rx: &watch::Receiver<Option<crate::ConnectionReply>>,
) -> bool {
    observed_disconnect && close_rx.borrow().is_none()
}

async fn run_host_route_writer<W>(
    mut transport: crate::ControlTransport<W>,
    mut outbound_rx: mpsc::UnboundedReceiver<HostOutboundMessage>,
    mut close_rx: watch::Receiver<Option<crate::ConnectionReply>>,
    mut cancel_rx: watch::Receiver<bool>,
) -> HostRouteWriterExit
where
    W: AsyncWrite + Unpin,
{
    let exit = loop {
        enum Next {
            Close(crate::ConnectionReply),
            Outbound(HostOutboundMessage),
        }

        let next = tokio::select! {
            biased;
            _ = wait_for_route_retirement(&mut cancel_rx) => {
                break HostRouteWriterExit::Cancelled;
            }
            reply = wait_for_host_route_close(&mut close_rx) => Next::Close(reply),
            outbound = outbound_rx.recv() => {
                let Some(message) = outbound else {
                    break HostRouteWriterExit::OutboundClosed;
                };
                Next::Outbound(message)
            }
        };

        if let Next::Close(reply) = next {
            // Native CloseConns makes one best-effort ConnRe send and then
            // closes immediately; it never drains stale OBuf first
            // (oracle-src-pinned src/C4Network2Client.cpp:104-118;
            // src/C4NetIO.cpp:1458-1468).
            if let Ok(frame) =
                transport.prepare_message_frame(ControlMessage::ConnectionReply(reply))
            {
                let _ = tokio::time::timeout(
                    HOST_ROUTE_CLOSE_WRITE_GRACE,
                    transport.send_prepared_frame(&frame),
                )
                .await;
            }
            break HostRouteWriterExit::ClosedByHost;
        }

        let frame = match next {
            Next::Outbound(HostOutboundMessage::Message(message)) => {
                transport.prepare_message_frame(message)
            }
            Next::Outbound(HostOutboundMessage::Raw(packet)) => {
                transport.prepare_complete_packet_frame(&packet)
            }
            Next::Close(_) => unreachable!("close handled before frame preparation"),
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => break HostRouteWriterExit::Failed(format!("send failed: {error}")),
        };
        let result = tokio::select! {
            biased;
            _reply = wait_for_host_route_close(&mut close_rx) => {
                break HostRouteWriterExit::ClosedByHost;
            }
            _ = wait_for_route_retirement(&mut cancel_rx) => {
                break HostRouteWriterExit::Cancelled;
            }
            result = transport.send_prepared_frame(&frame) => result,
        };
        if let Err(error) = result {
            break HostRouteWriterExit::Failed(format!("send failed: {error}"));
        }
    };

    outbound_rx.close();
    if !matches!(exit, HostRouteWriterExit::ClosedByHost) {
        while let Ok(message) = outbound_rx.try_recv() {
            let _ = match message {
                HostOutboundMessage::Message(message) => transport.retain_unsent_message(message),
                HostOutboundMessage::Raw(packet) => transport.retain_unsent_complete_packet(packet),
            };
        }
    }
    exit
}

fn enqueue_host_session_liveness_probe(
    liveness: &mut ConnectionLivenessState,
    outbound_tx: &mpsc::UnboundedSender<HostOutboundMessage>,
) -> Result<bool, String> {
    let ping = liveness
        .timer_tick()
        .map_err(|timeout| format!("connection {timeout:?} timeout"))?;
    let Some(ping) = ping else {
        return Ok(false);
    };
    let result = outbound_tx.send(HostOutboundMessage::Message(ControlMessage::Ping(ping)));
    // C4Network2IO calls OnPing after the send attempt even on failure
    // (oracle-src-pinned src/C4Network2IO.cpp:1141-1151).
    liveness.record_ping_dispatched();
    result.map_err(|_| "ping send failed: route writer closed".to_string())?;
    Ok(true)
}

pub(crate) struct ClientTask<S> {
    pub(crate) local_connection_id: u32,
    pub(crate) remote_connection_id: u32,
    pub(crate) client_id: ClientId,
    pub(crate) transport: crate::ControlTransport<S>,
    pub(crate) outbound_rx: HostOutboundReceiver,
    pub(crate) retire_rx: watch::Receiver<bool>,
    pub(crate) host_tx: mpsc::UnboundedSender<HostLoopMessage>,
    pub(crate) liveness: ConnectionLivenessState,
}

impl<S> ClientTask<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    pub(crate) async fn run(self) {
        let ClientTask {
            local_connection_id,
            remote_connection_id,
            client_id,
            transport,
            outbound_rx,
            mut retire_rx,
            host_tx,
            mut liveness,
        } = self;
        let (mut transport, writer) = transport.into_split();
        let (outbound_tx, outbound_rx, close_rx) = outbound_rx.into_parts();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let mut writer_task = tokio::spawn(run_host_route_writer(
            writer,
            outbound_rx,
            close_rx,
            cancel_rx,
        ));
        let mut writer_finished = false;
        let mut disconnect_reason = None;
        let mut notify_disconnect = true;
        let mut liveness_timer = new_liveness_timer(liveness.next_timer_at());
        loop {
            let liveness_deadline = liveness.next_timer_at();
            if liveness_timer.deadline() != liveness_deadline {
                liveness_timer.as_mut().reset(liveness_deadline);
            }
            tokio::select! {
                _ = wait_for_route_retirement(&mut retire_rx) => {
                    break;
                }
                writer_result = &mut writer_task => {
                    writer_finished = true;
                    match writer_result {
                        Ok(HostRouteWriterExit::Cancelled) => {}
                        Ok(HostRouteWriterExit::OutboundClosed)
                        | Ok(HostRouteWriterExit::ClosedByHost) => {
                            notify_disconnect = false;
                        }
                        Ok(HostRouteWriterExit::Failed(reason)) => {
                            disconnect_reason = Some(reason);
                        }
                        Err(error) => {
                            disconnect_reason = Some(format!("route writer task failed: {error}"));
                        }
                    }
                    break;
                }
                packet = transport.read_packet() => {
                    let result = match packet {
                        Ok(crate::transport::InboundPacket::Message(message)) => {
                            liveness.record_inbound_message(&message);
                            Ok(message)
                        }
                        Ok(crate::transport::InboundPacket::Ignored(packet_type)) => {
                            liveness.record_inbound_packet(packet_type);
                            if host_tx
                                .send(HostLoopMessage::UnhandledPacket {
                                    client_id: Some(client_id),
                                    packet_type,
                                })
                                .is_err()
                            {
                                notify_disconnect = false;
                                break;
                            }
                            continue;
                        }
                        Ok(crate::transport::InboundPacket::Empty) => continue,
                        Ok(crate::transport::InboundPacket::Invalid {
                            packet_type,
                            error,
                        }) => {
                            liveness.record_inbound_packet(packet_type);
                            Err(error)
                        }
                        Err(error) => Err(error),
                    };
                    match result {
                        Ok(ControlMessage::Ping(packet)) => {
                            if outbound_tx
                                .send(HostOutboundMessage::Message(ControlMessage::Pong(packet)))
                                .is_err()
                            {
                                disconnect_reason =
                                    Some("pong send failed: route writer closed".to_string());
                                break;
                            }
                        }
                        Ok(ControlMessage::Pong(packet)) => {
                            let round_trip_ms = liveness.record_pong(packet);
                            if host_tx
                                .send(HostLoopMessage::ConnectionPing {
                                    connection_id: local_connection_id,
                                    client_id,
                                    update: RoutePingUpdate::Measured(round_trip_ms),
                                })
                                .is_err()
                            {
                                notify_disconnect = false;
                                break;
                            }
                        }
                        Ok(ControlMessage::ConnectionReply(reply)) if !reply.ok => {
                            disconnect_reason = Some(
                                clonk_resources::decode_legacy_script_text(reply.message.as_bytes()),
                            );
                            break;
                        }
                        Ok(message) => {
                            let ping_ms = liveness
                                .connection()
                                .measured_ping_ms()
                                .unwrap_or(-1);
                            if host_tx
                                .send(HostLoopMessage::ClientMessage {
                                    connection_id: local_connection_id,
                                    client_id,
                                    message,
                                    ping_ms,
                                })
                                .is_err()
                            {
                                notify_disconnect = false;
                                break;
                            }
                        }
                        Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                            break;
                        }
                        Err(error) => {
                            disconnect_reason = Some(format!("read failed: {error}"));
                            break;
                        }
                    }
                }
                _ = liveness_timer.as_mut() => {
                    match enqueue_host_session_liveness_probe(&mut liveness, &outbound_tx) {
                        Ok(true) => {
                            if host_tx
                                .send(HostLoopMessage::ConnectionPing {
                                    connection_id: local_connection_id,
                                    client_id,
                                    update: RoutePingUpdate::Dispatched,
                                })
                                .is_err()
                            {
                                notify_disconnect = false;
                                break;
                            }
                        }
                        Ok(false) => {}
                        Err(reason) => {
                            disconnect_reason = Some(reason);
                            break;
                        }
                    }
                }
            }
        }
        cancel_tx.send_replace(true);
        drop(outbound_tx);
        if !writer_finished {
            let _ = writer_task.await;
        }
        if notify_disconnect {
            // A successful channel enqueue is a successful logical send.
            // The writer retained every accepted-but-unwritten packet before
            // completing cancellation, so the shared log is complete here.
            let next_outbound_packet = transport.outbound_packet_counter();
            let post_mortem = transport.create_post_mortem(remote_connection_id);
            let _ = host_tx.send(HostLoopMessage::ClientDisconnected {
                connection_id: local_connection_id,
                client_id,
                next_inbound_packet: liveness.connection().inbound_packet_counter(),
                next_outbound_packet,
                post_mortem,
                reason: disconnect_reason,
            });
        }
    }
}

pub(crate) struct UdpClientTask<S> {
    pub(crate) local_connection_id: u32,
    pub(crate) remote_connection_id: u32,
    pub(crate) client_id: ClientId,
    pub(crate) transport: crate::ControlTransport<S>,
    pub(crate) outbound: HostOutboundSender,
    pub(crate) retire_rx: watch::Receiver<bool>,
    pub(crate) host_tx: mpsc::UnboundedSender<HostLoopMessage>,
    pub(crate) liveness: ConnectionLivenessState,
}

impl<S> UdpClientTask<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    pub(crate) async fn run(self) {
        let UdpClientTask {
            local_connection_id,
            remote_connection_id,
            client_id,
            transport,
            outbound,
            mut retire_rx,
            host_tx,
            mut liveness,
        } = self;
        let (mut transport, writer) = transport.into_split();
        drop(writer);
        let mut close_rx = outbound.subscribe_close();
        let mut disconnect_reason = None;
        let mut notify_disconnect = true;
        let mut liveness_timer = new_liveness_timer(liveness.next_timer_at());
        loop {
            let liveness_deadline = liveness.next_timer_at();
            if liveness_timer.deadline() != liveness_deadline {
                liveness_timer.as_mut().reset(liveness_deadline);
            }
            tokio::select! {
                biased;
                _reply = wait_for_host_route_close(&mut close_rx) => {
                    notify_disconnect = false;
                    break;
                }
                _ = wait_for_route_retirement(&mut retire_rx) => break,
                packet = transport.read_packet() => {
                    let result = match packet {
                        Ok(crate::transport::InboundPacket::Message(message)) => {
                            liveness.record_inbound_message(&message);
                            Ok(message)
                        }
                        Ok(crate::transport::InboundPacket::Ignored(packet_type)) => {
                            liveness.record_inbound_packet(packet_type);
                            if host_tx
                                .send(HostLoopMessage::UnhandledPacket {
                                    client_id: Some(client_id),
                                    packet_type,
                                })
                                .is_err()
                            {
                                notify_disconnect = false;
                                break;
                            }
                            continue;
                        }
                        Ok(crate::transport::InboundPacket::Empty) => continue,
                        Ok(crate::transport::InboundPacket::Invalid { packet_type, error }) => {
                            liveness.record_inbound_packet(packet_type);
                            Err(error)
                        }
                        Err(error) => Err(error),
                    };
                    match result {
                        Ok(ControlMessage::Ping(packet)) => {
                            if outbound.try_send(ControlMessage::Pong(packet)).is_err() {
                                disconnect_reason =
                                    Some("pong send failed: UDP outbox closed".to_string());
                                break;
                            }
                        }
                        Ok(ControlMessage::Pong(packet)) => {
                            let round_trip_ms = liveness.record_pong(packet);
                            if host_tx
                                .send(HostLoopMessage::ConnectionPing {
                                    connection_id: local_connection_id,
                                    client_id,
                                    update: RoutePingUpdate::Measured(round_trip_ms),
                                })
                                .is_err()
                            {
                                notify_disconnect = false;
                                break;
                            }
                        }
                        Ok(ControlMessage::ConnectionReply(reply)) if !reply.ok => {
                            disconnect_reason = Some(
                                clonk_resources::decode_legacy_script_text(reply.message.as_bytes()),
                            );
                            break;
                        }
                        Ok(message) => {
                            let ping_ms = liveness
                                .connection()
                                .measured_ping_ms()
                                .unwrap_or(-1);
                            if host_tx
                                .send(HostLoopMessage::ClientMessage {
                                    connection_id: local_connection_id,
                                    client_id,
                                    message,
                                    ping_ms,
                                })
                                .is_err()
                            {
                                notify_disconnect = false;
                                break;
                            }
                        }
                        Err(TransportError::Io(error))
                            if error.kind() == io::ErrorKind::UnexpectedEof => break,
                        Err(error) => {
                            disconnect_reason = Some(format!("read failed: {error}"));
                            break;
                        }
                    }
                }
                _ = liveness_timer.as_mut() => {
                    let ping = match liveness.timer_tick() {
                        Ok(ping) => ping,
                        Err(timeout) => {
                            disconnect_reason = Some(format!("connection {timeout:?} timeout"));
                            break;
                        }
                    };
                    if let Some(ping) = ping {
                        let sent = outbound.try_send(ControlMessage::Ping(ping));
                        liveness.record_ping_dispatched();
                        if sent.is_err() {
                            disconnect_reason =
                                Some("ping send failed: UDP outbox closed".to_string());
                            break;
                        }
                        if host_tx
                            .send(HostLoopMessage::ConnectionPing {
                                connection_id: local_connection_id,
                                client_id,
                                update: RoutePingUpdate::Dispatched,
                            })
                            .is_err()
                        {
                            notify_disconnect = false;
                            break;
                        }
                    }
                }
            }
        }
        notify_disconnect = udp_route_exit_notifies_disconnect(notify_disconnect, &close_rx);
        if notify_disconnect {
            outbound.retire();
        }
        outbound.wait_udp_drained().await;
        if notify_disconnect {
            let next_outbound_packet = transport.outbound_packet_counter();
            let post_mortem = transport.create_post_mortem(remote_connection_id);
            let _ = host_tx.send(HostLoopMessage::ClientDisconnected {
                connection_id: local_connection_id,
                client_id,
                next_inbound_packet: liveness.connection().inbound_packet_counter(),
                next_outbound_packet,
                post_mortem,
                reason: disconnect_reason,
            });
        }
    }
}

pub(crate) enum ClientRouteCommand {
    Message(ControlMessage),
    Flush(oneshot::Sender<()>),
}

#[derive(Clone)]
pub(crate) struct ClientRouteSender {
    pub(crate) sender: mpsc::UnboundedSender<ClientRouteCommand>,
    pub(crate) retire: watch::Sender<bool>,
    pub(crate) post_failure: PostFailureBuffer<ClientRouteCommand>,
    pub(crate) udp: Option<crate::udp_session::ReliableUdpRouteSender>,
}

impl ClientRouteSender {
    pub(crate) fn is_closed(&self) -> bool {
        self.is_retiring() || !self.post_failure.is_accepting()
    }

    pub(crate) fn is_retiring(&self) -> bool {
        *self.retire.borrow()
    }

    fn accepts_post_failure_fifo(&self) -> bool {
        self.post_failure.is_accepting()
    }

    fn send(
        &self,
        command: ClientRouteCommand,
    ) -> Result<(), mpsc::error::SendError<ClientRouteCommand>> {
        if let Some(udp) = &self.udp {
            let command = match command {
                ClientRouteCommand::Message(message) => match udp.try_send(message) {
                    Ok(()) => return Ok(()),
                    Err(message) => ClientRouteCommand::Message(message),
                },
                ClientRouteCommand::Flush(completion) if udp.is_accepting() => {
                    let _ = completion.send(());
                    return Ok(());
                }
                ClientRouteCommand::Flush(completion) => ClientRouteCommand::Flush(completion),
            };
            return self
                .post_failure
                .retain(command)
                .map_err(mpsc::error::SendError);
        }
        match self.sender.send(command) {
            Ok(()) => Ok(()),
            Err(mpsc::error::SendError(command)) => self
                .post_failure
                .retain(command)
                .map_err(mpsc::error::SendError),
        }
    }

    pub(crate) fn retire(&self) {
        // Cancellation stops the route task, but only Disconnected may expose
        // a fallback: it first removes this route, then closes and drains the
        // retained suffix into the route's PostMortem packet.
        if let Some(udp) = &self.udp {
            udp.retire();
        }
        self.retire.send_replace(true);
    }

    fn retire_and_take_post_failure(&self) -> VecDeque<ClientRouteCommand> {
        if let Some(udp) = &self.udp {
            udp.retire();
        }
        let commands = self.post_failure.close_and_drain();
        self.retire.send_replace(true);
        commands
    }

    fn send_many(routes: &[Self], message: ControlMessage) -> Option<Vec<bool>> {
        let udp = routes
            .iter()
            .map(|route| route.udp.clone())
            .collect::<Option<Vec<_>>>()?;
        let accepted =
            crate::udp_session::ReliableUdpRouteSender::try_send_many(&udp, message.clone())?;
        Some(
            routes
                .iter()
                .zip(accepted)
                .map(|(route, accepted)| {
                    accepted
                        || route
                            .post_failure
                            .retain(ClientRouteCommand::Message(message.clone()))
                            .is_ok()
                })
                .collect(),
        )
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
        next_outbound_packet: u32,
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
    pub(crate) event_tx: mpsc::UnboundedSender<ClientRouteEvent>,
    event_rx: mpsc::UnboundedReceiver<ClientRouteEvent>,
    tasks: BTreeMap<u32, tokio::task::JoinHandle<()>>,
    pub(crate) closed_routes: crate::post_mortem::ClosedConnectionRouter,
    closed_route_peers: BTreeMap<u32, Option<SocketAddr>>,
    pub(crate) pending_post_mortems: BTreeMap<u32, crate::PostMortemPacket>,
    peer_ping_ms: BTreeMap<ClientId, i32>,
    control_send_time_dirty: bool,
    pub(crate) replay_packets: VecDeque<(
        ClientId,
        crate::transport::InboundPacket,
        Option<SocketAddr>,
    )>,
}

impl ClientRouteManager {
    pub(crate) fn new() -> Self {
        // C4InteractiveThread::PushEvent appends accepted network events to
        // an uncapped FIFO linked list. The network thread must never stop
        // parsing later Ping/Pong frames because bootstrap or the main thread
        // has not consumed an earlier event
        // (oracle-src-pinned src/C4InteractiveThread.cpp:70-100;
        // src/C4Packet2.cpp:51-73).
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            routes: BTreeMap::new(),
            event_tx,
            event_rx,
            tasks: BTreeMap::new(),
            closed_routes: crate::post_mortem::ClosedConnectionRouter::default(),
            closed_route_peers: BTreeMap::new(),
            pending_post_mortems: BTreeMap::new(),
            peer_ping_ms: BTreeMap::new(),
            control_send_time_dirty: true,
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_udp_route<S>(
        &mut self,
        local_connection_id: u32,
        remote_connection_id: u32,
        peer_addr: Option<SocketAddr>,
        transport: crate::ControlTransport<S>,
        liveness: ConnectionLivenessState,
        outbound: crate::udp_session::ReliableUdpRouteSender,
    ) where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.add_udp_peer_route(
            HOST_CLIENT_ID,
            HOST_CLIENT_ID,
            local_connection_id,
            remote_connection_id,
            peer_addr,
            transport,
            liveness,
            outbound,
        );
    }

    // Each argument is a distinct piece of classic route identity or owned
    // task state; keeping them explicit prevents local/remote IDs from being
    // silently conflated.
    #[allow(clippy::too_many_arguments)]
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
        self.add_peer_route_with_udp(
            peer_id,
            initiator_id,
            local_connection_id,
            remote_connection_id,
            protocol,
            peer_addr,
            transport,
            liveness,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_udp_peer_route<S>(
        &mut self,
        peer_id: ClientId,
        initiator_id: ClientId,
        local_connection_id: u32,
        remote_connection_id: u32,
        peer_addr: Option<SocketAddr>,
        transport: crate::ControlTransport<S>,
        liveness: ConnectionLivenessState,
        outbound: crate::udp_session::ReliableUdpRouteSender,
    ) -> bool
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.add_peer_route_with_udp(
            peer_id,
            initiator_id,
            local_connection_id,
            remote_connection_id,
            crate::NetworkProtocol::Udp,
            peer_addr,
            transport,
            liveness,
            Some(outbound),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_peer_route_with_udp<S>(
        &mut self,
        peer_id: ClientId,
        initiator_id: ClientId,
        local_connection_id: u32,
        remote_connection_id: u32,
        protocol: crate::NetworkProtocol,
        peer_addr: Option<SocketAddr>,
        transport: crate::ControlTransport<S>,
        liveness: ConnectionLivenessState,
        udp: Option<crate::udp_session::ReliableUdpRouteSender>,
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
        // C4Network2Client::SendMsg delegates to each connection's buffered
        // nonblocking Send; the UDP transport, not this app-level route queue,
        // owns the 10,000-packet retransmit window (oracle-src-pinned
        // src/C4Network2Client.cpp:121-124;
        // src/C4NetIO.cpp:1345-1357,1916,2788-2808).
        let (sender, outbound_rx) = mpsc::unbounded_channel();
        let route_tx = sender.clone();
        let (retire, retire_rx) = watch::channel(false);
        let post_failure = PostFailureBuffer::default();
        let udp_task = udp.clone();
        let replaced = self.routes.insert(
            local_connection_id,
            ClientRouteEntry {
                peer_id,
                initiator_id,
                remote_connection_id,
                protocol,
                peer_addr,
                ping: RoutePingLag::default(),
                outbound: ClientRouteSender {
                    sender,
                    retire,
                    post_failure,
                    udp,
                },
            },
        );
        self.control_send_time_dirty = true;
        debug_assert!(replaced.is_none());
        let events = self.event_tx.clone();
        let task = if let Some(udp) = udp_task {
            drop(outbound_rx);
            tokio::spawn(run_udp_client_route(
                local_connection_id,
                remote_connection_id,
                peer_addr,
                transport,
                udp,
                retire_rx,
                events,
                liveness,
            ))
        } else {
            tokio::spawn(run_client_route(
                local_connection_id,
                remote_connection_id,
                peer_addr,
                transport,
                route_tx,
                outbound_rx,
                retire_rx,
                events,
                liveness,
            ))
        };
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
        self.select_preferred_route_id(peer_id, traffic, false)
    }

    fn preferred_send_route_id(
        &self,
        peer_id: ClientId,
        traffic: ConnectionTrafficClass,
    ) -> Option<u32> {
        self.select_preferred_route_id(peer_id, traffic, true)
    }

    fn select_preferred_route_id(
        &self,
        peer_id: ClientId,
        traffic: ConnectionTrafficClass,
        include_retiring: bool,
    ) -> Option<u32> {
        self.routes
            .iter()
            .filter(|(_, route)| {
                route.peer_id == peer_id
                    && if include_retiring {
                        route.outbound.accepts_post_failure_fifo()
                    } else {
                        !route.outbound.is_closed()
                    }
            })
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

    pub(crate) async fn send_message(
        &mut self,
        message: ControlMessage,
    ) -> Result<(), TransportError> {
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
            let Some(route_id) = self.preferred_send_route_id(peer_id, traffic) else {
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
            match outbound.send(ClientRouteCommand::Message(message)) {
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
            let Some(route_id) = self.preferred_send_route_id(peer_id, traffic) else {
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
            match outbound.send(ClientRouteCommand::Message(message)) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::SendError(command)) => {
                    message = match command {
                        ClientRouteCommand::Message(message) => message,
                        ClientRouteCommand::Flush(_) => {
                            unreachable!("try_send_to only queues message commands")
                        }
                    };
                }
            }
        }
    }

    pub(crate) async fn flush_to(&mut self, peer_id: ClientId) -> Result<(), TransportError> {
        loop {
            let Some(route_id) =
                self.preferred_send_route_id(peer_id, ConnectionTrafficClass::Message)
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
            match outbound.send(ClientRouteCommand::Flush(completion)) {
                Ok(()) => {}
                Err(mpsc::error::SendError(_)) => continue,
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
            .filter(|route| route.outbound.accepts_post_failure_fifo())
            .map(|route| route.peer_id)
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn control_send_time_ms(
        &self,
        control_mode: i32,
        remote_client_ids: impl IntoIterator<Item = ClientId>,
    ) -> i32 {
        let remote_clients = remote_client_ids.into_iter().collect::<BTreeSet<_>>();
        let preferred_message_ping_ms = self.preferred_control_message_ping_ms(&remote_clients);

        control_send_time_ms(
            control_mode,
            remote_clients.into_iter().map(|client_id| {
                (
                    client_id,
                    preferred_message_ping_ms.get(&client_id).copied(),
                )
            }),
        )
    }

    pub(crate) fn publish_control_send_time(
        &mut self,
        snapshot: &ControlSendTimeSnapshot,
        control_mode: i32,
        local_client_id: ClientId,
        known_clients: BTreeSet<ClientId>,
    ) {
        if !self.control_send_time_dirty {
            return;
        }
        let preferred_message_ping_ms = self.preferred_control_message_ping_ms(&known_clients);
        snapshot.publish(
            control_mode,
            local_client_id,
            known_clients,
            preferred_message_ping_ms,
        );
        self.control_send_time_dirty = false;
    }

    pub(crate) fn control_send_time_needs_publish(&self) -> bool {
        self.control_send_time_dirty
    }

    pub(crate) fn invalidate_control_send_time(&mut self) {
        self.control_send_time_dirty = true;
    }

    fn preferred_control_message_ping_ms(
        &self,
        clients: &BTreeSet<ClientId>,
    ) -> BTreeMap<ClientId, i32> {
        let mut preferred_message_routes =
            BTreeMap::<ClientId, ((u8, ClientId, u32, u32), i32)>::new();

        for (route_id, route) in &self.routes {
            if route.outbound.is_closed() || !clients.contains(&route.peer_id) {
                continue;
            }
            let protocol_rank = match route.protocol {
                crate::NetworkProtocol::Udp => 0,
                crate::NetworkProtocol::Tcp => 1,
                _ => 2,
            };
            let preference = (
                protocol_rank,
                route.initiator_id,
                (*route_id).min(route.remote_connection_id),
                (*route_id).max(route.remote_connection_id),
            );
            if preferred_message_routes
                .get(&route.peer_id)
                .is_none_or(|(best, _)| preference < *best)
            {
                preferred_message_routes.insert(route.peer_id, (preference, route.ping.ping_ms()));
            }
        }

        preferred_message_routes
            .into_iter()
            .map(|(client_id, (_, ping_ms))| (client_id, ping_ms))
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

    pub(crate) fn disconnect_runtime_connection(&mut self, connection_id: u32) -> bool {
        let Some(route) = self
            .routes
            .get(&connection_id)
            .filter(|route| !route.outbound.is_retiring())
        else {
            return false;
        };
        route.outbound.retire();
        self.control_send_time_dirty = true;
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
        self.control_send_time_dirty = true;
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
        self.control_send_time_dirty = true;
    }

    pub(crate) fn send_to_connected_peers(&mut self, message: ControlMessage) -> Vec<ClientId> {
        // C++ selects each logical client's cached message connection, then
        // submits one broadcast through those selected connections
        // (src/C4Network2Client.cpp:497-541). Select one route per peer in a
        // single pass. Calling `try_send_to` for every peer would scan this
        // entire route registry once per peer.
        let mut preferred_routes = BTreeMap::<ClientId, ((u8, ClientId, u32, u32), u32)>::new();
        for (route_id, route) in &self.routes {
            if route.peer_id == HOST_CLIENT_ID || !route.outbound.accepts_post_failure_fifo() {
                continue;
            }
            let protocol_rank = match route.protocol {
                crate::NetworkProtocol::Udp => 0,
                crate::NetworkProtocol::Tcp => 1,
                _ => 2,
            };
            let preference = (
                protocol_rank,
                route.initiator_id,
                (*route_id).min(route.remote_connection_id),
                (*route_id).max(route.remote_connection_id),
            );
            if preferred_routes
                .get(&route.peer_id)
                .is_none_or(|(best, _)| preference < *best)
            {
                preferred_routes.insert(route.peer_id, (preference, *route_id));
            }
        }
        let selected = preferred_routes
            .into_iter()
            .map(|(peer_id, (_, route_id))| {
                (
                    peer_id,
                    self.routes
                        .get(&route_id)
                        .expect("selected client broadcast route exists")
                        .outbound
                        .clone(),
                )
            })
            .collect::<Vec<_>>();
        if let Some(results) = ClientRouteSender::send_many(
            &selected
                .iter()
                .map(|(_, outbound)| outbound.clone())
                .collect::<Vec<_>>(),
            message.clone(),
        ) {
            let mut sent = Vec::new();
            for ((peer_id, _), accepted) in selected.into_iter().zip(results) {
                if accepted || self.try_send_to(peer_id, message.clone()).is_ok() {
                    sent.push(peer_id);
                } else {
                    self.retire_peer_gracefully(peer_id);
                }
            }
            return sent;
        }

        let mut sent = Vec::new();
        for (peer_id, outbound) in selected {
            match outbound.send(ClientRouteCommand::Message(message.clone())) {
                Ok(()) => sent.push(peer_id),
                Err(mpsc::error::SendError(ClientRouteCommand::Message(message))) => {
                    if self.try_send_to(peer_id, message).is_ok() {
                        sent.push(peer_id);
                    } else {
                        self.retire_peer_gracefully(peer_id);
                    }
                }
                Err(mpsc::error::SendError(ClientRouteCommand::Flush(_))) => {
                    unreachable!("broadcast only queues message commands")
                }
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
            .filter(|route| source_peer_id == HOST_CLIENT_ID || route.peer_id == source_peer_id)
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
                    self.control_send_time_dirty = true;
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
                    mut next_outbound_packet,
                    mut post_mortem,
                    reason,
                } => {
                    self.tasks.remove(&route_id);
                    let Some(route) = self.routes.remove(&route_id) else {
                        continue;
                    };
                    self.control_send_time_dirty = true;
                    for command in route.outbound.retire_and_take_post_failure() {
                        match command {
                            ClientRouteCommand::Message(message) => {
                                if let Ok(packet) =
                                    crate::transport::encode_complete_message(message)
                                {
                                    crate::post_mortem::retain_post_failure_packet(
                                        &mut post_mortem,
                                        route.remote_connection_id,
                                        &mut next_outbound_packet,
                                        packet,
                                    );
                                }
                            }
                            ClientRouteCommand::Flush(completion) => {
                                let _ = completion.send(());
                            }
                        }
                    }
                    self.closed_routes
                        .retain(route_id, route.peer_id, next_inbound_packet);
                    self.closed_route_peers.insert(route_id, route.peer_addr);
                    if let Some(post_mortem) = self.pending_post_mortems.remove(&route_id) {
                        self.recover_post_mortem(route.peer_id, post_mortem);
                    }
                    let mut fallback_post_mortem = None;
                    if let Some(post_mortem) = post_mortem {
                        if self
                            .preferred_route_id(route.peer_id, ConnectionTrafficClass::Message)
                            .is_some()
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
                        routes_remaining: self.routes.values().any(|remaining| {
                            remaining.peer_id == route.peer_id && !remaining.outbound.is_closed()
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
        self.control_send_time_dirty = true;
        for outbound in senders {
            outbound.retire();
        }
        for (_, task) in std::mem::take(&mut self.tasks) {
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod udp_outbox_tests {
    use super::*;

    #[test]
    fn udp_explicit_close_dominates_simultaneously_ready_eof() {
        // CloseConns publishes its intentional route removal before the
        // resulting socket EOF can be reported as a second disconnect
        // (oracle-src-pinned src/C4Network2Client.cpp:104-119,457-492).
        let (close, close_rx) = watch::channel(None);
        close.send_replace(Some(crate::ConnectionReply {
            ok: false,
            message: clonk_engine::LegacyCString::from_bytes(b"closed".to_vec()).unwrap(),
            wrong_password: false,
        }));

        assert!(!udp_route_exit_notifies_disconnect(true, &close_rx));
    }

    #[test]
    fn udp_route_failure_moves_later_recoverable_sends_into_post_failure_fifo() {
        // A closed C4Network2IO connection retains packets accepted before
        // Ev_Net_Disconn removes the route, then emits them in PostMortem
        // order (oracle-src-pinned src/C4Network2IO.cpp:718-738,1437-1477).
        let (sender, _receiver) = mpsc::unbounded_channel();
        let (retire, _) = watch::channel(false);
        let udp = crate::udp_session::ReliableUdpRouteSender::test_sender();
        let outbound = ClientRouteSender {
            sender,
            retire,
            post_failure: PostFailureBuffer::default(),
            udp: Some(udp.clone()),
        };
        let accepted = ControlMessage::Status(crate::NetworkStatus {
            state: crate::NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 2,
        });
        let after_failure = [
            ControlMessage::Status(crate::NetworkStatus {
                state: crate::NETWORK_STATE_PAUSE,
                control_mode: 3,
                target_tick: 4,
            }),
            ControlMessage::StatusAck(crate::NetworkStatus {
                state: crate::NETWORK_STATE_GO,
                control_mode: 5,
                target_tick: 6,
            }),
        ];

        assert!(outbound.send(ClientRouteCommand::Message(accepted)).is_ok());
        udp.test_fail();
        for message in &after_failure {
            assert!(outbound
                .send(ClientRouteCommand::Message(message.clone()))
                .is_ok());
        }

        let retained = outbound.retire_and_take_post_failure();
        let expected = after_failure
            .into_iter()
            .map(|message| crate::transport::encode_complete_message(message).unwrap())
            .collect::<Vec<_>>();
        let mut post_mortem = None;
        let mut next_packet = 11;
        for command in retained {
            let ClientRouteCommand::Message(message) = command else {
                panic!("test queues only messages");
            };
            crate::post_mortem::retain_post_failure_packet(
                &mut post_mortem,
                23,
                &mut next_packet,
                crate::transport::encode_complete_message(message).unwrap(),
            );
        }
        assert_eq!(post_mortem.unwrap().packets, expected);
        assert_eq!(next_packet, 13);
    }
}
