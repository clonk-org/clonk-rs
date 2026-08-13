use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clonk_engine::{ClientCoreControlData, LegacyCString};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::{
    AddressPacket, AdmissionDecision, ClientAdmission, ConnectionAction, ConnectionLiveness,
    ConnectionRequest, ConnectionStatus, ConnectionTimeout, ControlMessage, ControlPacket,
    ControlTransport, JoinDataEnvelope, KnownPeerAdmission, LegacyConnection, LivenessClock,
    LobbyCountdownPacket, PingPacket, PingSchedule, ReadyCheckPacket, ResourcePacket,
    TransportError, NETWORK_TIMER_INTERVAL_MS,
};

/// The synchronized values established by the client-side C++ connection
/// admission exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConnectionHandshake {
    pub local_connection_id: u32,
    pub remote_connection_id: u32,
    pub peer_core: ClientCoreControlData,
    pub join_data: JoinDataEnvelope,
    /// Kept empty for API compatibility. C++ may receive resource packets
    /// before JoinData, but its resource list is still empty and ignores them;
    /// periodic discovery restarts negotiation after registration
    /// (`src/C4Network2.cpp:938-946,1768-1784,1820-1850`).
    pub pending_resources: Vec<ResourcePacket>,
    /// Complete controls received while C4GameControlNetwork is not enabled.
    pub pending_controls: Vec<ControlPacket>,
    /// Addresses for the already registered host received before JoinData
    /// installs the complete client registry.
    pub pending_addresses: Vec<AddressPacket>,
    /// Ready-check packets received after admission but before JoinData.
    pub pending_ready_checks: Vec<ReadyCheckPacket>,
    /// Lobby countdown packets received after admission but before JoinData.
    pub pending_lobby_countdowns: Vec<LobbyCountdownPacket>,
    pub liveness: ConnectionLivenessState,
}

/// One additional accepted transport route for an already joined logical
/// client. Unlike [`ClientConnectionHandshake`], this exchange deliberately
/// stops after `PID_Conn`/`PID_ConnRe`: a host does not send another JoinData
/// when it recognizes the client's canonical core on a second connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientRouteHandshake {
    pub local_connection_id: u32,
    pub remote_connection_id: u32,
    pub peer_core: ClientCoreControlData,
    pub liveness: ConnectionLivenessState,
}

/// A peer request handed from an I/O task to the serialized host state.
///
/// The receiver applies any `AdmissionDecision::Accept::before_reply` effects
/// before returning the decision through `decision_tx`.
#[derive(Debug)]
pub struct HostAdmissionRequest {
    pub connection_id: u32,
    pub request: ConnectionRequest,
    pub decision_tx: oneshot::Sender<AdmissionDecision>,
}

/// The canonical peer established by the host-side C++ admission exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConnectionHandshake {
    pub local_connection_id: u32,
    pub remote_connection_id: u32,
    pub peer_core: ClientCoreControlData,
    pub liveness: ConnectionLivenessState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WallClockSource {
    System,
    #[cfg(test)]
    Monotonic {
        origin_seconds: i64,
    },
}

/// Ping schedule, per-connection counters, and clock phase carried across the
/// admission/session boundary.
///
/// C++ owns the 500 ms timer and ping cadence in `C4Network2IO`, outside the
/// individual connection (`src/C4Network2IO.cpp:605-617,1139-1151`). Keeping
/// the next timer edge here lets the Rust session continue that phase without
/// restarting either the one-second gate or the connection timeout clocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionLivenessState {
    connection: ConnectionLiveness,
    ping_schedule: PingSchedule,
    monotonic_origin: Instant,
    monotonic_origin_ms: u64,
    next_timer_at: Instant,
    wall_clock: WallClockSource,
}

impl ConnectionLivenessState {
    pub(crate) fn new_system() -> Self {
        Self::new(
            Instant::now(),
            system_monotonic_seed_ms(),
            WallClockSource::System,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_accepted_system() -> Self {
        let mut state = Self::new_system();
        state.mark_accepted();
        state
    }

    #[cfg(test)]
    pub(crate) fn new_test(origin_ms: u64, origin_seconds: i64) -> Self {
        Self::new(
            Instant::now(),
            origin_ms,
            WallClockSource::Monotonic { origin_seconds },
        )
    }

    fn new(
        monotonic_origin: Instant,
        monotonic_origin_ms: u64,
        wall_clock: WallClockSource,
    ) -> Self {
        let wall_seconds = match wall_clock {
            WallClockSource::System => system_wall_seconds(),
            #[cfg(test)]
            WallClockSource::Monotonic { origin_seconds } => origin_seconds,
        };
        Self {
            connection: ConnectionLiveness::new_connected(wall_seconds),
            ping_schedule: PingSchedule::new(monotonic_origin_ms),
            monotonic_origin,
            monotonic_origin_ms,
            next_timer_at: monotonic_origin + Duration::from_millis(NETWORK_TIMER_INTERVAL_MS),
            wall_clock,
        }
    }

    pub fn connection(&self) -> &ConnectionLiveness {
        &self.connection
    }

    pub fn ping_schedule(&self) -> PingSchedule {
        self.ping_schedule
    }

    pub fn next_timer_at(&self) -> Instant {
        self.next_timer_at
    }

    pub fn now(&self) -> LivenessClock {
        self.clock_at(Instant::now())
    }

    /// Executes one `C4NetTimer` edge: timeout first, then the shared ping gate.
    pub fn timer_tick(&mut self) -> Result<Option<PingPacket>, ConnectionTimeout> {
        let now_instant = Instant::now();
        self.next_timer_at = now_instant + Duration::from_millis(NETWORK_TIMER_INTERVAL_MS);
        let now = self.clock_at(now_instant);
        if let Some(timeout) = self.connection.check_timeout(now) {
            return Err(timeout);
        }
        if !self.ping_schedule.take_due(now.monotonic_ms()) {
            return Ok(None);
        }
        let probe = self.connection.make_ping(now.monotonic_ms());
        Ok(Some(PingPacket {
            sent_at: probe.sent_at,
            packet_counter: probe.packet_counter,
        }))
    }

    /// Mirrors `OnPing`, which runs after the C++ send attempt.
    pub fn record_ping_dispatched(&mut self) {
        self.connection
            .record_ping_dispatched(self.now().monotonic_ms());
    }

    pub fn record_inbound_packet(&mut self, packet_type: u8) {
        self.connection.record_inbound_packet(packet_type);
    }

    pub fn record_inbound_message(&mut self, message: &ControlMessage) {
        self.record_inbound_packet(packet_type(message));
    }

    pub fn record_pong(&mut self, packet: PingPacket) -> i32 {
        self.connection
            .record_pong(packet.sent_at, self.now().monotonic_ms())
    }

    fn mark_half_accepted(&mut self) {
        self.connection.mark_half_accepted();
    }

    fn mark_accepted(&mut self) {
        self.connection.mark_accepted(self.now().wall_seconds());
    }

    fn clock_at(&self, now: Instant) -> LivenessClock {
        let elapsed = now.saturating_duration_since(self.monotonic_origin);
        let monotonic_ms = self
            .monotonic_origin_ms
            .wrapping_add(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX));
        let wall_seconds = match self.wall_clock {
            WallClockSource::System => system_wall_seconds(),
            #[cfg(test)]
            WallClockSource::Monotonic { origin_seconds } => {
                origin_seconds.saturating_add(i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
            }
        };
        LivenessClock::new(monotonic_ms, wall_seconds)
    }
}

fn system_wall_seconds() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        Err(error) => -i64::try_from(error.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

fn system_monotonic_seed_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn display_legacy_connection_message(message: &LegacyCString) -> String {
    clonk_resources::decode_legacy_script_text(message.as_bytes())
}

/// Failures which terminate a C++ connection admission exchange.
#[derive(Debug, Error)]
pub enum ConnectionHandshakeError {
    #[error("connection transport failed: {0}")]
    Transport(#[from] TransportError),
    #[error(
        "the local admission policy rejected the peer: {}",
        display_legacy_connection_message(.message)
    )]
    LocalRejection {
        message: LegacyCString,
        wrong_password: bool,
    },
    #[error(
        "the peer rejected the local connection: {}",
        display_legacy_connection_message(.message)
    )]
    PeerRejection {
        message: LegacyCString,
        wrong_password: bool,
    },
    #[error("{packet} is not accepted before mutual connection admission")]
    UnexpectedPreAdmissionPacket { packet: &'static str },
    #[error("host admission coordinator stopped before receiving the peer request")]
    HostAdmissionChannelClosed,
    #[error("host admission coordinator dropped the peer decision")]
    HostAdmissionDecisionDropped,
    #[error("host admission decision retained {count} unapplied before-reply action(s)")]
    UnappliedBeforeReplyActions { count: usize },
    #[error("connection admission timed out")]
    AdmissionTimeout,
    #[error("accepted connection ping timed out")]
    PingTimeout,
    #[error("connection admission reducer invariant failed: {0}")]
    ReducerInvariant(&'static str),
}

/// Runs the host half of the binary `PID_Conn`/`PID_ConnRe` exchange.
///
/// Peer admission is serialized through `admission_tx`, allowing the owning
/// session loop to assign the client ID and apply direct `ClientJoin` state
/// before the positive reply is placed on the wire.
pub async fn run_host_connection_handshake<S>(
    transport: &mut ControlTransport<S>,
    local_request: ConnectionRequest,
    admission_tx: &mpsc::Sender<HostAdmissionRequest>,
) -> Result<HostConnectionHandshake, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    run_host_connection_handshake_with_liveness(
        transport,
        local_request,
        admission_tx,
        ConnectionLivenessState::new_system(),
    )
    .await
}

async fn run_host_connection_handshake_with_liveness<S>(
    transport: &mut ControlTransport<S>,
    local_request: ConnectionRequest,
    admission_tx: &mpsc::Sender<HostAdmissionRequest>,
    mut liveness: ConnectionLivenessState,
) -> Result<HostConnectionHandshake, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let local_connection_id = local_request.connection_id;
    let mut connection = LegacyConnection::new(local_request);
    send_initial_request(transport, &mut connection).await?;
    let mut buffered_messages = VecDeque::new();

    loop {
        let message = match buffered_messages.pop_front() {
            Some(message) => message,
            None => read_handshake_message(transport, &mut liveness).await?,
        };
        match message {
            ControlMessage::Ping(packet) => {
                transport.send_message(ControlMessage::Pong(packet)).await?;
            }
            ControlMessage::Pong(packet) => {
                record_admitted_pong(&connection, &mut liveness, packet);
            }
            ControlMessage::ConnectionRequest(request) => {
                let (decision, received_while_deciding) = request_host_admission_decision(
                    transport,
                    admission_tx,
                    request.clone(),
                    local_connection_id,
                    &mut liveness,
                )
                .await?;
                buffered_messages.extend(received_while_deciding);
                let unapplied_action_count = match &decision {
                    AdmissionDecision::Accept { before_reply, .. } => before_reply.len(),
                    AdmissionDecision::Reject { .. } => 0,
                };
                if unapplied_action_count != 0 {
                    return Err(ConnectionHandshakeError::UnappliedBeforeReplyActions {
                        count: unapplied_action_count,
                    });
                }
                handle_host_peer_request(transport, &mut connection, request, decision).await?;
                if connection.status() == ConnectionStatus::HalfAccepted {
                    liveness.mark_half_accepted();
                }
            }
            ControlMessage::ConnectionReply(reply) => {
                if let Some(peer_core) = handle_peer_reply(&mut connection, reply)? {
                    liveness.mark_accepted();
                    let remote_connection_id = connection.remote_connection_id().ok_or(
                        ConnectionHandshakeError::ReducerInvariant(
                            "accepted connection has no peer connection ID",
                        ),
                    )?;
                    return Ok(HostConnectionHandshake {
                        local_connection_id,
                        remote_connection_id,
                        peer_core,
                        liveness,
                    });
                }
            }
            // The pre-half-accept filter consumed every disallowed PID before
            // decoding. Once half accepted, C++ logs packets whose handlers
            // are not enabled yet and keeps the connection open.
            _ => continue,
        }
    }
}

/// Runs the client half of the binary `PID_Conn`/`PID_ConnRe` exchange and
/// consumes the initial host `PID_JoinData` packet.
///
/// The transport remains borrowed so the caller can move it directly into the
/// normal accepted-session loop on success. The returned liveness state keeps
/// the C++ timer phase and counters continuous across that handoff.
pub async fn run_client_connection_handshake<S>(
    transport: &mut ControlTransport<S>,
    local_request: ConnectionRequest,
) -> Result<ClientConnectionHandshake, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    run_client_connection_handshake_with_liveness(
        transport,
        local_request,
        ConnectionLivenessState::new_system(),
    )
    .await
}

/// Runs the client half of admission for a second transport route belonging
/// to an already joined client. The peer must still be the host established by
/// the primary route, and no second JoinData packet is consumed or expected.
pub(crate) async fn run_client_route_handshake<S>(
    transport: &mut ControlTransport<S>,
    local_request: ConnectionRequest,
    expected_peer_core: &ClientCoreControlData,
) -> Result<ClientRouteHandshake, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    run_known_peer_connection_handshake(transport, local_request, expected_peer_core).await
}

/// Runs mutual admission for a transport whose logical peer is already in the
/// synchronized client registry.
///
/// Both inbound and outbound mesh sockets use the same C++ `PID_Conn` /
/// `PID_ConnRe` exchange. Supplying the registry-owned core up front permits a
/// positive reply to arrive before the peer request is dispatched, while the
/// request itself is still checked with [`KnownPeerAdmission`] before this
/// function returns. No `PID_JoinData` packet is consumed or expected.
pub(crate) async fn run_known_peer_connection_handshake<S>(
    transport: &mut ControlTransport<S>,
    local_request: ConnectionRequest,
    canonical_peer_core: &ClientCoreControlData,
) -> Result<ClientRouteHandshake, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let local_connection_id = local_request.connection_id;
    let compatibility_build = local_request.build;
    let mut connection =
        LegacyConnection::with_known_peer(local_request, canonical_peer_core.clone());
    let mut liveness = ConnectionLivenessState::new_system();
    send_initial_request(transport, &mut connection).await?;

    let mut authenticated_peer_request = false;
    let mut associated_peer = None;
    loop {
        let message = read_handshake_message(transport, &mut liveness).await?;
        match message {
            ControlMessage::Ping(packet) => {
                transport.send_message(ControlMessage::Pong(packet)).await?;
            }
            ControlMessage::Pong(packet) => {
                record_admitted_pong(&connection, &mut liveness, packet);
            }
            ControlMessage::ConnectionRequest(request) => {
                handle_known_peer_request(
                    transport,
                    &mut connection,
                    canonical_peer_core,
                    compatibility_build,
                    request,
                )
                .await?;
                authenticated_peer_request = true;
                if connection.status() == ConnectionStatus::HalfAccepted {
                    liveness.mark_half_accepted();
                }
            }
            ControlMessage::ConnectionReply(reply) => {
                if let Some(peer_core) = handle_peer_reply(&mut connection, reply)? {
                    liveness.mark_accepted();
                    associated_peer = Some(peer_core);
                }
            }
            _ => continue,
        }

        let Some(peer_core) = associated_peer.as_ref() else {
            continue;
        };
        if !authenticated_peer_request {
            continue;
        }
        if peer_core != canonical_peer_core {
            return Err(ConnectionHandshakeError::ReducerInvariant(
                "known-peer route associated a different canonical core",
            ));
        }
        let remote_connection_id =
            connection
                .remote_connection_id()
                .ok_or(ConnectionHandshakeError::ReducerInvariant(
                    "accepted known-peer route has no peer connection ID",
                ))?;
        return Ok(ClientRouteHandshake {
            local_connection_id,
            remote_connection_id,
            peer_core: peer_core.clone(),
            liveness,
        });
    }
}

/// Runs mutual admission for an inbound mesh transport whose logical peer is
/// not known until its `PID_Conn` is decoded.
///
/// The registry lookup is deliberately keyed only by the requested client ID;
/// [`KnownPeerAdmission`] then performs the C++ build and canonical
/// ID/name/nick checks. Unknown set IDs receive C++'s default negative
/// `connection denied` reply. As on every C++ connection, admission completes
/// only after both the peer request and its positive reply to our request have
/// been processed.
pub(crate) async fn run_registered_peer_connection_handshake<S>(
    transport: &mut ControlTransport<S>,
    local_request: ConnectionRequest,
    canonical_peer_cores: &BTreeMap<i32, ClientCoreControlData>,
) -> Result<ClientRouteHandshake, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let local_connection_id = local_request.connection_id;
    let compatibility_build = local_request.build;
    let mut connection = LegacyConnection::new(local_request);
    let mut liveness = ConnectionLivenessState::new_system();
    send_initial_request(transport, &mut connection).await?;

    let mut authenticated_peer = None;
    let mut associated_peer = None;
    loop {
        let message = read_handshake_message(transport, &mut liveness).await?;
        match message {
            ControlMessage::Ping(packet) => {
                transport.send_message(ControlMessage::Pong(packet)).await?;
            }
            ControlMessage::Pong(packet) => {
                record_admitted_pong(&connection, &mut liveness, packet);
            }
            ControlMessage::ConnectionRequest(request) => {
                authenticated_peer = Some(
                    handle_registered_peer_request(
                        transport,
                        &mut connection,
                        canonical_peer_cores,
                        compatibility_build,
                        request,
                    )
                    .await?,
                );
                if connection.status() == ConnectionStatus::HalfAccepted {
                    liveness.mark_half_accepted();
                }
            }
            ControlMessage::ConnectionReply(reply) => {
                if let Some(peer_core) = handle_peer_reply(&mut connection, reply)? {
                    liveness.mark_accepted();
                    associated_peer = Some(peer_core);
                }
            }
            _ => continue,
        }

        let (Some(authenticated_peer), Some(associated_peer)) =
            (authenticated_peer.as_ref(), associated_peer.as_ref())
        else {
            continue;
        };
        if associated_peer != authenticated_peer {
            return Err(ConnectionHandshakeError::ReducerInvariant(
                "registered-peer route associated a different canonical core",
            ));
        }
        let remote_connection_id =
            connection
                .remote_connection_id()
                .ok_or(ConnectionHandshakeError::ReducerInvariant(
                    "accepted registered-peer route has no peer connection ID",
                ))?;
        return Ok(ClientRouteHandshake {
            local_connection_id,
            remote_connection_id,
            peer_core: associated_peer.clone(),
            liveness,
        });
    }
}

pub(crate) async fn run_client_connection_handshake_with_liveness<S>(
    transport: &mut ControlTransport<S>,
    local_request: ConnectionRequest,
    mut liveness: ConnectionLivenessState,
) -> Result<ClientConnectionHandshake, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let local_connection_id = local_request.connection_id;
    let compatibility_build = local_request.build;
    let mut connection = LegacyConnection::new(local_request);
    send_initial_request(transport, &mut connection).await?;

    let mut registered_host = None;
    let peer_core = loop {
        let message = read_handshake_message(transport, &mut liveness).await?;
        match message {
            ControlMessage::Ping(packet) => {
                transport.send_message(ControlMessage::Pong(packet)).await?;
            }
            ControlMessage::Pong(packet) => {
                record_admitted_pong(&connection, &mut liveness, packet);
            }
            ControlMessage::ConnectionRequest(request) => {
                handle_peer_request(
                    transport,
                    &mut connection,
                    &mut registered_host,
                    compatibility_build,
                    request,
                )
                .await?;
                if connection.status() == ConnectionStatus::HalfAccepted {
                    liveness.mark_half_accepted();
                }
            }
            ControlMessage::ConnectionReply(reply) => {
                if let Some(peer_core) = handle_peer_reply(&mut connection, reply)? {
                    liveness.mark_accepted();
                    break peer_core;
                }
            }
            // The pre-half-accept filter consumed every disallowed PID before
            // decoding. Once half accepted, C++ logs packets whose handlers
            // are not enabled yet and keeps the connection open.
            _ => continue,
        }
    };

    if registered_host.as_ref() != Some(&peer_core) {
        return Err(ConnectionHandshakeError::ReducerInvariant(
            "accepted peer was not the provisionally registered host",
        ));
    }

    let pending_resources = Vec::new();
    let mut pending_controls = Vec::new();
    let mut pending_addresses = Vec::new();
    let mut pending_ready_checks = Vec::new();
    let mut pending_lobby_countdowns = Vec::new();
    let mut forwarded_messages = VecDeque::new();
    loop {
        let message = match forwarded_messages.pop_front() {
            Some(message) => message,
            None => read_handshake_message(transport, &mut liveness).await?,
        };
        match message {
            // Announced after the handshake completes; nothing to do with one
            // that arrives early beyond not treating it as a protocol error.
            ControlMessage::PortCapabilities(_) => continue,
            // A host restarting mid-handshake is about to close this
            // connection anyway. The join fails on its own; the notice is only
            // actionable once there is a round to leave.
            ControlMessage::HostRestarting { .. } => continue,
            // Likewise for the session-preserving notice: this connection has
            // no round yet, so there is nothing to return to a lobby. The join
            // in progress carries on into whichever round the host is building.
            ControlMessage::HostRestartLobby => continue,
            ControlMessage::ControlWaitAttribution(_) => continue,
            ControlMessage::JoinData(join_data) => {
                let remote_connection_id = connection.remote_connection_id().ok_or(
                    ConnectionHandshakeError::ReducerInvariant(
                        "accepted connection has no peer connection ID",
                    ),
                )?;
                return Ok(ClientConnectionHandshake {
                    local_connection_id,
                    remote_connection_id,
                    peer_core,
                    join_data: *join_data,
                    pending_resources,
                    pending_controls,
                    pending_addresses,
                    pending_ready_checks,
                    pending_lobby_countdowns,
                    liveness,
                });
            }
            ControlMessage::Resource(_) => {}
            ControlMessage::Control(packet) => pending_controls.push(packet),
            ControlMessage::Address(packet) if packet.client_id == peer_core.client_id => {
                pending_addresses.push(packet);
            }
            ControlMessage::ReadyCheck(packet) => pending_ready_checks.push(packet),
            ControlMessage::LobbyCountdown(packet) => pending_lobby_countdowns.push(packet),
            ControlMessage::Address(_)
            | ControlMessage::TcpSimOpen(_)
            | ControlMessage::Status(_)
            | ControlMessage::StatusAck(_)
            | ControlMessage::LeagueRoundResults(_)
            | ControlMessage::ActivationRequest { .. }
            | ControlMessage::PlayerInfoUpdate(_)
            | ControlMessage::Request { .. }
            | ControlMessage::Packet { .. }
            | ControlMessage::ExecSync { .. } => {}
            ControlMessage::Ping(packet) => {
                transport.send_message(ControlMessage::Pong(packet)).await?;
            }
            ControlMessage::Pong(packet) => {
                liveness.record_pong(packet);
            }
            ControlMessage::ConnectionRequest(request) => {
                handle_peer_request(
                    transport,
                    &mut connection,
                    &mut registered_host,
                    compatibility_build,
                    request,
                )
                .await?;
            }
            ControlMessage::ConnectionReply(reply) => {
                let duplicate_peer = handle_peer_reply(&mut connection, reply)?;
                if duplicate_peer
                    .as_ref()
                    .is_some_and(|core| core != &peer_core)
                {
                    return Err(ConnectionHandshakeError::ReducerInvariant(
                        "duplicate positive reply associated a different peer",
                    ));
                }
            }
            ControlMessage::ForwardRequest(packet) | ControlMessage::Forward(packet) => {
                if let Some(message) = pre_join_forwarded_message(&packet)? {
                    forwarded_messages.push_front(message);
                }
            }
            ControlMessage::PostMortem(_) => {
                return Err(ConnectionHandshakeError::UnexpectedPreAdmissionPacket {
                    packet: "PID_PostMortem",
                });
            }
        }
    }
}

fn pre_join_forwarded_message(
    packet: &crate::ForwardPacket,
) -> Result<Option<ControlMessage>, ConnectionHandshakeError> {
    // InitClient leaves NetIO.LCCore at C4ClientIDUnknown until JoinData. Both
    // forwarding handlers test that temporary ID before recursively unpacking
    // the nested packet (src/C4Network2.cpp:1231-1257,1574-1612;
    // src/C4Network2IO.cpp:1019-1033,1066-1117,1626-1636).
    let local_client_id = -1;
    let listed = packet.clients.contains(&local_client_id);
    let selected = if listed {
        !packet.negative_list
    } else {
        packet.negative_list
    };
    selected
        .then(|| crate::transport::parse_complete_packet(&packet.nested_packet))
        .transpose()
        .map(Option::flatten)
        .map_err(ConnectionHandshakeError::from)
}

async fn read_handshake_message<S>(
    transport: &mut ControlTransport<S>,
    liveness: &mut ConnectionLivenessState,
) -> Result<ControlMessage, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let timer_deadline = liveness.next_timer_at();
        tokio::select! {
            packet = transport.read_packet() => {
                if let Some(message) = accept_handshake_packet(
                    packet?,
                    liveness,
                )? {
                    return Ok(message);
                }
            }
            _ = tokio::time::sleep_until(timer_deadline) => {
                drive_liveness_timer(transport, liveness).await?;
            }
        }
    }
}

fn accept_handshake_packet(
    packet: crate::transport::InboundPacket,
    liveness: &mut ConnectionLivenessState,
) -> Result<Option<ControlMessage>, ConnectionHandshakeError> {
    let pre_half_accept = liveness.connection().phase() == crate::LivenessPhase::Connected;
    match packet {
        crate::transport::InboundPacket::Message(message) => {
            let packet_type = packet_type(&message);
            liveness.record_inbound_packet(packet_type);
            if pre_half_accept && !matches!(packet_type, 0x00 | 0x02 | 0x03) {
                return Ok(None);
            }
            Ok(Some(message))
        }
        crate::transport::InboundPacket::Ignored(packet_type) => {
            liveness.record_inbound_packet(packet_type);
            Ok(None)
        }
        crate::transport::InboundPacket::Empty => Ok(None),
        crate::transport::InboundPacket::Invalid { packet_type, error } => {
            liveness.record_inbound_packet(packet_type);
            if pre_half_accept && !matches!(packet_type, 0x00 | 0x02 | 0x03) {
                Ok(None)
            } else {
                Err(error.into())
            }
        }
    }
}

async fn drive_liveness_timer<S>(
    transport: &mut ControlTransport<S>,
    liveness: &mut ConnectionLivenessState,
) -> Result<(), ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ping = liveness.timer_tick().map_err(|timeout| match timeout {
        ConnectionTimeout::Acceptance => ConnectionHandshakeError::AdmissionTimeout,
        ConnectionTimeout::Ping => ConnectionHandshakeError::PingTimeout,
    })?;
    if let Some(ping) = ping {
        let send_result = transport.send_message(ControlMessage::Ping(ping)).await;
        // C4Network2IO::Ping invokes OnPing even when Send reports failure
        // (src/C4Network2IO.cpp:1139-1151).
        liveness.record_ping_dispatched();
        send_result?;
    }
    Ok(())
}

fn record_admitted_pong(
    connection: &LegacyConnection,
    liveness: &mut ConnectionLivenessState,
    packet: PingPacket,
) {
    if matches!(
        connection.status(),
        ConnectionStatus::HalfAccepted | ConnectionStatus::Accepted
    ) {
        liveness.record_pong(packet);
    }
}

fn packet_type(message: &ControlMessage) -> u8 {
    match message {
        ControlMessage::PortCapabilities(_) => crate::PID_PORT_CAPABILITIES,
        ControlMessage::HostRestarting { .. } => crate::PID_PORT_HOST_RESTARTING,
        ControlMessage::HostRestartLobby => crate::PID_PORT_HOST_RESTART_LOBBY,
        ControlMessage::ControlWaitAttribution(_) => crate::PID_PORT_CONTROL_WAIT_ATTRIBUTION,
        ControlMessage::Ping(_) => 0x00,
        ControlMessage::Pong(_) => 0x01,
        ControlMessage::ConnectionRequest(_) => 0x02,
        ControlMessage::ConnectionReply(_) => 0x03,
        ControlMessage::ForwardRequest(_) => 0x04,
        ControlMessage::Forward(_) => 0x05,
        ControlMessage::PostMortem(_) => 0x06,
        ControlMessage::Status(_) => 0x10,
        ControlMessage::StatusAck(_) => 0x11,
        ControlMessage::Address(_) => 0x12,
        ControlMessage::ActivationRequest { .. } => 0x13,
        ControlMessage::TcpSimOpen(_) => 0x14,
        ControlMessage::JoinData(_) => 0x15,
        ControlMessage::PlayerInfoUpdate(_) => 0x16,
        ControlMessage::LeagueRoundResults(_) => 0x17,
        ControlMessage::LobbyCountdown(_) => 0x20,
        ControlMessage::ReadyCheck(_) => 0x21,
        ControlMessage::Resource(packet) => match packet {
            ResourcePacket::Discover(_) => 0x30,
            ResourcePacket::Status(_) => 0x31,
            ResourcePacket::Derive(_) => 0x32,
            ResourcePacket::Request(_) => 0x33,
            ResourcePacket::Data(_) => 0x34,
        },
        ControlMessage::Control(_) => 0x40,
        ControlMessage::Request { .. } => 0x41,
        ControlMessage::Packet { .. } => 0x42,
        ControlMessage::ExecSync { .. } => 0x43,
    }
}

async fn request_host_admission_decision<S>(
    transport: &mut ControlTransport<S>,
    admission_tx: &mpsc::Sender<HostAdmissionRequest>,
    request: ConnectionRequest,
    connection_id: u32,
    liveness: &mut ConnectionLivenessState,
) -> Result<(AdmissionDecision, Vec<ControlMessage>), ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (decision_tx, mut decision_rx) = oneshot::channel();
    let mut buffered_messages = Vec::new();
    admission_tx
        .send(HostAdmissionRequest {
            connection_id,
            request,
            decision_tx,
        })
        .await
        .map_err(|_| ConnectionHandshakeError::HostAdmissionChannelClosed)?;

    loop {
        let timer_deadline = liveness.next_timer_at();
        tokio::select! {
            packet = transport.read_packet() => {
                let Some(message) = accept_handshake_packet(
                    packet?,
                    liveness,
                )? else {
                    continue;
                };
                match message {
                    ControlMessage::Ping(packet) => {
                        transport.send_message(ControlMessage::Pong(packet)).await?;
                    }
                    // A connection that is not half accepted silently drops
                    // Pong in C++'s pre-unpack gate.
                    ControlMessage::Pong(_) => {}
                    message @ (ControlMessage::ConnectionRequest(_)
                    | ControlMessage::ConnectionReply(_)) => {
                        buffered_messages.push(message);
                    }
                    other => {
                        return Err(ConnectionHandshakeError::UnexpectedPreAdmissionPacket {
                            packet: packet_name(&other),
                        });
                    }
                }
            }
            _ = tokio::time::sleep_until(timer_deadline) => {
                drive_liveness_timer(transport, liveness).await?;
            }
            decision = &mut decision_rx => {
                let decision = decision
                    .map_err(|_| ConnectionHandshakeError::HostAdmissionDecisionDropped)?;
                return Ok((decision, buffered_messages));
            }
        }
    }
}

async fn handle_host_peer_request<S>(
    transport: &mut ControlTransport<S>,
    connection: &mut LegacyConnection,
    request: ConnectionRequest,
    decision: AdmissionDecision,
) -> Result<(), ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let actions = connection.accept_peer_request(request, |_| decision);
    for action in actions {
        match action {
            ConnectionAction::SendReply(reply) => {
                let rejected = (!reply.ok).then(|| ConnectionHandshakeError::LocalRejection {
                    message: reply.message.clone(),
                    wrong_password: reply.wrong_password,
                });
                if let Err(error) = transport
                    .send_message(ControlMessage::ConnectionReply(reply))
                    .await
                {
                    let _ = connection.on_reply_sent(false);
                    return Err(error.into());
                }
                let follow_up = connection.on_reply_sent(true);
                if let Some(rejection) = rejected {
                    return Err(rejection);
                }
                if !follow_up.is_empty() {
                    return Err(ConnectionHandshakeError::ReducerInvariant(
                        "positive host connection-reply send emitted follow-up actions",
                    ));
                }
            }
            ConnectionAction::Close {
                message,
                wrong_password,
            } => {
                return Err(ConnectionHandshakeError::LocalRejection {
                    message,
                    wrong_password,
                });
            }
            ConnectionAction::SendRequest(_)
            | ConnectionAction::EmitDirectClientJoin(_)
            | ConnectionAction::RegisterHost(_)
            | ConnectionAction::AssociatePeer(_) => {
                return Err(ConnectionHandshakeError::ReducerInvariant(
                    "host admission emitted an unapplied or out-of-phase action",
                ));
            }
        }
    }
    Ok(())
}

async fn send_initial_request<S>(
    transport: &mut ControlTransport<S>,
    connection: &mut LegacyConnection,
) -> Result<(), ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let actions = connection.on_socket_open();
    let request = match actions.as_slice() {
        [ConnectionAction::SendRequest(request)] => request.clone(),
        _ => {
            return Err(ConnectionHandshakeError::ReducerInvariant(
                "socket-open did not emit exactly one connection request",
            ));
        }
    };

    if let Err(error) = transport
        .send_message(ControlMessage::ConnectionRequest(request))
        .await
    {
        let _ = connection.on_request_sent(false);
        return Err(error.into());
    }
    if !connection.on_request_sent(true).is_empty() {
        return Err(ConnectionHandshakeError::ReducerInvariant(
            "successful connection-request send emitted follow-up actions",
        ));
    }
    Ok(())
}

async fn handle_peer_request<S>(
    transport: &mut ControlTransport<S>,
    connection: &mut LegacyConnection,
    registered_host: &mut Option<ClientCoreControlData>,
    compatibility_build: i32,
    request: ConnectionRequest,
) -> Result<(), ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let actions = connection.accept_peer_request(request, |request| {
        ClientAdmission::admit_host_for_build(request, compatibility_build)
    });
    for action in actions {
        match action {
            ConnectionAction::RegisterHost(core) => {
                if registered_host.replace(core).is_some() {
                    return Err(ConnectionHandshakeError::ReducerInvariant(
                        "host was registered more than once",
                    ));
                }
            }
            ConnectionAction::SendReply(reply) => {
                let rejected = (!reply.ok).then(|| ConnectionHandshakeError::LocalRejection {
                    message: reply.message.clone(),
                    wrong_password: reply.wrong_password,
                });
                if let Err(error) = transport
                    .send_message(ControlMessage::ConnectionReply(reply))
                    .await
                {
                    let _ = connection.on_reply_sent(false);
                    return Err(error.into());
                }
                let follow_up = connection.on_reply_sent(true);
                if let Some(rejection) = rejected {
                    return Err(rejection);
                }
                if !follow_up.is_empty() {
                    return Err(ConnectionHandshakeError::ReducerInvariant(
                        "positive connection-reply send emitted follow-up actions",
                    ));
                }
            }
            ConnectionAction::Close {
                message,
                wrong_password,
            } => {
                return Err(ConnectionHandshakeError::LocalRejection {
                    message,
                    wrong_password,
                });
            }
            ConnectionAction::SendRequest(_)
            | ConnectionAction::EmitDirectClientJoin(_)
            | ConnectionAction::AssociatePeer(_) => {
                return Err(ConnectionHandshakeError::ReducerInvariant(
                    "client admission emitted a host-only or out-of-phase action",
                ));
            }
        }
    }
    Ok(())
}

async fn handle_known_peer_request<S>(
    transport: &mut ControlTransport<S>,
    connection: &mut LegacyConnection,
    canonical_peer_core: &ClientCoreControlData,
    compatibility_build: i32,
    request: ConnectionRequest,
) -> Result<(), ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let decision = KnownPeerAdmission::admit_for_build(
        &request,
        canonical_peer_core,
        false,
        compatibility_build,
    );
    handle_known_peer_decision(transport, connection, request, decision).await
}

async fn handle_registered_peer_request<S>(
    transport: &mut ControlTransport<S>,
    connection: &mut LegacyConnection,
    canonical_peer_cores: &BTreeMap<i32, ClientCoreControlData>,
    compatibility_build: i32,
    request: ConnectionRequest,
) -> Result<ClientCoreControlData, ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let canonical_peer_core = canonical_peer_cores.get(&request.core.client_id).cloned();
    let decision = match canonical_peer_core.as_ref() {
        Some(canonical_peer_core) => KnownPeerAdmission::admit_for_build(
            &request,
            canonical_peer_core,
            false,
            compatibility_build,
        ),
        None => AdmissionDecision::Reject {
            message: LegacyCString::from_bytes(b"connection denied".to_vec()).unwrap_or_default(),
            wrong_password: false,
        },
    };
    handle_known_peer_decision(transport, connection, request, decision).await?;
    canonical_peer_core.ok_or(ConnectionHandshakeError::ReducerInvariant(
        "unknown registered-peer request survived negative admission",
    ))
}

async fn handle_known_peer_decision<S>(
    transport: &mut ControlTransport<S>,
    connection: &mut LegacyConnection,
    request: ConnectionRequest,
    decision: AdmissionDecision,
) -> Result<(), ConnectionHandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let actions = connection.accept_peer_request(request, |_| decision);
    for action in actions {
        match action {
            ConnectionAction::SendReply(reply) => {
                let rejected = (!reply.ok).then(|| ConnectionHandshakeError::LocalRejection {
                    message: reply.message.clone(),
                    wrong_password: reply.wrong_password,
                });
                if let Err(error) = transport
                    .send_message(ControlMessage::ConnectionReply(reply))
                    .await
                {
                    let _ = connection.on_reply_sent(false);
                    return Err(error.into());
                }
                let follow_up = connection.on_reply_sent(true);
                if let Some(rejection) = rejected {
                    return Err(rejection);
                }
                if !follow_up.is_empty() {
                    return Err(ConnectionHandshakeError::ReducerInvariant(
                        "positive known-peer connection-reply send emitted follow-up actions",
                    ));
                }
            }
            ConnectionAction::Close {
                message,
                wrong_password,
            } => {
                return Err(ConnectionHandshakeError::LocalRejection {
                    message,
                    wrong_password,
                });
            }
            ConnectionAction::SendRequest(_)
            | ConnectionAction::EmitDirectClientJoin(_)
            | ConnectionAction::RegisterHost(_)
            | ConnectionAction::AssociatePeer(_) => {
                return Err(ConnectionHandshakeError::ReducerInvariant(
                    "known-peer admission emitted an out-of-phase action",
                ));
            }
        }
    }
    Ok(())
}

fn handle_peer_reply(
    connection: &mut LegacyConnection,
    reply: crate::ConnectionReply,
) -> Result<Option<ClientCoreControlData>, ConnectionHandshakeError> {
    let rejected = (!reply.ok).then(|| ConnectionHandshakeError::PeerRejection {
        message: reply.message.clone(),
        wrong_password: reply.wrong_password,
    });
    let actions = connection.receive_reply(reply);
    if let Some(rejection) = rejected {
        return Err(rejection);
    }

    let mut associated_peer = None;
    for action in actions {
        match action {
            ConnectionAction::AssociatePeer(core) if associated_peer.is_none() => {
                associated_peer = Some(core);
            }
            ConnectionAction::Close { .. } => {
                return Err(ConnectionHandshakeError::ReducerInvariant(
                    "positive connection reply arrived before an accepted host request",
                ));
            }
            _ => {
                return Err(ConnectionHandshakeError::ReducerInvariant(
                    "connection reply emitted an out-of-phase action",
                ));
            }
        }
    }

    if associated_peer.is_some() && connection.status() != ConnectionStatus::Accepted {
        return Err(ConnectionHandshakeError::ReducerInvariant(
            "peer association did not mark the connection accepted",
        ));
    }
    Ok(associated_peer)
}

fn packet_name(message: &ControlMessage) -> &'static str {
    match message {
        ControlMessage::PortCapabilities(_) => "PID_PortCapabilities",
        ControlMessage::HostRestarting { .. } => "PID_PortHostRestarting",
        ControlMessage::HostRestartLobby => "PID_PortHostRestartLobby",
        ControlMessage::ControlWaitAttribution(_) => "PID_PortControlWaitAttribution",
        ControlMessage::Ping(_) => "PID_Ping",
        ControlMessage::Pong(_) => "PID_Pong",
        ControlMessage::ConnectionRequest(_) => "PID_Conn",
        ControlMessage::ConnectionReply(_) => "PID_ConnRe",
        ControlMessage::ForwardRequest(_) => "PID_FwdReq",
        ControlMessage::Forward(_) => "PID_Fwd",
        ControlMessage::PostMortem(_) => "PID_PostMortem",
        ControlMessage::JoinData(_) => "PID_JoinData",
        ControlMessage::LeagueRoundResults(_) => "PID_LeagueRoundResults",
        ControlMessage::Address(_) => "PID_Addr",
        ControlMessage::TcpSimOpen(_) => "PID_TCPSimOpen",
        ControlMessage::Resource(packet) => match packet {
            ResourcePacket::Discover(_) => "PID_NetResDis",
            ResourcePacket::Status(_) => "PID_NetResStat",
            ResourcePacket::Derive(_) => "PID_NetResDerive",
            ResourcePacket::Request(_) => "PID_NetResReq",
            ResourcePacket::Data(_) => "PID_NetResData",
        },
        ControlMessage::Status(_) => "PID_Status",
        ControlMessage::StatusAck(_) => "PID_StatusAck",
        ControlMessage::ActivationRequest { .. } => "PID_ClientActReq",
        ControlMessage::PlayerInfoUpdate(_) => "PID_PlayerInfoUpdReq",
        ControlMessage::LobbyCountdown(_) => "PID_LobbyCountdown",
        ControlMessage::ReadyCheck(_) => "PID_ReadyCheck",
        ControlMessage::Control(_) => "PID_Control",
        ControlMessage::Request { .. } => "PID_ControlReq",
        ControlMessage::Packet { .. } => "PID_ControlPkt",
        ControlMessage::ExecSync { .. } => "PID_ExecSyncCtrl",
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use clonk_engine::{ClientCoreControlData, LegacyCString, NetworkResourceCore};
    use tokio::io::{duplex, AsyncWriteExt};
    use tokio::sync::mpsc;
    use tokio::time::{advance, timeout};

    use super::*;
    use crate::{
        AddressPacket, AdmissionDecision, ConnectionReply, ControlDelivery, ControlPacket,
        ForwardPacket, JoinClientRegistrySnapshot, JoinGameParametersEnvelope,
        JoinTeamListSnapshot, LivenessPhase, NetworkAddress, NetworkProtocol, NetworkStatus,
        PingPacket, PlayerInfoListSnapshot, PlayerInfoUpdateRequest, ResourceDiscoverPacket,
        ResourcePacket, NETWORK_STATE_LOBBY,
    };

    const CPP_COMPATIBILITY_BUILD: i32 = crate::CURRENT_GAME_BUILD + 2;

    fn wire_string(value: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(value.to_vec()).unwrap()
    }

    fn request(client_id: i32, name: &[u8], connection_id: u32) -> ConnectionRequest {
        ConnectionRequest {
            core: ClientCoreControlData {
                client_id,
                name: wire_string(name),
                nick: wire_string(name),
                ..Default::default()
            },
            build: 362,
            password: LegacyCString::default(),
            connection_id,
        }
    }

    fn accepted(message: &[u8]) -> ConnectionReply {
        ConnectionReply {
            ok: true,
            message: wire_string(message),
            wrong_password: false,
        }
    }

    fn tcp_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0xff];
        frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mutual_handshake_retains_both_cpp_connection_ids() {
        // C4Network2IO allocates an independent local ID for every socket and
        // records the peer's PID_Conn ID as iRemoteID; post-mortem recovery
        // later sends iRemoteID so the peer can find its local dead connection
        // (src/C4Network2IO.cpp:236-249,499-508,954-960,1379-1395).
        let expected_join_data = join_data();
        let (host_stream, client_stream) = duplex(4096);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let host_task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(host_stream);
            let handshake = run_host_connection_handshake(
                &mut transport,
                request(0, b"Host", 7),
                &admission_tx,
            )
            .await
            .unwrap();
            transport
                .send_message(ControlMessage::JoinData(Box::new(expected_join_data)))
                .await
                .unwrap();
            handshake
        });
        let client_task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 11))
                .await
                .unwrap()
        });

        let admission = admission_rx.recv().await.unwrap();
        admission
            .decision_tx
            .send(AdmissionDecision::Accept {
                peer_core: admission.request.core,
                before_reply: Vec::new(),
                message: wire_string(b"join accepted"),
            })
            .unwrap();

        let host = host_task.await.unwrap();
        let client = client_task.await.unwrap();
        assert_eq!(host.local_connection_id, 7);
        assert_eq!(host.remote_connection_id, 11);
        assert_eq!(client.local_connection_id, 11);
        assert_eq!(client.remote_connection_id, 7);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_handshake_accepts_the_selected_noncurrent_build() {
        // The game reference publishes C4XVERBUILD, and C++ accepts PID_Conn
        // only at that exact value (oracle-src-pinned
        // src/C4Network2Reference.cpp:79,100-102;
        // src/C4Network2.cpp:1291-1299).
        let expected_join_data = join_data();
        let (host_stream, client_stream) = duplex(4096);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let host_task = tokio::spawn(async move {
            let mut host_request = request(0, b"Host", 7);
            host_request.build = CPP_COMPATIBILITY_BUILD;
            let mut transport = ControlTransport::new(host_stream);
            run_host_connection_handshake(&mut transport, host_request, &admission_tx)
                .await
                .unwrap();
            transport
                .send_message(ControlMessage::JoinData(Box::new(expected_join_data)))
                .await
                .unwrap();
        });
        let client_task = tokio::spawn(async move {
            let mut client_request = request(-1, b"Alice", 11);
            client_request.build = CPP_COMPATIBILITY_BUILD;
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, client_request).await
        });

        let admission = admission_rx.recv().await.unwrap();
        assert_eq!(admission.request.build, CPP_COMPATIBILITY_BUILD);
        admission
            .decision_tx
            .send(AdmissionDecision::Accept {
                peer_core: admission.request.core,
                before_reply: Vec::new(),
                message: wire_string(b"join accepted"),
            })
            .unwrap();

        host_task.await.unwrap();
        let client = client_task.await.unwrap().unwrap();
        assert_eq!(client.local_connection_id, 11);
        assert_eq!(client.remote_connection_id, 7);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_handshake_rejects_a_host_outside_the_selected_build() {
        // Reciprocal C++ PID_Conn also carries C4XVERBUILD
        // (oracle-src-pinned src/C4Network2IO.cpp:1611-1618).
        let (client_stream, host_stream) = duplex(2048);
        let client_task = tokio::spawn(async move {
            let mut local_request = request(-1, b"Alice", 11);
            local_request.build = CPP_COMPATIBILITY_BUILD;
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, local_request).await
        });
        let mut host = ControlTransport::new(host_stream);

        let ControlMessage::ConnectionRequest(client_request) = host.read_message().await.unwrap()
        else {
            panic!("expected client PID_Conn");
        };
        assert_eq!(client_request.build, CPP_COMPATIBILITY_BUILD);

        let mut host_request = request(0, b"Host", 7);
        host_request.build = CPP_COMPATIBILITY_BUILD - 1;
        let expected = format!(
            "wrong engine ({}, I have {CPP_COMPATIBILITY_BUILD})",
            CPP_COMPATIBILITY_BUILD - 1
        );
        host.send_message(ControlMessage::ConnectionRequest(host_request))
            .await
            .unwrap();
        assert!(matches!(
            host.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(ConnectionReply {
                ok: false,
                ref message,
                wrong_password: false,
            }) if message.as_bytes() == expected.as_bytes()
        ));

        assert!(matches!(
            client_task.await.unwrap().unwrap_err(),
            ConnectionHandshakeError::LocalRejection {
                wrong_password: false,
                ..
            }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn known_peer_mesh_handshake_accepts_status_only_core_differences_without_join_data() {
        // CheckConn compares the synchronized ID/name/nick identity and keeps
        // the registry-owned core. Runtime status flags in the Conn payload do
        // not prevent two already joined clients from accepting a direct route
        // (src/C4Network2.cpp:1282-1363,1448-1499).
        let alice_core = request(3, b"Alice", 0).core;
        let bob_core = request(4, b"Bob", 0).core;
        let mut alice_request = request(3, b"Alice", 7);
        alice_request.build = CPP_COMPATIBILITY_BUILD;
        alice_request.core.activated = true;
        let mut bob_request = request(4, b"Bob", 11);
        bob_request.build = CPP_COMPATIBILITY_BUILD;
        bob_request.core.observer = true;

        let (alice_stream, bob_stream) = duplex(4096);
        let expected_bob = bob_core.clone();
        let alice_task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(alice_stream);
            run_known_peer_connection_handshake(&mut transport, alice_request, &expected_bob).await
        });
        let expected_alice = alice_core.clone();
        let bob_task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(bob_stream);
            run_known_peer_connection_handshake(&mut transport, bob_request, &expected_alice).await
        });

        let (alice, bob) = timeout(Duration::from_secs(1), async {
            (
                alice_task.await.unwrap().unwrap(),
                bob_task.await.unwrap().unwrap(),
            )
        })
        .await
        .expect("known peers should finish at mutual Conn/ConnRe admission");
        assert_eq!(alice.local_connection_id, 7);
        assert_eq!(alice.remote_connection_id, 11);
        assert_eq!(alice.peer_core, bob_core);
        assert_eq!(alice.liveness.connection().phase(), LivenessPhase::Accepted);
        assert_eq!(bob.local_connection_id, 11);
        assert_eq!(bob.remote_connection_id, 7);
        assert_eq!(bob.peer_core, alice_core);
        assert_eq!(bob.liveness.connection().phase(), LivenessPhase::Accepted);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn known_peer_mesh_handshake_rejects_changed_identity_and_wrong_build() {
        async fn assert_rejected(
            incoming: ConnectionRequest,
            expected_message: &[u8],
        ) -> ConnectionHandshakeError {
            let canonical_peer = request(4, b"Bob", 0).core;
            let (local_stream, peer_stream) = duplex(2048);
            let task = tokio::spawn(async move {
                let mut transport = ControlTransport::new(local_stream);
                let mut local_request = request(3, b"Alice", 7);
                local_request.build = CPP_COMPATIBILITY_BUILD;
                run_known_peer_connection_handshake(&mut transport, local_request, &canonical_peer)
                    .await
            });
            let mut peer = ControlTransport::new(peer_stream);
            assert!(matches!(
                peer.read_message().await.unwrap(),
                ControlMessage::ConnectionRequest(_)
            ));
            peer.send_message(ControlMessage::ConnectionRequest(incoming))
                .await
                .unwrap();
            assert!(matches!(
                peer.read_message().await.unwrap(),
                ControlMessage::ConnectionReply(ConnectionReply {
                    ok: false,
                    ref message,
                    wrong_password: false,
                }) if message.as_bytes() == expected_message
            ));
            task.await.unwrap().unwrap_err()
        }

        let mut changed_identity = request(4, b"Bob", 11);
        changed_identity.build = CPP_COMPATIBILITY_BUILD;
        changed_identity.core.nick = wire_string(b"Impostor");
        assert!(matches!(
            assert_rejected(changed_identity, b"wrong client core").await,
            ConnectionHandshakeError::LocalRejection {
                wrong_password: false,
                ..
            }
        ));

        let mut wrong_build = request(4, b"Bob", 12);
        wrong_build.build = CPP_COMPATIBILITY_BUILD - 1;
        let expected = format!(
            "wrong engine ({}, I have {CPP_COMPATIBILITY_BUILD})",
            CPP_COMPATIBILITY_BUILD - 1
        );
        assert!(matches!(
            assert_rejected(wrong_build, expected.as_bytes()).await,
            ConnectionHandshakeError::LocalRejection {
                wrong_password: false,
                ..
            }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registered_peer_mesh_handshake_selects_canonical_core_from_inbound_conn() {
        let alice_core = request(3, b"Alice", 0).core;
        let bob_core = request(4, b"Bob", 0).core;
        let mut inbound_registry = BTreeMap::new();
        inbound_registry.insert(bob_core.client_id, bob_core.clone());
        let mut bob_request = request(4, b"Bob", 11);
        bob_request.build = CPP_COMPATIBILITY_BUILD;
        bob_request.core.activated = true;

        let (inbound_stream, outbound_stream) = duplex(4096);
        let inbound_task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(inbound_stream);
            let mut alice_request = request(3, b"Alice", 7);
            alice_request.build = CPP_COMPATIBILITY_BUILD;
            run_registered_peer_connection_handshake(
                &mut transport,
                alice_request,
                &inbound_registry,
            )
            .await
        });
        let expected_alice = alice_core.clone();
        let outbound_task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(outbound_stream);
            run_known_peer_connection_handshake(&mut transport, bob_request, &expected_alice).await
        });

        let (inbound, outbound) = timeout(Duration::from_secs(1), async {
            (
                inbound_task.await.unwrap().unwrap(),
                outbound_task.await.unwrap().unwrap(),
            )
        })
        .await
        .expect("inbound registry admission should finish without JoinData");
        assert_eq!(inbound.local_connection_id, 7);
        assert_eq!(inbound.remote_connection_id, 11);
        assert_eq!(inbound.peer_core, bob_core);
        assert_eq!(outbound.local_connection_id, 11);
        assert_eq!(outbound.remote_connection_id, 7);
        assert_eq!(outbound.peer_core, alice_core);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn registered_peer_mesh_handshake_rejects_unknown_set_id_before_close() {
        let bob_core = request(4, b"Bob", 0).core;
        let registry = BTreeMap::from([(bob_core.client_id, bob_core)]);
        let (inbound_stream, peer_stream) = duplex(2048);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(inbound_stream);
            run_registered_peer_connection_handshake(
                &mut transport,
                request(3, b"Alice", 7),
                &registry,
            )
            .await
        });
        let mut peer = ControlTransport::new(peer_stream);
        assert!(matches!(
            peer.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        peer.send_message(ControlMessage::ConnectionRequest(request(
            99, b"Unknown", 11,
        )))
        .await
        .unwrap();
        assert!(matches!(
            peer.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(ConnectionReply {
                ok: false,
                ref message,
                wrong_password: false,
            }) if message.as_bytes() == b"connection denied"
        ));
        assert!(matches!(
            task.await.unwrap(),
            Err(ConnectionHandshakeError::LocalRejection {
                ref message,
                wrong_password: false,
            }) if message.as_bytes() == b"connection denied"
        ));
    }

    #[test]
    fn forwarding_packets_advance_the_cpp_recoverable_packet_counter() {
        // PID_FwdReq and PID_Fwd are 0x04/0x05, at and above
        // PID_PacketLogStart, so OnPacketReceived counts both
        // (src/C4PacketBase.h:95-102; src/C4Network2IO.cpp:1362-1366).
        let packet = ForwardPacket {
            negative_list: true,
            clients: Vec::new(),
            nested_packet: vec![0xff],
        };
        let mut liveness = ConnectionLivenessState::new_test(0, 0);

        liveness.record_inbound_message(&ControlMessage::ForwardRequest(packet.clone()));
        liveness.record_inbound_message(&ControlMessage::Forward(packet));

        assert_eq!(liveness.connection().inbound_packet_counter(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn ignored_cpp_packets_survive_timer_cancellation_and_advance_the_counter() {
        fn frame(payload: &[u8]) -> Vec<u8> {
            let mut frame = vec![0xff];
            frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
            frame.extend_from_slice(payload);
            frame
        }

        let tcp_sim_open = [
            0x14, 0x07, 0x01, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':',
            b'7', b']', b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ];
        let league_results = [0x17, 0x01, b'O', b'K', 0x00, 0x00];
        let ping = PingPacket {
            sent_at: 0x1122_3344,
            packet_counter: 7,
        };
        let mut ping_payload = vec![0x00];
        ping_payload.extend_from_slice(&ping.sent_at.to_ne_bytes());
        ping_payload.extend_from_slice(&ping.packet_counter.to_ne_bytes());

        let (client, mut peer) = duplex(256);
        let mut initial_frames = frame(&[]);
        initial_frames.extend(frame(&tcp_sim_open));
        peer.write_all(&initial_frames).await.unwrap();
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client);
            let mut liveness = ConnectionLivenessState::new_test(0, 0);
            let message = read_handshake_message(&mut transport, &mut liveness)
                .await
                .unwrap();
            (message, liveness)
        });

        // The first ignored frame is accounted before the timer branch
        // cancels and recreates the pending transport read.
        tokio::task::yield_now().await;
        advance(Duration::from_millis(500)).await;
        tokio::task::yield_now().await;

        let mut remaining_frames = frame(&league_results);
        remaining_frames.extend(frame(&ping_payload));
        peer.write_all(&remaining_frames).await.unwrap();
        let (message, liveness) = task.await.unwrap();

        assert_eq!(message, ControlMessage::Ping(ping));
        assert_eq!(liveness.connection().inbound_packet_counter(), 2);
    }

    fn join_data() -> JoinDataEnvelope {
        let empty_players = PlayerInfoListSnapshot {
            last_player_id: 0,
            clients: Vec::new(),
        };
        JoinDataEnvelope {
            client_id: 3,
            start_control_tick: 17,
            status: NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 1,
                target_tick: -1,
            },
            dynamic: NetworkResourceCore::default(),
            parameters: JoinGameParametersEnvelope {
                random_seed: 0,
                startup_player_count: 0,
                max_players: 8,
                use_fair_crew: false,
                fair_crew_forced: false,
                fair_crew_strength: 0,
                allow_debug: true,
                is_network_game: true,
                control_rate: 1,
                auto_frame_skip: false,
                rules: Vec::new(),
                goals: Vec::new(),
                league: LegacyCString::default(),
                league_address: LegacyCString::default(),
                title: wire_string(b"No title"),
                scenario: NetworkResourceCore::default(),
                game_resources: Vec::new(),
                player_infos: empty_players.clone(),
                restore_player_infos: empty_players,
                teams: JoinTeamListSnapshot {
                    active: 1,
                    custom: 0,
                    allow_hostility_change: 1,
                    allow_team_switch: 0,
                    auto_generate_teams: 1,
                    last_team_id: 0,
                    team_distribution: 0,
                    team_colors: 0,
                    max_script_players: 0,
                    script_player_names: LegacyCString::default(),
                    random_team_count: 0,
                    teams: Vec::new(),
                },
                clients: JoinClientRegistrySnapshot {
                    clients: Vec::new(),
                    local_client_id: None,
                },
            },
        }
    }

    #[tokio::test(start_paused = true)]
    async fn client_originates_ping_on_the_cpp_network_timer_cadence() {
        // C4Network2IO executes on a 500 ms timer and pings every open
        // connection only once iLastPing is strictly outside the preceding
        // 1000 ms window. From the initialization edge, the first probe is
        // therefore emitted 1500 ms later
        // (src/C4Network2IO.cpp:101,605-617,1139-1151;
        // src/C4Network2IO.h:34-38).
        let (client_stream, host_stream) = duplex(1024);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake_with_liveness(
                &mut transport,
                request(-1, b"Alice", 7),
                ConnectionLivenessState::new_test(10_000, 100),
            )
            .await
        });
        let mut host = ControlTransport::new(host_stream);
        assert!(matches!(
            host.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));

        for _ in 0..3 {
            advance(Duration::from_millis(500)).await;
            tokio::task::yield_now().await;
        }

        assert_eq!(
            timeout(Duration::from_millis(1), host.read_message())
                .await
                .expect("client must originate its first probe at 1500 ms")
                .unwrap(),
            ControlMessage::Ping(PingPacket {
                sent_at: 11_500,
                packet_counter: 0,
            })
        );
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn host_originates_ping_while_the_central_admission_decision_is_pending() {
        // C4Network2IO's network-thread timer keeps pinging while Conn waits in
        // C4Network2's main-thread admission path (src/C4Network2IO.cpp:
        // 605-617,807-812,1139-1151; src/C4Network2.cpp:923-936,1274-1363).
        let (host_stream, client_stream) = duplex(1024);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(host_stream);
            run_host_connection_handshake_with_liveness(
                &mut transport,
                request(0, b"Host", 7),
                &admission_tx,
                ConnectionLivenessState::new_test(20_000, 100),
            )
            .await
        });
        let mut client = ControlTransport::new(client_stream);
        let _ = client.read_message().await.unwrap();
        client
            .send_message(ControlMessage::ConnectionRequest(request(-1, b"Alice", 11)))
            .await
            .unwrap();
        let _pending_decision = admission_rx.recv().await.unwrap();

        for _ in 0..3 {
            advance(Duration::from_millis(500)).await;
            tokio::task::yield_now().await;
        }

        assert_eq!(
            timeout(Duration::from_millis(1), client.read_message())
                .await
                .expect("host must keep probing during serialized admission")
                .unwrap(),
            ControlMessage::Ping(PingPacket {
                sent_at: 21_500,
                packet_counter: 0,
            })
        );
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn admission_timeout_uses_the_cpp_strict_whole_second_boundary() {
        // CheckTimeout closes a non-accepted connection only when
        // difftime(now, status_timestamp) is strictly greater than ten
        // seconds (src/C4Network2IO.cpp:1155-1169).
        let (client_stream, _host_stream) = duplex(1024);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake_with_liveness(
                &mut transport,
                request(-1, b"Alice", 7),
                ConnectionLivenessState::new_test(30_000, 100),
            )
            .await
        });
        tokio::task::yield_now().await;

        advance(Duration::from_secs(10)).await;
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "ten whole seconds is still inside the C++ acceptance window"
        );
        advance(Duration::from_secs(1)).await;

        assert!(matches!(
            task.await.unwrap(),
            Err(ConnectionHandshakeError::AdmissionTimeout)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_client_probes_and_tracks_pong_and_packets_while_join_data_is_delayed() {
        // Once Conn/ConnRe make the socket accepted, Ping/Pong remain active
        // while SendJoinData waits for a dynamic. The Ping snapshots every
        // received packet at or above PID_PacketLogStart, including NetResDis
        // (src/C4Network2IO.cpp:595,807-812,1007-1028,1139-1151,1362-1366;
        // src/C4Network2.cpp:1768-1784,1820-1850).
        let expected_join_data = join_data();
        let (client_stream, host_stream) = duplex(2048);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake_with_liveness(
                &mut transport,
                request(-1, b"Alice", 7),
                ConnectionLivenessState::new_test(40_000, 100),
            )
            .await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        assert!(matches!(
            host.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(ConnectionReply { ok: true, .. })
        ));
        host.send_message(ControlMessage::ConnectionReply(accepted(b"accepted")))
            .await
            .unwrap();
        let discovery = ResourcePacket::Discover(ResourceDiscoverPacket {
            resource_ids: vec![9],
        });
        host.send_message(ControlMessage::Resource(discovery.clone()))
            .await
            .unwrap();
        let pending_control = ControlPacket::builder(0, 17).payload(vec![0xff]);
        host.send_message(ControlMessage::Control(pending_control.clone()))
            .await
            .unwrap();
        let pending_address = AddressPacket {
            client_id: 0,
            address: NetworkAddress::new(
                NetworkProtocol::Tcp,
                "203.0.113.7:11112".parse().unwrap(),
            ),
        };
        host.send_message(ControlMessage::Address(pending_address))
            .await
            .unwrap();

        for _ in 0..3 {
            advance(Duration::from_millis(500)).await;
            tokio::task::yield_now().await;
        }
        let ping = match host.read_message().await.unwrap() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected client liveness probe, got {other:?}"),
        };
        assert_eq!(
            ping,
            PingPacket {
                sent_at: 41_500,
                packet_counter: 3,
            }
        );

        advance(Duration::from_millis(25)).await;
        host.send_message(ControlMessage::Pong(ping)).await.unwrap();
        let host_ping = PingPacket {
            sent_at: 0x0102_0304,
            packet_counter: 7,
        };
        host.send_message(ControlMessage::Ping(host_ping))
            .await
            .unwrap();
        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Pong(host_ping)
        );
        host.send_message(ControlMessage::JoinData(Box::new(
            expected_join_data.clone(),
        )))
        .await
        .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result.join_data, expected_join_data);
        assert!(result.pending_resources.is_empty());
        assert_eq!(result.pending_controls, vec![pending_control]);
        assert_eq!(result.pending_addresses, vec![pending_address]);
        assert_eq!(
            result.liveness.connection().phase(),
            LivenessPhase::Accepted
        );
        assert_eq!(result.liveness.connection().measured_ping_ms(), Some(25));
        assert_eq!(result.liveness.connection().inbound_packet_counter(), 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pre_join_forwarding_uses_the_cpp_unknown_local_client_id() {
        // Until HandleJoinData installs the assigned core, C++ keeps LCCore at
        // C4ClientIDUnknown (-1). PID_Fwd/PID_FwdReq therefore ignore a list
        // selecting the future ID without unpacking its nested bytes, while an
        // empty negative list selects -1 and recursively dispatches PID_Control;
        // control packets are retained even before control Init
        // (src/C4Network2.cpp:1231-1257,1574-1612;
        // src/C4Network2IO.cpp:1019-1033,1066-1117,1626-1636;
        // src/C4GameControlNetwork.cpp:517-545).
        let expected_join_data = join_data();
        let assigned_client_id = expected_join_data.client_id;
        let (client_stream, host_stream) = duplex(2048);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"accepted")))
            .await
            .unwrap();

        host.send_message(ControlMessage::ForwardRequest(ForwardPacket {
            negative_list: false,
            clients: vec![assigned_client_id],
            nested_packet: vec![0xff],
        }))
        .await
        .unwrap();
        let pending_control = ControlPacket::builder(0, 17).payload(vec![0xff]);
        host.send_message(ControlMessage::Forward(ForwardPacket {
            negative_list: true,
            clients: Vec::new(),
            nested_packet: crate::transport::encode_complete_control_packet(&pending_control)
                .unwrap(),
        }))
        .await
        .unwrap();
        host.send_message(ControlMessage::JoinData(Box::new(
            expected_join_data.clone(),
        )))
        .await
        .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result.join_data, expected_join_data);
        assert_eq!(result.pending_controls, vec![pending_control]);
    }

    #[tokio::test(start_paused = true)]
    async fn delayed_join_data_uses_the_cpp_strict_accepted_ping_timeout() {
        // Accepted connections without a first Pong fall back to whole seconds
        // since CS_Accepted and close only above 30000 ms
        // (src/C4Network2IO.cpp:1155-1177,1343-1354).
        let (client_stream, host_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake_with_liveness(
                &mut transport,
                request(-1, b"Alice", 7),
                ConnectionLivenessState::new_test(50_000, 100),
            )
            .await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"accepted")))
            .await
            .unwrap();
        tokio::task::yield_now().await;

        advance(Duration::from_secs(30)).await;
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "exactly 30000 ms remains inside the C++ accepted timeout"
        );
        advance(Duration::from_secs(1)).await;

        assert!(matches!(
            task.await.unwrap(),
            Err(ConnectionHandshakeError::PingTimeout)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn sends_conn_first_and_answers_ping_before_the_host_conn() {
        // On socket-open C++ immediately sends PID_Conn. Before either half is
        // accepted, PID_Ping is the one non-admission packet allowed and its
        // body is echoed as PID_Pong (src/C4Network2IO.cpp:478-525,807-812,
        // 1007-1017,1223-1254). The client registers only an ID-zero host and
        // waits for positive mutual ConnRe before JoinData
        // (src/C4Network2.cpp:1305-1315,1383-1392,1448-1499,1574-1623).
        let local = request(-1, b"Alice", 7);
        let host_request = request(0, b"Host", 11);
        let expected_join_data = join_data();
        let (client_stream, host_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, local).await
        });
        let mut host = ControlTransport::new(host_stream);

        assert!(matches!(
            host.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(request) if request.core.client_id == -1
        ));

        let ping = PingPacket {
            sent_at: 0x0102_0304,
            packet_counter: 0x1122_3344,
        };
        host.send_message(ControlMessage::Ping(ping)).await.unwrap();
        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Pong(ping)
        );

        host.send_message(ControlMessage::ConnectionRequest(host_request.clone()))
            .await
            .unwrap();
        assert!(matches!(
            host.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(ConnectionReply { ok: true, .. })
        ));
        host.send_message(ControlMessage::ConnectionReply(accepted(b"join accepted")))
            .await
            .unwrap();
        let discovery = ResourcePacket::Discover(ResourceDiscoverPacket {
            resource_ids: vec![9, 4, 1],
        });
        host.send_message(ControlMessage::Resource(discovery.clone()))
            .await
            .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(11)).await;
        assert!(
            !task.is_finished(),
            "accepted client must keep waiting for delayed JoinData"
        );
        host.send_message(ControlMessage::JoinData(Box::new(
            expected_join_data.clone(),
        )))
        .await
        .unwrap();

        let result = timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.peer_core, host_request.core);
        assert_eq!(result.join_data, expected_join_data);
        assert!(result.pending_resources.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_asks_the_central_admission_loop_before_assigning_the_peer() {
        // The host sends Conn from C4Network2IO immediately. Its main-thread
        // C4Network2::HandleConn/Join path assigns the client ID before the
        // positive ConnRe, while the network thread continues answering Ping
        // (src/C4Network2IO.cpp:478-525,807-812,1007-1017,1223-1254;
        // src/C4Network2.cpp:1274-1363,1395-1445,1448-1499).
        let local_host = request(0, b"Host", 7);
        let joining_request = request(-1, b"Alice", 11);
        let (host_stream, client_stream) = duplex(2048);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(host_stream);
            run_host_connection_handshake(&mut transport, local_host, &admission_tx).await
        });
        let mut client = ControlTransport::new(client_stream);

        assert!(matches!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(request) if request.core.client_id == 0
        ));
        let ping = PingPacket {
            sent_at: 0x1020_3040,
            packet_counter: 9,
        };
        client
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::Pong(ping)
        );

        client
            .send_message(ControlMessage::ConnectionRequest(joining_request.clone()))
            .await
            .unwrap();
        let central_request = timeout(Duration::from_secs(1), admission_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(central_request.request, joining_request);
        let decision_wait_ping = PingPacket {
            sent_at: 0x5060_7080,
            packet_counter: 10,
        };
        client
            .send_message(ControlMessage::Ping(decision_wait_ping))
            .await
            .unwrap();
        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::Pong(decision_wait_ping)
        );
        let mut assigned_core = central_request.request.core.clone();
        assigned_core.client_id = 3;
        central_request
            .decision_tx
            .send(AdmissionDecision::Accept {
                peer_core: assigned_core.clone(),
                before_reply: Vec::new(),
                message: wire_string(b"join accepted"),
            })
            .unwrap();

        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(accepted(b"join accepted"))
        );
        client
            .send_message(ControlMessage::ConnectionReply(accepted(
                b"host connection accepted",
            )))
            .await
            .unwrap();

        let result = timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.peer_core, assigned_core);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_buffers_conn_re_while_the_central_decision_is_pending() {
        // Conn and ConnRe are queued to the C++ main thread in arrival order,
        // while Ping stays on the network thread. A fast peer may therefore
        // send ConnRe while the serialized Conn decision is still pending;
        // Conn must finish first, without delaying Ping/Pong
        // (src/C4Network2IO.cpp:807-812,832-879,965-1028;
        // src/C4Network2.cpp:923-936,1274-1363,1448-1499).
        let (host_stream, client_stream) = duplex(2048);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(host_stream);
            run_host_connection_handshake(&mut transport, request(0, b"Host", 7), &admission_tx)
                .await
        });
        let mut client = ControlTransport::new(client_stream);
        let _ = client.read_message().await.unwrap();
        client
            .send_message(ControlMessage::ConnectionRequest(request(-1, b"Alice", 11)))
            .await
            .unwrap();
        let central_request = admission_rx.recv().await.unwrap();

        client
            .send_message(ControlMessage::ConnectionReply(accepted(
                b"host connection accepted",
            )))
            .await
            .unwrap();
        let ping = PingPacket {
            sent_at: 0x90a0_b0c0,
            packet_counter: 12,
        };
        client
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::Pong(ping)
        );

        let mut assigned_core = central_request.request.core.clone();
        assigned_core.client_id = 3;
        central_request
            .decision_tx
            .send(AdmissionDecision::Accept {
                peer_core: assigned_core.clone(),
                before_reply: Vec::new(),
                message: wire_string(b"join accepted"),
            })
            .unwrap();
        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(accepted(b"join accepted"))
        );

        let result = timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.peer_core, assigned_core);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_requires_central_loop_to_apply_before_reply_actions() {
        // C4Network2::Join executes the direct ClientJoin (which creates the
        // canonical client) before HandleConn sends its positive ConnRe. The
        // I/O task must never apply that synchronized main-thread effect
        // itself (src/C4Network2.cpp:1316-1352,1395-1445).
        let (host_stream, client_stream) = duplex(1024);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(host_stream);
            run_host_connection_handshake(&mut transport, request(0, b"Host", 7), &admission_tx)
                .await
        });
        let mut client = ControlTransport::new(client_stream);
        let _ = client.read_message().await.unwrap();
        client
            .send_message(ControlMessage::ConnectionRequest(request(-1, b"Alice", 11)))
            .await
            .unwrap();
        let central_request = admission_rx.recv().await.unwrap();
        let mut assigned_core = central_request.request.core.clone();
        assigned_core.client_id = 3;
        central_request
            .decision_tx
            .send(AdmissionDecision::Accept {
                peer_core: assigned_core.clone(),
                before_reply: vec![ConnectionAction::EmitDirectClientJoin(
                    clonk_engine::ClientJoinControlData {
                        core: assigned_core,
                        by_client: 0,
                    },
                )],
                message: wire_string(b"join accepted"),
            })
            .unwrap();

        assert!(matches!(
            timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap(),
            Err(ConnectionHandshakeError::UnappliedBeforeReplyActions { count: 1 })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_sends_central_negative_decision_before_closing() {
        // HandleConn sends the policy's negative ConnRe (including the wrong
        // password flag) and then closes the C++ connection
        // (src/C4Network2.cpp:1274-1363).
        let (host_stream, client_stream) = duplex(1024);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(host_stream);
            run_host_connection_handshake(&mut transport, request(0, b"Host", 7), &admission_tx)
                .await
        });
        let mut client = ControlTransport::new(client_stream);
        let _ = client.read_message().await.unwrap();
        client
            .send_message(ControlMessage::ConnectionRequest(request(-1, b"Alice", 11)))
            .await
            .unwrap();
        admission_rx
            .recv()
            .await
            .unwrap()
            .decision_tx
            .send(AdmissionDecision::Reject {
                message: wire_string(b"wrong password"),
                wrong_password: true,
            })
            .unwrap();

        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(ConnectionReply {
                ok: false,
                message: wire_string(b"wrong password"),
                wrong_password: true,
            })
        );
        assert!(matches!(
            timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap(),
            Err(ConnectionHandshakeError::LocalRejection {
                ref message,
                wrong_password: true,
            }) if message.as_bytes() == b"wrong password"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn drops_pong_before_half_accept_without_terminating_admission() {
        // The C++ pre-half-accept gate admits Conn, ConnRe and Ping, but simply
        // returns false for Pong; it does not close the connection
        // (src/C4Network2IO.cpp:807-812).
        let (client_stream, host_stream) = duplex(512);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::Pong(PingPacket {
            sent_at: 1,
            packet_counter: 2,
        }))
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert!(!task.is_finished());

        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"accepted")))
            .await
            .unwrap();
        host.send_message(ControlMessage::JoinData(Box::new(join_data())))
            .await
            .unwrap();
        assert!(task.await.unwrap().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_admission_frame_before_half_accept_is_counted_and_ignored() {
        // OnPacketReceived counts a nonempty PID at or above
        // PID_PacketLogStart before HandlePacket applies its pre-unpack gate.
        // Consequently, even a malformed early FwdReq is counted and dropped
        // without terminating admission (src/C4Network2IO.cpp:582,807-812,
        // 1362-1366).
        let (client_stream, mut host_stream) = duplex(512);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });

        host_stream.write_all(&tcp_frame(&[0x04])).await.unwrap();
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"accepted")))
            .await
            .unwrap();
        host.send_message(ControlMessage::JoinData(Box::new(join_data())))
            .await
            .unwrap();

        let handshake = task.await.unwrap().unwrap();
        assert_eq!(handshake.liveness.connection().inbound_packet_counter(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn zero_frame_before_half_accept_is_ignored_without_counting() {
        // C++ OnPacket returns for an empty body before OnPacketReceived can
        // advance the recoverable packet counter (src/C4Network2IO.cpp:582,
        // 1362-1366).
        let (client_stream, mut host_stream) = duplex(512);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });

        host_stream.write_all(&tcp_frame(&[])).await.unwrap();
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"accepted")))
            .await
            .unwrap();
        host.send_message(ControlMessage::JoinData(Box::new(join_data())))
            .await
            .unwrap();

        let handshake = task.await.unwrap().unwrap();
        assert_eq!(handshake.liveness.connection().inbound_packet_counter(), 1);
    }

    #[test]
    fn malformed_packet_filter_ends_at_half_accept() {
        let mut connected = ConnectionLivenessState::new_test(0, 0);
        assert!(matches!(
            accept_handshake_packet(
                crate::transport::InboundPacket::Invalid {
                    packet_type: 0x14,
                    error: TransportError::Malformed("invalid TCP simultaneous-open packet"),
                },
                &mut connected,
            ),
            Ok(None)
        ));
        assert_eq!(connected.connection().inbound_packet_counter(), 1);

        assert!(matches!(
            accept_handshake_packet(
                crate::transport::InboundPacket::Invalid {
                    packet_type: 0x7e,
                    error: TransportError::UnsupportedPacket(0x7e),
                },
                &mut connected,
            ),
            Ok(None)
        ));
        assert_eq!(connected.connection().inbound_packet_counter(), 2);

        assert!(matches!(
            accept_handshake_packet(
                crate::transport::InboundPacket::Invalid {
                    packet_type: 0x02,
                    error: TransportError::Malformed("invalid connection request"),
                },
                &mut connected,
            ),
            Err(ConnectionHandshakeError::Transport(
                TransportError::Malformed("invalid connection request")
            ))
        ));

        connected.mark_half_accepted();
        assert!(matches!(
            accept_handshake_packet(
                crate::transport::InboundPacket::Invalid {
                    packet_type: 0x14,
                    error: TransportError::Malformed("invalid TCP simultaneous-open packet"),
                },
                &mut connected,
            ),
            Err(ConnectionHandshakeError::Transport(
                TransportError::Malformed("invalid TCP simultaneous-open packet")
            ))
        ));
        assert_eq!(connected.connection().inbound_packet_counter(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn half_accepted_unhandled_packet_does_not_terminate_admission() {
        let (client_stream, host_stream) = duplex(1024);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();

        // The client is now half accepted. PID_FwdReq is structurally valid,
        // but its native handler is accepted-only, so C++ logs and ignores it
        // rather than closing the connection.
        host.send_message(ControlMessage::ForwardRequest(ForwardPacket {
            negative_list: true,
            clients: Vec::new(),
            nested_packet: vec![0x00],
        }))
        .await
        .unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"accepted")))
            .await
            .unwrap();
        host.send_message(ControlMessage::JoinData(Box::new(join_data())))
            .await
            .unwrap();

        let handshake = task.await.unwrap().unwrap();
        assert_eq!(handshake.liveness.connection().inbound_packet_counter(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn positive_conn_re_before_host_conn_is_rejected() {
        // The network thread allows ConnRe through the pre-accept gate, but
        // C4Network2::HandleConnRe closes when no peer client was created by a
        // preceding Conn (src/C4Network2IO.cpp:807-812,988-1006;
        // src/C4Network2.cpp:1448-1458).
        let (client_stream, host_stream) = duplex(512);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"join accepted")))
            .await
            .unwrap();

        assert!(matches!(
            timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap(),
            Err(ConnectionHandshakeError::ReducerInvariant(
                "positive connection reply arrived before an accepted host request"
            ))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_host_conn_gets_cpp_negative_reply_before_close() {
        // During GS_Init a client accepts only client ID zero as its host. It
        // sends "not host" in ConnRe before closing the rejected connection
        // (src/C4Network2.cpp:1305-1315,1343-1362,1383-1392).
        let (client_stream, host_stream) = duplex(512);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });
        let mut peer = ControlTransport::new(host_stream);
        let _ = peer.read_message().await.unwrap();
        peer.send_message(ControlMessage::ConnectionRequest(request(
            4,
            b"Impostor",
            11,
        )))
        .await
        .unwrap();

        assert_eq!(
            peer.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(ConnectionReply {
                ok: false,
                message: wire_string(b"not host"),
                wrong_password: false,
            })
        );
        assert!(matches!(
            timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap(),
            Err(ConnectionHandshakeError::LocalRejection {
                ref message,
                wrong_password: false,
            }) if message.as_bytes() == b"not host"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gs_init_buffers_lobby_countdown_until_join_data_installs_the_lobby() {
        // PID_LobbyCountdown is a lossless lobby packet handled by MainDlg;
        // retain it across the admission boundary when JoinData and countdown
        // arrive together
        // (src/C4Packet2.cpp:81; src/C4GameLobby.cpp:392-418,695-701).
        let expected_join_data = join_data();
        let countdown = crate::transport::LobbyCountdownPacket::new(5);
        let (client_stream, host_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"join accepted")))
            .await
            .unwrap();
        host.send_message(ControlMessage::LobbyCountdown(countdown))
            .await
            .unwrap();
        host.send_message(ControlMessage::JoinData(Box::new(
            expected_join_data.clone(),
        )))
        .await
        .unwrap();

        let result = task.await.unwrap().unwrap();
        assert_eq!(result.join_data, expected_join_data);
        assert_eq!(result.pending_lobby_countdowns, vec![countdown]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gs_init_absorbs_packets_whose_handlers_are_not_enabled_before_join_data() {
        // Status/StatusAck/ClientActReq require a non-GS_Init network state;
        // ControlReq/ControlPkt/ExecSync require initialized game control; and
        // a host-originated player-info request is ignored. Unknown address
        // owners are likewise absent from the pre-JoinData client list
        // (src/C4Network2.cpp:956-991; src/C4GameControlNetwork.cpp:477-529;
        // src/C4Network2Client.cpp:569-598; src/C4Network2Players.cpp:392-412).
        let expected_join_data = join_data();
        let (client_stream, host_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut transport = ControlTransport::new(client_stream);
            run_client_connection_handshake(&mut transport, request(-1, b"Alice", 7)).await
        });
        let mut host = ControlTransport::new(host_stream);
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionRequest(request(0, b"Host", 11)))
            .await
            .unwrap();
        let _ = host.read_message().await.unwrap();
        host.send_message(ControlMessage::ConnectionReply(accepted(b"join accepted")))
            .await
            .unwrap();
        host.send_message(ControlMessage::Status(NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: -1,
        }))
        .await
        .unwrap();
        host.send_message(ControlMessage::StatusAck(NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: -1,
        }))
        .await
        .unwrap();
        host.send_message(ControlMessage::ActivationRequest { tick: 17 })
            .await
            .unwrap();
        host.send_message(ControlMessage::Request { from_tick: 17 })
            .await
            .unwrap();
        let direct_control = crate::encode_control_entry_payload(
            &clonk_engine::ControlPacket::ActivateGameGoalMenu(
                clonk_engine::ActivateGameGoalMenuControlData {
                    player: 0,
                    by_client: 0,
                },
            ),
        )
        .unwrap();
        host.send_message(ControlMessage::Packet {
            delivery: ControlDelivery::Direct,
            data: direct_control,
        })
        .await
        .unwrap();
        host.send_message(ControlMessage::ExecSync { control_tick: 17 })
            .await
            .unwrap();
        host.send_message(ControlMessage::PlayerInfoUpdate(PlayerInfoUpdateRequest {
            client_id: 0,
            flags: 0,
            players: Vec::new(),
        }))
        .await
        .unwrap();
        host.send_message(ControlMessage::Address(AddressPacket {
            client_id: 99,
            address: NetworkAddress::new(
                NetworkProtocol::Tcp,
                "198.51.100.9:11112".parse().unwrap(),
            ),
        }))
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert!(!task.is_finished());

        host.send_message(ControlMessage::JoinData(Box::new(
            expected_join_data.clone(),
        )))
        .await
        .unwrap();
        let result = task.await.unwrap().unwrap();
        assert_eq!(result.join_data, expected_join_data);
        assert!(result.pending_controls.is_empty());
        assert!(result.pending_addresses.is_empty());
    }
}
