use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::future::{poll_fn, Future};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering as AtomicOrdering},
    Arc, Mutex,
};
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::interval;

use crate::legacy::{decode_control_entry_payload, validate_control_envelope};
use crate::{
    aggregate_ready_batch, reconcile_join_client_registry, run_client_connection_handshake,
    run_host_connection_handshake, AdmissionDecision, BarrierEffect, BarrierPhase, ClientId,
    ConnectionAction, ConnectionLivenessState, ControlBacklog, ControlCoordinator, ControlDelivery,
    ControlMessage, ControlOutcome, ControlPacket, HostAdmission, HostAdmissionRequest,
    JoinClientRegistrySnapshot, JoinDataEnvelope, LobbyCountdownPacket, MissingRange,
    NetpuncherAddressFamily, NetpuncherGameIds, NetpuncherIoEvent, NetpuncherPacket,
    NetpuncherRole, NetpuncherRuntimeState, NetworkStatus, ParticipantKind, ReadyBatch,
    ReadyCheckPacket, RemoteBarrierState, ResourcePacket, ResyncScheduler, StatusBarrier, Tick,
    TransportError, CURRENT_GAME_BUILD, NETWORK_STATE_GO, NETWORK_STATE_LOBBY, NETWORK_STATE_PAUSE,
};

#[cfg(test)]
thread_local! {
    static LIVENESS_TIMER_ARMS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn new_liveness_timer(deadline: tokio::time::Instant) -> Pin<Box<tokio::time::Sleep>> {
    #[cfg(test)]
    LIVENESS_TIMER_ARMS.set(LIVENESS_TIMER_ARMS.get() + 1);
    Box::pin(tokio::time::sleep_until(deadline))
}

#[cfg(test)]
fn reset_liveness_timer_arms() {
    LIVENESS_TIMER_ARMS.set(0);
}

#[cfg(test)]
fn liveness_timer_arms() -> usize {
    LIVENESS_TIMER_ARMS.get()
}

mod api;
mod client_loop;
mod client_routes;
mod connect;
mod connection_state;
mod host_dispatch;
mod host_loop;
mod host_state;

pub use api::*;
pub(crate) use client_loop::*;
pub(crate) use client_routes::*;
pub use connect::*;
pub use connection_state::ControlSendTimeSnapshot;
pub(crate) use connection_state::*;
pub(crate) use host_dispatch::*;
pub(crate) use host_loop::*;
pub(crate) use host_state::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decode_control_packet, encode_control_entry_payload, encode_control_packet,
        LegacyControlFrame, NetworkStatus, ParticipantKind, NETWORK_STATE_GO,
    };
    use clonk_engine::{
        ClientUpdateControlData, ControlPacket as EngineControlPacket, PlayerControlData,
        CLIENT_UPDATE_ACTIVATE,
    };
    use clonk_resources::{c4group_file_crc, compress_c4group_image, Group, MutableGroup};
    use std::fs;
    use std::future::{pending, ready};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream, ReadBuf};
    use tokio::net::UdpSocket;
    use tokio::time::{timeout, timeout_at};

    const CPP_COMPATIBILITY_BUILD: i32 = CURRENT_GAME_BUILD + 2;

    macro_rules! host_config {
        ($($field:ident: $value:expr),* $(,)?) => {
            HostConfig {
                $($field: $value,)*
                ..HostConfig::default()
            }
        };
    }

    macro_rules! network_core {
        ($($fields:tt)*) => {
            clonk_engine::NetworkResourceCore {
                $($fields)*,
                ..Default::default()
            }
        };
    }

    trait TestValueExt<T> {
        fn test_value(self) -> T;
    }

    impl<T> TestValueExt<T> for Option<T> {
        #[track_caller]
        fn test_value(self) -> T {
            Option::expect(self, "network-test value exists")
        }
    }

    impl<T, E: std::fmt::Debug> TestValueExt<T> for Result<T, E> {
        #[track_caller]
        fn test_value(self) -> T {
            Result::expect(self, "network-test operation succeeds")
        }
    }

    async fn await_test<T, U>(future: impl Future<Output = U>) -> T
    where
        U: TestValueExt<T>,
    {
        timeout(EVENT_WAIT, future)
            .await
            .expect("network-test operation completes before the deadline")
            .test_value()
    }

    fn c4(bytes: impl AsRef<[u8]>) -> clonk_engine::LegacyCString {
        clonk_engine::LegacyCString::from_bytes(bytes.as_ref().to_vec())
            .expect("valid fixture CString")
    }

    struct FailingWriteStream;

    impl AsyncRead for FailingWriteStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for FailingWriteStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "forced writer failure",
            )))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn test_control_send_time_snapshot() -> ControlSendTimeSnapshot {
        ControlSendTimeSnapshot::default()
    }

    fn test_client_core(
        client_id: i32,
        name: clonk_engine::LegacyCString,
        lobby_ready: bool,
    ) -> clonk_engine::ClientCoreControlData {
        clonk_engine::ClientCoreControlData {
            client_id,
            activated: true,
            observer: false,
            name: name.clone(),
            nick: name,
            lobby_ready,
        }
    }

    fn compatibility_test_core(client_id: i32, name: &[u8]) -> clonk_engine::ClientCoreControlData {
        test_client_core(client_id, c4(name), false)
    }

    fn test_connection_request(
        core: clonk_engine::ClientCoreControlData,
        connection_id: u32,
        port_protocol: bool,
    ) -> crate::ConnectionRequest {
        crate::ConnectionRequest {
            core,
            build: CURRENT_GAME_BUILD,
            password: clonk_engine::LegacyCString::default(),
            connection_id,
            port_protocol,
        }
    }

    fn test_connection_reply(
        ok: bool,
        message: clonk_engine::LegacyCString,
        port_protocol: bool,
    ) -> crate::ConnectionReply {
        crate::ConnectionReply {
            ok,
            message,
            wrong_password: false,
            port_protocol,
        }
    }

    fn legacy_frame(
        client_id: ClientId,
        tick: Tick,
        controls: Vec<EngineControlPacket>,
    ) -> LegacyControlFrame {
        LegacyControlFrame {
            client_id,
            tick,
            timestamp_ms: 0,
            controls,
        }
    }

    fn assert_single_control_author(
        control: &EngineControlPacket,
        author: i32,
        spoofed_author: i32,
    ) {
        let payload = encode_control_entry_payload(control).test_value();
        assert_eq!(
            authenticated_single_control(&payload, author).expect("matching author"),
            control.clone()
        );
        let error = authenticated_single_control(&payload, spoofed_author)
            .expect_err("reject spoofed control author");
        assert!(error.contains(&format!("claimed author {author}")));
        assert!(error.contains(&format!("authenticated author is {spoofed_author}")));
    }

    fn assert_queued_control_author(
        control: impl Fn(i32) -> EngineControlPacket,
        control_name: &str,
    ) {
        let packet = |author| {
            encode_control_packet(&legacy_frame(7, 12, vec![control(author)])).test_value()
        };
        validate_queued_control_authors(&packet(7)).test_value();
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge the host author");
        assert!(error.contains(control_name), "{control_name}: {error}");
        assert!(
            error.contains("claimed author 0"),
            "{control_name}: {error}"
        );
        assert!(
            error.contains("authenticated author is 7"),
            "{control_name}: {error}"
        );
    }

    fn test_join_data(
        client_id: i32,
        status: NetworkStatus,
        snapshot: HostJoinSnapshot,
    ) -> JoinDataEnvelope {
        JoinDataEnvelope {
            client_id,
            start_control_tick: snapshot.dynamic_tick,
            status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        }
    }

    fn empty_client_resource_state(client_id: i32, directory: PathBuf) -> ClientResourceState {
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = test_join_data(client_id, host.initial_status, snapshot);
        ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directory),
        )
        .test_value()
    }

    async fn admit_and_send_test_join_data<S, F>(
        transport: &mut crate::ControlTransport<S>,
        snapshot: F,
    ) where
        S: AsyncRead + AsyncWrite + Unpin,
        F: FnOnce(&clonk_engine::ClientCoreControlData) -> HostJoinSnapshot,
    {
        let host_core = test_client_core(0, c4(b"Host"), false);
        let request = test_connection_request(host_core.clone(), 9, false);
        let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(1);
        let admission = tokio::spawn(async move {
            let request = admission_rx.recv().await.test_value();
            let mut assigned = request.request.core.clone();
            assigned.client_id = 1;
            request
                .decision_tx
                .send(AdmissionDecision::Accept {
                    peer_core: assigned.clone(),
                    before_reply: Vec::new(),
                    message: c4(b"join accepted"),
                })
                .test_value();
            assigned
        });
        run_host_connection_handshake(transport, request, &admission_tx)
            .await
            .test_value();
        let assigned = admission.await.test_value();
        let mut snapshot = snapshot(&host_core);
        snapshot.parameters.clients =
            JoinClientRegistrySnapshot::new(vec![host_core, assigned.clone()]);
        transport
            .send_message(ControlMessage::JoinData(Box::new(test_join_data(
                assigned.client_id,
                NetworkStatus::new(NETWORK_STATE_LOBBY, 0, -1),
                snapshot,
            ))))
            .await
            .test_value();
    }

    async fn read_compatibility_request<S>(stream: S) -> crate::ConnectionRequest
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut transport = crate::ControlTransport::new(stream);
        match transport.read_message().await.test_value() {
            ControlMessage::ConnectionRequest(request) => request,
            other => panic!("expected client connection request, got {other:?}"),
        }
    }

    async fn bind_test_listener() -> (SocketAddr, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener binds");
        (listener.local_addr().test_value(), listener)
    }

    async fn start_test_host(config: HostConfig) -> (SocketAddr, HostHandle) {
        let (address, listener) = bind_test_listener().await;
        let host = start_host(listener, config).await.test_value();
        (address, host)
    }

    fn command_test_host_handle() -> (HostHandle, mpsc::Receiver<HostCommand>) {
        let (command_tx, commands) = mpsc::channel(1);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        (
            HostHandle {
                command_tx,
                control_send_time: test_control_send_time_snapshot(),
                event_rx: Some(event_rx),
                voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
                voice_event_rx: Some(mpsc::channel(1).1),
                shutdown_tx: Some(shutdown_tx),
                join_handle: tokio::spawn(async {}),
                udp_local_addr: None,
                io_statistics: crate::NetworkIoStatistics::new(0),
            },
            commands,
        )
    }

    async fn connect_test_player(address: SocketAddr, name: impl Into<String>) -> ClientHandle {
        Result::expect(
            connect_client(address, ClientConfig::new(name, ParticipantKind::Player)).await,
            "test player connects",
        )
    }

    async fn shutdown_test_session(client: ClientHandle, host: HostHandle) {
        client.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    type TestClientLoop = (
        DuplexStream,
        mpsc::Sender<ClientCommand>,
        mpsc::Receiver<ClientEvent>,
        oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    );

    fn start_test_client_loop(
        buffer: usize,
        command_capacity: usize,
        event_capacity: usize,
    ) -> TestClientLoop {
        start_test_client_loop_with_state(
            buffer,
            command_capacity,
            event_capacity,
            BTreeMap::new(),
            ClientResourceState::empty(),
        )
    }

    fn start_test_client_loop_with_state(
        buffer: usize,
        command_capacity: usize,
        event_capacity: usize,
        client_addresses: BTreeMap<i32, Vec<crate::NetworkAddress>>,
        resource_state: ClientResourceState,
    ) -> TestClientLoop {
        let (client_stream, host_stream) = duplex(buffer);
        let (command_tx, command_rx) = mpsc::channel(command_capacity);
        let (event_tx, event_rx) = mpsc::channel(event_capacity);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            client_addresses,
            resource_state,
        ));
        (host_stream, command_tx, event_rx, shutdown_tx, task)
    }

    fn start_test_host_route<S>(
        stream: S,
        client_id: ClientId,
    ) -> (
        HostOutboundSender,
        mpsc::UnboundedReceiver<HostLoopMessage>,
        tokio::task::JoinHandle<()>,
    )
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (outbound, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound.subscribe_retire();
        let (host_tx, host_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id,
                transport: crate::ControlTransport::new(stream),
                outbound_rx,
                retire_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );
        (outbound, host_rx, task)
    }

    type TestClientRoute = (
        mpsc::UnboundedSender<ClientRouteCommand>,
        watch::Sender<bool>,
        mpsc::UnboundedReceiver<ClientRouteEvent>,
        tokio::task::JoinHandle<()>,
    );

    fn start_test_client_route(stream: DuplexStream) -> TestClientRoute {
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let (retire_tx, retire_rx) = watch::channel(false);
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_client_route(
            1,
            11,
            None,
            crate::ControlTransport::new(stream),
            outbound_tx.clone(),
            outbound_rx,
            retire_rx,
            event_tx,
            ConnectionLivenessState::new_accepted_system(),
        ));
        (outbound_tx, retire_tx, event_rx, task)
    }

    fn single_test_client_route<S>(stream: S) -> ClientRouteManager
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(stream),
            ConnectionLivenessState::new_accepted_system(),
        );
        routes
    }

    #[test]
    fn async_control_wait_deadline_uses_the_reached_target_fps() {
        let reached_at = tokio::time::Instant::now();
        let wait = |target_fps| AsyncControlWait {
            tick: 0,
            reached_at,
            control_rate: 2,
            target_fps,
        };

        // floor(2 * 2 * 1000 / 38) + the strict-equality millisecond.
        assert_eq!(
            wait(DEFAULT_CONTROL_TARGET_FPS).deadline(2),
            reached_at + Duration::from_millis(106)
        );
        // Doubling the target FPS changes the native deadline exactly:
        // floor(2 * 2 * 1000 / 76) + 1 = 53ms.
        assert_eq!(wait(76).deadline(2), reached_at + Duration::from_millis(53));
    }

    #[test]
    fn control_send_time_matches_cpp_decentral_and_central_topologies() {
        // CalcPerformance samples each activated remote client's preferred
        // message connection. Direct decentralized paths use half the average;
        // a tunnel is costed as another host path and disables that halving
        // (oracle-src-pinned src/C4GameControlNetwork.cpp:382-447).
        assert_eq!(
            control_send_time_ms(0, [(7, Some(100)), (8, Some(300))]),
            66,
            "an activated host must sample its direct remote clients"
        );
        assert_eq!(
            control_send_time_ms(
                0,
                [(HOST_CLIENT_ID, Some(40)), (7, Some(80)), (8, Some(120)),],
            ),
            40,
            "an all-direct client uses half the host/peer average"
        );
        assert_eq!(
            control_send_time_ms(0, [(HOST_CLIENT_ID, Some(40)), (7, Some(100)), (8, None),],),
            60,
            "a tunneled client uses the full host-weighted average"
        );
        assert_eq!(
            control_send_time_ms(1, [(HOST_CLIENT_ID, Some(40)), (7, Some(100)), (8, None),],),
            40,
            "central control is paced only by the host route"
        );
        assert_eq!(
            control_send_time_ms(1, [(7, Some(100)), (8, Some(300))]),
            0,
            "the central host has no remote host route"
        );
    }

    #[test]
    fn client_performance_stats_match_signed_cpp_ewma_at_consumption() {
        let base = tokio::time::Instant::now();
        let mut stats = ClientPerformanceStats::new(16);
        let mut expected_scaled = 0_i32;
        let waits = [100_i32, -100, 37, -12];
        let consumed_at = base + Duration::from_secs(20);

        for (tick, wait_ms) in waits.into_iter().enumerate() {
            let tick = tick as Tick;
            let before_sample = stats.scaled_wait_ms.get(&7).copied();
            let reached_at = base + Duration::from_secs(u64::from(tick) + 1);
            let arrived_at = if wait_ms >= 0 {
                reached_at + Duration::from_millis(wait_ms as u64)
            } else {
                reached_at - Duration::from_millis(wait_ms.unsigned_abs() as u64)
            };

            match tick % 3 {
                0 => {
                    stats.record_arrival(7, tick, arrived_at);
                    stats.record_cadence(tick, reached_at);
                    assert_eq!(stats.scaled_wait_ms.get(&7).copied(), before_sample);
                    stats.mark_consumed(tick, consumed_at, [7]);
                }
                1 => {
                    stats.record_arrival(7, tick, arrived_at);
                    stats.mark_consumed(tick, consumed_at, [7]);
                    assert_eq!(stats.scaled_wait_ms.get(&7).copied(), before_sample);
                    stats.record_cadence(tick, reached_at);
                }
                _ => {
                    stats.record_cadence(tick, reached_at);
                    // The first cadence is authoritative even if a repeated
                    // stalled-frame probe arrives later.
                    stats.record_cadence(tick, reached_at + Duration::from_secs(10));
                    stats.record_arrival(7, tick, arrived_at);
                    stats.mark_consumed(tick, consumed_at, [7]);
                }
            }

            expected_scaled += (wait_ms * 100 - expected_scaled) / 100;
            assert_eq!(stats.scaled_wait_ms.get(&7), Some(&expected_scaled));
            assert_eq!(stats.wait_ms(7), expected_scaled / 100);

            // Duplicate observations must not apply the EWMA twice.
            stats.record_arrival(7, tick, arrived_at + Duration::from_secs(1));
            stats.mark_consumed(tick, consumed_at, [7]);
            assert_eq!(stats.scaled_wait_ms.get(&7), Some(&expected_scaled));
        }

        let consumed_wait = stats.scaled_wait_ms.get(&7).copied();
        stats.record_cadence(99, base);
        stats.mark_consumed(99, base + Duration::from_millis(100), [7]);
        stats.record_arrival(7, 99, base + Duration::from_secs(1));
        assert_eq!(stats.scaled_wait_ms.get(&7).copied(), consumed_wait);
        assert!(!stats
            .arrivals
            .get(&99)
            .is_some_and(|arrivals| arrivals.contains_key(&7)));

        // Command scheduling may deliver a pre-cutoff timestamp after the
        // consumption message; the timestamp, not handler order, is native.
        stats.mark_consumed(100, base + Duration::from_secs(2), [8]);
        stats.record_cadence(100, base);
        stats.record_arrival(8, 100, base + Duration::from_secs(1));
        assert_eq!(stats.wait_ms(8), 10);

        stats.record_cadence(101, base);
        stats.record_arrival(9, 101, base + Duration::from_secs(1));
        stats.mark_consumed(101, base + Duration::from_secs(2), [7]);
        assert_eq!(stats.wait_ms(9), 0, "inactive clients are not sampled");

        let pre_reset_reached_at = base + Duration::from_secs(30);
        stats.record_arrival(7, 102, pre_reset_reached_at + Duration::from_millis(500));
        stats.mark_consumed(102, pre_reset_reached_at + Duration::from_secs(1), [7]);
        stats.reset_accumulators();
        stats.record_cadence(102, pre_reset_reached_at);
        assert_eq!(stats.wait_ms(7), 0);
        assert_eq!(stats.wait_ms(8), 0);
        let reset_reached_at = base + Duration::from_secs(31);
        stats.record_cadence(103, reset_reached_at);
        stats.record_arrival(7, 103, reset_reached_at + Duration::from_millis(500));
        stats.mark_consumed(103, reset_reached_at + Duration::from_secs(1), [7]);
        assert_eq!(stats.wait_ms(7), 5);
    }

    #[test]
    fn repeated_performance_observations_consider_tick_retention_once() {
        let reached_at = tokio::time::Instant::now();
        let mut stats = ClientPerformanceStats::new(2);

        stats.record_cadence(10, reached_at);
        for client_id in 1..=24 {
            stats.record_arrival(client_id, 10, reached_at);
        }
        stats.mark_consumed(10, reached_at, 1..=24);

        assert_eq!(stats.retention_considerations(), 1);
    }

    #[test]
    fn unbounded_performance_history_does_not_duplicate_tick_tracking() {
        let reached_at = tokio::time::Instant::now();
        let mut stats = ClientPerformanceStats::new(0);

        for tick in 0..1_024 {
            stats.record_cadence(tick, reached_at);
        }

        assert_eq!(stats.tracked_tick_count(), 0);
        assert_eq!(stats.retention_considerations(), 0);
    }

    fn tcp_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0xff];
        frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    async fn write_udp_session_payload(stream: &mut crate::ReliableUdpPeerStream, payload: &[u8]) {
        stream.write_all(&tcp_frame(payload)).await.test_value();
        stream.flush().await.test_value();
    }

    async fn read_udp_session_payload(stream: &mut crate::ReliableUdpPeerStream) -> Vec<u8> {
        let mut header = [0_u8; 5];
        stream.read_exact(&mut header).await.test_value();
        assert_eq!(header[0], 0xff);
        let length = u32::from_ne_bytes(header[1..].try_into().test_value()) as usize;
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).await.test_value();
        payload
    }

    #[derive(Clone, Default)]
    struct RecordingPortMappingBackend {
        started: Arc<Mutex<Vec<Vec<crate::upnp::PortMappingRequest>>>>,
        released: Arc<Mutex<Vec<Vec<crate::upnp::PortMappingRequest>>>>,
    }

    struct RecordingActivePortMappings {
        requests: Vec<crate::upnp::PortMappingRequest>,
        released: Arc<Mutex<Vec<Vec<crate::upnp::PortMappingRequest>>>>,
    }

    impl crate::upnp::ActivePortMappings for RecordingActivePortMappings {
        fn shutdown(self: Box<Self>) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
            Box::pin(async move {
                self.released.lock().test_value().push(self.requests);
            })
        }
    }

    impl crate::upnp::PortMappingBackend for RecordingPortMappingBackend {
        fn start(
            &self,
            requests: &[crate::upnp::PortMappingRequest],
        ) -> Box<dyn crate::upnp::ActivePortMappings> {
            self.started.lock().test_value().push(requests.to_vec());
            Box::new(RecordingActivePortMappings {
                requests: requests.to_vec(),
                released: Arc::clone(&self.released),
            })
        }
    }

    #[test]
    fn upnp_mapping_requests_require_enablement_and_live_bound_transports() {
        let mut config = host_config!(enable_upnp: true,
        configured_tcp_port: Some(31_112),
        configured_udp_port: Some(31_113));
        let tcp = SocketAddr::from(([127, 0, 0, 1], 40_001));
        let udp = SocketAddr::from(([127, 0, 0, 1], 40_002));
        assert_eq!(
            host_port_mapping_requests(&config, Some(tcp), Some(udp)),
            vec![
                crate::upnp::PortMappingRequest {
                    protocol: crate::upnp::PortMappingProtocol::Tcp,
                    internal_port: 31_112,
                    external_port: 0,
                },
                crate::upnp::PortMappingRequest {
                    protocol: crate::upnp::PortMappingProtocol::Udp,
                    internal_port: 31_113,
                    external_port: 0,
                },
            ]
        );
        assert_eq!(
            host_port_mapping_requests(&config, Some(tcp), None),
            vec![crate::upnp::PortMappingRequest {
                protocol: crate::upnp::PortMappingProtocol::Tcp,
                internal_port: 31_112,
                external_port: 0,
            }],
            "an unavailable UDP listener must not map its configured port"
        );
        assert_eq!(
            host_port_mapping_requests(&config, None, Some(udp)),
            vec![crate::upnp::PortMappingRequest {
                protocol: crate::upnp::PortMappingProtocol::Udp,
                internal_port: 31_113,
                external_port: 0,
            }],
            "an unavailable TCP listener must not map its configured port"
        );

        config.enable_upnp = false;
        assert!(host_port_mapping_requests(&config, Some(tcp), Some(udp)).is_empty());
    }

    #[test]
    fn configured_zero_udp_port_disables_the_udp_binding() {
        let binding = HostUdpBinding::bind(
            &host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
        netpuncher_addresses: vec![SocketAddr::from(([127, 0, 0, 1], 11_115))],
        configured_udp_port: Some(0)),
        );

        assert_eq!(binding.local_addr(), None);
        assert_eq!(binding.bind_error(), None);
    }

    #[tokio::test]
    async fn enabled_upnp_host_requests_tcp_udp_and_releases_on_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let config = host_config!(enable_upnp: true,
        udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
        configured_tcp_port: Some(31_112),
        configured_udp_port: Some(31_113));
        let udp_binding = HostUdpBinding::bind(&config);
        assert!(udp_binding.local_addr().is_some());
        let backend = RecordingPortMappingBackend::default();
        let host =
            start_host_with_udp_binding_and_backend(Some(listener), config, udp_binding, &backend)
                .await
                .test_value();
        let expected = vec![
            crate::upnp::PortMappingRequest {
                protocol: crate::upnp::PortMappingProtocol::Tcp,
                internal_port: 31_112,
                external_port: 0,
            },
            crate::upnp::PortMappingRequest {
                protocol: crate::upnp::PortMappingProtocol::Udp,
                internal_port: 31_113,
                external_port: 0,
            },
        ];
        assert_eq!(
            &*backend.started.lock().unwrap(),
            std::slice::from_ref(&expected)
        );

        host.shutdown().await.test_value();
        assert_eq!(&*backend.released.lock().unwrap(), &[expected]);
    }

    #[tokio::test]
    async fn host_session_requests_puncher_id_punches_and_reports_assigned_state() {
        let mut puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let puncher_address = puncher.local_addr();
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
            netpuncher_addresses: vec![puncher_address],
            configured_tcp_port: Some(31_112),
            configured_udp_port: Some(31_113)),
        )
        .await
        .test_value();
        let host_udp_address = host.udp_local_addr().test_value();
        let mut puncher_stream = timeout(Duration::from_secs(2), puncher.accept())
            .await
            .unwrap()
            .test_value();
        let observed_address = puncher_stream.peer_addr();

        let request = timeout(
            Duration::from_secs(2),
            read_udp_session_payload(&mut puncher_stream),
        )
        .await
        .test_value();
        assert_eq!(
            crate::decode_netpuncher_packet(&request).unwrap(),
            NetpuncherPacket::IdRequest
        );

        let local_addresses = timeout(Duration::from_secs(2), async {
            loop {
                match host.events().recv().await {
                    Some(HostEvent::LocalAddressesChanged { local_addresses }) => {
                        break local_addresses;
                    }
                    Some(_) => continue,
                    None => panic!("host event stream ended"),
                }
            }
        })
        .await
        .test_value();
        let observed_udp =
            crate::NetworkAddress::new(crate::NetworkProtocol::Udp, observed_address);
        let mut configured_udp = observed_address;
        configured_udp.set_port(31_113);
        let configured_udp =
            crate::NetworkAddress::new(crate::NetworkProtocol::Udp, configured_udp);
        let mut configured_tcp = observed_address;
        configured_tcp.set_port(31_112);
        let configured_tcp =
            crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, configured_tcp);
        assert!(local_addresses.contains(&observed_udp));
        assert!(local_addresses.contains(&configured_udp));
        assert!(local_addresses.contains(&configured_tcp));

        let assigned_id = 0x1122_3344;
        write_udp_session_payload(
            &mut puncher_stream,
            &crate::encode_netpuncher_packet(&NetpuncherPacket::AssignId { id: assigned_id }),
        )
        .await;
        let (game_ids, assigned_addresses) = timeout(Duration::from_secs(2), async {
            loop {
                match host.events().recv().await {
                    Some(HostEvent::NetpuncherStateChanged {
                        game_ids,
                        local_addresses,
                    }) => break (game_ids, local_addresses),
                    Some(_) => continue,
                    None => panic!("host event stream ended"),
                }
            }
        })
        .await
        .test_value();
        assert_eq!(game_ids.ipv4, assigned_id);
        assert_eq!(game_ids.ipv6, 0);
        assert_eq!(assigned_addresses, local_addresses);

        let target = UdpSocket::bind("127.0.0.1:0").await.test_value();
        let target_address = target.local_addr().test_value();
        write_udp_session_payload(
            &mut puncher_stream,
            &crate::encode_netpuncher_packet(&NetpuncherPacket::ClientRequest {
                address: target_address,
            }),
        )
        .await;
        let mut raw_punch = [0_u8; 16];
        let (length, source) = timeout(Duration::from_secs(2), target.recv_from(&mut raw_punch))
            .await
            .unwrap()
            .test_value();
        assert_eq!(
            crate::canonical_reliable_udp_peer_address(source),
            host_udp_address
        );
        assert_eq!(length, 9);
        assert_eq!(raw_punch[0], 0x01);

        host.shutdown().await.test_value();
        puncher.shutdown().await.test_value();
    }

    #[tokio::test]
    async fn live_host_initializes_one_netpuncher_per_resolved_family() {
        let mut ipv4_puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let mut ipv6_puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 0)))
                .test_value();
        let ipv4_address = ipv4_puncher.local_addr();
        let ipv6_address = ipv6_puncher.local_addr();
        let ignored_same_family = SocketAddr::from(([127, 0, 0, 2], ipv4_address.port()));
        let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 0)))
            .await
            .test_value();
        let host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([0_u16; 8], 0))),
            netpuncher_addresses: Vec::new()),
        )
        .await
        .test_value();

        host.init_netpunchers(vec![ipv4_address, ignored_same_family, ipv6_address])
            .await
            .test_value();
        let (mut ipv4_stream, mut ipv6_stream) = tokio::join!(
            async {
                timeout(Duration::from_secs(2), ipv4_puncher.accept())
                    .await
                    .unwrap()
                    .unwrap()
            },
            async {
                timeout(Duration::from_secs(2), ipv6_puncher.accept())
                    .await
                    .unwrap()
                    .unwrap()
            }
        );
        for stream in [&mut ipv4_stream, &mut ipv6_stream] {
            let payload = timeout(Duration::from_secs(2), read_udp_session_payload(stream))
                .await
                .test_value();
            assert_eq!(
                crate::decode_netpuncher_packet(&payload).unwrap(),
                NetpuncherPacket::IdRequest
            );
        }

        host.shutdown().await.test_value();
        ipv4_puncher.shutdown().await.test_value();
        ipv6_puncher.shutdown().await.test_value();
    }

    fn add_test_route_queue(
        routes: &mut ClientRouteManager,
        route_id: u32,
        peer_id: ClientId,
        protocol: crate::NetworkProtocol,
    ) -> mpsc::UnboundedReceiver<ClientRouteCommand> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (retire, _retire_rx) = watch::channel(false);
        assert!(routes
            .routes
            .insert(
                route_id,
                ClientRouteEntry {
                    peer_id,
                    initiator_id: peer_id,
                    remote_connection_id: route_id.wrapping_add(1_000),
                    protocol,
                    peer_addr: None,
                    ping: RoutePingLag::default(),
                    outbound: ClientRouteSender {
                        sender,
                        retire,
                        post_failure: PostFailureBuffer::default(),
                        udp: None,
                    },
                    voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                    peer_is_port: true,
                },
            )
            .is_none());
        receiver
    }

    #[tokio::test]
    async fn retained_round_restart_route_discards_auxiliary_round_traffic() {
        let mut routes = ClientRouteManager::new();
        let _tcp_commands =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        let _udp_commands =
            add_test_route_queue(&mut routes, 2, HOST_CLIENT_ID, crate::NetworkProtocol::Udp);
        let _mesh_commands = add_test_route_queue(&mut routes, 3, 7, crate::NetworkProtocol::Tcp);
        routes.closed_routes.retain(4, HOST_CLIENT_ID, 0);
        routes.pending_post_mortems.insert(
            1,
            crate::PostMortemPacket {
                connection_id: 11,
                packet_counter: 0,
                packets: Vec::new(),
            },
        );
        routes.replay_packets.push_back((
            HOST_CLIENT_ID,
            crate::transport::InboundPacket::Message(ControlMessage::Resource(
                ResourcePacket::Data(crate::ResourceDataPacket {
                    resource_id: 9,
                    chunk: 0,
                    data: vec![0xaa],
                }),
            )),
            None,
        ));

        assert_eq!(
            routes.retain_round_restart_route(2),
            Some(RoundRestartRetiredHostRoutes {
                tcp: true,
                udp: false,
            })
        );

        assert_eq!(routes.routes.keys().copied().collect::<Vec<_>>(), vec![2]);
        assert!(routes.pending_post_mortems.is_empty());
        assert!(routes.replay_packets.is_empty());
        assert!(!routes.closed_routes.contains(4));
    }

    async fn expect_control_wait_attribution_capability<S>(
        transport: &mut crate::ControlTransport<S>,
    ) where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        assert!(matches!(
            transport.read_message().await.test_value(),
            ControlMessage::PortCapabilities(capabilities)
                if capabilities == crate::PortCapabilities::supported_without_voice()
        ));
    }

    #[tokio::test]
    async fn first_host_routed_control_announces_wait_attribution_capability_before_the_control() {
        let mut routes = ClientRouteManager::new();
        let mut host =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        let first = legacy_packet(7, 73, 0x31);

        routes
            .send_message(ControlMessage::Control(first.clone()))
            .await
            .test_value();

        assert!(matches!(
            host.try_recv().test_value(),
            ClientRouteCommand::Message(ControlMessage::PortCapabilities(capabilities))
                if capabilities == crate::PortCapabilities::supported_without_voice()
        ));
        assert!(matches!(
            host.try_recv().test_value(),
            ClientRouteCommand::Message(ControlMessage::Control(packet)) if packet == first
        ));

        let second = legacy_packet(7, 74, 0x32);
        routes
            .send_message(ControlMessage::Control(second.clone()))
            .await
            .test_value();
        assert!(matches!(
            host.try_recv().test_value(),
            ClientRouteCommand::Message(ControlMessage::Control(packet)) if packet == second
        ));
        assert!(host.try_recv().is_err());
    }

    #[tokio::test]
    async fn first_host_routed_control_skips_port_capability_for_stock_peer() {
        let mut routes = ClientRouteManager::new();
        let mut host =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        routes.routes.get_mut(&1).test_value().peer_is_port = false;
        let first = legacy_packet(7, 73, 0x31);

        routes
            .send_message(ControlMessage::Control(first.clone()))
            .await
            .test_value();

        assert!(matches!(
            host.try_recv().test_value(),
            ClientRouteCommand::Message(ControlMessage::Control(packet)) if packet == first
        ));
        assert!(host.try_recv().is_err());
    }

    fn host_state_with_test_route(client_id: ClientId, outbound: HostOutboundSender) -> HostState {
        let config = HostConfig::default();
        let backlog_limit = config.backlog_limit;
        let mut coordinator = ControlCoordinator::with_start_tick(backlog_limit, config.start_tick);
        coordinator.register_client(HOST_CLIENT_ID).test_value();
        let (event_tx, _event_rx) = mpsc::channel(1);
        let resource_resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &crate::ClientBootstrapLocalCandidates::default(),
            PathBuf::from("Network"),
        );
        let peer_core = compatibility_test_core(client_id as i32, b"Peer");

        HostState {
            coordinator,
            game_control_tick: config.start_tick,
            pending_complete: BTreeMap::new(),
            backlog: ControlBacklog::new(backlog_limit),
            client_performance: ClientPerformanceStats::new(backlog_limit),
            local_control_backlog: ControlBacklog::new(backlog_limit),
            scheduler: ResyncScheduler::new(config.resync_cooldown),
            clients: BTreeMap::from([(
                client_id,
                ClientConnection {
                    outbound: outbound.clone(),
                    core: peer_core.clone(),
                    peer_addr: "127.0.0.1:11112".parse().test_value(),
                    join_data_sent: true,
                    join_data_needed_emitted: false,
                },
            )]),
            accepted_routes: BTreeMap::from([(
                1,
                AcceptedConnectionRoute {
                    client_id,
                    remote_connection_id: 2,
                    peer_addr: "127.0.0.1:11112".parse().test_value(),
                    protocol: crate::NetworkProtocol::Tcp,
                    ping: RoutePingLag::default(),
                    outbound,
                    voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                    peer_is_port: false,
                },
            )]),
            accepted_route_waiters: Vec::new(),
            control_send_time_epoch: 0,
            closed_routes: crate::post_mortem::ClosedConnectionRouter::default(),
            pending_sync: Vec::new(),
            status_barrier: StatusBarrier::stable(config.initial_status),
            last_chase_target_update: None,
            game_started: false,
            control_mode: config.initial_status.control_mode,
            control_waiting_clients: BTreeMap::new(),
            control_discarded_clients: BTreeMap::new(),
            straggler_late: Default::default(),
            peer_capabilities: Default::default(),
            async_control_wait: None,
            admission: HostAdmission::new(
                1,
                true,
                None,
                [config.local_core.name.clone(), peer_core.name.clone()],
            ),
            client_cores: BTreeMap::from([
                (HOST_CLIENT_ID as i32, config.local_core.clone()),
                (client_id as i32, peer_core),
            ]),
            client_addresses: BTreeMap::new(),
            netpuncher_game_ids: NetpuncherGameIds::default(),
            pending_kinds: BTreeMap::new(),
            join_snapshot: config.initial_join_snapshot.clone(),
            dynamic_required_clients: BTreeSet::new(),
            resource_catalog: crate::ResourceCatalog::new(HOST_CLIENT_ID as i32),
            resource_backend: None,
            published_player_sources: BTreeMap::new(),
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
            event_tx,
            config,
        }
    }

    fn host_state_with_pending_accept(
        connection_id: u32,
        client_id: ClientId,
    ) -> (HostState, clonk_engine::ClientCoreControlData) {
        let (seed_outbound, _seed_receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, seed_outbound);
        let core = compatibility_test_core(client_id as i32, b"Joining peer");
        state.clients.clear();
        state.accepted_routes.clear();
        state
            .client_cores
            .retain(|known_client_id, _| *known_client_id == HOST_CLIENT_ID as i32);
        state.client_cores.insert(core.client_id, core.clone());
        state.pending_route_clients.insert(connection_id, client_id);
        state.join_snapshot = Some(synthetic_join_snapshot(
            state.config.local_core.clone(),
            state.config.max_players,
        ));
        state.client_addresses.insert(
            HOST_CLIENT_ID as i32,
            vec![crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "127.0.0.1:11112".parse().unwrap(),
            )],
        );
        (state, core)
    }

    /// A port peer announcing a different compatibility profile is refused
    /// when it announces, before any lobby or game state exists
    /// (clonk-org/clonk-rs#583).
    #[tokio::test]
    async fn a_peer_announcing_a_different_profile_is_disconnected() {
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        state.config.compat_profile_legacy = true;
        let connection_id = *state
            .accepted_routes
            .keys()
            .next()
            .expect("the harness route exists");

        // The peer says it is a port running the Normal profile.
        let peer =
            crate::PortCapabilities::from_bits(crate::PortCapabilities::COMPAT_PROFILE_ANNOUNCED);
        crate::session::host_dispatch::handle_client_message(
            connection_id,
            client_id,
            ControlMessage::PortCapabilities(peer),
            0,
            &mut state,
        )
        .await;

        assert!(
            !state.accepted_routes.contains_key(&connection_id),
            "a mismatched profile must drop the route"
        );
        assert!(
            !state.peer_capabilities.peer_supports(
                client_id as i32,
                crate::PortCapabilities::COMPAT_PROFILE_ANNOUNCED
            ),
            "a refused peer must not be recorded as a capable participant"
        );
    }

    /// A stock C++ peer announces nothing at all, so it never reaches this
    /// path — and a port peer on the same profile is admitted normally.
    #[tokio::test]
    async fn a_peer_announcing_the_same_profile_is_admitted() {
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        state.config.compat_profile_legacy = true;
        let connection_id = *state
            .accepted_routes
            .keys()
            .next()
            .expect("the harness route exists");

        let peer = crate::PortCapabilities::from_bits(
            crate::PortCapabilities::COMPAT_PROFILE_ANNOUNCED
                | crate::PortCapabilities::COMPAT_PROFILE_LEGACY_CLONK,
        );
        crate::session::host_dispatch::handle_client_message(
            connection_id,
            client_id,
            ControlMessage::PortCapabilities(peer),
            0,
            &mut state,
        )
        .await;

        assert!(
            state.accepted_routes.contains_key(&connection_id),
            "a matching profile must keep the route"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_wait_attribution_precedes_the_host_routed_aggregate() {
        let client_id = 7;
        let (outbound, mut outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        state.control_mode = 1;
        state.coordinator.register_client(client_id).test_value();
        state
            .peer_capabilities
            .record(client_id as i32, crate::PortCapabilities::supported());
        state
            .control_waiting_clients
            .insert(0, BTreeSet::from([client_id]));

        assert!(state
            .coordinator
            .ingest(legacy_packet(HOST_CLIENT_ID, 0, 0x31))
            .test_value()
            .ready
            .is_empty());
        let ready = state
            .coordinator
            .ingest(legacy_packet(client_id, 0, 0x32))
            .test_value()
            .ready;

        resolve_host_ready(ready, &mut state).await;

        assert!(matches!(
            outbound_rx.try_recv().test_value(),
            HostOutboundMessage::Message(ControlMessage::ControlWaitAttribution(
                crate::ControlWaitAttribution {
                    tick: 0,
                    waited_for_recipient: true,
                    waited_for_other: false,
                    discarded_recipient_control: false,
                }
            ))
        ));
        assert!(matches!(
            outbound_rx.try_recv().test_value(),
            HostOutboundMessage::Message(ControlMessage::Control(packet))
                if packet.client_id() == BROADCAST_CLIENT_ID && packet.tick() == 0
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn accepted_host_route_queues_join_prefix_before_general_traffic() {
        // OnClientConnect calls SendJoinData synchronously, and SendJoinData
        // appends JoinData plus every address before resource discovery and
        // before the main network loop can dispatch Status/Control traffic
        // (oracle-src-pinned src/C4Network2.cpp:1750-1800,1836-1865).
        let connection_id = 41;
        let client_id = 7;
        let (mut state, core) = host_state_with_pending_accept(connection_id, client_id);
        let (outbound, mut outbound_rx) = HostOutboundSender::channel();
        let (setup_tx, setup_rx) = oneshot::channel();

        handle_client_accepted(
            connection_id,
            91,
            core,
            false,
            "127.0.0.1:11113".parse().test_value(),
            crate::NetworkProtocol::Tcp,
            outbound,
            setup_tx,
            &mut state,
        )
        .await;
        assert!(
            setup_rx.await.unwrap().is_ok(),
            "transport setup must remain accepted"
        );

        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 0, 0);
        let resource = ResourcePacket::Discover(crate::ResourceDiscoverPacket {
            resource_ids: vec![17],
        });
        let control = legacy_packet(HOST_CLIENT_ID, 0, 0x31);
        assert!(try_send_host_message(
            &state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Status(status),
        ));
        assert!(try_send_host_message(
            &state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Resource(resource.clone()),
        ));
        assert!(try_send_host_message(
            &state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Control(control.clone()),
        ));

        assert!(matches!(
            outbound_rx.try_recv().unwrap(),
            HostOutboundMessage::Message(ControlMessage::JoinData(_))
        ));
        assert!(matches!(
            outbound_rx.try_recv().unwrap(),
            HostOutboundMessage::Message(ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                ..
            }))
        ));
        assert!(matches!(
            outbound_rx.try_recv().unwrap(),
            HostOutboundMessage::Message(ControlMessage::Status(received)) if received == status
        ));
        assert!(matches!(
            outbound_rx.try_recv().unwrap(),
            HostOutboundMessage::Message(ControlMessage::Resource(received))
                if received == resource
        ));
        assert!(matches!(
            outbound_rx.try_recv().unwrap(),
            HostOutboundMessage::Message(ControlMessage::Control(received))
                if received == control
        ));
    }

    #[tokio::test]
    async fn accepted_udp_route_skips_voice_capability_for_stock_peer() {
        let connection_id = 43;
        let client_id = 7;
        let (mut state, core) = host_state_with_pending_accept(connection_id, client_id);
        let (outbound, mut outbound_rx) = HostOutboundSender::channel();
        let (setup_tx, setup_rx) = oneshot::channel();

        handle_client_accepted(
            connection_id,
            93,
            core,
            false,
            "127.0.0.1:11115".parse().test_value(),
            crate::NetworkProtocol::Udp,
            outbound,
            setup_tx,
            &mut state,
        )
        .await;
        assert!(setup_rx.await.unwrap().is_ok());

        while let Ok(message) = outbound_rx.try_recv() {
            assert!(!matches!(
                message,
                HostOutboundMessage::Message(ControlMessage::PortCapabilities(_))
            ));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejected_host_join_prefix_fully_removes_the_accepted_route() {
        // A failed SendMsg during synchronous OnClientConnect falls through
        // the ordinary connection-loss path. It must not leave the accepted
        // route or logical client visible after its outbound queue is gone
        // (oracle-src-pinned src/C4Network2.cpp:1750-1800;
        // src/C4Network2IO.cpp:1379-1396).
        let connection_id = 42;
        let client_id = 7;
        let (mut state, core) = host_state_with_pending_accept(connection_id, client_id);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        state.event_tx = event_tx;
        let (outbound, outbound_rx) = HostOutboundSender::channel();
        drop(outbound_rx);
        // The post-failure FIFO deliberately accepts messages after only the
        // socket writer disappears. Finalized removal closes that final
        // lossless handoff as well, reproducing a route that cannot accept
        // JoinData.
        let _ = outbound.retire_and_take_post_failure();
        let (setup_tx, setup_rx) = oneshot::channel();

        handle_client_accepted(
            connection_id,
            92,
            core,
            false,
            "127.0.0.1:11114".parse().test_value(),
            crate::NetworkProtocol::Tcp,
            outbound,
            setup_tx,
            &mut state,
        )
        .await;

        assert!(
            setup_rx.await.unwrap().is_err(),
            "the transport task must be released with the setup failure"
        );
        assert!(!state.accepted_routes.contains_key(&connection_id));
        assert!(!state.clients.contains_key(&client_id));
        assert!(
            state.removing_clients.contains(&client_id)
                || !state.client_cores.contains_key(&(client_id as i32)),
            "cleanup must retain a synchronized removal or finish it immediately"
        );
        let mut saw_connection_failed = false;
        let mut saw_client_left = false;
        let mut saw_setup_diagnostic = false;
        while let Ok(event) = event_rx.try_recv() {
            match event {
                HostEvent::ClientConnectionFailed {
                    client_id: failed_client_id,
                } if failed_client_id == client_id => saw_connection_failed = true,
                HostEvent::ClientLeft {
                    client_id: left_client_id,
                } if left_client_id == client_id => saw_client_left = true,
                HostEvent::RecoverableRouteDiagnostic {
                    client_id: Some(diagnostic_client_id),
                    error,
                } if diagnostic_client_id == client_id && error.contains("JoinData") => {
                    saw_setup_diagnostic = true;
                }
                _ => {}
            }
        }
        assert!(saw_connection_failed);
        assert!(saw_client_left);
        assert!(saw_setup_diagnostic);
    }

    #[test]
    fn activated_host_control_send_time_uses_preferred_remote_message_routes() {
        // The benchmark's console host remains an activated control client
        // even without a local player. C++ therefore grows its PreSend from
        // remote route pings; a client-only HostPing event can never pace this
        // host (src/C4GameControlNetwork.cpp:389-425).
        let (first_outbound, _first_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, first_outbound);
        assert!(state.config.local_core.activated);
        state
            .accepted_routes
            .get_mut(&1)
            .test_value()
            .ping
            .record_pong(100);
        assert_eq!(
            host_control_send_time_ms(&state, &[HOST_CLIENT_ID, 7, 99]),
            25,
            "an activated ID absent from the logical client registry is ignored"
        );

        let second_id = 8;
        let second_core = compatibility_test_core(second_id as i32, b"Second");
        let (second_outbound, _second_rx) = HostOutboundSender::channel();
        state.clients.insert(
            second_id,
            ClientConnection {
                outbound: second_outbound.clone(),
                core: second_core.clone(),
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                join_data_sent: true,
                join_data_needed_emitted: false,
            },
        );
        state.client_cores.insert(second_id as i32, second_core);
        assert_eq!(
            host_control_send_time_ms(&state, &[HOST_CLIENT_ID, 7, second_id]),
            33,
            "a known logical client without a message route is a tunnel"
        );
        let mut second_ping = RoutePingLag::default();
        second_ping.record_pong(300);
        state.accepted_routes.insert(
            3,
            AcceptedConnectionRoute {
                client_id: second_id,
                remote_connection_id: 4,
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: second_ping,
                outbound: second_outbound,
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: false,
            },
        );

        assert_eq!(
            host_control_send_time_ms(&state, &[HOST_CLIENT_ID, 7, second_id]),
            66
        );
        let snapshot = ControlSendTimeSnapshot::default();
        let game_thread_snapshot = snapshot.clone();
        publish_host_control_send_time(&state, &snapshot);
        assert_eq!(
            game_thread_snapshot.sample(&[HOST_CLIENT_ID, 7, second_id]),
            66,
            "a clone handed to the game thread observes later host publications"
        );
    }

    #[test]
    fn host_broadcast_selects_each_clients_preferred_route_in_one_pass() {
        // C4Network2ClientList::BroadcastMsgToConnClients walks the client
        // list once and submits to each cached Msg connection
        // (oracle-src-pinned src/C4Network2Client.cpp:497-513).
        let first_id = 7;
        let (first_tcp, mut first_tcp_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(first_id, first_tcp);
        let (first_udp, mut first_udp_rx) = HostOutboundSender::channel();
        state.accepted_routes.insert(
            2,
            AcceptedConnectionRoute {
                client_id: first_id,
                remote_connection_id: 12,
                peer_addr: "127.0.0.1:11112".parse().test_value(),
                protocol: crate::NetworkProtocol::Udp,
                ping: RoutePingLag::default(),
                outbound: first_udp,
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: false,
            },
        );
        let second_id = 8;
        let second_core = compatibility_test_core(second_id as i32, b"Second");
        let (second_tcp, mut second_tcp_rx) = HostOutboundSender::channel();
        state.clients.insert(
            second_id,
            ClientConnection {
                outbound: second_tcp.clone(),
                core: second_core.clone(),
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                join_data_sent: true,
                join_data_needed_emitted: false,
            },
        );
        state.client_cores.insert(second_id as i32, second_core);
        state.accepted_routes.insert(
            3,
            AcceptedConnectionRoute {
                client_id: second_id,
                remote_connection_id: 13,
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: RoutePingLag::default(),
                outbound: second_tcp,
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: false,
            },
        );
        let message = ControlMessage::Status(NetworkStatus::new(NETWORK_STATE_GO, 0, 9));

        assert_eq!(
            broadcast_host_message(
                &state,
                ConnectionTrafficClass::Message,
                message.clone(),
                None,
            ),
            vec![first_id, second_id]
        );
        assert!(
            first_tcp_rx.try_recv().is_err(),
            "TCP is not selected while a live UDP message route exists"
        );
        assert!(matches!(
            first_udp_rx.try_recv(),
            Ok(HostOutboundMessage::Message(observed)) if observed == message
        ));
        assert!(matches!(
            second_tcp_rx.try_recv(),
            Ok(HostOutboundMessage::Message(observed)) if observed == message
        ));

        assert_eq!(
            broadcast_host_message(
                &state,
                ConnectionTrafficClass::Message,
                message.clone(),
                Some(first_id),
            ),
            vec![second_id]
        );
        assert!(first_udp_rx.try_recv().is_err());
        assert!(matches!(
            second_tcp_rx.try_recv(),
            Ok(HostOutboundMessage::Message(observed)) if observed == message
        ));
    }

    #[tokio::test]
    async fn client_control_send_time_publishes_only_after_route_or_topology_changes() {
        let mut routes = ClientRouteManager::new();
        let _host_tcp =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        routes.routes.get_mut(&1).test_value().ping.record_pong(900);
        let _host_udp =
            add_test_route_queue(&mut routes, 2, HOST_CLIENT_ID, crate::NetworkProtocol::Udp);
        routes.routes.get_mut(&2).test_value().ping.record_pong(40);
        let _peer = add_test_route_queue(&mut routes, 3, 7, crate::NetworkProtocol::Tcp);
        routes.routes.get_mut(&3).test_value().ping.record_pong(100);

        assert_eq!(
            routes.control_send_time_ms(0, [HOST_CLIENT_ID, 7, 8]),
            60,
            "UDP supplies the host message ping and absent peer 8 is a tunnel"
        );
        let snapshot = ControlSendTimeSnapshot::default();
        let game_thread_snapshot = snapshot.clone();
        routes.publish_control_send_time(
            &snapshot,
            0,
            1,
            BTreeSet::from([HOST_CLIENT_ID, 1, 7, 8]),
        );
        assert_eq!(
            game_thread_snapshot.sample(&[HOST_CLIENT_ID, 1, 7, 8]),
            60,
            "a clone handed to the game thread observes later client publications"
        );
        assert!(
            !routes.control_send_time_needs_publish(),
            "a complete publication clears the route-topology dirty edge"
        );

        routes
            .event_tx
            .send(ClientRouteEvent::PingMeasured {
                route_id: 2,
                round_trip_ms: 80,
            })
            .test_value();
        assert!(matches!(
            routes.read_event().await.unwrap(),
            ClientRouteRead::PingMeasured {
                peer_id: HOST_CLIENT_ID,
                round_trip_ms: 80,
            }
        ));
        assert!(
            routes.control_send_time_needs_publish(),
            "a preferred-route pong invalidates the synchronous sampler"
        );
        routes.publish_control_send_time(
            &snapshot,
            0,
            1,
            BTreeSet::from([HOST_CLIENT_ID, 1, 7, 8]),
        );
        assert_eq!(
            game_thread_snapshot.sample(&[HOST_CLIENT_ID, 1, 7, 8]),
            86,
            "the next publication observes the changed preferred-route ping"
        );

        routes.invalidate_control_send_time();
        routes.publish_control_send_time(&snapshot, 1, 1, BTreeSet::from([HOST_CLIENT_ID, 1, 7]));
        assert_eq!(
            game_thread_snapshot.sample(&[HOST_CLIENT_ID, 1, 7]),
            80,
            "mode and logical-client changes publish even without a route event"
        );
    }

    #[tokio::test]
    async fn decentralized_client_broadcast_selects_one_preferred_route_per_peer() {
        // C4Network2ClientList::BroadcastMsgToClients selects each logical
        // client's cached Msg connection once, preferring UDP over TCP
        // (oracle-src-pinned src/C4Network2Client.cpp:497-541;
        // src/C4Network2IO.cpp:350-375).
        let mut routes = ClientRouteManager::new();
        let mut host =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Udp);
        let mut peer_tcp = add_test_route_queue(&mut routes, 2, 7, crate::NetworkProtocol::Tcp);
        let mut peer_udp = add_test_route_queue(&mut routes, 3, 7, crate::NetworkProtocol::Udp);
        let mut second_peer = add_test_route_queue(&mut routes, 4, 8, crate::NetworkProtocol::Tcp);
        let message = ControlMessage::Status(NetworkStatus::new(NETWORK_STATE_GO, 0, 9));

        assert_eq!(routes.send_to_connected_peers(message.clone()), vec![7, 8]);
        assert!(
            host.try_recv().is_err(),
            "the host receives its separate send"
        );
        assert!(
            peer_tcp.try_recv().is_err(),
            "TCP is not selected while the peer has a live UDP message route"
        );
        assert!(matches!(
            peer_udp.try_recv(),
            Ok(ClientRouteCommand::Message(observed)) if observed == message
        ));
        assert!(matches!(
            second_peer.try_recv(),
            Ok(ClientRouteCommand::Message(observed)) if observed == message
        ));
    }

    #[tokio::test]
    async fn unencodable_advanced_ready_tick_is_a_fatal_host_event() {
        // Native PackCompleteCtrl always creates, queues, and publishes the
        // authoritative complete tick. Once Rust's coordinator has advanced,
        // failure to do the same cannot be treated as a peer-local warning
        // because no later lockstep packet can fill that hole
        // (src/C4GameControlNetwork.cpp:741-777).
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let (event_tx, mut event_rx) = mpsc::channel(4);
        state.event_tx = event_tx;
        let unencodable_tick = u32::try_from(i32::MAX).test_value() + 1;
        state.coordinator =
            ControlCoordinator::with_start_tick(state.config.backlog_limit, unencodable_tick);
        state
            .coordinator
            .register_client(HOST_CLIENT_ID)
            .test_value();
        let payload = legacy_packet(HOST_CLIENT_ID, 0, 0x71).payload().to_vec();

        ingest_control(
            ControlPacket::builder(HOST_CLIENT_ID, unencodable_tick).payload(payload),
            ControlIngress::Local,
            &mut state,
        )
        .await;

        match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
            Some(HostEvent::FatalError { error }) => {
                assert!(error.contains("failed to aggregate ready tick"));
            }
            other => panic!("unpublishable authoritative tick was not fatal: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejected_local_coordinator_input_is_a_fatal_host_event() {
        // C++ adds the host's own contribution to the same authoritative
        // client list PackCompleteCtrl walks. If Rust loses that registration,
        // the host can no longer publish the next complete control and must
        // stop instead of presenting a peer-local transport warning
        // (src/C4GameControlNetwork.cpp:644-727,741-777).
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let (event_tx, mut event_rx) = mpsc::channel(4);
        state.event_tx = event_tx;
        state.coordinator = ControlCoordinator::new(state.config.backlog_limit);

        ingest_control(
            legacy_packet(HOST_CLIENT_ID, 0, 0x72),
            ControlIngress::Local,
            &mut state,
        )
        .await;

        match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
            Some(HostEvent::FatalError { error }) => {
                assert!(error.contains("not registered"));
            }
            other => panic!("rejected authoritative host control was not fatal: {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_provisional_route_keeps_a_concurrently_accepted_route() {
        // OnConnectFail calls OnClientDisconnect only when the logical client
        // has no surviving connection. A second route that completed while
        // the original admission was pending therefore keeps the client and
        // receives only the failed-route diagnostic
        // (src/C4Network2.cpp:1761-1771).
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        state.event_tx = event_tx;
        state.pending_route_clients.insert(99, client_id);
        state
            .pending_admissions
            .insert(99, i32::try_from(client_id).test_value());

        handle_admission_failed(99, "original route timed out".to_string(), &mut state).await;

        assert!(state.clients.contains_key(&client_id));
        assert!(!state.removing_clients.contains(&client_id));
        assert!(
            state.pending_sync.is_empty(),
            "a surviving route must not receive a synchronized ClientRemove"
        );
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(HostEvent::RecoverableRouteDiagnostic {
                client_id: Some(source),
                error,
            })) if source == client_id && error == "original route timed out"
        ));
        assert!(
            event_rx.try_recv().is_err(),
            "surviving client received a logical disconnect event"
        );
    }

    #[tokio::test]
    async fn unassociated_admission_failure_is_logged_below_the_lobby() {
        // OnConnectFail looks up the connection's client ID. A socket that
        // never reached PID_Conn has none, so C++ logs at info rather than the
        // warn its GUI sink shows — it still records the failure
        // (src/C4Network2.cpp:1745-1747; src/C4Network2IO.cpp:533-566;
        // src/C4Log.cpp:307).
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        state.event_tx = event_tx;

        handle_admission_failed(
            99,
            "connection admission from [2603:6011:c800:6644:e446:ea8c:39da:237f]:11113 failed: \
             connection transport failed: I/O error: unexpected end of file"
                .to_string(),
            &mut state,
        )
        .await;

        assert!(state.clients.contains_key(&7));
        match event_rx.try_recv() {
            Ok(HostEvent::UnassociatedConnectionFailed { error }) => {
                assert!(error.contains("unexpected end of file"));
            }
            other => panic!("an unassociated peer close must still be recorded: {other:?}"),
        }
    }

    #[tokio::test]
    async fn refused_admission_is_logged_below_the_lobby() {
        // HandleConn logs every refusal — wrong engine, wrong password, a
        // duplicate core — as `connection by X blocked: <reason>` at info. The
        // socket never named a client, so the reason would otherwise be the
        // only record a host has of why a join failed
        // (src/C4Network2.cpp:1292-1330,1361).
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        state.event_tx = event_tx;

        handle_admission_failed(
            99,
            "connection admission from 127.0.0.1:11113 failed: wrong password".to_string(),
            &mut state,
        )
        .await;

        match event_rx.try_recv() {
            Ok(HostEvent::UnassociatedConnectionFailed { error }) => {
                assert!(error.contains("wrong password"));
            }
            other => panic!("a refused admission must still be recorded: {other:?}"),
        }
    }

    #[tokio::test]
    async fn host_outbound_queue_is_lossless_beyond_the_udp_retransmit_window() {
        const PACKET_COUNT: i32 = 10_001;

        // C++ queues app packets without blocking the global scheduler: TCP
        // appends to OBuf, while UDP separately retains a 10,000-packet
        // retransmit window (oracle-src-pinned
        // src/C4NetIO.cpp:1345-1357,1916,2788-2808).
        let buffered_client_id = 7;
        let live_client_id = 8;
        let (buffered, mut buffered_receiver) = HostOutboundSender::channel();
        let (live, mut live_receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(buffered_client_id, buffered.clone());
        let delivered = ControlMessage::Status(NetworkStatus::new(NETWORK_STATE_GO, 0, 9));
        let live_core = compatibility_test_core(live_client_id as i32, b"Live");
        state.clients.insert(
            live_client_id,
            ClientConnection {
                outbound: live.clone(),
                core: live_core.clone(),
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                join_data_sent: true,
                join_data_needed_emitted: false,
            },
        );
        state.client_cores.insert(live_client_id as i32, live_core);
        state.accepted_routes.insert(
            3,
            AcceptedConnectionRoute {
                client_id: live_client_id,
                remote_connection_id: 4,
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: RoutePingLag::default(),
                outbound: live,
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: false,
            },
        );

        for target_tick in 0..PACKET_COUNT {
            assert!(
                send_host_message(
                    &state,
                    buffered_client_id,
                    ConnectionTrafficClass::Message,
                    ControlMessage::Status(NetworkStatus {
                        state: NETWORK_STATE_LOBBY,
                        control_mode: 1,
                        target_tick,
                    }),
                )
                .await,
                "logical packet {target_tick} was dropped"
            );
        }
        assert!(!buffered.is_closed());
        assert!(timeout(
            Duration::from_millis(20),
            send_host_message(
                &state,
                live_client_id,
                ConnectionTrafficClass::Message,
                delivered.clone(),
            ),
        )
        .await
        .expect("one route's backlog must not block another route"));
        assert!(matches!(
            live_receiver.recv().await,
            Some(HostOutboundMessage::Message(message)) if message == delivered
        ));
        for target_tick in 0..PACKET_COUNT {
            assert!(matches!(
                buffered_receiver.try_recv(),
                Ok(HostOutboundMessage::Message(ControlMessage::Status(status)))
                    if status.target_tick == target_tick
            ));
        }
    }

    #[test]
    fn client_resource_queue_is_lossless_beyond_the_udp_retransmit_window() {
        const PACKET_COUNT: i32 = 10_001;

        // C4Network2Client::SendMsg delegates to the protocol's nonblocking
        // buffered Send. Resource fanout therefore does not impose an
        // app-layer 64/10,000-message loss cliff (oracle-src-pinned
        // src/C4Network2Client.cpp:121-124;
        // src/C4NetIO.cpp:1345-1357,1916,2788-2808).
        let mut routes = ClientRouteManager::new();
        let mut buffered_receiver =
            add_test_route_queue(&mut routes, 1, 7, crate::NetworkProtocol::Tcp);
        let mut live_receiver =
            add_test_route_queue(&mut routes, 2, 8, crate::NetworkProtocol::Tcp);
        for resource_id in 0..PACKET_COUNT {
            routes
                .try_send_to(
                    7,
                    ControlMessage::Resource(ResourcePacket::Discover(
                        crate::ResourceDiscoverPacket {
                            resource_ids: vec![resource_id],
                        },
                    )),
                )
                .unwrap_or_else(|error| {
                    panic!("resource packet {resource_id} was dropped: {error}")
                });
        }
        assert!(!routes.routes[&1].outbound.is_closed());
        let delivered = ControlMessage::Status(NetworkStatus::new(NETWORK_STATE_GO, 0, 9));
        routes.try_send_to(8, delivered.clone()).test_value();
        assert!(matches!(
            live_receiver.try_recv(),
            Ok(ClientRouteCommand::Message(message)) if message == delivered
        ));
        for resource_id in 0..PACKET_COUNT {
            assert!(matches!(
                buffered_receiver.try_recv(),
                Ok(ClientRouteCommand::Message(ControlMessage::Resource(
                    ResourcePacket::Discover(packet)
                ))) if packet.resource_ids == [resource_id]
            ));
        }
    }

    #[test]
    fn client_runtime_states_use_backlog_and_central_nonhost_special_case() {
        let mut routes = ClientRouteManager::new();
        let _host_rx =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        let _peer_rx = add_test_route_queue(&mut routes, 2, 7, crate::NetworkProtocol::Udp);
        let tick = 9;
        let mut backlog = ControlBacklog::new(16);
        backlog.record_packet(&legacy_packet(7, tick, 0x21));
        let mut performance = ClientPerformanceStats::new(16);
        let reached_at = tokio::time::Instant::now();
        performance.record_cadence(tick, reached_at);
        performance.record_arrival(7, tick, reached_at + Duration::from_millis(200));
        performance.mark_consumed(tick, reached_at + Duration::from_millis(300), [7]);

        assert_eq!(
            routes.runtime_client_states(0, tick, [HOST_CLIENT_ID, 7, 9], &backlog, &performance),
            vec![
                RuntimeNetworkClientState {
                    client_id: HOST_CLIENT_ID,
                    status: RemoteBarrierState::Ready,
                    control_ready: false,
                    wait_ms: 0,
                },
                RuntimeNetworkClientState {
                    client_id: 7,
                    status: RemoteBarrierState::Ready,
                    control_ready: true,
                    wait_ms: 2,
                },
                RuntimeNetworkClientState {
                    client_id: 9,
                    status: RemoteBarrierState::Ready,
                    control_ready: false,
                    wait_ms: 0,
                },
            ]
        );
        assert_eq!(
            routes.runtime_client_states(1, tick, [HOST_CLIENT_ID, 7, 9], &backlog, &performance),
            vec![
                RuntimeNetworkClientState {
                    client_id: HOST_CLIENT_ID,
                    status: RemoteBarrierState::Ready,
                    control_ready: true,
                    wait_ms: 1,
                },
                RuntimeNetworkClientState {
                    client_id: 7,
                    status: RemoteBarrierState::Ready,
                    control_ready: true,
                    wait_ms: 1,
                },
                RuntimeNetworkClientState {
                    client_id: 9,
                    status: RemoteBarrierState::Ready,
                    control_ready: true,
                    wait_ms: 1,
                },
            ]
        );
    }

    #[tokio::test]
    async fn client_control_recovery_routes_central_to_host_and_decentral_to_all_peers() {
        fn take_message(
            receiver: &mut mpsc::UnboundedReceiver<ClientRouteCommand>,
        ) -> ControlMessage {
            match receiver.try_recv().test_value() {
                ClientRouteCommand::Message(message) => message,
                ClientRouteCommand::Flush(_) => panic!("unexpected recovery flush"),
            }
        }

        let mut routes = ClientRouteManager::new();
        let mut host =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        let mut peer = add_test_route_queue(&mut routes, 2, 7, crate::NetworkProtocol::Tcp);

        send_client_recovery_request(&mut routes, 1, 17)
            .await
            .test_value();
        assert_eq!(
            take_message(&mut host),
            ControlMessage::Request { from_tick: 17 }
        );
        assert!(peer.try_recv().is_err());

        send_client_recovery_request(&mut routes, 0, 18)
            .await
            .test_value();
        assert_eq!(
            take_message(&mut host),
            ControlMessage::Request { from_tick: 18 }
        );
        assert_eq!(
            take_message(&mut peer),
            ControlMessage::Request { from_tick: 18 }
        );
    }

    fn take_queued_resource(
        receiver: &mut mpsc::UnboundedReceiver<ClientRouteCommand>,
    ) -> ResourcePacket {
        match receiver.try_recv().test_value() {
            ClientRouteCommand::Message(ControlMessage::Resource(packet)) => packet,
            ClientRouteCommand::Message(other) => {
                panic!("unexpected queued resource message: {other:?}")
            }
            ClientRouteCommand::Flush(_) => panic!("unexpected queued resource flush"),
        }
    }

    #[tokio::test]
    async fn client_mesh_warms_puncher_and_sends_family_server_request_before_host_join() {
        let mut puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let puncher_address = puncher.local_addr();
        let config = ClientConfig::new("Client", ParticipantKind::Player)
            .with_mesh_udp_bind_address(SocketAddr::from(([127, 0, 0, 1], 0)))
            .with_mesh_punchers([ClientMeshPuncherConfig {
                address: puncher_address,
                game_id: 0x1122_3344,
            }]);

        let mut prepared = prepare_client_mesh(&config, false).await.test_value();
        let local_address = prepared.udp_hub.as_ref().test_value().local_addr();
        let mut puncher_stream = timeout(EVENT_WAIT, puncher.accept())
            .await
            .unwrap()
            .test_value();
        let mut header = [0_u8; 5];
        timeout(EVENT_WAIT, puncher_stream.read_exact(&mut header))
            .await
            .unwrap()
            .test_value();
        assert_eq!(header[0], 0xff);
        let payload_len = u32::from_ne_bytes(header[1..].try_into().test_value()) as usize;
        let mut payload = vec![0_u8; payload_len];
        puncher_stream.read_exact(&mut payload).await.test_value();
        assert_eq!(
            payload,
            crate::encode_netpuncher_packet(&crate::NetpuncherPacket::ServerRequest {
                id: 0x1122_3344,
            })
        );
        assert_eq!(
            prepared
                .puncher_init
                .lock()
                .expect("puncher initialization lock")
                .observations
                .as_slice(),
            [local_address]
        );
        assert_eq!(
            timeout(EVENT_WAIT, prepared.puncher_events.as_mut().unwrap().recv())
                .await
                .unwrap(),
            Some(crate::NetpuncherIoEvent::Connected {
                family: crate::NetpuncherAddressFamily::Ipv4,
                puncher_address,
                observed_address: local_address,
            })
        );

        let mut game_peer =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let handle = prepared.udp_hub.as_ref().test_value().handle();
        let (game_stream, incoming_game_stream) =
            tokio::join!(handle.connect(game_peer.local_addr()), game_peer.accept());
        let game_stream = game_stream.test_value();
        let incoming_game_stream = incoming_game_stream.test_value();
        assert_eq!(incoming_game_stream.peer_addr(), local_address);
        drop(game_stream);
        drop(incoming_game_stream);

        prepared.puncher_init.lock().test_value().initializing = false;
        handle.close_puncher(puncher_address).await.test_value();
        let mut closed = [0_u8; 1];
        assert_eq!(
            timeout(EVENT_WAIT, puncher_stream.read(&mut closed))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        drop(puncher_stream);
        handle
            .init_puncher(puncher_address, NetpuncherRole::Client)
            .await
            .test_value();
        let mut reconnected = timeout(EVENT_WAIT, puncher.accept())
            .await
            .unwrap()
            .test_value();
        assert!(timeout(
            Duration::from_millis(100),
            read_udp_session_payload(&mut reconnected),
        )
        .await
        .is_err());
        drop(reconnected);

        if let Some(hub) = prepared.udp_hub.take() {
            hub.shutdown().await.test_value();
        }
        game_peer.shutdown().await.test_value();
        puncher.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_unwraps_a_selected_forwarded_control() {
        // PID_Fwd dispatches its complete nested packet exactly once when the
        // local client matches the positive list (pristine C++
        // src/C4Network2IO.cpp:1026-1033).
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.change_mode(0, 0).test_value();
        resource_state.control.register(0).test_value();
        resource_state.control.register(1).test_value();
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop_with_state(512, 2, 2, BTreeMap::new(), resource_state);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let local = legacy_packet(1, 0, 0x22);
        command_tx
            .send(ClientCommand::SubmitControl(local))
            .await
            .test_value();
        assert!(matches!(
            host_transport.read_message().await.unwrap(),
            ControlMessage::ForwardRequest(_)
        ));

        let host = legacy_packet(0, 0, 0x11);
        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: crate::transport::encode_complete_control_packet(&host).unwrap(),
            }))
            .await
            .test_value();

        let ready = match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
            Some(ClientEvent::Ready { packet }) => packet,
            other => panic!("expected forwarded aggregate, got {other:?}"),
        };
        assert_eq!(control_commands(&ready), vec![0x11, 0x22]);

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_selected_forwarded_league_results() {
        // PID_Fwd recursively enters HandlePacket. League results selected for
        // this client retain their typed host-only route and do not close the
        // connection (src/C4Network2IO.cpp:1037-1045;
        // src/C4Network2Players.cpp:392-419).
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop_with_state(512, 1, 1, BTreeMap::new(), resource_state);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let league_results = vec![0x17, 0x01, b'O', b'K', 0x00, 0x00];

        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: league_results,
            }))
            .await
            .test_value();
        let event = await_test(event_rx.recv()).await;
        let ClientEvent::LeagueRoundResults { packet } = event else {
            panic!("expected typed forwarded league results, got {event:?}");
        };
        assert_eq!(
            packet,
            crate::LeagueRoundResultsPacket {
                success: true,
                result_string: c4(b"OK"),
                players: Vec::new(),
            }
        );

        let ping = crate::PingPacket {
            sent_at: 29,
            packet_counter: 0,
        };
        host_transport
            .send_message(ControlMessage::Ping(ping))
            .await
            .test_value();
        assert_eq!(
            host_transport.read_message().await.unwrap(),
            ControlMessage::Pong(ping)
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ignores_unselected_malformed_forward_and_bounds_recursion() {
        // DoFwdTo is evaluated before the nested packet is unpacked. A selected
        // recursive PID_Fwd is bounded instead of reproducing C++'s unbounded
        // recursive HandlePacket call (pristine C++
        // src/C4Network2IO.cpp:1026-1033,1626-1636).
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let (host_stream, _command_tx, mut event_rx, _shutdown_tx, client_handle) =
            start_test_client_loop_with_state(512, 2, 2, BTreeMap::new(), resource_state);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![2],
                nested_packet: vec![0x40],
            }))
            .await
            .test_value();
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 0, 0);
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::Status(received))) if received == status
        ));

        let mut recursive = vec![crate::PID_FORWARD];
        recursive.extend(
            crate::encode_forward_packet_payload(&crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: vec![0xff],
            })
            .test_value(),
        );
        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: recursive,
            }))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::Disconnected { reason: Some(reason) }))
                if reason == "recursive forwarding packet is not accepted"
        ));
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decentral_client_sends_cpp_forward_request_for_local_control() {
        // BroadcastMsgToClients excludes the directly connected host, records
        // no other direct peers in the negative list, and sends the complete
        // PID_Control inside PID_FwdReq (pristine C++
        // src/C4Network2Client.cpp:515-541; src/C4GameControlNetwork.cpp:156-174).
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.change_mode(0, 0).test_value();
        resource_state.control.register(0).test_value();
        resource_state.control.register(1).test_value();
        let (mut host_stream, command_tx, _event_rx, shutdown_tx, client_handle) =
            start_test_client_loop_with_state(128, 1, 1, BTreeMap::new(), resource_state);

        command_tx
            .send(ClientCommand::SubmitControl(
                ControlPacket::builder(1, 0).payload(vec![0xff]),
            ))
            .await
            .test_value();
        let mut bytes = vec![0; 64];
        let count = await_test(host_stream.read(&mut bytes)).await;
        bytes.truncate(count);
        assert_eq!(
            bytes,
            [0xff, 0x08, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x04, 0x40, 0x01, 0x00, 0xff,]
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_direct_and_private_packets_send_only_forward_request() {
        // CDT_Direct and CDT_Private exclude the host from the direct leg.
        // With no peer mesh, only the host FwdReq remains and its negative
        // list is empty (pristine C++ src/C4Network2Client.cpp:515-541;
        // src/C4GameControlNetwork.cpp:224-240).
        let (host_stream, command_tx, _event_rx, shutdown_tx, client_handle) =
            start_test_client_loop(512, 2, 1);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        for delivery in [ControlDelivery::Direct, ControlDelivery::Private] {
            command_tx
                .send(ClientCommand::SubmitPacket {
                    delivery,
                    data: vec![0xaa, 0xbb],
                })
                .await
                .test_value();
            assert_eq!(
                timeout(EVENT_WAIT, host_transport.read_message())
                    .await
                    .expect("forward request send wait")
                    .expect("read forward request"),
                ControlMessage::ForwardRequest(crate::ForwardPacket {
                    negative_list: true,
                    clients: Vec::new(),
                    nested_packet: vec![0x42, u8::from(delivery), 0xaa, 0xbb],
                })
            );
        }
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err(),
            "direct/private submission emitted an extra raw packet"
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ready_check_sends_raw_then_host_excluding_forward_request() {
        // ReadyCheck uses includeHost=true: the host receives the raw packet
        // first and is then excluded from the fallback FwdReq (pristine C++
        // src/C4Network2Client.cpp:515-541; src/C4GameLobby.cpp:329-343).
        let mut resource_state = ClientResourceState::empty();
        resource_state.host_peer_id = 7;
        let (host_stream, command_tx, _event_rx, shutdown_tx, client_handle) =
            start_test_client_loop_with_state(512, 1, 1, BTreeMap::new(), resource_state);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let packet = ReadyCheckPacket::new(12, crate::ReadyCheckData::Ready);

        command_tx
            .send(ClientCommand::SubmitReadyCheck(packet))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .expect("raw ready-check send wait")
                .expect("read raw ready-check"),
            ControlMessage::ReadyCheck(packet)
        );
        let mut nested_packet = vec![0x21];
        nested_packet.extend_from_slice(&packet.client_id.to_ne_bytes());
        nested_packet.extend_from_slice(&i32::from(packet.data).to_ne_bytes());
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .expect("ready-check forward request send wait")
                .expect("read ready-check forward request"),
            ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: vec![7],
                nested_packet,
            })
        );
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err(),
            "ready-check submission emitted an extra packet"
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rust_client_direct_packet_reaches_rust_host_and_observer_once() {
        // The client now reaches the host only through FwdReq for direct
        // packets. Preserve Rust-host interoperability while the generic
        // opaque forwarding router remains separate work.
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let source = connect_test_player(address, "Source").await;
        let mut observer_a = connect_test_player(address, "Observer A").await;
        let mut observer_a_events = observer_a.take_event_receiver();
        let mut observer_b = connect_test_player(address, "Observer B").await;
        let mut observer_b_events = observer_b.take_event_receiver();
        let source_id = source.client_id();
        let data = encode_control_entry_payload(&EngineControlPacket::PlayerControl(
            PlayerControlData::new(
                i32::try_from(source_id).unwrap(),
                0x22,
                0x33,
                i32::try_from(source_id).unwrap(),
            ),
        ))
        .test_value();

        source
            .submit_packet(ControlDelivery::Direct, data.clone())
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if client_id == source_id && received == data => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == source_id => {
                    panic!("source forwarding failed: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before direct packet"),
            }
        }
        for (name, events) in [
            ("observer A", &mut observer_a_events),
            ("observer B", &mut observer_b_events),
        ] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                    Some(ClientEvent::Direct {
                        delivery: ControlDelivery::Direct,
                        data: received,
                    }) if received == data => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("{name} disconnected during direct forwarding: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("{name} event stream ended before direct packet"),
                }
            }
        }

        let host_duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(host_duplicate_deadline, host_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    HostEvent::Direct {
                        client_id,
                        delivery: ControlDelivery::Direct,
                        data: ref received,
                    } if client_id == source_id && *received == data
                ),
                "host executed the forwarded direct packet twice"
            );
            assert!(
                !matches!(
                    event,
                    HostEvent::TransportError {
                        client_id: Some(client_id),
                        ..
                    } if client_id == source_id
                ),
                "host rejected the direct forwarding leg"
            );
        }
        for (name, events) in [
            ("observer A", &mut observer_a_events),
            ("observer B", &mut observer_b_events),
        ] {
            let duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
            while let Ok(Some(event)) = timeout_at(duplicate_deadline, events.recv()).await {
                assert!(
                    !matches!(
                        event,
                        ClientEvent::Direct {
                            delivery: ControlDelivery::Direct,
                            data: ref received,
                        } if *received == data
                    ),
                    "{name} received the forwarded direct packet twice"
                );
            }
        }

        source.shutdown().await.test_value();
        observer_a.shutdown().await.test_value();
        observer_b.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_ignores_client_originated_league_results_and_keeps_the_connection() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut client, _) = raw_client_transport(address, b"Source").await;
        drain_raw_client(&mut client).await;

        client
            .send_message(ControlMessage::LeagueRoundResults(
                crate::LeagueRoundResultsPacket {
                    success: true,
                    result_string: c4(b"OK"),
                    players: Vec::new(),
                },
            ))
            .await
            .test_value();
        let ping = crate::PingPacket {
            sent_at: 31,
            packet_counter: 0,
        };
        client
            .send_message(ControlMessage::Ping(ping))
            .await
            .test_value();

        assert_eq!(
            timeout(EVENT_WAIT, client.read_message())
                .await
                .expect("host kept the accepted connection responsive")
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        drop(client);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_broadcasts_league_round_results_to_every_logical_client() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut alice, _) = raw_client_transport(address, b"Alice").await;
        let (mut bob, _) = raw_client_transport(address, b"Bob").await;
        drain_raw_client(&mut alice).await;
        drain_raw_client(&mut bob).await;
        let packet = crate::LeagueRoundResultsPacket {
            success: true,
            result_string: c4(b"Counted"),
            players: vec![crate::LeagueRoundResultsPlayer {
                player_info_id: 17,
                total_playing_time: 900,
                settlement_score_old: 2,
                settlement_score_new: 4,
                league_score_new: 120,
                league_score_gain: 7,
                league_rank_new: 3,
                league_rank_symbol_new: 2,
                league_progress_data: c4(b"p=2"),
                status: crate::LeagueRoundPlayerStatus::Won,
            }],
        };

        host.broadcast_league_round_results(packet.clone())
            .await
            .test_value();

        let expected = ControlMessage::LeagueRoundResults(packet);
        assert!(raw_client_received_message(&mut alice, &expected, EVENT_WAIT).await);
        assert!(raw_client_received_message(&mut bob, &expected, EVENT_WAIT).await);

        drop(alice);
        drop(bob);
        host.shutdown().await.test_value();
    }

    /// A restart notice is only meaningful from the host, and the client can
    /// only judge that by which route it arrived on. `PID_FwdReq` relays a
    /// client's opaque nested packet onto the host's own route
    /// (src/C4Network2IO.cpp:1066-1082), so the relay — not the receiver — is
    /// the only place that can tell the two apart. Left open, any admitted peer
    /// could tear down another player's round.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_cannot_forge_a_restart_notice_through_the_forward_relay() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut attacker, _attacker_id) = raw_client_transport(address, b"Mallory").await;
        let (mut victim, _victim_id) = raw_client_transport(address, b"Alice").await;
        drain_raw_client(&mut attacker).await;
        drain_raw_client(&mut victim).await;

        attacker
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::encode_host_restart_notice(30),
            }))
            .await
            .test_value();

        let forged = ControlMessage::HostRestarting { rejoin_seconds: 30 };
        assert!(
            !raw_client_received_message(&mut victim, &forged, Duration::from_millis(200)).await,
            "the host relayed a peer's restart notice as its own"
        );

        drop(attacker);
        drop(victim);
        host.shutdown().await.test_value();
    }

    /// The session-preserving notice is the more dangerous one to relay: it
    /// does not predict a disconnect, so a forged one ends a round that is
    /// still perfectly healthy and drops every victim back to a lobby. The
    /// whole `0x7x` range is refused on the relay for exactly this reason.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_cannot_forge_a_lobby_restart_through_the_forward_relay() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut attacker, _attacker_id) = raw_client_transport(address, b"Mallory").await;
        let (mut victim, _victim_id) = raw_client_transport(address, b"Alice").await;
        drain_raw_client(&mut attacker).await;
        drain_raw_client(&mut victim).await;

        attacker
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::encode_host_restart_lobby_notice(0),
            }))
            .await
            .test_value();

        assert!(
            !raw_client_received_message(
                &mut victim,
                &ControlMessage::HostRestartLobby { restart_nonce: 0 },
                Duration::from_millis(200)
            )
            .await,
            "the host relayed a peer's lobby restart as its own"
        );

        drop(attacker);
        drop(victim);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restarting_a_round_reuses_routes_and_sends_fresh_join_data() {
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(address, "Alice").await;
        let client_id = client.client_id();
        let before_routes = host
            .runtime_connections()
            .await
            .test_value()
            .into_iter()
            .map(|route| {
                (
                    route.connection_id,
                    route.client_id,
                    route.protocol,
                    route.peer_address,
                )
            })
            .collect::<Vec<_>>();
        let mut events = client.take_event_receiver();
        let mut fresh_config = HostConfig::default();
        let mut fresh_snapshot =
            synthetic_join_snapshot(fresh_config.local_core.clone(), fresh_config.max_players);
        fresh_snapshot.parameters.title = c4(b"Fresh round");
        fresh_config.initial_join_snapshot = Some(fresh_snapshot.clone());

        host.restart_round_in_lobby(fresh_config).await.test_value();

        let mut saw_notice = false;
        let restarted = loop {
            match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                Some(ClientEvent::HostRestartLobby) => saw_notice = true,
                Some(ClientEvent::JoinData { join_data }) => break *join_data,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client disconnected while restarting the round: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before fresh JoinData"),
            }
        };
        assert!(saw_notice, "fresh JoinData overtook the restart marker");
        assert_eq!(restarted.client_id, client_id as i32);
        assert_eq!(restarted.parameters.title, c4(b"Fresh round"));
        assert_eq!(restarted.dynamic, fresh_snapshot.dynamic);
        client.acknowledge_round_restart().await.test_value();
        let after_routes = host
            .runtime_connections()
            .await
            .test_value()
            .into_iter()
            .map(|route| {
                (
                    route.connection_id,
                    route.client_id,
                    route.protocol,
                    route.peer_address,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(after_routes, before_routes);

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x55))
            .await
            .test_value();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0x55]);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_rejects_a_second_round_restart_until_the_first_is_acknowledged() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let mut client = connect_test_player(address, "Alice").await;
        let mut events = client.take_event_receiver();
        let mut first = HostConfig::default();
        first
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .title = c4(b"First fresh round");

        host.restart_round_in_lobby(first).await.test_value();

        let first_join_data = loop {
            match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                Some(ClientEvent::JoinData { join_data }) => break *join_data,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client disconnected during the first restart: {reason:?}")
                }
                Some(_) => {}
                None => panic!("client event stream ended during the first restart"),
            }
        };
        assert_eq!(first_join_data.parameters.title, c4(b"First fresh round"));

        let mut premature_second = HostConfig::default();
        premature_second
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .title = c4(b"Premature second round");
        let error = host
            .restart_round_in_lobby(premature_second)
            .await
            .expect_err("the first restart fence must survive until its client ACK");

        assert!(
            error.to_string().contains("acknowledgement"),
            "unexpected second-restart rejection: {error}"
        );
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, events.recv()).await {
            assert!(
                !matches!(
                    event,
                    ClientEvent::HostRestartLobby | ClientEvent::JoinData { .. }
                ),
                "rejected second restart published another client fence: {event:?}"
            );
        }

        client.acknowledge_round_restart().await.test_value();
        timeout(EVENT_WAIT, async {
            loop {
                let mut second = HostConfig::default();
                second
                    .initial_join_snapshot
                    .as_mut()
                    .test_value()
                    .parameters
                    .title = c4(b"Second fresh round");
                match host.restart_round_in_lobby(second).await {
                    Ok(()) => break,
                    Err(error) if error.to_string().contains("acknowledgement") => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("second restart failed after ACK: {error}"),
                }
            }
        })
        .await
        .expect("host did not reduce the first restart ACK");

        let second_join_data = loop {
            match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                Some(ClientEvent::JoinData { join_data }) => break *join_data,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client disconnected during the second restart: {reason:?}")
                }
                Some(_) => {}
                None => panic!("client event stream ended during the second restart"),
            }
        };
        assert_eq!(second_join_data.parameters.title, c4(b"Second fresh round"));
        client.acknowledge_round_restart().await.test_value();

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_route_round_restart_reconnects_auxiliary_route_without_rejoining() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let tcp_address = listener.local_addr().test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let udp_address = host.udp_local_addr().test_value();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_dual_client(
            tcp_address,
            udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();
        let client_id = client.client_id();
        let route_wait = Duration::from_millis(crate::PING_TIMEOUT_MS as u64);
        timeout(route_wait, async {
            while host.accepted_routes().await.len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        wait_for_client_host_protocols(&client, route_wait).await;
        while host_events.try_recv().is_ok() {}

        let mut events = client.take_event_receiver();
        let mut fresh = HostConfig::default();
        fresh
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .title = c4(b"Dual-route fresh round");
        host.restart_round_in_lobby(fresh).await.test_value();

        let join_data = loop {
            match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                Some(ClientEvent::JoinData { join_data }) => break *join_data,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("dual-route client disconnected during restart: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("dual-route client events ended during restart"),
            }
        };
        assert_eq!(join_data.client_id, client_id as i32);
        assert_eq!(join_data.parameters.title, c4(b"Dual-route fresh round"));
        client.acknowledge_round_restart().await.test_value();

        timeout(route_wait, async {
            while host.accepted_routes().await.len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        wait_for_client_host_protocols(&client, route_wait).await;
        while let Ok(event) = host_events.try_recv() {
            assert!(
                !matches!(event, HostEvent::ClientJoined { client_id: joined, .. } if joined == client_id),
                "auxiliary route rejoined the retained logical client"
            );
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stock_peer_rejects_atomic_restart_without_mutating_the_live_session() {
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut modern = connect_test_player(address, "Alice").await;
        let mut modern_events = modern.take_event_receiver();
        let (mut stock, _) = raw_client_transport(address, b"Legacy Bob").await;
        drain_raw_client(&mut stock).await;
        while host_events.try_recv().is_ok() {}
        while modern_events.try_recv().is_ok() {}
        let before = host.runtime_connections().await.test_value();
        let mut fresh = HostConfig::default();
        fresh
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .title = c4(b"Rejected fresh round");

        let error = host
            .restart_round_in_lobby(fresh)
            .await
            .expect_err("stock peer cannot install the port-only restart extension");

        assert!(error
            .to_string()
            .contains("does not support atomic round restart"));
        assert_eq!(host.runtime_connections().await.test_value(), before);
        assert!(
            !raw_client_received_message(
                &mut stock,
                &ControlMessage::HostRestartLobby { restart_nonce: 0 },
                Duration::from_millis(150),
            )
            .await
        );
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, modern_events.recv()).await {
            assert!(!matches!(event, ClientEvent::HostRestartLobby));
        }
        while let Ok(event) = host_events.try_recv() {
            assert!(!matches!(event, HostEvent::RoundRestarted));
        }

        drop(stock);
        shutdown_test_session(modern, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn solo_host_can_restart_directly_into_the_fresh_lobby() {
        let (_address, mut host) = start_test_host(HostConfig::default()).await;
        let mut events = host.take_event_receiver();
        let mut fresh = HostConfig::default();
        fresh
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .title = c4(b"Solo fresh round");

        host.restart_round_in_lobby(fresh).await.test_value();

        assert!(matches!(
            timeout(EVENT_WAIT, events.recv()).await.test_value(),
            Some(HostEvent::RoundRestarted)
        ));
        assert!(host.runtime_connections().await.test_value().is_empty());
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restart_ack_starts_transfer_of_a_replaced_dynamic_resource() {
        let directories = SessionResourceDirectories::new();
        let old_path = directories.host.join("DynFixture.c4s");
        let fresh_path = directories.host.join("DynFixture_2.c4s");
        fs::write(&old_path, b"local").test_value();
        fs::write(&fresh_path, b"local").test_value();
        let old_dynamic = network_core!(resource_type: 2,
        id: 4,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        contents_crc: 0x8bd6_88e8,
        chunk_size: 2,
        filename: c4(b"DynFixture.c4s"));
        let fresh_dynamic = network_core!(resource_type: 2,
        id: 4,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        contents_crc: 0x8bd6_88e8,
        chunk_size: 2,
        filename: c4(b"DynFixture_2.c4s"));

        let initial_defaults = HostConfig::default();
        let mut initial_snapshot = synthetic_join_snapshot(
            initial_defaults.local_core.clone(),
            initial_defaults.max_players,
        );
        initial_snapshot.dynamic = old_dynamic.clone();
        let initial_config = HostConfig {
            initial_join_snapshot: Some(initial_snapshot),
            resource_directory: Some(directories.host.clone()),
            resource_registrations: vec![crate::ResourceRegistration::from_core(
                &old_dynamic,
                true,
                false,
            )],
            resource_files: vec![HostedResourceFile {
                core: old_dynamic.clone(),
                path: old_path,
                ownership: crate::ResourceFileOwnership::Temporary,
                binary_compatible: true,
            }],
            ..initial_defaults
        };

        let (address, host) = start_test_host(initial_config).await;
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .test_value();
        loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .test_value()
                .test_value()
            {
                ClientEvent::ResourceComplete { resource_id: 4, .. } => break,
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected loading the first dynamic: {reason:?}")
                }
                _ => {}
            }
        }

        let fresh_defaults = HostConfig::default();
        let mut fresh_snapshot = synthetic_join_snapshot(
            fresh_defaults.local_core.clone(),
            fresh_defaults.max_players,
        );
        fresh_snapshot.dynamic = fresh_dynamic.clone();
        let fresh_config = HostConfig {
            initial_join_snapshot: Some(fresh_snapshot),
            resource_directory: Some(directories.host.clone()),
            resource_registrations: vec![crate::ResourceRegistration::from_core(
                &fresh_dynamic,
                true,
                false,
            )],
            resource_files: vec![HostedResourceFile {
                core: fresh_dynamic.clone(),
                path: fresh_path,
                ownership: crate::ResourceFileOwnership::Temporary,
                binary_compatible: true,
            }],
            ..fresh_defaults
        };

        host.restart_round_in_lobby(fresh_config).await.test_value();
        loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .test_value()
                .test_value()
            {
                ClientEvent::JoinData { .. } => break,
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected installing fresh JoinData: {reason:?}")
                }
                _ => {}
            }
        }
        client.acknowledge_round_restart().await.test_value();

        loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("fresh dynamic transfer stalled")
                .test_value()
            {
                ClientEvent::ResourceComplete {
                    resource_id: 4,
                    core,
                    path,
                    local: false,
                } => {
                    assert_eq!(core, fresh_dynamic);
                    assert_eq!(fs::read(path).test_value(), b"local");
                    break;
                }
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected loading the fresh dynamic: {reason:?}")
                }
                _ => {}
            }
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejected_round_restart_emits_no_client_marker_or_host_event_fence() {
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut client, _) = raw_client_transport(address, b"Alice").await;
        drain_raw_client(&mut client).await;
        while host_events.try_recv().is_ok() {}
        let before_routes = host.runtime_connections().await.test_value();
        let mut invalid = HostConfig {
            start_tick: 7,
            ..HostConfig::default()
        };
        invalid
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .dynamic_tick = 8;

        assert!(host.restart_round_in_lobby(invalid).await.is_err());

        assert!(
            !raw_client_received_message(
                &mut client,
                &ControlMessage::HostRestartLobby { restart_nonce: 0 },
                Duration::from_millis(150),
            )
            .await
        );
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, host_events.recv()).await {
            assert!(
                !matches!(event, HostEvent::RoundRestarted),
                "rejected restart emitted its host event fence"
            );
        }
        assert_eq!(host.runtime_connections().await.test_value(), before_routes);

        drop(client);
        host.shutdown().await.test_value();
    }

    /// The restart notice exists only to be read *before* the disconnect it
    /// predicts, and the host sends it while already on its way down. If the
    /// teardown could overtake it, every client would still see a bare socket
    /// close and fall back to the native dead-host path
    /// (src/C4Network2.cpp:1826-1832) — the notice has to survive the shutdown
    /// that immediately follows it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_notice_outruns_the_host_shutdown_behind_it() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut alice, _) = raw_client_transport(address, b"Alice").await;
        let (mut bob, _) = raw_client_transport(address, b"Bob").await;
        drain_raw_client(&mut alice).await;
        drain_raw_client(&mut bob).await;

        host.broadcast_host_restarting(30).await.test_value();
        host.shutdown().await.test_value();

        let expected = ControlMessage::HostRestarting { rejoin_seconds: 30 };
        assert!(raw_client_received_message(&mut alice, &expected, EVENT_WAIT).await);
        assert!(raw_client_received_message(&mut bob, &expected, EVENT_WAIT).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_shutdown_does_not_report_a_client_connection_failure() {
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (client, client_id) = raw_client_transport(address, b"Alice").await;
        while host_events.try_recv().is_ok() {}

        host.shutdown().await.test_value();

        let mut saw_left = false;
        while let Some(event) = host_events.recv().await {
            match event {
                HostEvent::ClientLeft { client_id: left } if left == client_id => {
                    saw_left = true;
                }
                HostEvent::ClientConnectionFailed { client_id: failed } if failed == client_id => {
                    panic!("orderly host shutdown emitted ClientConnectionFailed")
                }
                _ => {}
            }
        }
        assert!(saw_left, "host shutdown did not surface the ordinary leave");
        drop(client);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_does_not_rebroadcast_a_raw_client_control() {
        // PID_Control dispatches only to HandleControl, which stores the
        // contribution; only HandleFwdReq performs fallback fanout (pristine
        // C++ src/C4GameControlNetwork.cpp:517-529;
        // src/C4Network2IO.cpp:1066-1117).
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer_a, _) = raw_client_transport(address, b"Observer A").await;
        let (mut observer_b, _) = raw_client_transport(address, b"Observer B").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer_a).await;
        drain_raw_client(&mut observer_b).await;

        let packet = ControlPacket::builder(source_id, 0).payload(vec![0xff]);
        source
            .send_message(ControlMessage::Control(packet.clone()))
            .await
            .test_value();
        for observer in [&mut observer_a, &mut observer_b] {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
            let mut rebroadcast = false;
            while let Ok(Ok(message)) = timeout_at(deadline, observer.read_message()).await {
                if message == ControlMessage::Control(packet.clone()) {
                    rebroadcast = true;
                    break;
                }
            }
            assert!(!rebroadcast, "raw PID_Control was incorrectly relayed");
        }

        drop(source);
        drop(observer_a);
        drop(observer_b);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_control_request_falls_back_to_unregistered_partial_packet() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        drain_raw_client(&mut source).await;

        let partial = legacy_packet(source_id, 0, 0x22);
        source
            .send_message(ControlMessage::Control(partial.clone()))
            .await
            .test_value();
        raw_client_ping_barrier(&mut source).await;
        source
            .send_message(ControlMessage::Request { from_tick: 0 })
            .await
            .test_value();

        assert!(raw_client_received_control(&mut source, &partial, EVENT_WAIT).await);
        assert_ne!(partial.client_id(), BROADCAST_CLIENT_ID);

        drop(source);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_accepts_received_complete_controls_in_tick_order_and_retains_them() {
        // PID_Control accepts and stores a complete C4ClientIDAll frame even
        // though PackCompleteCtrl left each embedded ByClient unchanged, and
        // CheckCompleteCtrl consumes that complete frame before partials
        // (src/C4GameControlNetwork.cpp:449-490,517-529,679-719,741-777).
        let config = host_config!(initial_status: NetworkStatus::new(NETWORK_STATE_GO, 0, 0));
        let (address, mut host) = start_test_host(config).await;
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        drain_raw_client(&mut source).await;
        let source_author = i32::try_from(source_id).test_value();
        let complete = encode_control_packet(&legacy_frame(
            BROADCAST_CLIENT_ID,
            0,
            vec![
                EngineControlPacket::PlayerControl(PlayerControlData {
                    player: 1,
                    command: 2,
                    data: 3,
                    by_client: 0,
                }),
                EngineControlPacket::PlayerControl(PlayerControlData {
                    player: 4,
                    command: 5,
                    data: 6,
                    by_client: source_author,
                }),
            ],
        ))
        .test_value();
        let future_complete = |command| {
            encode_control_packet(&legacy_frame(
                BROADCAST_CLIENT_ID,
                1,
                vec![EngineControlPacket::PlayerControl(PlayerControlData {
                    player: 4,
                    command,
                    data: command,
                    by_client: source_author,
                })],
            ))
            .test_value()
        };
        let first_future = future_complete(0x21);
        let duplicate_future = future_complete(0x22);

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0x11))
            .await
            .test_value();
        source
            .send_message(ControlMessage::Control(first_future.clone()))
            .await
            .test_value();
        source
            .send_message(ControlMessage::Control(duplicate_future))
            .await
            .test_value();
        raw_client_ping_barrier(&mut source).await;
        while let Ok(event) = host_events.try_recv() {
            assert!(
                !matches!(event, HostEvent::Ready { .. }),
                "a future complete control became ready across an earlier gap"
            );
        }
        source
            .send_message(ControlMessage::Control(complete.clone()))
            .await
            .test_value();
        raw_client_ping_barrier(&mut source).await;
        source
            .send_message(ControlMessage::Request { from_tick: 0 })
            .await
            .test_value();

        assert!(raw_client_received_control(&mut source, &complete, EVENT_WAIT).await);
        assert!(raw_client_received_control(&mut source, &first_future, EVENT_WAIT).await);
        assert_eq!(
            wait_for_host_ready(&mut host_events, EVENT_WAIT).await,
            complete
        );
        assert_eq!(
            wait_for_host_ready(&mut host_events, EVENT_WAIT).await,
            first_future,
            "the first received complete must override partials and duplicates"
        );
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 2, 0x12))
            .await
            .test_value();
        assert_eq!(
            wait_for_host_ready(&mut host_events, EVENT_WAIT)
                .await
                .tick(),
            2,
            "received complete ticks must advance host coordination"
        );

        drop(source);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_routes_forward_request_without_echoing_its_origin() {
        // HandleFwdReq excludes the requester from remote targets, sends the
        // nested packet directly when at most two remote clients remain, then
        // dispatches it locally when the negative list selects the host
        // (pristine C++ src/C4Network2IO.cpp:1066-1117).
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        activate_joined_client(&host, &mut host_events, source_id).await;
        let (mut observer, _) = raw_client_transport(address, b"Observer").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer).await;

        let host_packet = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let source_packet = legacy_packet(source_id, 0, 0x22);
        host.submit_local_control(host_packet).await.test_value();
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&source_packet)
                    .unwrap(),
            }))
            .await
            .test_value();

        assert!(raw_client_received_control(&mut observer, &source_packet, EVENT_WAIT).await);
        assert!(
            !raw_client_received_control(&mut source, &source_packet, Duration::from_millis(100))
                .await,
            "forward request echoed its nested control to the origin"
        );
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(control_commands(&ready), vec![0x11, 0x22]);

        drop(source);
        drop(observer);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_routes_forwarded_direct_control_and_checks_self_dispatch_author() {
        // HandleFwdReq relays the opaque ControlPkt before its independent
        // self leg applies C4GameControlNetwork's ByClient check
        // (src/C4Network2IO.cpp:1077-1128;
        // src/C4GameControlNetwork.cpp:477-492).
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer_a, _) = raw_client_transport(address, b"Observer A").await;
        let (mut observer_b, _) = raw_client_transport(address, b"Observer B").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer_a).await;
        drain_raw_client(&mut observer_b).await;

        let direct_data = encode_control_entry_payload(&EngineControlPacket::PlayerControl(
            PlayerControlData::new(
                i32::try_from(source_id).unwrap(),
                0x22,
                0x33,
                i32::try_from(source_id).unwrap(),
            ),
        ))
        .test_value();
        let mut nested_packet = vec![0x42, u8::from(ControlDelivery::Direct)];
        nested_packet.extend_from_slice(&direct_data);
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet,
            }))
            .await
            .test_value();

        let expected_direct = ControlMessage::Packet {
            delivery: ControlDelivery::Direct,
            data: direct_data.clone(),
        };
        for observer in [&mut observer_a, &mut observer_b] {
            assert!(raw_client_received_message(observer, &expected_direct, EVENT_WAIT).await);
            assert!(
                !raw_client_received_message(observer, &expected_direct, Duration::from_millis(50))
                    .await,
                "direct ControlPkt was relayed more than once"
            );
        }
        let host_deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(host_deadline, host_events.recv())
                .await
                .test_value()
            {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data,
                }) if client_id == source_id && data == direct_data => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == source_id => panic!("valid self dispatch failed: {error}"),
                Some(_) => continue,
                None => panic!("host event stream ended before direct self dispatch"),
            }
        }

        let spoofed_data = encode_control_entry_payload(&EngineControlPacket::PlayerControl(
            PlayerControlData::new(
                i32::try_from(source_id).unwrap(),
                0x44,
                0x55,
                i32::try_from(source_id + 1).unwrap(),
            ),
        ))
        .test_value();
        let mut spoofed_nested = vec![0x42, u8::from(ControlDelivery::Direct)];
        spoofed_nested.extend_from_slice(&spoofed_data);
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: spoofed_nested,
            }))
            .await
            .test_value();

        let expected_spoofed = ControlMessage::Packet {
            delivery: ControlDelivery::Direct,
            data: spoofed_data.clone(),
        };
        for observer in [&mut observer_a, &mut observer_b] {
            assert!(raw_client_received_message(observer, &expected_spoofed, EVENT_WAIT).await);
        }
        let error_deadline = tokio::time::Instant::now() + EVENT_WAIT;
        let error = loop {
            match timeout_at(error_deadline, host_events.recv())
                .await
                .test_value()
            {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data,
                }) if client_id == source_id && data == spoofed_data => {
                    panic!("spoofed ControlPkt executed before its author error")
                }
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == source_id => break error,
                Some(_) => continue,
                None => panic!("host event stream ended before ControlPkt author error"),
            }
        };
        assert!(error.contains("claimed author"));
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, host_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    HostEvent::Direct {
                        client_id,
                        delivery: ControlDelivery::Direct,
                        ref data,
                    } if client_id == source_id && *data == spoofed_data
                ),
                "spoofed ControlPkt executed on the host"
            );
        }

        drop(source);
        drop(observer_a);
        drop(observer_b);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_relays_forwarded_ready_check_opaquely_without_self_dispatch() {
        // A ReadyCheck can be selected for remote peers while the negative
        // list excludes the host. Its trailing bytes survive the direct relay
        // (src/C4Network2IO.cpp:1077-1128; src/C4GameLobby.cpp:329-343).
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer, _) = raw_client_transport(address, b"Observer").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer).await;
        let mut observer = observer.into_inner();
        let ready = ReadyCheckPacket::new(
            i32::try_from(source_id).test_value(),
            crate::ReadyCheckData::Ready,
        );
        let mut nested_packet = vec![0x21];
        nested_packet.extend_from_slice(&ready.client_id.to_ne_bytes());
        nested_packet.extend_from_slice(&i32::from(ready.data).to_ne_bytes());
        nested_packet.extend_from_slice(&[0xde, 0xad]);

        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: vec![HOST_CLIENT_ID as i32],
                nested_packet: nested_packet.clone(),
            }))
            .await
            .test_value();
        assert!(raw_tcp_received_frame(&mut observer, &nested_packet, EVENT_WAIT).await);
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, host_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    HostEvent::TransportError {
                        client_id: Some(client_id),
                        ..
                    } if client_id == source_id
                ),
                "opaque ReadyCheck relay was reported as an error"
            );
            assert!(
                !matches!(event, HostEvent::ReadyCheck { packet } if packet == ready),
                "host was excluded from the forwarding list"
            );
        }

        drop(source);
        drop(observer);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_ignores_self_forwarded_client_league_results() {
        // HandleFwdReq relays first and then recursively handles the self leg.
        // A host recognizes league results but silently rejects them when they
        // originated from an ordinary client, without closing that client's
        // connection (src/C4Network2IO.cpp:1077-1129;
        // src/C4Network2Players.cpp:392-419).
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        drain_raw_client(&mut source).await;
        let league_results = vec![0x17, 0x01, b'O', b'K', 0x00, 0x00];

        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: league_results,
            }))
            .await
            .test_value();
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, host_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    HostEvent::UnhandledPacket {
                        client_id: Some(client_id),
                        packet_type: 0x17,
                    } if client_id == source_id
                ),
                "typed league results were reported as unhandled"
            );
            assert!(
                !matches!(
                    event,
                    HostEvent::TransportError {
                        client_id: Some(client_id),
                        ..
                    } if client_id == source_id
                ),
                "host rejected league results by failing the connection"
            );
        }

        let ping = crate::PingPacket {
            sent_at: 31,
            packet_counter: 0,
        };
        source
            .send_message(ControlMessage::Ping(ping))
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, source.read_message())
                .await
                .test_value()
            {
                Ok(ControlMessage::Pong(received)) if received == ping => break,
                Ok(_) => continue,
                Err(error) => panic!("connection closed after league-results forwarding: {error}"),
            }
        }

        drop(source);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_closes_only_the_source_route_for_a_malformed_nested_forward() {
        // Recursive forwarding sends selected nested bytes through the normal
        // packet unpacker. A compiler failure closes only pConn in release; the
        // network scheduler and unrelated routes remain live
        // (src/C4Network2IO.cpp:822-835,1041-1055,1088-1140).
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut witness, witness_id) = raw_client_transport(address, b"Witness").await;

        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: false,
                clients: vec![i32::try_from(HOST_CLIENT_ID).unwrap()],
                nested_packet: vec![0x40],
            }))
            .await
            .test_value();
        assert!(wait_for_host_error(&mut host_events, source_id)
            .await
            .contains("invalid forwarded packet"));

        let mut failed = false;
        let mut left = false;
        timeout(EVENT_WAIT, async {
            while !failed || !left {
                match host_events.recv().await {
                    Some(HostEvent::ClientConnectionFailed { client_id })
                        if client_id == source_id =>
                    {
                        failed = true;
                    }
                    Some(HostEvent::ClientLeft { client_id }) if client_id == source_id => {
                        left = true;
                    }
                    Some(HostEvent::ClientConnectionFailed { client_id })
                        if client_id == witness_id =>
                    {
                        panic!("malformed source packet closed the witness route")
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended before source-route cleanup"),
                }
            }
        })
        .await
        .test_value();

        let routes = host.accepted_routes().await;
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].1, witness_id);
        timeout(EVENT_WAIT, async {
            loop {
                if source.read_message().await.is_err() {
                    break;
                }
            }
        })
        .await
        .test_value();
        raw_client_ping_barrier(&mut witness).await;

        drop(source);
        drop(witness);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_uses_cpp_forward_wrapper_for_more_than_two_remote_targets() {
        // HandleFwdReq switches from direct nested sends to one positive-list
        // PID_Fwd broadcast when more than two remote client IDs are selected
        // (pristine C++ src/C4Network2IO.cpp:1083-1112).
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer_a, observer_a_id) = raw_client_transport(address, b"A").await;
        let (mut observer_b, observer_b_id) = raw_client_transport(address, b"B").await;
        let (mut observer_c, observer_c_id) = raw_client_transport(address, b"C").await;
        for transport in [
            &mut source,
            &mut observer_a,
            &mut observer_b,
            &mut observer_c,
        ] {
            drain_raw_client(transport).await;
        }

        let control = ControlPacket::builder(source_id, 0).payload(vec![0xff]);
        let nested_packet = crate::transport::encode_complete_control_packet(&control).test_value();
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: nested_packet.clone(),
            }))
            .await
            .test_value();
        let expected = crate::ForwardPacket {
            negative_list: false,
            clients: vec![observer_c_id, observer_b_id, observer_a_id]
                .into_iter()
                .map(|client_id| i32::try_from(client_id).test_value())
                .collect(),
            nested_packet,
        };
        for transport in [&mut observer_a, &mut observer_b, &mut observer_c] {
            assert!(raw_client_received_forward(transport, &expected, EVENT_WAIT).await);
            assert!(
                !raw_client_received_control(transport, &control, Duration::from_millis(50)).await,
                "more-than-two target routing also sent a raw nested packet"
            );
        }
        assert!(
            !raw_client_received_forward(&mut source, &expected, Duration::from_millis(100)).await,
            "wrapper broadcast echoed to its origin"
        );

        drop(source);
        drop(observer_a);
        drop(observer_b);
        drop(observer_c);
        host.shutdown().await.test_value();
    }

    #[tokio::test]
    async fn host_join_gate_returns_only_after_the_live_state_applies() {
        // C4Network2::AllowJoin mutates fAllowJoin before returning; callers
        // enter DoLobby only after that synchronous transition
        // (src/C4Network2.cpp:835-843; src/C4Game.cpp:3874-3880).
        let (handle, mut commands) = command_test_host_handle();
        let setter = tokio::spawn(async move { handle.set_join_allowed(true).await });

        let HostCommand::SetJoinAllowed {
            allowed,
            completion,
        } = commands.recv().await.test_value()
        else {
            panic!("expected gate command");
        };
        assert!(allowed);
        assert!(!setter.is_finished(), "host state has not applied the gate");
        completion.send(()).test_value();
        setter.await.expect("setter task").test_value();
    }

    #[tokio::test]
    async fn host_begin_go_carries_status_and_admission_in_one_acknowledged_command() {
        let (handle, mut commands) = command_test_host_handle();
        let status = NetworkStatus::new(NETWORK_STATE_GO, 2, 41);
        let starter = tokio::spawn(async move { handle.begin_go(status, false).await });

        let HostCommand::BeginGo {
            status: requested_status,
            join_allowed,
            completion,
        } = commands.recv().await.test_value()
        else {
            panic!("expected atomic Go command");
        };
        assert_eq!(requested_status, status);
        assert!(!join_allowed);
        assert!(
            !starter.is_finished(),
            "caller must wait until both host states have been applied"
        );
        completion.send(()).test_value();
        starter.await.expect("starter task").test_value();
    }

    #[tokio::test]
    async fn host_begin_go_reports_a_dropped_apply_acknowledgement() {
        let (handle, mut commands) = command_test_host_handle();
        let status = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);
        let starter = tokio::spawn(async move { handle.begin_go(status, true).await });

        let HostCommand::BeginGo { completion, .. } = commands.recv().await.test_value() else {
            panic!("expected atomic Go command");
        };
        drop(completion);
        assert!(matches!(
            starter.await.expect("starter task"),
            Err(HostError::HostLoopGone)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn host_runtime_wait_uses_local_arrival_after_consumed_tick() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let reached_at = tokio::time::Instant::now();
        host.control_tick_reached(0, 1, DEFAULT_CONTROL_TARGET_FPS, reached_at)
            .await
            .test_value();
        tokio::time::advance(Duration::from_millis(200)).await;
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .test_value();
        host.control_tick_consumed(0, tokio::time::Instant::now(), vec![HOST_CLIENT_ID], false)
            .await
            .test_value();
        assert_eq!(
            host.control_send_time_ms(&[HOST_CLIENT_ID]),
            0,
            "a central/decentral host with no remote route has no timing sample"
        );

        assert_eq!(
            host.runtime_client_states(0, false).await.unwrap(),
            vec![RuntimeNetworkClientState {
                client_id: HOST_CLIENT_ID,
                status: RemoteBarrierState::Ready,
                control_ready: true,
                // The first 200ms sample moves a 1%-step scaled EWMA from
                // zero to 2ms, exactly like C4GameControlClient::AddPerf.
                wait_ms: 2,
            }]
        );
        assert_eq!(
            host.runtime_client_states(0, true).await.unwrap()[0].wait_ms,
            0,
            "CopyClientList-style resets clear the displayed EWMA"
        );
        host.shutdown().await.test_value();
    }

    #[tokio::test]
    async fn control_tick_consumed_public_handles_only_wait_for_enqueue() {
        let status = NetworkStatus::new(NETWORK_STATE_GO, 0, 7);

        let (host_command_tx, mut host_commands) = mpsc::channel(1);
        let (host_event_tx, host_event_rx) = mpsc::channel(1);
        host_event_tx
            .send(HostEvent::StatusCommitted(status))
            .await
            .test_value();
        let (host_shutdown_tx, _host_shutdown_rx) = oneshot::channel();
        let host = HostHandle {
            command_tx: host_command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(host_event_rx),
            voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
            voice_event_rx: Some(mpsc::channel(1).1),
            shutdown_tx: Some(host_shutdown_tx),
            join_handle: tokio::spawn(async {}),
            udp_local_addr: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };
        timeout(
            Duration::from_millis(50),
            host.control_tick_consumed(7, tokio::time::Instant::now(), vec![HOST_CLIENT_ID], false),
        )
        .await
        .test_value()
        .test_value();
        assert!(matches!(
            host_commands.recv().await,
            Some(HostCommand::ControlTickConsumed { tick: 7, .. })
        ));

        let (client_command_tx, mut client_commands) = mpsc::channel(1);
        let (client_event_tx, client_event_rx) = mpsc::channel(1);
        client_event_tx
            .send(ClientEvent::Status(status))
            .await
            .test_value();
        let (client_shutdown_tx, _client_shutdown_rx) = oneshot::channel();
        let client = ClientHandle {
            command_tx: client_command_tx,
            control_send_time: test_control_send_time_snapshot(),
            control_wait_attribution: Default::default(),
            event_rx: Some(client_event_rx),
            voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
            voice_event_rx: Some(mpsc::channel(1).1),
            shutdown_tx: Some(client_shutdown_tx),
            join_handle: tokio::spawn(async {}),
            client_id: 1,
            join_data: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };
        timeout(
            Duration::from_millis(50),
            client.control_tick_consumed(
                7,
                tokio::time::Instant::now(),
                vec![HOST_CLIENT_ID, 1],
                false,
            ),
        )
        .await
        .test_value()
        .test_value();
        assert!(matches!(
            client_commands.recv().await,
            Some(ClientCommand::ControlTickConsumed { tick: 7, .. })
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_commands_are_serviced_while_route_events_remain_saturated() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let (mut client, client_id) = raw_client_transport(address, b"Flood").await;
        drain_raw_client(&mut client).await;
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let flood = tokio::spawn(async move {
            while !*stop_rx.borrow_and_update() {
                let sent_at = (network_statistics_now_ms() as u32).wrapping_sub(100);
                if client
                    .send_message(ControlMessage::Pong(crate::PingPacket {
                        sent_at,
                        packet_counter: 0,
                    }))
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        });

        timeout(EVENT_WAIT, async {
            loop {
                if host.control_send_time_ms(&[HOST_CLIENT_ID, client_id]) > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();

        timeout(Duration::from_millis(250), host.set_join_allowed(false))
            .await
            .test_value()
            .test_value();

        stop_tx.send_replace(true);
        flood.await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_commands_are_serviced_while_route_events_remain_saturated() {
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let host_event_drain =
            tokio::spawn(async move { while host_events.recv().await.is_some() {} });
        let mut client = connect_test_player(address, "Flood").await;
        let mut events = client.take_event_receiver();
        let flood_data = encode_control_entry_payload(&EngineControlPacket::PlayerControl(
            PlayerControlData::new(0, 0x55, 0, HOST_CLIENT_ID as i32),
        ))
        .test_value();
        let (saturated_tx, saturated_rx) = oneshot::channel();
        let event_drain = tokio::spawn(async move {
            let mut direct_count = 0;
            let mut saturated_tx = Some(saturated_tx);
            while let Some(event) = events.recv().await {
                if matches!(event, ClientEvent::Direct { .. }) {
                    direct_count += 1;
                    if direct_count == 64 {
                        let _ = saturated_tx.take().test_value().send(());
                    }
                }
            }
        });
        let flood_tx = host.command_tx.clone();
        let flood = tokio::spawn(async move {
            loop {
                if flood_tx
                    .send(HostCommand::SubmitPacket {
                        delivery: ControlDelivery::Private,
                        data: flood_data.clone(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        await_test(saturated_rx).await;

        timeout(
            Duration::from_millis(250),
            client.runtime_client_states(0, false),
        )
        .await
        .test_value()
        .test_value();

        flood.abort();
        let _ = flood.await;
        client.shutdown().await.test_value();
        event_drain.await.test_value();
        host.shutdown().await.test_value();
        host_event_drain.await.test_value();
    }

    #[tokio::test]
    async fn host_password_setter_returns_only_after_the_live_state_applies() {
        let (handle, mut commands) = command_test_host_handle();
        let secret = c4(b"secret");
        let setter = tokio::spawn(async move { handle.set_password(Some(secret)).await });

        let HostCommand::SetPassword {
            password,
            completion,
        } = commands.recv().await.test_value()
        else {
            panic!("expected password command");
        };
        assert_eq!(password.unwrap().as_bytes(), b"secret");
        assert!(
            !setter.is_finished(),
            "host state has not applied the password"
        );
        completion.send(()).test_value();
        setter.await.expect("setter task").test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_handles_inspect_client_states_and_retire_the_live_tcp_route() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let client = connect_test_player(address, "Alice").await;

        assert_eq!(
            host.runtime_client_states(0, false).await.unwrap(),
            vec![
                RuntimeNetworkClientState {
                    client_id: HOST_CLIENT_ID,
                    status: RemoteBarrierState::Ready,
                    control_ready: false,
                    wait_ms: 0,
                },
                RuntimeNetworkClientState {
                    client_id: client.client_id(),
                    status: RemoteBarrierState::Chasing,
                    control_ready: false,
                    wait_ms: 0,
                },
            ]
        );
        assert_eq!(
            client.runtime_client_states(0, false).await.unwrap(),
            vec![
                RuntimeNetworkClientState {
                    client_id: HOST_CLIENT_ID,
                    status: RemoteBarrierState::Ready,
                    control_ready: false,
                    wait_ms: 0,
                },
                RuntimeNetworkClientState {
                    client_id: client.client_id(),
                    status: RemoteBarrierState::Ready,
                    control_ready: false,
                    wait_ms: 0,
                },
            ]
        );

        let host_connections = host.runtime_connections().await.test_value();
        assert_eq!(host_connections.len(), 1);
        assert_eq!(host_connections[0].client_id, client.client_id());
        assert_eq!(host_connections[0].usage, "Data/Msg");
        assert_eq!(host_connections[0].protocol, crate::NetworkProtocol::Tcp);
        assert!(host_connections[0].peer_address.is_some());

        let client_connections = client.runtime_connections().await.test_value();
        assert_eq!(client_connections.len(), 1);
        assert_eq!(client_connections[0].client_id, HOST_CLIENT_ID);
        assert_eq!(client_connections[0].usage, "Data/Msg");
        assert_eq!(client_connections[0].protocol, crate::NetworkProtocol::Tcp);
        assert!(client_connections[0].peer_address.is_some());

        let host_lobby_telemetry = host
            .lobby_client_telemetry(vec![client.client_id()])
            .await
            .test_value();
        assert_eq!(
            host_lobby_telemetry
                .connections
                .iter()
                .map(|connection| (connection.client_id, connection.usage.as_str()))
                .collect::<Vec<_>>(),
            vec![(client.client_id(), "Data/Msg")]
        );
        assert_eq!(
            host_lobby_telemetry.resource_progress,
            vec![(client.client_id(), 100)],
            "no peer resource status is native-complete"
        );

        let client_lobby_telemetry = client
            .lobby_client_telemetry(vec![HOST_CLIENT_ID])
            .await
            .test_value();
        assert_eq!(
            client_lobby_telemetry
                .connections
                .iter()
                .map(|connection| (connection.client_id, connection.usage.as_str()))
                .collect::<Vec<_>>(),
            vec![(HOST_CLIENT_ID, "Data/Msg")]
        );
        assert_eq!(
            client_lobby_telemetry.resource_progress,
            vec![(HOST_CLIENT_ID, 100)]
        );

        client
            .disconnect_runtime_connection(client_connections[0].connection_id)
            .await
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                if host.runtime_connections().await.test_value().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();

        client.shutdown().await.test_value();

        let second_client = connect_test_player(address, "Bob").await;
        let second_connections = host.runtime_connections().await.test_value();
        assert_eq!(second_connections.len(), 1);
        assert_eq!(second_connections[0].client_id, second_client.client_id());
        host.disconnect_runtime_connection(second_connections[0].connection_id)
            .await
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                if host.runtime_connections().await.test_value().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        second_client.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_queued_client_remove_projects_removing_before_sync_execution() {
        let (address, listener) = bind_test_listener().await;
        let config = host_config!(initial_status: NetworkStatus::new(NETWORK_STATE_GO, 0, 0));
        let host = start_host(listener, config).await.test_value();
        let client = connect_test_player(address, "Alice").await;
        let remove = encode_control_entry_payload(&EngineControlPacket::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: i32::try_from(client.client_id()).unwrap(),
                reason: c4(b"removed"),
                by_client: HOST_CLIENT_ID as i32,
            },
        ))
        .test_value();
        host.submit_packet(ControlDelivery::Sync, remove)
            .await
            .test_value();

        let states = host.runtime_client_states(0, false).await.test_value();
        assert_eq!(
            states
                .iter()
                .find(|state| state.client_id == HOST_CLIENT_ID)
                .map(|state| state.status),
            Some(RemoteBarrierState::NotReady)
        );
        assert_eq!(
            states
                .iter()
                .find(|state| state.client_id == client.client_id())
                .map(|state| state.status),
            Some(RemoteBarrierState::Removing)
        );

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_restart_excludes_client_whose_remove_awaits_sync_execution() {
        let (address, mut host) = start_test_host(host_config!(
            initial_status: NetworkStatus::new(NETWORK_STATE_GO, 0, 0)
        ))
        .await;
        let mut host_events = host.take_event_receiver();
        let mut removing = connect_test_player(address, "Alice").await;
        let removing_id = removing.client_id();
        let mut retained = connect_test_player(address, "Bob").await;
        let retained_id = retained.client_id();
        let mut removing_events = removing.take_event_receiver();
        let mut retained_events = retained.take_event_receiver();
        while host_events.try_recv().is_ok() {}
        while removing_events.try_recv().is_ok() {}
        while retained_events.try_recv().is_ok() {}
        let remove = encode_control_entry_payload(&EngineControlPacket::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: i32::try_from(removing_id).unwrap(),
                reason: c4(b"removed"),
                by_client: HOST_CLIENT_ID as i32,
            },
        ))
        .test_value();
        host.submit_packet(ControlDelivery::Sync, remove)
            .await
            .test_value();
        let before_routes = host.runtime_connections().await.test_value();
        let mut fresh = HostConfig::default();
        fresh
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .title = c4(b"Fresh round");

        host.restart_round_in_lobby(fresh).await.test_value();

        let restarted = loop {
            match timeout(EVENT_WAIT, retained_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::JoinData { join_data }) => break *join_data,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("retained client disconnected during restart: {reason:?}")
                }
                Some(_) => {}
                None => panic!("retained client event stream ended during restart"),
            }
        };
        assert_eq!(restarted.client_id, retained_id as i32);
        assert_eq!(restarted.parameters.title, c4(b"Fresh round"));
        assert_eq!(
            restarted
                .parameters
                .clients
                .clients
                .iter()
                .map(|core| core.client_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([HOST_CLIENT_ID as i32, retained_id as i32])
        );
        retained.acknowledge_round_restart().await.test_value();

        loop {
            match timeout(EVENT_WAIT, removing_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::HostRestartLobby) => {
                    panic!("client awaiting removal was retained into the fresh round")
                }
                Some(ClientEvent::Disconnected { .. }) | None => break,
                Some(_) => {}
            }
        }
        timeout(EVENT_WAIT, async {
            loop {
                let routes = host.runtime_connections().await.test_value();
                if routes.len() == 1 && routes[0].client_id == retained_id {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert_eq!(before_routes.len(), 2);

        timeout(EVENT_WAIT, async {
            loop {
                let mut second = HostConfig::default();
                second
                    .initial_join_snapshot
                    .as_mut()
                    .test_value()
                    .parameters
                    .title = c4(b"Second fresh round");
                match host.restart_round_in_lobby(second).await {
                    Ok(()) => break,
                    Err(error) if error.to_string().contains("acknowledgement") => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("second restart failed after retained ACK: {error}"),
                }
            }
        })
        .await
        .expect("removed client remained in the restart acknowledgement fence");
        loop {
            match timeout(EVENT_WAIT, retained_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::JoinData { join_data }) => {
                    assert_eq!(join_data.parameters.title, c4(b"Second fresh round"));
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("retained client disconnected during second restart: {reason:?}")
                }
                Some(_) => {}
                None => panic!("retained client event stream ended during second restart"),
            }
        }
        retained.acknowledge_round_restart().await.test_value();

        let mut saw_left = false;
        let mut saw_restart = false;
        while let Ok(event) = host_events.try_recv() {
            saw_left |=
                matches!(event, HostEvent::ClientLeft { client_id } if client_id == removing_id);
            saw_restart |= matches!(event, HostEvent::RoundRestarted);
        }
        assert!(saw_left, "restart did not finalize the pending removal");
        assert!(saw_restart, "restart did not publish its host event fence");

        drop(removing);
        shutdown_test_session(retained, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reliable_udp_client_completes_session_admission_and_control() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let udp_address = host.udp_local_addr().test_value();
        let mut host_events = host.take_event_receiver();
        let client = connect_udp_client(
            udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();

        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x12))
            .await
            .test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x34))
            .await
            .test_value();
        let packet = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(control_commands(&packet), vec![0x34, 0x12]);

        let udp_key = crate::ConnectionStatisticsKey::new(0, crate::NetworkProtocol::Udp);
        assert!(host
            .io_statistics()
            .connection_statistics(udp_key)
            .is_some());
        assert!(client
            .io_statistics()
            .connection_statistics(udp_key)
            .is_some());
        assert!(host
            .io_statistics()
            .snapshot()
            .connections
            .iter()
            .all(|(key, _)| key.connection_id != u32::MAX));
        assert!(client
            .io_statistics()
            .snapshot()
            .connections
            .iter()
            .all(|(key, _)| key.connection_id != u32::MAX));

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn negotiated_udp_voice_round_trips_with_route_authenticated_sources() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let mut client = connect_udp_client(
            host.udp_local_addr().test_value(),
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();
        let client_id = client.client_id();
        timeout(EVENT_WAIT, async {
            while !host.voice_available() || !client.voice_available() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        let mut host_voice = host.take_voice_receiver();
        let mut client_voice = client.take_voice_receiver();

        let mut client_frame = crate::VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).test_value();
        client_frame.client_id = 99;
        client.voice_sender().try_send(client_frame).test_value();
        let received = await_test(host_voice.recv()).await;
        assert_eq!(received.client_id, client_id);

        let mut host_frame = crate::VoiceFrame::outbound(8, 12, 30, vec![0xa5; 164]).test_value();
        host_frame.client_id = 99;
        host.voice_sender().try_send(host_frame).test_value();
        let received = await_test(client_voice.recv()).await;
        assert_eq!(received.client_id, HOST_CLIENT_ID);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn udp_host_relays_authenticated_client_voice_to_another_client_without_self_echo() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let host_address = host.udp_local_addr().test_value();
        let mut alpha = connect_udp_client(
            host_address,
            ClientConfig::new("Alpha", ParticipantKind::Player),
        )
        .await
        .test_value();
        let alpha_id = alpha.client_id();
        let mut beta = connect_udp_client(
            host_address,
            ClientConfig::new("Beta", ParticipantKind::Player),
        )
        .await
        .test_value();
        let beta_id = beta.client_id();
        timeout(EVENT_WAIT, async {
            while !host.voice_available() || !alpha.voice_available() || !beta.voice_available() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert!(!alpha.mesh_peer_ids().await.contains(&beta_id));
        assert!(!beta.mesh_peer_ids().await.contains(&alpha_id));
        let mut host_voice = host.take_voice_receiver();
        let mut alpha_voice = alpha.take_voice_receiver();
        let mut beta_voice = beta.take_voice_receiver();
        let frame = crate::VoiceFrame::outbound(17, 3, 9, vec![0x5a; 164]).test_value();

        alpha.voice_sender().try_send(frame.clone()).test_value();

        let received_by_host = await_test(host_voice.recv()).await;
        let received_by_beta = await_test(beta_voice.recv()).await;
        for received in [received_by_host, received_by_beta] {
            assert_eq!(received.client_id, alpha_id);
            assert_eq!(received.player_id, frame.player_id);
            assert_eq!(received.stream_epoch, frame.stream_epoch);
            assert_eq!(received.sequence, frame.sequence);
            assert_eq!(received.payload, frame.payload);
        }
        assert!(
            timeout(Duration::from_millis(100), alpha_voice.recv())
                .await
                .is_err(),
            "the host relay must not echo a client's own voice frame back to it",
        );

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[test]
    fn application_voice_queue_holds_at_most_160_milliseconds() {
        assert!(
            VOICE_APP_CHANNEL_CAPACITY * usize::from(crate::VOICE_FRAME_DURATION_MS) <= 160,
            "each bounded application stage must hold little encoded speech"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn voice_policy_requires_both_udp_endpoints_to_opt_in() {
        async fn assert_mixed_policy(host_voice_enabled: bool, client_voice_enabled: bool) {
            let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
            let mut host = start_host(
                listener,
                host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                voice_enabled: host_voice_enabled),
            )
            .await
            .test_value();
            let mut client = connect_udp_client(
                host.udp_local_addr().test_value(),
                ClientConfig::new("Alice", ParticipantKind::Player)
                    .with_voice_enabled(client_voice_enabled),
            )
            .await
            .test_value();
            if !host_voice_enabled {
                assert!(!host.voice_sender().is_available());
            }
            if !client_voice_enabled {
                assert!(!client.voice_sender().is_available());
            }
            let mut host_voice = host.take_voice_receiver();
            let mut client_voice = client.take_voice_receiver();

            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(!host.voice_available());
            assert!(!client.voice_available());

            client
                .voice_sender()
                .try_send(crate::VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).test_value())
                .test_value();
            host.voice_sender()
                .try_send(crate::VoiceFrame::outbound(8, 12, 30, vec![0xa5; 164]).test_value())
                .test_value();
            assert!(timeout(Duration::from_millis(100), host_voice.recv())
                .await
                .is_err());
            assert!(timeout(Duration::from_millis(100), client_voice.recv())
                .await
                .is_err());

            shutdown_test_session(client, host).await;
        }

        assert_mixed_policy(true, false).await;
        assert_mixed_policy(false, true).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn udp_only_host_completes_session_admission_and_control() {
        let config = host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
        configured_tcp_port: Some(0));
        let udp_binding = HostUdpBinding::bind(&config);
        let udp_address = udp_binding.local_addr().test_value();
        let mut host = start_host_with_bindings(None, config, udp_binding)
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let client = connect_udp_client(
            udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();

        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x12))
            .await
            .test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x34))
            .await
            .test_value();
        let packet = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(control_commands(&packet), vec![0x34, 0x12]);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn occupied_udp_listener_falls_back_to_a_healthy_tcp_host() {
        let occupied = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .test_value();
        let occupied_address = occupied.local_addr().test_value();
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let tcp_address = listener.local_addr().test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(occupied_address)),
        )
        .await
        .test_value();
        assert_eq!(host.udp_local_addr(), None);
        let mut host_events = host.take_event_receiver();
        assert!(matches!(
            timeout(EVENT_WAIT, host_events.recv()).await,
            Ok(Some(HostEvent::TransportError {
                client_id: None,
                error,
            })) if error.contains("failed to start reliable-UDP listener")
        ));

        let client = connect_test_player(tcp_address, "Alice").await;
        assert_eq!(host.accepted_routes().await.len(), 1);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcp_accept_failure_keeps_existing_routes_and_retries_the_listener() {
        // A failed native Accept makes that scheduler pass report false, but
        // the no-op OnError leaves the TCP proc installed and the scheduler
        // thread keeps executing it (src/C4NetIO.cpp:610-625,1038-1053;
        // src/StdScheduler.cpp:160-191,229-244;
        // src/StdScheduler.h:95-98).
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut witness, witness_id) = raw_client_transport(address, b"Witness").await;
        assert_eq!(host.accepted_routes().await.len(), 1);

        inject_tcp_accept_failure(address);
        timeout(EVENT_WAIT, async {
            loop {
                match host_events.recv().await {
                    Some(HostEvent::TransportError {
                        client_id: None,
                        error,
                    }) if error.contains("injected TCP accept failure") => break,
                    Some(_) => continue,
                    None => panic!("host event stream ended before the accept diagnostic"),
                }
            }
        })
        .await
        .test_value();

        raw_client_ping_barrier(&mut witness).await;
        let (mut successor, successor_id) =
            timeout(EVENT_WAIT, raw_client_transport(address, b"Successor"))
                .await
                .test_value();
        assert_ne!(successor_id, witness_id);
        raw_client_ping_barrier(&mut successor).await;
        let accepted_client_ids = host
            .accepted_routes()
            .await
            .into_iter()
            .map(|(_, client_id, ..)| client_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            accepted_client_ids,
            BTreeSet::from([witness_id, successor_id])
        );

        drop(witness);
        drop(successor);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn occupied_client_udp_port_falls_back_to_a_healthy_tcp_route() {
        let occupied = tokio::net::UdpSocket::bind("[::]:0").await.test_value();
        let occupied_port = occupied.local_addr().test_value().port();
        let (tcp_address, host) = start_test_host(HostConfig::default()).await;

        let client = connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, tcp_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Udp, tcp_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_mesh_tcp_bind_address(SocketAddr::from(([0_u16; 8], 0)))
                .with_mesh_udp_bind_address(SocketAddr::from(([0_u16; 8], occupied_port))),
        )
        .await
        .test_value();

        assert_eq!(host.accepted_routes().await.len(), 1);
        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_shutdown_releases_the_configured_shared_udp_port() {
        let reservation = tokio::net::UdpSocket::bind("[::]:0").await.test_value();
        let configured_udp_port = reservation.local_addr().test_value().port();
        drop(reservation);
        let mut puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let puncher_address = puncher.local_addr();
        let (tcp_address, host) = start_test_host(HostConfig::default()).await;

        let client = connect_client(
            tcp_address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_mesh_udp_bind_address(SocketAddr::from(([0_u16; 8], configured_udp_port)))
                .with_mesh_punchers([ClientMeshPuncherConfig {
                    address: puncher_address,
                    game_id: 0x1122_3344,
                }]),
        )
        .await
        .test_value();
        let _puncher_stream = await_test(puncher.accept()).await;

        client.shutdown().await.test_value();
        let rebound =
            tokio::net::UdpSocket::bind(SocketAddr::from(([0_u16; 8], configured_udp_port)))
                .await
                .test_value();
        drop(rebound);
        host.shutdown().await.test_value();
        puncher.shutdown().await.test_value();
    }

    #[tokio::test]
    async fn first_client_mesh_route_sends_one_resource_discover_per_logical_peer() {
        let mut resource_state = ClientResourceState::empty();
        assert!(resource_state
            .catalog
            .register(crate::ResourceRegistration {
                resource_id: 41,
                chunk_count: 2,
                binary_compatible: true,
                loading: false,
            }));
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut routes = ClientRouteManager::new();
        let (tcp_client, tcp_peer) = duplex(4_096);
        let mut tcp_peer = crate::ControlTransport::new(tcp_peer);

        let first_route = routes.add_peer_route(
            7,
            7,
            70,
            700,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(tcp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        assert!(first_route);
        if first_route {
            dispatch_client_resource_peer_connected(7, &mut resource_state, &mut routes, &event_tx)
                .await
                .test_value();
        }
        assert_eq!(
            timeout(EVENT_WAIT, tcp_peer.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Resource(ResourcePacket::Discover(crate::ResourceDiscoverPacket {
                resource_ids: vec![41],
            },))
        );

        let (udp_client, udp_peer) = duplex(4_096);
        let mut udp_peer = crate::ControlTransport::new(udp_peer);
        let second_route = routes.add_peer_route(
            7,
            7,
            71,
            701,
            crate::NetworkProtocol::Udp,
            None,
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        assert!(!second_route);
        if second_route {
            dispatch_client_resource_peer_connected(7, &mut resource_state, &mut routes, &event_tx)
                .await
                .test_value();
        }
        assert!(timeout(Duration::from_millis(50), udp_peer.read_message())
            .await
            .is_err());

        routes.shutdown().await;
    }

    #[tokio::test]
    async fn client_resource_dispatch_fans_out_and_selects_cpp_message_and_data_routes() {
        let mut resource_state = ClientResourceState::empty();
        let mut routes = ClientRouteManager::new();
        let mut host =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        let mut peer_tcp = add_test_route_queue(&mut routes, 70, 7, crate::NetworkProtocol::Tcp);
        let mut peer_udp = add_test_route_queue(&mut routes, 71, 7, crate::NetworkProtocol::Udp);
        let mut second_peer = add_test_route_queue(&mut routes, 90, 9, crate::NetworkProtocol::Tcp);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let status = ResourcePacket::Status(crate::ResourceStatusPacket {
            resource_id: 12,
            chunks: crate::ResourceChunkAvailability {
                chunk_count: 1,
                ranges: vec![crate::ResourceChunkRange {
                    start: 0,
                    length: 1,
                }],
            },
        });

        dispatch_client_resource_actions(
            vec![crate::ResourceCatalogAction::Broadcast {
                packet: status.clone(),
            }],
            &mut resource_state,
            &mut routes,
            &event_tx,
        )
        .await
        .test_value();
        assert_eq!(take_queued_resource(&mut host), status);
        assert_eq!(take_queued_resource(&mut peer_udp), status);
        assert_eq!(take_queued_resource(&mut second_peer), status);
        assert!(peer_tcp.try_recv().is_err());

        let request = ResourcePacket::Request(crate::ResourceRequestPacket {
            resource_id: 12,
            chunk: 0,
        });
        dispatch_client_resource_actions(
            vec![crate::ResourceCatalogAction::SendToPeer {
                peer_id: 7,
                packet: request.clone(),
            }],
            &mut resource_state,
            &mut routes,
            &event_tx,
        )
        .await
        .test_value();
        assert_eq!(take_queued_resource(&mut peer_udp), request);
        assert!(host.try_recv().is_err());
        assert!(second_peer.try_recv().is_err());

        let data = ResourcePacket::Data(crate::ResourceDataPacket {
            resource_id: 12,
            chunk: 0,
            data: vec![0x55],
        });
        dispatch_client_resource_actions(
            vec![crate::ResourceCatalogAction::SendToPeer {
                peer_id: 7,
                packet: data.clone(),
            }],
            &mut resource_state,
            &mut routes,
            &event_tx,
        )
        .await
        .test_value();
        assert_eq!(take_queued_resource(&mut peer_tcp), data);
        assert!(host.try_recv().is_err());
        assert!(peer_udp.try_recv().is_err());
        assert!(second_peer.try_recv().is_err());

        routes.retire_peer_gracefully(7);
        dispatch_client_resource_actions(
            vec![crate::ResourceCatalogAction::SendToPeer {
                peer_id: 7,
                packet: request,
            }],
            &mut resource_state,
            &mut routes,
            &event_tx,
        )
        .await
        .test_value();
        assert!(routes.connected_peer_ids().contains(&HOST_CLIENT_ID));
        assert!(host.try_recv().is_err());
    }

    #[tokio::test]
    async fn failed_client_peer_requests_rollback_and_refill_from_live_peers() {
        let mut resource_state = ClientResourceState::empty();
        assert!(resource_state
            .catalog
            .register(crate::ResourceRegistration {
                resource_id: 91,
                chunk_count: 64,
                binary_compatible: true,
                loading: true,
            }));
        // C++'s own thresholds, so the rollback/refill counts below stay the
        // ones StartLoad produces rather than the port's scaled lobby window.
        resource_state.catalog.set_max_loads_per_peer(3);
        resource_state.catalog.set_max_loads(20);
        let availability = crate::ResourceChunkAvailability {
            chunk_count: 64,
            ranges: vec![crate::ResourceChunkRange {
                start: 0,
                length: 64,
            }],
        };
        for peer_id in 1_i32..=9 {
            assert_eq!(
                resource_state.catalog.record_peer_status(
                    peer_id,
                    &crate::ResourceStatusPacket {
                        resource_id: 91,
                        chunks: availability.clone(),
                    },
                ),
                crate::PeerStatusOutcome::Recorded
            );
        }
        let actions = resource_state.catalog.refill_requests(91, 0, |_| 0);
        assert_eq!(actions.len(), 19);
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(
                    action,
                    crate::ResourceCatalogAction::SendToPeer { peer_id: 9, .. }
                ))
                .count(),
            3
        );
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(
                    action,
                    crate::ResourceCatalogAction::SendToPeer { peer_id: 8, .. }
                ))
                .count(),
            3
        );

        let mut routes = ClientRouteManager::new();
        let mut host =
            add_test_route_queue(&mut routes, 1, HOST_CLIENT_ID, crate::NetworkProtocol::Tcp);
        let mut receivers = BTreeMap::new();
        for peer_id in 1_u32..=7 {
            receivers.insert(
                peer_id,
                add_test_route_queue(
                    &mut routes,
                    200 + peer_id,
                    peer_id,
                    crate::NetworkProtocol::Udp,
                ),
            );
        }
        let (event_tx, _event_rx) = mpsc::channel(8);
        dispatch_client_resource_actions(actions, &mut resource_state, &mut routes, &event_tx)
            .await
            .test_value();

        let mut sent = Vec::new();
        for (peer_id, receiver) in &mut receivers {
            while let Ok(command) = receiver.try_recv() {
                let ClientRouteCommand::Message(ControlMessage::Resource(ResourcePacket::Request(
                    request,
                ))) = command
                else {
                    panic!("refill queued a non-request resource command");
                };
                sent.push((*peer_id, request.chunk));
            }
        }
        assert_eq!(resource_state.catalog.outstanding_load_count(91), 19);
        assert_eq!(sent.len(), 19);
        assert_eq!(
            sent.iter()
                .map(|(_, chunk)| *chunk)
                .collect::<BTreeSet<_>>()
                .len(),
            19
        );
        assert!(sent.iter().all(|(peer_id, _)| *peer_id < 8));
        assert!(routes.connected_peer_ids().contains(&HOST_CLIENT_ID));
        assert!(host.try_recv().is_err());
    }

    #[tokio::test]
    async fn client_peer_resource_statuses_fill_cpp_swarm_limits_and_serve_chunks() {
        let directories = SessionResourceDirectories::new();
        let mut resource_state = ClientResourceState::empty();
        resource_state.backend =
            Some(crate::ResourceTransferBackend::new(9, directories.client.clone()).test_value());
        let core = network_core!(resource_type: 2,
        id: 77,
        loadable: true,
        file_size: 512,
        chunk_size: 1,
        filename: c4(b"Swarm.bin"));
        resource_state
            .backend
            .as_mut()
            .unwrap()
            .register_remote_loadable(core.clone())
            .test_value();
        let mut routes = ClientRouteManager::new();
        let mut receivers = BTreeMap::new();
        for peer_id in 0_i32..=6 {
            receivers.insert(
                peer_id,
                add_test_route_queue(
                    &mut routes,
                    100 + peer_id as u32,
                    peer_id as ClientId,
                    crate::NetworkProtocol::Udp,
                ),
            );
        }
        let (event_tx, _event_rx) = mpsc::channel(8);
        let status = ResourcePacket::Status(crate::ResourceStatusPacket {
            resource_id: core.id,
            chunks: crate::ResourceChunkAvailability {
                chunk_count: 512,
                ranges: vec![crate::ResourceChunkRange {
                    start: 0,
                    length: 512,
                }],
            },
        });

        for peer_id in 0_i32..=6 {
            dispatch_client_resource_packet(
                peer_id,
                &status,
                &mut resource_state,
                &mut routes,
                &event_tx,
            )
            .await
            .test_value();
        }

        let mut outstanding = Vec::new();
        let mut fulfilled = None;
        for peer_id in 0_i32..=6 {
            let ResourcePacket::Request(request) =
                take_queued_resource(receivers.get_mut(&peer_id).test_value())
            else {
                panic!("peer status did not schedule a resource request");
            };
            if peer_id == 1 {
                fulfilled = Some(request);
            } else {
                outstanding.push((peer_id, request.chunk));
            }
        }
        let fulfilled = fulfilled.test_value();
        // One chunk is one stride, not the whole tail: C4Network2ResChunk::Set
        // sizes by the core's chunk size (src/C4Network2Res.cpp:1268-1269).
        let fulfilled_data = vec![
            0x5a;
            usize::try_from(core.chunk_size).unwrap().min(
                usize::try_from(core.file_size).unwrap()
                    - usize::try_from(fulfilled.chunk).unwrap()
            )
        ];
        dispatch_client_resource_packet(
            1,
            &ResourcePacket::Data(crate::ResourceDataPacket {
                resource_id: core.id,
                chunk: u32::try_from(fulfilled.chunk).unwrap(),
                data: fulfilled_data.clone(),
            }),
            &mut resource_state,
            &mut routes,
            &event_tx,
        )
        .await
        .test_value();

        for (peer_id, receiver) in &mut receivers {
            while let Ok(command) = receiver.try_recv() {
                let ClientRouteCommand::Message(ControlMessage::Resource(ResourcePacket::Request(
                    request,
                ))) = command
                else {
                    panic!("swarm refill queued a non-request resource command");
                };
                outstanding.push((*peer_id, request.chunk));
            }
        }
        // Both caps bind: the swarm saturates the per-resource total, no single
        // peer is asked for more than the per-peer window, and every request is
        // for a distinct chunk. Asserted through the constants because the port
        // scales them with its smaller chunk size; the byte window they buy is
        // pinned against C++ by `the_lobby_load_caps_hold_the_cpp_byte_window`.
        let backend = resource_state.backend.as_ref().test_value();
        let total = crate::RESOURCE_MAX_LOADS - 1;
        assert_eq!(backend.catalog().outstanding_load_count(core.id), total);
        assert_eq!(outstanding.len(), total);
        assert_eq!(
            outstanding
                .iter()
                .map(|(_, chunk)| *chunk)
                .collect::<BTreeSet<_>>()
                .len(),
            total
        );
        let mut per_peer = BTreeMap::<i32, usize>::new();
        for (peer_id, _) in &outstanding {
            *per_peer.entry(*peer_id).or_default() += 1;
        }
        let mut counts = per_peer.values().copied().collect::<Vec<_>>();
        counts.sort_unstable();
        assert_eq!(counts.len(), 7);
        assert_eq!(counts.iter().sum::<usize>(), total);
        assert!(counts
            .iter()
            .all(|count| *count <= crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE));
        assert_eq!(
            counts.last().copied(),
            Some(crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE)
        );
        assert_eq!(
            backend
                .catalog()
                .peer_ids(core.id)
                .into_iter()
                .collect::<BTreeSet<_>>(),
            (0_i32..=6).collect()
        );

        dispatch_client_resource_packet(
            2,
            &ResourcePacket::Request(crate::ResourceRequestPacket {
                resource_id: core.id,
                chunk: fulfilled.chunk,
            }),
            &mut resource_state,
            &mut routes,
            &event_tx,
        )
        .await
        .test_value();
        assert_eq!(
            take_queued_resource(receivers.get_mut(&2).unwrap()),
            ResourcePacket::Data(crate::ResourceDataPacket {
                resource_id: core.id,
                chunk: u32::try_from(fulfilled.chunk).unwrap(),
                data: fulfilled_data,
            })
        );
    }

    #[tokio::test]
    async fn client_route_manager_splits_traffic_falls_back_and_pongs_per_route() {
        fn flatten_recovery(message: ControlMessage, messages: &mut Vec<ControlMessage>) {
            match message {
                ControlMessage::PostMortem(packet) => {
                    for packet in packet.packets {
                        if let Some(message) =
                            crate::transport::parse_complete_packet(&packet).test_value()
                        {
                            flatten_recovery(message, messages);
                        }
                    }
                }
                message => messages.push(message),
            }
        }

        let (tcp_client, tcp_peer) = duplex(4096);
        let (udp_client, udp_peer) = duplex(4096);
        let mut tcp = crate::ControlTransport::new(tcp_peer);
        let mut udp = crate::ControlTransport::new(udp_peer);
        let mut routes = single_test_client_route(tcp_client);
        routes.add_route(
            2,
            12,
            crate::NetworkProtocol::Udp,
            None,
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );

        let countdown = crate::LobbyCountdownPacket::new(9);
        routes
            .send_message(ControlMessage::LobbyCountdown(countdown))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, udp.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::LobbyCountdown(countdown)
        );

        let data = crate::ResourceDataPacket {
            resource_id: 7,
            chunk: 3,
            data: vec![1, 2, 3],
        };
        routes
            .send_message(ControlMessage::Resource(ResourcePacket::Data(data.clone())))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, tcp.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Resource(ResourcePacket::Data(data))
        );

        let tcp_ping = crate::PingPacket {
            sent_at: 101,
            packet_counter: 0,
        };
        let udp_ping = crate::PingPacket {
            sent_at: 202,
            packet_counter: 0,
        };
        tcp.send_message(ControlMessage::Ping(tcp_ping))
            .await
            .test_value();
        udp.send_message(ControlMessage::Ping(udp_ping))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, tcp.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Pong(tcp_ping)
        );
        assert_eq!(
            timeout(EVENT_WAIT, udp.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Pong(udp_ping)
        );

        drop(udp);
        timeout(EVENT_WAIT, async {
            loop {
                if routes
                    .routes
                    .get(&2)
                    .is_some_and(|route| route.outbound.sender.is_closed())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 0);
        routes
            .send_message(ControlMessage::StatusAck(status))
            .await
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                if matches!(
                    routes.read_event().await.unwrap(),
                    ClientRouteRead::Disconnected {
                        protocol: crate::NetworkProtocol::Udp,
                        routes_remaining: true,
                        ..
                    }
                ) {
                    break;
                }
            }
        })
        .await
        .test_value();
        let mut recovered = Vec::new();
        timeout(EVENT_WAIT, async {
            while !recovered.contains(&ControlMessage::StatusAck(status)) {
                match tcp.read_message().await.test_value() {
                    ControlMessage::Ping(packet) => tcp
                        .send_message(ControlMessage::Pong(packet))
                        .await
                        .test_value(),
                    message => flatten_recovery(message, &mut recovered),
                }
            }
        })
        .await
        .test_value();

        let (fallback_udp_client, fallback_udp_peer) = duplex(4096);
        let mut fallback_udp = crate::ControlTransport::new(fallback_udp_peer);
        routes.add_route(
            3,
            13,
            crate::NetworkProtocol::Udp,
            None,
            crate::ControlTransport::new(fallback_udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        drop(tcp);
        timeout(EVENT_WAIT, async {
            loop {
                if routes
                    .routes
                    .get(&1)
                    .is_some_and(|route| route.outbound.sender.is_closed())
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        let fallback_data = crate::ResourceDataPacket {
            resource_id: 8,
            chunk: 4,
            data: vec![4, 5, 6],
        };
        routes
            .send_message(ControlMessage::Resource(ResourcePacket::Data(
                fallback_data.clone(),
            )))
            .await
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                if matches!(
                    routes.read_event().await.unwrap(),
                    ClientRouteRead::Disconnected {
                        protocol: crate::NetworkProtocol::Tcp,
                        routes_remaining: true,
                        ..
                    }
                ) {
                    break;
                }
            }
        })
        .await
        .test_value();
        let expected = ControlMessage::Resource(ResourcePacket::Data(fallback_data));
        recovered.clear();
        timeout(EVENT_WAIT, async {
            while !recovered.contains(&expected) {
                match fallback_udp.read_message().await.test_value() {
                    ControlMessage::Ping(packet) => fallback_udp
                        .send_message(ControlMessage::Pong(packet))
                        .await
                        .test_value(),
                    message => flatten_recovery(message, &mut recovered),
                }
            }
        })
        .await
        .test_value();
        routes.shutdown().await;
    }

    #[tokio::test]
    async fn client_route_manager_preserves_the_actual_ingress_peer_address() {
        let (udp_client, udp_peer) = duplex(1024);
        let mut udp = crate::ControlTransport::new(udp_peer);
        let udp_peer_addr = SocketAddr::from(([127, 0, 0, 1], 22_222));
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            2,
            12,
            crate::NetworkProtocol::Udp,
            Some(udp_peer_addr),
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        let address = crate::AddressPacket {
            client_id: 0,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Udp,
                "[::]:0".parse().test_value(),
            ),
        };
        udp.send_message(ControlMessage::Address(address))
            .await
            .test_value();

        let (packet, ingress_peer_addr) = timeout(EVENT_WAIT, routes.read_packet())
            .await
            .unwrap()
            .test_value();
        assert!(matches!(
            packet,
            crate::transport::InboundPacket::Message(ControlMessage::Address(received))
                if received == address
        ));
        assert_eq!(ingress_peer_addr, Some(udp_peer_addr));

        routes.shutdown().await;
    }

    #[tokio::test]
    async fn client_route_manager_peer_try_send_queues_without_retiring_a_slow_route() {
        // C4Network2Client::SendMsg delegates to the selected connection's
        // buffered nonblocking Send, so application fanout does not retire a
        // healthy route merely because its socket is slow (oracle-src-pinned
        // src/C4Network2Client.cpp:121-124;
        // src/C4NetIO.cpp:1345-1357,2788-2808).
        let (client_stream, _peer_stream) = duplex(1);
        let mut routes = ClientRouteManager::new();
        routes.add_peer_route(
            7,
            7,
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(client_stream),
            ConnectionLivenessState::new_accepted_system(),
        );
        let message = ControlMessage::Packet {
            delivery: ControlDelivery::Direct,
            data: vec![0x55; 1_024],
        };

        for _ in 0..128 {
            routes.try_send_to(7, message.clone()).test_value();
        }
        assert!(!routes.routes[&1].outbound.is_closed());

        timeout(EVENT_WAIT, routes.shutdown()).await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_address_burst_only_waits_for_cpp_output_buffer_acceptance() {
        // SendAddresses only appends each PID_Addr to the connection's OBuf;
        // it neither waits for the socket to drain nor fails the route on an
        // EWOULDBLOCK result (oracle-src-pinned
        // src/C4Network2Client.cpp:319-337,616-621;
        // src/C4NetIO.cpp:1345-1396).
        let (client_stream, _host_stream) = duplex(1);
        let mut routes = single_test_client_route(client_stream);
        let announcements = (0..160)
            .map(|index| crate::AddressPacket {
                client_id: 1,
                address: crate::NetworkAddress::new(
                    crate::NetworkProtocol::Tcp,
                    SocketAddr::from(([127, 0, 0, 1], 10_000 + index)),
                ),
            })
            .collect();

        timeout(
            Duration::from_millis(50),
            connect::send_client_route_address_announcements(&mut routes, announcements),
        )
        .await
        .test_value()
        .test_value();
        assert!(!routes.routes[&1].outbound.is_closed());

        timeout(EVENT_WAIT, routes.shutdown()).await.test_value();
    }

    #[tokio::test]
    async fn client_route_manager_retires_the_deterministic_crossed_route_loser() {
        let (winner_stream, winner_peer) = duplex(1_024);
        let mut winner_peer = crate::ControlTransport::new(winner_peer);
        let (loser_stream, _loser_peer) = duplex(1_024);
        let mut routes = ClientRouteManager::new();
        routes.add_peer_route(
            9,
            7,
            10,
            20,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(winner_stream),
            ConnectionLivenessState::new_accepted_system(),
        );
        routes.add_peer_route(
            9,
            8,
            11,
            21,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(loser_stream),
            ConnectionLivenessState::new_accepted_system(),
        );

        assert!(matches!(
            timeout(EVENT_WAIT, routes.read_event())
                .await
                .unwrap()
                .unwrap(),
            ClientRouteRead::Disconnected {
                peer_id: 9,
                protocol: crate::NetworkProtocol::Tcp,
                routes_remaining: true,
                ..
            }
        ));
        assert!(routes.closed_routes.contains(11));
        let countdown = crate::LobbyCountdownPacket::new(3);
        routes
            .send_to(9, ControlMessage::LobbyCountdown(countdown))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, winner_peer.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::LobbyCountdown(countdown)
        );

        routes.shutdown().await;
    }

    #[tokio::test]
    async fn last_peer_route_disconnect_surfaces_post_mortem_for_host_fallback() {
        let (client_stream, peer_stream) = duplex(2_048);
        let mut peer = crate::ControlTransport::new(peer_stream);
        let mut routes = ClientRouteManager::new();
        routes.add_peer_route(
            9,
            9,
            10,
            20,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(client_stream),
            ConnectionLivenessState::new_accepted_system(),
        );
        let countdown = crate::LobbyCountdownPacket::new(4);
        routes
            .send_to(9, ControlMessage::LobbyCountdown(countdown))
            .await
            .test_value();
        assert_eq!(
            peer.read_message().await.unwrap(),
            ControlMessage::LobbyCountdown(countdown)
        );
        drop(peer);

        let event = timeout(EVENT_WAIT, routes.read_event())
            .await
            .unwrap()
            .test_value();
        let ClientRouteRead::Disconnected {
            peer_id: 9,
            routes_remaining: false,
            post_mortem: Some(post_mortem),
            ..
        } = event
        else {
            panic!("last peer route did not surface its recovery backlog");
        };
        assert_eq!(post_mortem.connection_id, 20);
        assert_eq!(post_mortem.packets.len(), 1);

        routes.shutdown().await;
    }

    #[tokio::test]
    async fn client_route_manager_retires_an_asymmetric_udp_reconnect_request() {
        let (tcp_client, _tcp_peer) = duplex(1024);
        let (udp_client, udp_peer) = duplex(1024);
        let mut udp = crate::ControlTransport::new(udp_peer);
        let mut routes = single_test_client_route(tcp_client);
        routes.add_route(
            2,
            12,
            crate::NetworkProtocol::Udp,
            None,
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        udp.send_message(ControlMessage::ConnectionRequest(test_connection_request(
            clonk_engine::ClientCoreControlData::default(),
            99,
            false,
        )))
        .await
        .test_value();

        let event = await_test(routes.read_event()).await;
        assert!(matches!(
            event,
            ClientRouteRead::Disconnected {
                protocol: crate::NetworkProtocol::Udp,
                routes_remaining: true,
                ..
            }
        ));
        routes.shutdown().await;
    }

    #[tokio::test]
    async fn client_route_manager_replays_post_mortem_over_the_surviving_route() {
        let (tcp_client, tcp_peer) = duplex(2048);
        let (udp_client, udp_peer) = duplex(2048);
        let mut tcp = crate::ControlTransport::new(tcp_peer);
        let mut udp = crate::ControlTransport::new(udp_peer);
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            Some(SocketAddr::from(([127, 0, 0, 1], 11_111))),
            crate::ControlTransport::new(tcp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        routes.add_route(
            2,
            12,
            crate::NetworkProtocol::Udp,
            Some(SocketAddr::from(([127, 0, 0, 1], 22_222))),
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );

        let outgoing_data = crate::ResourceDataPacket {
            resource_id: 9,
            chunk: 0,
            data: vec![9, 8, 7],
        };
        routes
            .send_message(ControlMessage::Resource(ResourcePacket::Data(
                outgoing_data.clone(),
            )))
            .await
            .test_value();
        assert_eq!(
            tcp.read_message().await.unwrap(),
            ControlMessage::Resource(ResourcePacket::Data(outgoing_data))
        );

        let countdown = crate::LobbyCountdownPacket::new(5);
        let (encoder_stream, mut encoded_stream) = duplex(1024);
        let mut encoder = crate::ControlTransport::new(encoder_stream);
        encoder
            .send_message(ControlMessage::LobbyCountdown(countdown))
            .await
            .test_value();
        let mut header = [0_u8; 5];
        encoded_stream.read_exact(&mut header).await.test_value();
        let mut complete_packet =
            vec![0_u8; u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize];
        encoded_stream
            .read_exact(&mut complete_packet)
            .await
            .test_value();

        udp.send_message(ControlMessage::PostMortem(crate::PostMortemPacket {
            connection_id: 1,
            packet_counter: 1,
            packets: vec![complete_packet],
        }))
        .await
        .test_value();
        let (replayed, peer_addr) = await_test(routes.read_packet()).await;
        assert!(matches!(
            replayed,
            crate::transport::InboundPacket::Message(
                ControlMessage::LobbyCountdown(packet)
            ) if packet == countdown
        ));
        assert_eq!(peer_addr, Some(SocketAddr::from(([127, 0, 0, 1], 11_111))));

        let reciprocal = loop {
            match timeout(EVENT_WAIT, udp.read_message())
                .await
                .unwrap()
                .test_value()
            {
                ControlMessage::PostMortem(packet) => break packet,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected reciprocal post-mortem, got {other:?}"),
            }
        };
        assert_eq!(reciprocal.connection_id, 11);
        assert_eq!(reciprocal.packet_counter, 1);
        assert_eq!(reciprocal.packets.len(), 1);

        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 0);
        routes
            .send_message(ControlMessage::StatusAck(status))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, udp.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::StatusAck(status)
        );

        routes.shutdown().await;
    }

    #[tokio::test]
    async fn client_route_manager_recovers_a_post_mortem_nested_in_another_replay() {
        async fn complete_packet(message: ControlMessage) -> Vec<u8> {
            let (writer, mut reader) = duplex(2048);
            let mut transport = crate::ControlTransport::new(writer);
            transport.send_message(message).await.test_value();
            let mut header = [0_u8; 5];
            reader.read_exact(&mut header).await.test_value();
            let mut body = vec![0; u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize];
            reader.read_exact(&mut body).await.test_value();
            body
        }

        let countdown = crate::LobbyCountdownPacket::new(4);
        let inner = crate::PostMortemPacket {
            connection_id: 1,
            packet_counter: 1,
            packets: vec![complete_packet(ControlMessage::LobbyCountdown(countdown)).await],
        };
        let outer = crate::PostMortemPacket {
            connection_id: 2,
            packet_counter: 1,
            packets: vec![complete_packet(ControlMessage::PostMortem(inner)).await],
        };
        let mut routes = ClientRouteManager::new();
        routes.closed_routes.retain(1, HOST_CLIENT_ID, 0);
        routes.closed_routes.retain(2, HOST_CLIENT_ID, 0);
        routes.handle_post_mortem(HOST_CLIENT_ID, outer);

        let Some((HOST_CLIENT_ID, crate::transport::InboundPacket::Message(message), _)) =
            routes.replay_packets.pop_front()
        else {
            panic!("nested post-mortem suffix was not replayed");
        };
        assert_eq!(message, ControlMessage::LobbyCountdown(countdown));
        assert!(routes.replay_packets.is_empty());
    }

    #[tokio::test]
    async fn mesh_peer_post_mortem_cannot_retire_or_replay_as_the_host_route() {
        let (host_client, _host_peer) = duplex(512);
        let mut routes = single_test_client_route(host_client);
        routes.closed_routes.retain(3, HOST_CLIENT_ID, 0);
        let forged_live = crate::PostMortemPacket {
            connection_id: 1,
            packet_counter: 0,
            packets: Vec::new(),
        };
        let forged_closed = crate::PostMortemPacket {
            connection_id: 3,
            packet_counter: 0,
            packets: Vec::new(),
        };

        routes.handle_post_mortem(7, forged_live);
        routes.handle_post_mortem(7, forged_closed);

        assert!(routes.routes.contains_key(&1));
        assert!(routes.pending_post_mortems.is_empty());
        assert!(routes.closed_routes.contains(3));
        assert!(routes.replay_packets.is_empty());
        routes.shutdown().await;
    }

    #[tokio::test]
    async fn host_relayed_post_mortem_recovers_the_registered_peer_route() {
        let countdown = crate::LobbyCountdownPacket::new(8);
        let (writer, mut reader) = duplex(512);
        let mut encoder = crate::ControlTransport::new(writer);
        encoder
            .send_message(ControlMessage::LobbyCountdown(countdown))
            .await
            .test_value();
        let mut header = [0_u8; 5];
        reader.read_exact(&mut header).await.test_value();
        let mut complete_packet =
            vec![0; u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize];
        reader.read_exact(&mut complete_packet).await.test_value();

        let mut routes = ClientRouteManager::new();
        routes.closed_routes.retain(3, 9, 0);
        routes.handle_post_mortem(
            HOST_CLIENT_ID,
            crate::PostMortemPacket {
                connection_id: 3,
                packet_counter: 1,
                packets: vec![complete_packet],
            },
        );

        let Some((9, crate::transport::InboundPacket::Message(message), _)) =
            routes.replay_packets.pop_front()
        else {
            panic!("trusted host relay did not recover the peer packet");
        };
        assert_eq!(message, ControlMessage::LobbyCountdown(countdown));
    }

    #[tokio::test]
    async fn client_route_shutdown_interrupts_a_blocked_write_with_queued_commands() {
        let (client_stream, _peer_stream) = duplex(1);
        let mut routes = single_test_client_route(client_stream);
        let outbound = routes.routes[&1].outbound.clone();
        for _ in 0..64 {
            let _ = outbound
                .sender
                .send(ClientRouteCommand::Message(ControlMessage::Packet {
                    delivery: ControlDelivery::Direct,
                    data: vec![0x55; 1_024],
                }));
        }
        tokio::task::yield_now().await;

        timeout(EVENT_WAIT, routes.shutdown()).await.test_value();
    }

    #[tokio::test]
    async fn host_handle_shutdown_bypasses_full_command_and_event_queues() {
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 0, 0);
        let (command_tx, _command_rx) = mpsc::channel(1);
        command_tx.send(HostCommand::Shutdown).await.test_value();
        let (event_tx, event_rx) = mpsc::channel(1);
        event_tx
            .send(HostEvent::StatusCommitted(status))
            .await
            .test_value();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join_handle = tokio::spawn(async move {
            let _ = event_tx.send(HostEvent::StatusCommitted(status)).await;
            let _ = shutdown_rx.await;
        });
        let handle = HostHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
            voice_event_rx: Some(mpsc::channel(1).1),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            udp_local_addr: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        await_test(handle.shutdown()).await;
    }

    #[tokio::test]
    async fn client_handle_shutdown_bypasses_full_command_and_event_queues() {
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 0, 0);
        let (command_tx, _command_rx) = mpsc::channel(1);
        command_tx.send(ClientCommand::Shutdown).await.test_value();
        let (event_tx, event_rx) = mpsc::channel(1);
        event_tx
            .send(ClientEvent::Status(status))
            .await
            .test_value();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join_handle = tokio::spawn(async move {
            let _ = event_tx.send(ClientEvent::Status(status)).await;
            let _ = shutdown_rx.await;
        });
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            control_wait_attribution: Default::default(),
            event_rx: Some(event_rx),
            voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
            voice_event_rx: Some(mpsc::channel(1).1),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            client_id: 1,
            join_data: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        await_test(handle.shutdown()).await;
    }

    #[tokio::test]
    async fn failed_client_route_retains_commands_already_accepted_by_its_queue() {
        let (client_stream, peer_stream) = duplex(256);
        let (outbound_tx, _retire_tx, mut event_rx, task) = start_test_client_route(client_stream);
        let first = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 7);
        let second = NetworkStatus::new(NETWORK_STATE_PAUSE, 2, 8);
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Status(first)))
            .test_value();
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Status(second)))
            .test_value();
        drop(peer_stream);
        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("failed route did not report its recovery backlog")
            .test_value();
        let ClientRouteEvent::Disconnected {
            post_mortem: Some(post_mortem),
            ..
        } = event
        else {
            panic!("failed route did not retain queued commands");
        };
        assert_eq!(post_mortem.connection_id, 11);
        assert_eq!(post_mortem.packet_counter, 2);
        assert_eq!(
            post_mortem
                .packets
                .iter()
                .map(|packet| crate::transport::parse_complete_packet(packet).unwrap())
                .collect::<Vec<_>>(),
            vec![
                Some(ControlMessage::Status(first)),
                Some(ControlMessage::Status(second)),
            ]
        );
        task.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_writer_failure_preserves_fifo_across_post_mortem_fallback() {
        // A closed native connection remains the client's selected message
        // connection until the main-thread Ev_Net_Disconn handler removes it.
        // Sends accepted in that interval join the same packet log, so
        // CreatePostMortem replays A before B on the fallback connection
        // (oracle-src-pinned src/C4Network2IO.cpp:539-568,1397-1421,
        // 1451-1491; src/C4Network2.cpp:873-912).
        let (fallback_stream, fallback_peer) = duplex(4_096);
        let mut fallback = crate::ControlTransport::new(fallback_peer);
        let mut routes = ClientRouteManager::new();
        routes.add_peer_route(
            7,
            7,
            1,
            11,
            crate::NetworkProtocol::Udp,
            None,
            crate::ControlTransport::new(FailingWriteStream),
            ConnectionLivenessState::new_accepted_system(),
        );
        routes.add_peer_route(
            7,
            7,
            2,
            12,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(fallback_stream),
            ConnectionLivenessState::new_accepted_system(),
        );
        let first = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 41);
        let second = NetworkStatus::new(NETWORK_STATE_PAUSE, 1, 42);

        routes
            .try_send_to(7, ControlMessage::Status(first))
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                if routes.routes[&1].outbound.sender.is_closed() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        routes.routes[&1].outbound.retire();
        routes
            .try_send_to(7, ControlMessage::Status(second))
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, routes.read_event())
                .await
                .unwrap()
                .unwrap(),
            ClientRouteRead::Disconnected {
                peer_id: 7,
                routes_remaining: true,
                ..
            }
        ));

        let mut logical_order = Vec::new();
        while logical_order.len() < 2 {
            match await_test(fallback.read_message()).await {
                ControlMessage::PostMortem(packet) => {
                    logical_order.extend(packet.packets.into_iter().map(|packet| {
                        crate::transport::parse_complete_packet(&packet)
                            .test_value()
                            .test_value()
                    }));
                }
                message => logical_order.push(message),
            }
        }
        assert_eq!(
            logical_order,
            vec![
                ControlMessage::Status(first),
                ControlMessage::Status(second),
            ]
        );

        routes.shutdown().await;
    }

    #[tokio::test]
    async fn client_route_reads_inbound_while_its_socket_write_is_blocked() {
        // C++ appends would-block output to OBuf, then services readable
        // sockets independently before the next writable flush
        // (oracle-src-pinned src/C4NetIO.cpp:690-761,1345-1396).
        let (client_stream, peer_stream) = duplex(64);
        let mut peer = crate::ControlTransport::new(peer_stream);
        let (outbound_tx, retire_tx, mut event_rx, task) = start_test_client_route(client_stream);
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: vec![0x55; 1_024 * 1_024],
            }))
            .test_value();
        tokio::task::yield_now().await;
        let inbound = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 7);
        peer.send_message(ControlMessage::Status(inbound))
            .await
            .test_value();

        assert!(matches!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .expect("blocked output prevented full-duplex inbound progress"),
            Some(ClientRouteEvent::Packet {
                packet: crate::transport::InboundPacket::Message(ControlMessage::Status(status)),
                ..
            }) if status == inbound
        ));

        retire_tx.send_replace(true);
        timeout(EVENT_WAIT, task).await.unwrap().test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_route_reuses_its_liveness_timer_across_packets() {
        let (client_stream, peer_stream) = duplex(4_096);
        let mut peer = crate::ControlTransport::new(peer_stream);
        reset_liveness_timer_arms();
        let (_outbound_tx, retire_tx, mut event_rx, task) = start_test_client_route(client_stream);

        for target_tick in 1..=3 {
            let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, target_tick);
            peer.send_message(ControlMessage::Status(status))
                .await
                .test_value();
            assert!(matches!(
                timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
                Some(ClientRouteEvent::Packet {
                    packet: crate::transport::InboundPacket::Message(ControlMessage::Status(
                        received,
                    )),
                    ..
                }) if received == status
            ));
        }

        assert_eq!(liveness_timer_arms(), 1);

        retire_tx.send_replace(true);
        timeout(EVENT_WAIT, task).await.unwrap().test_value();
    }

    #[tokio::test]
    async fn retiring_client_route_cancels_an_inflight_write_into_post_mortem() {
        let (client_stream, mut peer_stream) = duplex(1);
        let (outbound_tx, retire_tx, mut event_rx, task) = start_test_client_route(client_stream);
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 9);
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Status(status)))
            .test_value();
        let mut first_wire_byte = [0_u8; 1];
        await_test(peer_stream.read_exact(&mut first_wire_byte)).await;
        retire_tx.send_replace(true);

        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("inflight route did not retire")
            .test_value();
        let ClientRouteEvent::Disconnected {
            post_mortem: Some(post_mortem),
            ..
        } = event
        else {
            panic!("cancelled inflight send was not retained");
        };
        assert_eq!(post_mortem.packet_counter, 1);
        assert_eq!(
            crate::transport::parse_complete_packet(&post_mortem.packets[0]).unwrap(),
            Some(ControlMessage::Status(status))
        );
        task.await.test_value();
    }

    #[test]
    fn route_ping_lag_mirrors_cpp_get_lag_branches() {
        let start = Instant::now();
        let mut lag = RoutePingLag::default();
        assert_eq!(lag.ping_ms(), -1);
        assert_eq!(lag.lag_ms(start), -1);

        // A dispatched ping without any measured RTT stays -1: getLag only
        // grows once iPingTime != -1 (src/C4Network2IO.cpp:1286).
        lag.record_ping_dispatched(start);
        assert_eq!(lag.lag_ms(start + Duration::from_secs(9)), -1);

        lag.record_pong(140);
        assert_eq!(lag.ping_ms(), 140);
        assert_eq!(
            lag.lag_ms(start + Duration::from_secs(60)),
            140,
            "an answered connection reports the last measurement"
        );

        // While a ping is unanswered the elapsed wait replaces the RTT only
        // once it grows past it (src/C4Network2IO.cpp:1287-1291).
        let sent = start + Duration::from_secs(120);
        lag.record_ping_dispatched(sent);
        assert_eq!(lag.lag_ms(sent + Duration::from_millis(40)), 140);
        assert_eq!(lag.lag_ms(sent + Duration::from_millis(141)), 141);
        assert_eq!(lag.lag_ms(sent + Duration::from_secs(5)), 5_000);

        // OnPing keeps the FIRST unanswered timestamp
        // (src/C4Network2IO.cpp:1326-1333).
        lag.record_ping_dispatched(sent + Duration::from_secs(1));
        assert_eq!(lag.lag_ms(sent + Duration::from_secs(5)), 5_000);

        lag.record_pong(90);
        assert_eq!(
            lag.lag_ms(sent + Duration::from_secs(10)),
            90,
            "the next pong answers the wait"
        );
    }

    #[tokio::test]
    async fn client_runtime_connections_follow_route_ownership_ping_and_retirement() {
        let (tcp_client, _tcp_peer) = duplex(1_024);
        let (udp_client, _udp_peer) = duplex(1_024);
        let tcp_peer_address = SocketAddr::from(([127, 0, 0, 1], 11_111));
        let udp_peer_address = SocketAddr::from(([127, 0, 0, 1], 22_222));
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            Some(tcp_peer_address),
            crate::ControlTransport::new(tcp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        routes.add_route(
            2,
            12,
            crate::NetworkProtocol::Udp,
            Some(udp_peer_address),
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        routes
            .event_tx
            .send(ClientRouteEvent::PingMeasured {
                route_id: 2,
                round_trip_ms: 37,
            })
            .test_value();
        assert!(matches!(
            routes.read_event().await.unwrap(),
            ClientRouteRead::PingMeasured {
                peer_id: HOST_CLIENT_ID,
                round_trip_ms: 37,
            }
        ));

        assert_eq!(
            routes.runtime_connections(),
            vec![
                RuntimeNetworkConnection {
                    connection_id: 1,
                    client_id: HOST_CLIENT_ID,
                    usage: "Data".to_string(),
                    protocol: crate::NetworkProtocol::Tcp,
                    peer_address: Some(tcp_peer_address),
                    packet_loss: 0,
                    ping_ms: -1,
                    lag_ms: -1,
                },
                RuntimeNetworkConnection {
                    connection_id: 2,
                    client_id: HOST_CLIENT_ID,
                    usage: "Msg".to_string(),
                    protocol: crate::NetworkProtocol::Udp,
                    peer_address: Some(udp_peer_address),
                    packet_loss: 0,
                    ping_ms: 37,
                    lag_ms: 37,
                },
            ]
        );

        // Outstanding pings feed getLag at snapshot time: route 2 reports at
        // least its measured RTT while unanswered, and route 1 stays hidden
        // because C++ getLag requires a measured iPingTime before growing
        // (src/C4Network2IO.cpp:1283-1295).
        for route_id in [1, 2] {
            routes
                .event_tx
                .send(ClientRouteEvent::PingDispatched { route_id })
                .test_value();
        }
        routes
            .event_tx
            .send(ClientRouteEvent::Packet {
                route_id: 2,
                peer_addr: Some(udp_peer_address),
                packet: crate::transport::InboundPacket::Empty,
            })
            .test_value();
        assert!(matches!(
            routes.read_event().await.unwrap(),
            ClientRouteRead::Packet { .. }
        ));
        let connections = routes.runtime_connections();
        assert_eq!(connections[0].ping_ms, -1);
        assert_eq!(
            connections[0].lag_ms, -1,
            "an unanswered ping without any measurement stays hidden"
        );
        assert_eq!(connections[1].ping_ms, 37);
        assert!(
            connections[1].lag_ms >= 37,
            "an unanswered ping reports max(elapsed, measured)"
        );

        assert!(routes.disconnect_runtime_connection(2));
        assert!(!routes.disconnect_runtime_connection(2));
        let remaining = routes.runtime_connections();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].connection_id, 1);
        assert_eq!(remaining[0].usage, "Data/Msg");
        routes.shutdown().await;
    }

    #[tokio::test(start_paused = true)]
    async fn client_routes_drive_independent_liveness_timers() {
        let (tcp_client, tcp_peer) = duplex(1024);
        let (udp_client, udp_peer) = duplex(1024);
        let mut tcp = crate::ControlTransport::new(tcp_peer);
        let mut udp = crate::ControlTransport::new(udp_peer);
        let mut routes = single_test_client_route(tcp_client);
        routes.add_route(
            2,
            12,
            crate::NetworkProtocol::Udp,
            None,
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );

        tokio::time::advance(Duration::from_millis(1_001)).await;
        tokio::task::yield_now().await;
        let tcp_ping = match timeout(EVENT_WAIT, tcp.read_message())
            .await
            .unwrap()
            .test_value()
        {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected TCP route liveness Ping, got {other:?}"),
        };
        let udp_ping = match timeout(EVENT_WAIT, udp.read_message())
            .await
            .unwrap()
            .test_value()
        {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected UDP route liveness Ping, got {other:?}"),
        };
        tcp.send_message(ControlMessage::Pong(tcp_ping))
            .await
            .test_value();
        udp.send_message(ControlMessage::Pong(udp_ping))
            .await
            .test_value();

        routes.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_client_handle_adds_udp_route_without_duplicate_join() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let tcp_address = listener.local_addr().test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let udp_address = host.udp_local_addr().test_value();
        let mut host_events = host.take_event_receiver();
        let client = connect_dual_client(
            tcp_address,
            udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();

        timeout(EVENT_WAIT, async {
            loop {
                if host.accepted_routes().await.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();

        let mut joined = 0;
        while let Ok(event) = host_events.try_recv() {
            if matches!(
                event,
                HostEvent::ClientJoined {
                    client_id,
                    ..
                } if client_id == client.client_id()
            ) {
                joined += 1;
            }
        }
        assert_eq!(joined, 1, "secondary route must not re-Join the client");

        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 0);
        client.submit_status_ack(status).await.test_value();
        timeout(EVENT_WAIT, async {
            loop {
                if matches!(
                    host_events.recv().await,
                    Some(HostEvent::StatusAck {
                        client_id,
                        status: received,
                    }) if client_id == client.client_id() && received == status
                ) {
                    break;
                }
            }
        })
        .await
        .test_value();

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_client_reconnects_a_missing_tcp_route() {
        let host_listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let host_tcp_address = host_listener.local_addr().test_value();
        let mut host = start_host(
            host_listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let host_udp_address = host.udp_local_addr().test_value();
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let proxy_address = proxy_listener.local_addr().test_value();
        let (cut_first, cut_first_rx) = oneshot::channel();
        let (first_cut, first_cut_rx) = oneshot::channel();
        let (resume_reconnect, resume_reconnect_rx) = oneshot::channel();
        let proxy = tokio::spawn(async move {
            let (mut client, _) = proxy_listener.accept().await.test_value();
            let mut host = TcpStream::connect(host_tcp_address).await.test_value();
            let first =
                tokio::spawn(
                    async move { tokio::io::copy_bidirectional(&mut client, &mut host).await },
                );
            let _ = cut_first_rx.await;
            first.abort();
            let _ = first.await;
            let _ = first_cut.send(());
            let _ = resume_reconnect_rx.await;

            let (mut client, _) = proxy_listener.accept().await.test_value();
            let mut host = TcpStream::connect(host_tcp_address).await.test_value();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut host).await;
        });
        let client = connect_dual_client(
            proxy_address,
            host_udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();
        let mut host_events = host.take_event_receiver();
        // This is a deadlock guard at native C4NetPingTimeout, not a
        // reconnect-performance assertion. Native retries are timer-driven
        // and may legitimately wait beyond five seconds
        // (oracle-src-pinned src/C4Network2Client.cpp:126-184;
        // src/C4Network2IO.cpp:1155-1182).
        let route_lifecycle_wait = Duration::from_millis(crate::PING_TIMEOUT_MS as u64);
        let initial_routes = timeout(
            route_lifecycle_wait,
            host.wait_for_accepted_routes_change(BTreeSet::new(), 2),
        )
        .await
        .test_value();
        let initial_ids = initial_routes
            .iter()
            .map(|(connection_id, _, _)| *connection_id)
            .collect::<BTreeSet<_>>();
        // Host acceptance can precede the client's HandleConnRe equivalent.
        // Wait until both host routes are locally usable before testing
        // RemoveConn fallback (oracle-src-pinned src/C4Network2.cpp:1472-1498;
        // src/C4Network2Client.cpp:90-124).
        wait_for_client_host_protocols(&client, route_lifecycle_wait).await;
        cut_first.send(()).test_value();
        // C4Network2IO reports a disconnect only after the socket has closed,
        // then C4Network2 removes that route before recovery can reconnect it
        // (oracle-src-pinned src/C4Network2IO.cpp:533-567;
        // src/C4Network2.cpp:866-905). Start the recovery deadline at that
        // same observable boundary, not when this task merely asks the proxy
        // task to cut the route.
        timeout(route_lifecycle_wait, first_cut_rx)
            .await
            .expect("proxy did not cut the initial TCP route")
            .test_value();

        let surviving_routes = timeout(
            route_lifecycle_wait,
            host.wait_for_accepted_routes_change(initial_ids.clone(), 1),
        )
        .await
        .test_value();
        assert_eq!(surviving_routes.len(), 1);

        while host_events.try_recv().is_ok() {}
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 17);
        client.submit_status_ack(status).await.test_value();
        timeout(route_lifecycle_wait, async {
            loop {
                if matches!(
                    host_events.recv().await,
                    Some(HostEvent::StatusAck {
                        client_id,
                        status: received,
                    }) if client_id == client.client_id() && received == status
                ) {
                    break;
                }
            }
        })
        .await
        .test_value();

        resume_reconnect.send(()).test_value();
        let reconnected_routes = timeout(
            route_lifecycle_wait,
            host.wait_for_accepted_routes_change(initial_ids.clone(), 2),
        )
        .await
        .test_value();
        let reconnected_ids = reconnected_routes
            .iter()
            .map(|(connection_id, _, _)| *connection_id)
            .collect::<BTreeSet<_>>();
        assert_ne!(reconnected_ids, initial_ids);
        wait_for_client_host_protocols(&client, route_lifecycle_wait).await;

        shutdown_test_session(client, host).await;
        proxy.abort();
        let _ = proxy.await;
    }

    async fn wait_for_client_host_protocols(client: &ClientHandle, deadline: Duration) {
        timeout(deadline, async {
            loop {
                let routes = client.runtime_connections().await.test_value();
                let has_host_protocol = |protocol| {
                    routes.iter().any(|route| {
                        route.client_id == HOST_CLIENT_ID && route.protocol == protocol
                    })
                };
                if has_host_protocol(crate::NetworkProtocol::Tcp)
                    && has_host_protocol(crate::NetworkProtocol::Udp)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("client did not install both accepted host routes");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_client_keeps_the_healthy_tcp_route_when_udp_is_unreachable() {
        let (tcp_address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let udp_blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .test_value();
        let client = timeout(
            Duration::from_secs(2),
            connect_dual_client(
                tcp_address,
                udp_blackhole.local_addr().test_value(),
                ClientConfig::new("Alice", ParticipantKind::Player),
            ),
        )
        .await
        .expect("optional reliable-UDP attempt stayed bounded")
        .test_value();
        assert_eq!(host.accepted_routes().await.len(), 1);

        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 0);
        client.submit_status_ack(status).await.test_value();
        timeout(EVENT_WAIT, async {
            loop {
                if matches!(
                    host_events.recv().await,
                    Some(HostEvent::StatusAck {
                        client_id,
                        status: received,
                    }) if client_id == client.client_id() && received == status
                ) {
                    break;
                }
            }
        })
        .await
        .test_value();

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_protocol_routes_prefer_udp_messages_and_tcp_resource_data() {
        let directories = SessionResourceDirectories::new();
        let source = directories.root.join("RouteSplit.c4d");
        let resource_bytes = b"resource data takes the TCP route";
        fs::write(&source, resource_bytes).test_value();
        let publication = crate::build_host_resource_core(
            &source,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Definitions,
                7,
                c4(b"RouteSplit.c4d"),
                "",
            ),
        )
        .test_value();
        let core = publication.core.clone();
        let hosted_path = publication.standalone_path.test_value();
        let hosted_ownership = publication.standalone_ownership.test_value();

        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let tcp_address = listener.local_addr().test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
            resource_directory: Some(directories.host.clone()),
            resource_registrations: vec![crate::ResourceRegistration::from_core(
                &core, true, false,
            )],
            resource_files: vec![HostedResourceFile {
                core: core.clone(),
                path: hosted_path,
                ownership: hosted_ownership,
                binary_compatible: true,
            }]),
        )
        .await
        .test_value();
        let udp_address = host.udp_local_addr().test_value();
        let mut host_events = host.take_event_receiver();
        let (mut tcp, client_id) = raw_client_transport(tcp_address, b"Alice").await;

        while host_events.try_recv().is_ok() {}
        let initial_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(initial_deadline, tcp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    tcp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => panic!("TCP route closed while draining join setup: {error}"),
            }
        }

        let udp_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let udp_stream = await_test(udp_hub.connect_owned(udp_address)).await;
        let mut udp = crate::ControlTransport::new(udp_stream);
        let host_request = loop {
            match await_test(udp.read_message()).await {
                ControlMessage::ConnectionRequest(request) => break request,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected host UDP connection request, got {other:?}"),
            }
        };
        let remote_connection_id = 37;
        let name = c4(b"Alice");
        udp.send_message(ControlMessage::ConnectionRequest(test_connection_request(
            test_client_core(i32::try_from(client_id).unwrap(), name, true),
            remote_connection_id,
            true,
        )))
        .await
        .test_value();
        loop {
            match await_test(udp.read_message()).await {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected positive host UDP connection reply, got {other:?}"),
            }
        }
        udp.send_message(ControlMessage::ConnectionReply(test_connection_reply(
            true,
            c4(b"connection accepted"),
            false,
        )))
        .await
        .test_value();

        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 2 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert!(routes.contains(&(host_request.connection_id, client_id, remote_connection_id,)));
        while let Ok(event) = host_events.try_recv() {
            assert!(
                !matches!(
                    event,
                    HostEvent::ClientJoined {
                        client_id: joined,
                        ..
                    } if joined == client_id
                ),
                "secondary reliable-UDP route emitted duplicate ClientJoined"
            );
        }

        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        let mut saw_voice_route_cookie = false;
        loop {
            match timeout_at(quiet_deadline, udp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    udp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                Ok(Ok(ControlMessage::PortCapabilities(capabilities))) => {
                    assert!(capabilities.has(crate::PortCapabilities::VOICE_CHAT));
                    assert!(capabilities.voice_cookie().is_some());
                    saw_voice_route_cookie = true;
                }
                Ok(Ok(message)) => {
                    panic!(
                        "secondary reliable-UDP route received duplicate join setup: {message:?}"
                    )
                }
                Ok(Err(error)) => panic!("reliable-UDP route closed unexpectedly: {error}"),
            }
        }
        assert!(
            saw_voice_route_cookie,
            "the admitted UDP route did not establish its media cookie reliably"
        );

        let countdown = crate::LobbyCountdownPacket::new(7);
        host.submit_lobby_countdown(countdown).await.test_value();
        loop {
            match await_test(udp.read_message()).await {
                ControlMessage::LobbyCountdown(packet) if packet == countdown => break,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected UDP lobby countdown, got {other:?}"),
            }
        }
        let tcp_quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(tcp_quiet_deadline, tcp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    tcp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                Ok(Ok(ControlMessage::LobbyCountdown(packet))) if packet == countdown => {
                    panic!("message traffic also used the TCP data route")
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => panic!("TCP route closed unexpectedly: {error}"),
            }
        }

        udp.send_message(ControlMessage::Resource(ResourcePacket::Request(
            crate::ResourceRequestPacket {
                resource_id: core.id,
                chunk: 0,
            },
        )))
        .await
        .test_value();
        loop {
            match await_test(tcp.read_message()).await {
                ControlMessage::Resource(ResourcePacket::Data(data))
                    if data.resource_id == core.id =>
                {
                    assert_eq!(data.chunk, 0);
                    assert_eq!(data.data, resource_bytes);
                    break;
                }
                ControlMessage::Ping(ping) => {
                    tcp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => {}
            }
        }
        let udp_quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(udp_quiet_deadline, udp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    udp.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                Ok(Ok(ControlMessage::Resource(ResourcePacket::Data(data))))
                    if data.resource_id == core.id =>
                {
                    panic!("resource data also used the reliable-UDP message route")
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => panic!("reliable-UDP route closed unexpectedly: {error}"),
            }
        }

        drop(udp);
        drop(tcp);
        host.shutdown().await.test_value();
    }

    #[test]
    fn loading_resource_advertises_received_chunks_for_cpp_peer_sharing() {
        // SetLoad assigns szStandalone immediately, so IsBinaryCompatible is
        // true while the file is still loading. Discovery therefore receives
        // a status containing the currently present chunk ranges
        // (src/C4Network2Res.cpp:496-523,553-567,831-845,1557-1568).
        let host = HostConfig::default();
        let core = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 8,
        chunk_size: 4);
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.dynamic = core.clone();
        let join_data = test_join_data(1, host.initial_status, snapshot);
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            std::env::temp_dir(),
        )
        .test_value();
        let mut state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            None,
        )
        .test_value();

        assert_eq!(
            state.catalog.on_packet(
                0,
                &ResourcePacket::Discover(crate::ResourceDiscoverPacket {
                    resource_ids: vec![core.id],
                }),
            ),
            vec![crate::ResourceCatalogAction::SendToPeer {
                peer_id: 0,
                packet: ResourcePacket::Status(crate::ResourceStatusPacket {
                    resource_id: core.id,
                    chunks: crate::ResourceChunkAvailability {
                        chunk_count: 2,
                        ranges: Vec::new(),
                    },
                }),
            }]
        );
    }

    #[test]
    fn post_join_resource_registration_includes_scenario_last() {
        // HandleJoinData first registers GameRes, dynamic and players. After
        // InitClient returns, C4GameParameters::InitNetwork adds Scenario;
        // C4Network2ResList::Add prepends it to discovery order
        // (src/C4Network2.cpp:329-331,1612-1620;
        // src/C4GameParameters.cpp:541-549;
        // src/C4Network2Res.cpp:1431-1441).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.dynamic = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 1,
        chunk_size: 1);
        snapshot.parameters.scenario = network_core!(resource_type: 1,
        id: 8,
        loadable: true,
        file_size: 1,
        chunk_size: 1);
        let join_data = test_join_data(1, host.initial_status, snapshot);
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            std::env::temp_dir(),
        )
        .test_value();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            None,
        )
        .test_value();

        assert_eq!(state.catalog.discovery_packet().resource_ids, vec![8, 7]);
    }

    #[test]
    fn restarted_client_forgets_stale_round_resources_and_retains_remote_player_resource() {
        let directories = SessionResourceDirectories::new();
        let stale_path = directories.root.join("stale.c4d");
        fs::write(&stale_path, b"local").test_value();
        let stale = network_core!(resource_type: 4,
        id: 900,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 5,
        filename: c4(b"Stale.c4d"));
        let retained_player = network_core!(resource_type: 3,
        id: 1 << 16,
        loadable: true,
        file_size: 5,
        chunk_size: 5,
        filename: c4(b"Remote.c4p"));
        let mut state = empty_client_resource_state(1, directories.client.clone());
        state
            .catalog
            .set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME);
        let backend = state.backend.as_mut().test_value();
        backend.set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME);
        backend
            .register_hosted_resource(
                stale.clone(),
                &stale_path,
                crate::ResourceFileOwnership::Persistent,
                true,
            )
            .test_value();
        let retained_path = backend
            .register_remote_loadable(retained_player.clone())
            .test_value();
        let mut no_random = |_| 0;
        let completion = backend
            .on_packet(
                0,
                &ResourcePacket::Data(crate::ResourceDataPacket {
                    resource_id: retained_player.id,
                    chunk: 0,
                    data: b"local".to_vec(),
                }),
                0,
                &mut no_random,
            )
            .test_value();
        assert!(completion.iter().any(|event| matches!(
            event,
            crate::ResourceTransferEvent::Completed { resource_id, .. }
                if *resource_id == retained_player.id
        )));
        assert!(!backend.is_local(retained_player.id));
        assert!(state
            .catalog
            .register(crate::ResourceRegistration::from_core(&stale, true, false,)));
        assert!(state
            .catalog
            .register(crate::ResourceRegistration::from_core(
                &retained_player,
                true,
                true,
            )));

        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.player_infos = crate::PlayerInfoListSnapshot {
            last_player_id: 1,
            clients: vec![crate::ClientPlayerInfosSnapshot {
                client_id: 1,
                flags: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(retained_player.clone()),
                    ..Default::default()
                }],
            }],
        };
        let join_data = test_join_data(1, host.initial_status, snapshot);

        state.apply_restart_join_data(join_data).test_value();

        assert!(!state.catalog.contains_resource(stale.id));
        assert!(state.catalog.contains_resource(retained_player.id));
        assert_eq!(
            state.catalog.max_loads_per_peer(),
            crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE
        );
        assert!(state
            .initial_complete_resources
            .iter()
            .any(|(core, path, local)| core == &retained_player
                && path == &retained_path
                && !local));
        let backend = state.backend.as_mut().test_value();
        assert_eq!(
            backend.catalog().max_loads_per_peer(),
            crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE
        );
        assert_eq!(backend.core(stale.id), None);
        assert_eq!(backend.core(retained_player.id), Some(&retained_player));
        assert_eq!(
            backend.path(retained_player.id),
            Some(retained_path.as_path())
        );
        let stale_request = backend
            .on_packet(
                0,
                &ResourcePacket::Request(crate::ResourceRequestPacket {
                    resource_id: stale.id,
                    chunk: 0,
                }),
                0,
                &mut no_random,
            )
            .test_value();
        assert!(stale_request.is_empty());
        assert!(stale_path.is_file());
        assert!(retained_path.is_file());
    }

    #[test]
    fn rejected_round_restart_preserves_the_live_host_state() {
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        state
            .resource_catalog
            .register(crate::ResourceRegistration {
                resource_id: 900,
                chunk_count: 1,
                binary_compatible: true,
                loading: false,
            });
        let old_title = state
            .join_snapshot
            .as_ref()
            .test_value()
            .parameters
            .title
            .clone();
        let old_tick = state.coordinator.current_tick();
        let old_client_cores = state.client_cores.clone();

        let mut invalid = HostConfig {
            start_tick: 7,
            ..HostConfig::default()
        };
        invalid
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .dynamic_tick = 8;

        assert!(install_host_round_config(invalid, &mut state).is_err());
        assert_eq!(state.coordinator.current_tick(), old_tick);
        assert_eq!(state.client_cores, old_client_cores);
        assert_eq!(
            state.join_snapshot.as_ref().test_value().parameters.title,
            old_title
        );
        assert!(state.resource_catalog.contains_resource(900));
        assert!(state.round_restart_pending_clients.is_empty());
    }

    #[test]
    fn round_restart_rejects_unequal_resource_cores_that_share_an_id() {
        let client_id = 7;
        let (outbound, _outbound_receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        let old_snapshot = state.join_snapshot.clone();
        let mut fresh = HostConfig::default();
        let snapshot = fresh.initial_join_snapshot.as_mut().test_value();
        let colliding_player = network_core!(resource_type: 3,
        id: snapshot.dynamic.id,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        contents_crc: 0x8bd6_88e8,
        chunk_size: 5,
        filename: c4(b"Retained.c4p"));
        snapshot.parameters.player_infos.clients = vec![crate::ClientPlayerInfosSnapshot {
            client_id: client_id as i32,
            flags: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(colliding_player),
                ..Default::default()
            }],
        }];

        let error = install_host_round_config(fresh, &mut state)
            .expect_err("one resource ID cannot name unequal fresh-round cores");

        assert!(
            error.contains("conflicting resource ID 1"),
            "unexpected resource collision rejection: {error}"
        );
        assert_eq!(state.join_snapshot, old_snapshot);
        assert!(state.round_restart_pending_clients.is_empty());
    }

    #[test]
    fn round_restart_rejects_resource_file_core_conflicting_with_join_data() {
        let mut fresh = HostConfig::default();
        let dynamic = fresh
            .initial_join_snapshot
            .as_ref()
            .test_value()
            .dynamic
            .clone();
        let mut conflicting = dynamic.clone();
        conflicting.filename = c4(b"Different.c4s");
        fresh.resource_directory = Some(PathBuf::from("Network"));
        fresh.resource_files = vec![HostedResourceFile {
            core: conflicting,
            path: PathBuf::from("Network/Different.c4s"),
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: true,
        }];

        let error = validate_host_round_config(&fresh)
            .expect_err("a hosted file must describe the same core published in JoinData");

        assert!(
            error.contains(&format!("resource file ID {}", dynamic.id)),
            "unexpected hosted resource conflict rejection: {error}"
        );
    }

    #[test]
    fn round_restart_installs_fresh_password_for_admission() {
        let (outbound, _outbound_receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let secret = c4(b"fresh secret");
        let fresh = host_config!(password: secret.clone());

        install_host_round_config(fresh, &mut state).test_value();

        let mut wrong = test_connection_request(compatibility_test_core(-1, b"Wrong"), 8, true);
        wrong.password = c4(b"old secret");
        assert!(matches!(
            state.admission.admit_new_peer(&wrong),
            AdmissionDecision::Reject {
                wrong_password: true,
                ..
            }
        ));
        let mut correct = test_connection_request(compatibility_test_core(-1, b"Correct"), 9, true);
        correct.password = secret;
        assert!(matches!(
            state.admission.admit_new_peer(&correct),
            AdmissionDecision::Accept { .. }
        ));
    }

    #[test]
    fn round_restart_allows_identical_resource_cores_that_share_an_id() {
        let client_id = 7;
        let (outbound, _outbound_receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        let shared_player_resource = network_core!(resource_type: 3,
        id: 77,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        contents_crc: 0x8bd6_88e8,
        chunk_size: 5,
        filename: c4(b"Shared.c4p"));
        let mut fresh = HostConfig::default();
        fresh
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .player_infos
            .clients = vec![crate::ClientPlayerInfosSnapshot {
            client_id: client_id as i32,
            flags: 0,
            players: vec![
                clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(shared_player_resource.clone()),
                    ..Default::default()
                },
                clonk_engine::ControlPlayerInfoEntry {
                    id: 2,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(shared_player_resource),
                    ..Default::default()
                },
            ],
        }];

        install_host_round_config(fresh, &mut state)
            .expect("multiple active players may reference the same resource core");
    }

    #[test]
    fn restarted_coordinator_waits_only_for_the_host_until_synchronized_activation() {
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        state.coordinator.register_client(client_id).test_value();

        install_host_round_config(HostConfig::default(), &mut state).test_value();

        let outcome = state
            .coordinator
            .ingest(legacy_packet(HOST_CLIENT_ID, 0, 0x55))
            .test_value();
        assert_eq!(outcome.ready.len(), 1);
        assert_eq!(outcome.ready[0].packets().len(), 1);
        assert_eq!(control_commands(&outcome.ready[0].packets()[0]), vec![0x55]);
        assert_eq!(
            state.round_restart_pending_clients,
            BTreeMap::from([(client_id, 1)])
        );
        assert!(!state.client_cores[&(client_id as i32)].activated);
    }

    #[test]
    fn host_round_restart_retains_only_each_clients_marker_route() {
        let client_id = 7;
        let (tcp_outbound, _tcp_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, tcp_outbound);
        let (udp_outbound, _udp_rx) = HostOutboundSender::channel();
        state.accepted_routes.insert(
            2,
            AcceptedConnectionRoute {
                client_id,
                remote_connection_id: 12,
                peer_addr: "127.0.0.1:11112".parse().test_value(),
                protocol: crate::NetworkProtocol::Udp,
                outbound: udp_outbound.clone(),
                ping: RoutePingLag::default(),
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: true,
            },
        );
        state.closed_routes.retain(3, client_id, 0);
        state.pending_post_mortems.insert(
            1,
            (
                client_id,
                crate::PostMortemPacket {
                    connection_id: 11,
                    packet_counter: 0,
                    packets: Vec::new(),
                },
                0,
            ),
        );

        let retained = prepare_host_restart_routes(&state).test_value();
        retain_host_restart_routes(&retained, &mut state);

        assert_eq!(retained, BTreeMap::from([(client_id, 2)]));
        assert_eq!(
            state.accepted_routes.keys().copied().collect::<Vec<_>>(),
            vec![2]
        );
        assert!(state.clients[&client_id]
            .outbound
            .same_channel(&udp_outbound));
        assert!(state.pending_post_mortems.is_empty());
        assert!(!state.closed_routes.contains(3));
    }

    #[test]
    fn round_restart_preflight_skips_writer_dead_route_before_disconnect_reduction() {
        let client_id = 7;
        let (dead, dead_receiver) = HostOutboundSender::channel();
        drop(dead_receiver);
        assert!(dead.writer_channel_is_closed());
        assert!(
            dead.accepts_post_failure_fifo(),
            "the failed route must still retain ordinary sends for PostMortem"
        );
        let mut state = host_state_with_test_route(client_id, dead);
        let (healthy, _healthy_receiver) = HostOutboundSender::channel();
        state.accepted_routes.insert(
            2,
            AcceptedConnectionRoute {
                client_id,
                remote_connection_id: 12,
                peer_addr: "127.0.0.1:11112".parse().test_value(),
                protocol: crate::NetworkProtocol::Tcp,
                outbound: healthy,
                ping: RoutePingLag::default(),
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: true,
            },
        );

        assert_eq!(
            prepare_host_restart_routes(&state).test_value(),
            BTreeMap::from([(client_id, 2)]),
            "restart must use the healthy route while the dead route's disconnect event is queued"
        );
    }

    #[tokio::test]
    async fn round_restart_cancels_provisional_admission_and_retains_established_clients() {
        let (outbound, _outbound_receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let connection_id = 99;
        state
            .pending_route_peers
            .insert(connection_id, "127.0.0.1:11113".parse().test_value());
        let (decision_tx, decision_rx) = oneshot::channel();

        handle_host_admission_request(
            HostAdmissionRequest {
                connection_id,
                request: test_connection_request(
                    compatibility_test_core(-1, b"Joining peer"),
                    12,
                    true,
                ),
                decision_tx,
            },
            &mut state,
        )
        .await;

        assert!(matches!(
            decision_rx.await.test_value(),
            AdmissionDecision::Accept { .. }
        ));
        assert!(state.pending_route_peers.contains_key(&connection_id));
        assert!(state.pending_route_clients.contains_key(&connection_id));
        assert!(state.pending_admissions.contains_key(&connection_id));
        let provisional_id = state.pending_admissions[&connection_id];
        let retained = prepare_host_restart_routes(&state).test_value();
        let prepared = prepare_host_round_config(HostConfig::default(), &state).test_value();
        install_prepared_host_round_config(prepared, &mut state);
        retain_host_restart_routes(&retained, &mut state);

        assert_eq!(retained, BTreeMap::from([(7, 1)]));
        assert!(state.pending_route_peers.is_empty());
        assert!(state.pending_route_clients.is_empty());
        assert!(state.pending_admissions.is_empty());
        assert!(!state.client_cores.contains_key(&provisional_id));
        assert!(state.clients.contains_key(&7));
        let retry = test_connection_request(
            compatibility_test_core(-1, b"Joining peer"),
            connection_id + 1,
            true,
        );
        assert!(matches!(
            state.admission.admit_new_peer(&retry),
            AdmissionDecision::Accept { .. }
        ));
    }

    #[tokio::test]
    async fn fresh_restart_join_data_pins_its_dynamic_for_the_retained_client() {
        let client_id = 7;
        let (outbound, mut outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);

        install_host_round_config(HostConfig::default(), &mut state).test_value();
        assert!(state.dynamic_required_clients.is_empty());

        publish_pending_join_data(&mut state).await;

        assert!(matches!(
            outbound_rx.try_recv().test_value(),
            HostOutboundMessage::Message(ControlMessage::JoinData(_))
        ));
        assert_eq!(state.dynamic_required_clients, BTreeSet::from([client_id]));
    }

    #[test]
    fn restarted_join_data_excludes_a_disconnected_client_from_a_stale_snapshot() {
        let live_client_id = 7;
        let stale_client_id = 8_i32;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(live_client_id, outbound);
        let stale_core = compatibility_test_core(stale_client_id, b"Disconnected");
        let mut fresh = HostConfig::default();
        let snapshot = fresh.initial_join_snapshot.as_mut().test_value();
        snapshot.parameters.clients.clients.push(stale_core);
        snapshot
            .parameters
            .player_infos
            .clients
            .push(crate::ClientPlayerInfosSnapshot {
                client_id: stale_client_id,
                flags: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 88,
                    ..Default::default()
                }],
            });

        install_host_round_config(fresh, &mut state).test_value();

        let snapshot = state.join_snapshot.as_ref().test_value();
        assert_eq!(
            snapshot
                .parameters
                .clients
                .clients
                .iter()
                .map(|core| core.client_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([HOST_CLIENT_ID as i32, live_client_id as i32])
        );
        assert!(snapshot
            .parameters
            .player_infos
            .clients
            .iter()
            .all(|client| client.client_id != stale_client_id));
        assert!(!state.client_cores.contains_key(&stale_client_id));
    }

    #[test]
    fn restarted_join_data_preserves_non_live_savegame_restore_rows() {
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        let restore = crate::ClientPlayerInfosSnapshot {
            client_id: -1,
            flags: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 88,
                savegame_player: 1,
                ..Default::default()
            }],
        };
        let mut fresh = HostConfig::default();
        fresh
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .restore_player_infos
            .clients = vec![restore.clone()];

        install_host_round_config(fresh, &mut state).test_value();

        assert_eq!(
            state
                .join_snapshot
                .as_ref()
                .test_value()
                .parameters
                .restore_player_infos
                .clients,
            vec![restore]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn retained_client_ingress_is_quarantined_until_its_restart_ack() {
        let client_id = 7;
        let (outbound, mut outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        let (event_tx, mut events) = mpsc::channel(16);
        state.event_tx = event_tx;
        state.round_restart_pending_clients.insert(client_id, 9);
        assert!(state
            .resource_catalog
            .register(crate::ResourceRegistration {
                resource_id: 77,
                chunk_count: 1,
                binary_compatible: true,
                loading: false,
            }));
        let old_backlog = legacy_packet(HOST_CLIENT_ID, 0, 0x31);
        state.backlog.record_packet(&old_backlog);

        for message in [
            ControlMessage::Control(legacy_packet(client_id, 0, 0x41)),
            ControlMessage::Request { from_tick: 0 },
            ControlMessage::Resource(ResourcePacket::Discover(crate::ResourceDiscoverPacket {
                resource_ids: vec![77],
            })),
            ControlMessage::ActivationRequest { tick: 0 },
        ] {
            handle_client_message_with_restart_fence(1, client_id, message, 17, &mut state).await;
        }
        assert!(outbound_rx.try_recv().is_err());
        assert!(events.try_recv().is_err());

        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::RoundRestartAck { restart_nonce: 9 },
            17,
            &mut state,
        )
        .await;
        assert!(!state.round_restart_pending_clients.contains_key(&client_id));
        while outbound_rx.try_recv().is_ok() {}

        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::Request { from_tick: 0 },
            17,
            &mut state,
        )
        .await;
        assert!(matches!(
            outbound_rx.try_recv().test_value(),
            HostOutboundMessage::Message(ControlMessage::Control(packet))
                if packet == old_backlog
        ));

        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::Resource(ResourcePacket::Discover(crate::ResourceDiscoverPacket {
                resource_ids: vec![77],
            })),
            17,
            &mut state,
        )
        .await;
        assert!(matches!(
            outbound_rx.try_recv().test_value(),
            HostOutboundMessage::Message(ControlMessage::Resource(ResourcePacket::Status(
                crate::ResourceStatusPacket {
                    resource_id: 77,
                    ..
                }
            )))
        ));

        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::ActivationRequest { tick: 0 },
            17,
            &mut state,
        )
        .await;
        assert!(matches!(
            events.recv().await,
            Some(HostEvent::ActivationRequest {
                client_id: 7,
                tick: 0,
                ping_ms: 17,
                ..
            })
        ));

        state.coordinator.register_client(client_id).test_value();
        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::Control(legacy_packet(client_id, 0, 0x42)),
            17,
            &mut state,
        )
        .await;
        ingest_control(
            legacy_packet(HOST_CLIENT_ID, 0, 0x32),
            ControlIngress::Local,
            &mut state,
        )
        .await;
        assert!(matches!(
            events.recv().await,
            Some(HostEvent::Ready { packet })
                if control_commands(&packet) == vec![0x32, 0x42]
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_ack_queued_before_restart_cannot_release_the_new_quarantine() {
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        state.round_restart_nonce = 40;
        let mut already_queued =
            VecDeque::from([ControlMessage::RoundRestartAck { restart_nonce: 40 }]);

        install_host_round_config(HostConfig::default(), &mut state).test_value();
        let message = already_queued.pop_front().test_value();
        handle_client_message_with_restart_fence(1, client_id, message, 0, &mut state).await;

        assert_eq!(
            state.round_restart_pending_clients.get(&client_id),
            Some(&41)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_the_current_restart_nonce_releases_an_active_quarantine() {
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        state.round_restart_nonce = 8;
        install_host_round_config(HostConfig::default(), &mut state).test_value();

        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::RoundRestartAck { restart_nonce: 8 },
            0,
            &mut state,
        )
        .await;
        assert_eq!(
            state.round_restart_pending_clients.get(&client_id),
            Some(&9)
        );

        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::RoundRestartAck { restart_nonce: 9 },
            0,
            &mut state,
        )
        .await;
        assert!(!state.round_restart_pending_clients.contains_key(&client_id));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restart_ack_must_return_on_the_marker_route() {
        let client_id = 7;
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        state.round_restart_pending_clients.insert(client_id, 9);
        state.round_restart_routes.insert(client_id, 2);

        handle_client_message_with_restart_fence(
            1,
            client_id,
            ControlMessage::RoundRestartAck { restart_nonce: 9 },
            0,
            &mut state,
        )
        .await;
        assert_eq!(
            state.round_restart_pending_clients.get(&client_id),
            Some(&9)
        );

        handle_client_message_with_restart_fence(
            2,
            client_id,
            ControlMessage::RoundRestartAck { restart_nonce: 9 },
            0,
            &mut state,
        )
        .await;
        assert!(!state.round_restart_pending_clients.contains_key(&client_id));
    }

    #[test]
    fn restarted_host_forgets_stale_round_resources_and_retains_remote_player_resource() {
        let directories = SessionResourceDirectories::new();
        let stale_path = directories.root.join("stale.c4d");
        fs::write(&stale_path, b"local").test_value();
        let stale = network_core!(resource_type: 4,
        id: 900,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 5,
        filename: c4(b"Stale.c4d"));
        let retained_player = network_core!(resource_type: 3,
        id: 7 << 16,
        loadable: true,
        file_size: 5,
        chunk_size: 5,
        filename: c4(b"Remote.c4p"));
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        state
            .resource_catalog
            .set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME);
        let mut backend =
            crate::ResourceTransferBackend::new(0, directories.host.clone()).test_value();
        backend.set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_IN_GAME);
        backend
            .register_hosted_resource(
                stale.clone(),
                &stale_path,
                crate::ResourceFileOwnership::Persistent,
                true,
            )
            .test_value();
        let retained_path = backend
            .register_remote_loadable(retained_player.clone())
            .test_value();
        state.resource_backend = Some(backend);
        assert!(state
            .resource_catalog
            .register(crate::ResourceRegistration::from_core(&stale, true, false),));
        assert!(state
            .resource_catalog
            .register(crate::ResourceRegistration::from_core(
                &retained_player,
                true,
                true
            ),));
        state
            .published_player_sources
            .insert(stale_path.clone(), stale.clone());

        let mut fresh_config = HostConfig::default();
        let snapshot = fresh_config.initial_join_snapshot.as_mut().test_value();
        snapshot.parameters.player_infos = crate::PlayerInfoListSnapshot {
            last_player_id: 1,
            clients: vec![crate::ClientPlayerInfosSnapshot {
                client_id: 7,
                flags: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(retained_player.clone()),
                    ..Default::default()
                }],
            }],
        };

        install_host_round_config(fresh_config, &mut state).test_value();

        assert!(!state.resource_catalog.contains_resource(stale.id));
        assert!(state.resource_catalog.contains_resource(retained_player.id));
        assert_eq!(
            state.resource_catalog.max_loads_per_peer(),
            crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE
        );
        assert!(state.published_player_sources.is_empty());
        let backend = state.resource_backend.as_mut().test_value();
        assert_eq!(
            backend.catalog().max_loads_per_peer(),
            crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE
        );
        assert_eq!(backend.core(stale.id), None);
        assert_eq!(backend.core(retained_player.id), Some(&retained_player));
        assert_eq!(
            backend.path(retained_player.id),
            Some(retained_path.as_path())
        );
        let mut no_random = |_| 0;
        let stale_request = backend
            .on_packet(
                7,
                &ResourcePacket::Request(crate::ResourceRequestPacket {
                    resource_id: stale.id,
                    chunk: 0,
                }),
                0,
                &mut no_random,
            )
            .test_value();
        assert!(stale_request.is_empty());
        assert!(stale_path.is_file());
        assert!(retained_path.is_file());
    }

    #[test]
    fn client_bootstrap_installs_an_exact_local_loadable_without_redownloading_it() {
        // SetByCore keeps a contents-identical binary-compatible local file;
        // AddByCore must not replace it with SetLoad or a Network temporary
        // (src/C4Network2Res.cpp:441-493,1473-1516).
        let directories = SessionResourceDirectories::new();
        let local_dynamic = directories.root.join("local-dynamic.c4d");
        fs::write(&local_dynamic, b"local").test_value();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 2,
        contents_crc: 0x8bd6_88e8,
        filename: c4(b"Dynamic.c4d"));
        snapshot.dynamic = core.clone();
        let join_data = test_join_data(1, host.initial_status, snapshot);
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![local_dynamic.clone()]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .test_value();

        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .test_value();

        let backend = state.backend.test_value();
        assert_eq!(backend.path(core.id), Some(local_dynamic.as_path()));
        assert_eq!(backend.core(core.id), Some(&core));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_bootstrap_reports_an_exact_local_resource_as_complete() {
        // SetByCore leaves a contents-identical resource complete with its
        // local file immediately available through getFile; AddByCore then
        // returns that complete resource without starting SetLoad
        // (pristine 9ffa0a5d src/C4Network2Res.h:238-244;
        // src/C4Network2Res.cpp:441-457,1473-1496).
        let directories = SessionResourceDirectories::new();
        let local_dynamic = directories.root.join("local-dynamic.c4d");
        fs::write(&local_dynamic, b"local").test_value();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 2,
        contents_crc: 0x8bd6_88e8,
        filename: c4(b"Dynamic.c4d"));
        snapshot.dynamic = core.clone();
        let join_data = test_join_data(1, host.initial_status, snapshot);
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![local_dynamic.clone()]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .test_value();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .test_value();
        let (_host_stream, _command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop_with_state(4096, 1, 1, BTreeMap::new(), state);

        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("local resource completion event stalled")
            .test_value();
        let ClientEvent::ResourceComplete {
            resource_id,
            core: completed_core,
            path,
            local,
        } = event
        else {
            panic!("unexpected client bootstrap event: {event:?}");
        };
        assert_eq!(resource_id, core.id);
        assert_eq!(completed_core, core);
        assert_eq!(path, local_dynamic);
        assert!(local);

        shutdown_tx.send(()).test_value();
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_resource_chunk_failure_stays_connected_and_later_completes() {
        // Once HandleJoinData registers the dynamic, resource Status and Data
        // packets run through C4Network2Res::OnStatus/OnChunk. OnChunk writes
        // the bytes before marking the chunk present and ending the load. A
        // malformed chunk is dropped without aborting the buffered packet
        // batch or disconnecting the accepted client
        // (pristine 9ffa0a5d src/C4Network2.cpp:1612-1617;
        // src/C4Network2Res.cpp:886-940,1263-1318,1571-1615).
        let directories = SessionResourceDirectories::new();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = network_core!(resource_type: 2,
        id: 7,
        derived_id: -1,
        loadable: true,
        file_size: 5,
        chunk_size: 5,
        filename: c4(b"Dynamic.c4d"));
        snapshot.dynamic = core.clone();
        let join_data = test_join_data(1, host.initial_status, snapshot);
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            directories.client.clone(),
        )
        .test_value();
        let initial_packets = vec![
            ResourcePacket::Status(crate::ResourceStatusPacket {
                resource_id: core.id,
                chunks: crate::ResourceChunkAvailability {
                    chunk_count: 1,
                    ranges: vec![crate::ResourceChunkRange {
                        start: 0,
                        length: 1,
                    }],
                },
            }),
            ResourcePacket::Data(crate::ResourceDataPacket {
                resource_id: core.id,
                chunk: 1,
                data: b"malformed".to_vec(),
            }),
            ResourcePacket::Data(crate::ResourceDataPacket {
                resource_id: core.id,
                chunk: 0,
                data: b"early".to_vec(),
            }),
        ];
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            initial_packets,
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .test_value();
        let (_host_stream, _command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop_with_state(4096, 1, 1, BTreeMap::new(), state);

        let progress = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("buffered resource progress stalled")
            .test_value();
        assert!(matches!(
            progress,
            ClientEvent::ResourceProgress {
                resource_id: 7,
                present_percent: 100,
            }
        ));
        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("buffered resource completion stalled")
            .test_value();
        let ClientEvent::ResourceComplete {
            resource_id,
            core: completed_core,
            path,
            local,
        } = event
        else {
            panic!("unexpected buffered resource event: {event:?}");
        };
        assert_eq!(resource_id, core.id);
        assert_eq!(completed_core, core);
        assert_eq!(fs::read(&path).unwrap(), b"early");
        assert!(path.is_file());
        assert!(!local);

        shutdown_tx.send(()).test_value();
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_removes_the_merged_dynamic_before_next_discovery() {
        // RetrieveScenario marks the dynamic resource removed immediately
        // after its files merge successfully; removed resources stay retained
        // but are excluded from subsequent discovery packets
        // (pristine 9ffa0a5d src/C4Network2.cpp:656-669;
        // src/C4Network2Res.cpp:825-829,1677-1688).
        let directories = SessionResourceDirectories::new();
        let local_dynamic = directories.root.join("local-dynamic.c4d");
        fs::write(&local_dynamic, b"local").test_value();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let dynamic = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 2,
        contents_crc: 0x8bd6_88e8,
        filename: c4(b"Dynamic.c4d"));
        snapshot.dynamic = dynamic.clone();
        let scenario_id = snapshot.parameters.scenario.id;
        let join_data = test_join_data(1, host.initial_status, snapshot);
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(dynamic.id, vec![local_dynamic]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .test_value();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .test_value();
        let (client_stream, host_stream) = duplex(4096);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = mpsc::channel(2);
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            control_wait_attribution: Default::default(),
            event_rx: Some(event_rx),
            voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
            voice_event_rx: Some(mpsc::channel(1).1),
            shutdown_tx: None,
            join_handle: tokio::spawn(async {}),
            client_id: 1,
            join_data: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };
        let dynamic_id = dynamic.id;
        let removal = tokio::spawn(async move { handle.remove_resource(dynamic_id).await });
        tokio::task::yield_now().await;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            state,
        ));

        removal.await.expect("resource-removal task").test_value();
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let message = timeout(EVENT_WAIT, host_transport.read_message())
            .await
            .expect("post-removal discovery stalled")
            .test_value();
        let ControlMessage::Resource(ResourcePacket::Discover(discovery)) = message else {
            panic!("unexpected post-removal message: {message:?}");
        };
        assert_eq!(discovery.resource_ids, vec![scenario_id]);

        shutdown_tx.send(()).test_value();
        client_loop.await.test_value();
    }

    #[test]
    fn client_player_publication_reuses_the_same_source_with_different_wire_metadata() {
        // LoadFromLocalFile and AddByFile search the resource list by the
        // normalized source path before allocating an ID. A hit reuses the
        // existing core even when the requested resource name or maker differs
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104;
        // src/C4Network2Res.cpp:1397-1417,1443-1471).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        let mut group = MutableGroup::new("Shared.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .test_value();
        fs::write(&player, group.pack().unwrap()).test_value();
        let mut state = empty_client_resource_state(7, directories.client.clone());
        let request = |wire_name: &[u8], maker: &[u8]| crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: c4(wire_name),
            group_maker: c4(maker),
        };

        let original = state
            .publish_player_resource(request(b"First.c4p", b"First maker"))
            .test_value();
        let reused = state
            .publish_player_resource(request(b"Second.c4p", b"Second maker"))
            .test_value();

        assert_eq!(reused, original);
        assert_eq!(state.catalog.allocate_resource_id(), (7 << 16) + 1);
    }

    #[test]
    fn client_player_publication_reuses_a_locally_resolved_bootstrap_source() {
        // Received player resources are first admitted through AddByCore. If
        // that resolves to a local file, a later AddByFile lookup by the same
        // path reuses the existing core before allocating a client resource ID
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104,275-292;
        // src/C4Network2Res.cpp:1397-1417,1443-1477).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        let mut group = MutableGroup::new("Shared.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .test_value();
        fs::write(&player, group.pack().unwrap()).test_value();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                c4(b"Shared.c4p"),
                "",
            ),
        )
        .test_value();
        let mut state = empty_client_resource_state(7, directories.client.clone());
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(publication.core.id, vec![player.clone()]);
        let resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        );
        let resource = resolver
            .resolve(
                crate::ClientBootstrapResourceRole::Player,
                &publication.core,
            )
            .test_value();
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        let reused = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: c4(b"Renamed.c4p"),
                group_maker: c4(b"Client maker"),
            })
            .test_value();

        assert_eq!(reused, publication.core);
        assert_eq!(state.catalog.allocate_resource_id(), 7 << 16);
    }

    #[test]
    fn client_bootstrap_keeps_a_nested_player_source_as_the_lookup_key() {
        // C4Group::Open retains a packed child's full mother/child name in
        // szFile. GetStandalone copies that child into a gzip-wrapped
        // temporary standalone but, unlike the directory branch, does not
        // replace szFile. AddByFile therefore still finds it by the original
        // nested path (pristine
        // 9ffa0a5d src/C4Group.cpp:656-715,1792-1816,2408-2419;
        // src/C4Network2Res.cpp:431-449,516-588,1397-1417).
        let directories = SessionResourceDirectories::new();
        let mother_path = directories.root.join("Players.c4f");
        let mut player = MutableGroup::new("Shared.c4p");
        player
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .test_value();
        let contents_crc = player.contents_crc();
        let mut mother = MutableGroup::new("Players.c4f");
        mother
            .add_child_with_metadata("Shared.c4p", player, 1, false)
            .test_value();
        fs::write(&mother_path, mother.pack().unwrap()).test_value();
        // The core describes the child image the mother actually stored,
        // gzipped. Packing the player a second time would restamp
        // Head.Creation with the current time and disagree across a second
        // boundary (src/C4Group.cpp:937-939).
        let player_standalone = compress_c4group_image(
            &Group::open(&mother_path)
                .unwrap()
                .read_file("Shared.c4p")
                .unwrap(),
        )
        .test_value();
        let nested_player = mother_path.join("Shared.c4p");
        let core = network_core!(resource_type: crate::HostResourceType::Player as u8,
        id: 1 << 16,
        derived_id: -1,
        loadable: true,
        file_size: player_standalone.len() as u32,
        file_crc: c4group_file_crc(&player_standalone),
        chunk_size: 100 * 1024,
        contents_crc,
        filename: c4(b"Players.c4f/Shared.c4p"));
        let mut state = empty_client_resource_state(7, directories.client.clone());
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![nested_player.clone()]);
        let resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        );
        let resource = resolver
            .resolve(crate::ClientBootstrapResourceRole::Player, &core)
            .test_value();
        let standalone_path = match &resource.source {
            crate::ClientBootstrapResourceSource::Local(local) => {
                assert!(local.binary_compatible());
                assert_eq!(local.source_path(), nested_player);
                assert_ne!(local.path(), nested_player);
                local.path().to_path_buf()
            }
            source => panic!("expected a local packed child, got {source:?}"),
        };
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        assert_eq!(
            state.local_resource_sources.get(&nested_player),
            Some(&core)
        );
        assert!(!state.local_resource_sources.contains_key(&standalone_path));
    }

    #[test]
    fn client_bootstrap_does_not_reuse_the_original_player_directory_path() {
        // SetByCore packs a directory and replaces szFile with the temporary
        // standalone before checking physical compatibility. Therefore a
        // later AddByFile of the original directory path does not find that
        // resource and allocates a new client ID (pristine 9ffa0a5d
        // src/C4Network2Res.cpp:431-449,516-588,1397-1417,1443-1477).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        fs::create_dir(&player).test_value();
        fs::write(player.join("Player.txt"), b"player core").test_value();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                c4(b"Shared.c4p"),
                "Host maker",
            ),
        )
        .test_value();
        let mut state = empty_client_resource_state(7, directories.client.clone());
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(publication.core.id, vec![player.clone()]);
        let resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        );
        let resource = resolver
            .resolve(
                crate::ClientBootstrapResourceRole::Player,
                &publication.core,
            )
            .test_value();
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        let published = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: c4(b"Shared.c4p"),
                group_maker: c4(b"Client maker"),
            })
            .test_value();

        assert_ne!(published, publication.core);
        assert_eq!(published.id, 7 << 16);
    }

    #[test]
    fn client_player_publication_reuses_an_authoritative_local_player_source() {
        // HandlePlayerInfo immediately loads each received player resource via
        // AddByCore. If that resolves to a local file, a later AddByFile of
        // the same path reuses the core before allocating a client resource ID
        // (pristine 9ffa0a5d src/C4Network2Players.cpp:245-260;
        // src/C4PlayerInfo.cpp:70-104,275-292;
        // src/C4Network2Res.cpp:1397-1417,1443-1477).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        let mut group = MutableGroup::new("Shared.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .test_value();
        fs::write(&player, group.pack().unwrap()).test_value();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                c4(b"Shared.c4p"),
                "",
            ),
        )
        .test_value();
        let mut state = empty_client_resource_state(7, directories.client.clone());
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(publication.core.id, vec![player.clone()]);
        state.retain_resource_resolver(crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        ));
        let mut info = clonk_engine::PlayerInfoControlData {
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(publication.core.clone()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let completed = state.load_authoritative_player_resources(&mut info);
        assert_eq!(completed, vec![(player.clone(), publication.core.clone())]);

        let reused = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: c4(b"Renamed.c4p"),
                group_maker: c4(b"Client maker"),
            })
            .test_value();

        assert_eq!(reused, publication.core);
        assert_eq!(state.catalog.allocate_resource_id(), 7 << 16);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_handle_publishes_the_selected_player_into_both_resource_registries() {
        // After SetLocalID, AddByFile allocates from the assigned client's
        // high-word namespace. NRT_Player publication protects the persistent
        // source with a temporary copy before OptimizeStandalone, and Add
        // makes that complete file visible to discovery and chunk requests
        // (pristine 9ffa0a5d src/C4Network2Res.cpp:1168-1205,1361-1385,
        // 1431-1471; src/C4PlayerInfo.cpp:70-104).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .test_value();
        group
            .add_file_with_metadata("Portrait.png", b"portrait".to_vec(), 2, false)
            .test_value();
        let original = group.pack().test_value();
        fs::write(&player, &original).test_value();
        let request = crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: c4(b"Players.c4f/Alice.c4p"),
            group_maker: c4(b"Alice"),
        };
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = test_join_data(7, host.initial_status, snapshot);

        let direct_directory = directories.root.join("direct");
        let mut direct_state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(direct_directory),
        )
        .test_value();
        let direct_core = direct_state
            .publish_player_resource(request.clone())
            .test_value();
        assert_eq!(direct_core.id, 7 << 16);
        assert!(direct_state.catalog.contains_resource(direct_core.id));
        let direct_backend = direct_state.backend.as_ref().test_value();
        assert_eq!(direct_backend.core(direct_core.id), Some(&direct_core));
        assert!(direct_backend.path(direct_core.id).unwrap().is_file());

        let loop_directory = directories.root.join("loop");
        let resource_state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(loop_directory),
        )
        .test_value();
        let (host_stream, command_tx, event_rx, shutdown_tx, join_handle) =
            start_test_client_loop_with_state(4096, 4, 4, BTreeMap::new(), resource_state);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            control_wait_attribution: Default::default(),
            event_rx: Some(event_rx),
            voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
            voice_event_rx: Some(mpsc::channel(1).1),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            client_id: 7,
            join_data: Some(join_data),
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        let core = handle.publish_player_resource(request).await.test_value();
        assert_eq!(core.id, 7 << 16);
        assert_eq!(core.resource_type, crate::HostResourceType::Player as u8);
        assert_eq!(fs::read(&player).unwrap(), original);

        host_transport
            .send_message(ControlMessage::Resource(ResourcePacket::Discover(
                crate::ResourceDiscoverPacket {
                    resource_ids: vec![core.id],
                },
            )))
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .test_value()
            {
                ControlMessage::Resource(ResourcePacket::Status(status))
                    if status.resource_id == core.id =>
                {
                    assert_eq!(status.chunks.ranges[0].start, 0);
                    break;
                }
                ControlMessage::Ping(ping) => {
                    host_transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => {}
            }
        }
        host_transport
            .send_message(ControlMessage::Resource(ResourcePacket::Request(
                crate::ResourceRequestPacket {
                    resource_id: core.id,
                    chunk: 0,
                },
            )))
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .test_value()
            {
                ControlMessage::Resource(ResourcePacket::Data(data))
                    if data.resource_id == core.id =>
                {
                    assert_eq!(data.chunk, 0);
                    assert!(!data.data.is_empty());
                    break;
                }
                ControlMessage::Ping(ping) => {
                    host_transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => {}
            }
        }

        handle.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_handle_reuses_initial_and_serves_runtime_player_resources() {
        // LoadFromLocalFile searches the entire local resource list by source
        // path before AddByFile, including players published during InitHost.
        // A miss registers the new NRT_Player so an already-connected peer can
        // discover its complete chunks and ask for their bytes (pristine
        // 9ffa0a5d src/C4PlayerInfo.cpp:91-104; src/C4Network2Res.cpp:831-865,
        // 1168-1205,1431-1471,1557-1615).
        let directories = SessionResourceDirectories::new();
        let initial_player = directories.root.join("HostInitial.c4p");
        let mut initial_group = MutableGroup::new("HostInitial.c4p");
        initial_group
            .add_file_with_metadata("Player.txt", b"host initial player".to_vec(), 1, false)
            .test_value();
        fs::write(&initial_player, initial_group.pack().unwrap()).test_value();
        let initial_wire = c4(b"HostInitial.c4p");
        let maker = c4(b"Host");
        let initial_request = crate::ClientPlayerResourceRequest {
            source_path: initial_player.clone(),
            wire_name: initial_wire.clone(),
            group_maker: maker.clone(),
        };
        let initial_publication =
            crate::publish_client_player_resource(crate::ClientPlayerResourcePublicationSpec {
                resource_id: 0,
                source_path: initial_player.clone(),
                wire_name: initial_wire,
                network_directory: directories.host.clone(),
                group_maker: maker.clone(),
            })
            .test_value();
        let initial_core = initial_publication.core.clone();

        let player = directories.root.join("HostRuntime.c4p");
        let mut group = MutableGroup::new("HostRuntime.c4p");
        group
            .add_file_with_metadata("Player.txt", b"host runtime player".to_vec(), 1, false)
            .test_value();
        let original = group.pack().test_value();
        fs::write(&player, &original).test_value();
        let publication = crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: c4(b"HostRuntime.c4p"),
            group_maker: maker,
        };

        let (address, listener) = bind_test_listener().await;
        let host = start_host(
            listener,
            host_config!(resource_registrations: vec![initial_publication.registration],
            resource_directory: Some(directories.host.clone()),
            resource_files: vec![initial_publication.resource_file],
            player_resource_sources: vec![(initial_player, initial_core.clone())]),
        )
        .await
        .test_value();
        let stream = TcpStream::connect(address).await.test_value();
        let mut peer = crate::ControlTransport::new(stream);
        let peer_name = c4(b"Peer");
        run_client_connection_handshake(
            &mut peer,
            test_connection_request(
                clonk_engine::ClientCoreControlData {
                    client_id: -1,
                    name: peer_name.clone(),
                    nick: peer_name,
                    ..Default::default()
                },
                0,
                false,
            ),
        )
        .await
        .test_value();

        assert_eq!(
            host.publish_player_resource(initial_request).await.unwrap(),
            initial_core,
            "an InitHost player source reuses its existing core"
        );
        let core = host
            .publish_player_resource(publication.clone())
            .await
            .test_value();
        let reused = host.publish_player_resource(publication).await.test_value();
        assert_eq!(reused, core, "the same source path reuses one resource");
        assert_eq!(core.id, 1);
        assert_eq!(core.resource_type, crate::HostResourceType::Player as u8);
        assert_eq!(fs::read(&player).unwrap(), original);

        peer.send_message(ControlMessage::Resource(ResourcePacket::Discover(
            crate::ResourceDiscoverPacket {
                resource_ids: vec![core.id],
            },
        )))
        .await
        .test_value();
        loop {
            match await_test(peer.read_message()).await {
                ControlMessage::Resource(ResourcePacket::Status(status))
                    if status.resource_id == core.id =>
                {
                    assert_eq!(status.chunks.ranges[0].start, 0);
                    break;
                }
                ControlMessage::Ping(ping) => {
                    peer.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => {}
            }
        }
        peer.send_message(ControlMessage::Resource(ResourcePacket::Request(
            crate::ResourceRequestPacket {
                resource_id: core.id,
                chunk: 0,
            },
        )))
        .await
        .test_value();
        loop {
            match await_test(peer.read_message()).await {
                ControlMessage::Resource(ResourcePacket::Data(data))
                    if data.resource_id == core.id =>
                {
                    assert_eq!(data.chunk, 0);
                    assert!(!data.data.is_empty());
                    break;
                }
                ControlMessage::Ping(ping) => {
                    peer.send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => {}
            }
        }

        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authoritative_player_info_loads_remote_resources_and_preserves_cpp_flag_rules() {
        // HandlePlayerInfo merges the authoritative list and immediately calls
        // LoadResources. Each eligible PIF_HasRes entry uses AddByCore(true):
        // an existing ID is reused, an identical local file wins, otherwise a
        // loadable core starts a download. Removed entries are untouched;
        // InScenario and unavailable non-loadable entries lose HasResource
        // locally (pristine 9ffa0a5d src/C4Network2Players.cpp:245-260;
        // src/C4PlayerInfo.cpp:275-292; src/C4Network2Res.cpp:1473-1516).
        let directories = SessionResourceDirectories::new();
        let source = directories.root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .test_value();
        fs::write(&source, group.pack().unwrap()).test_value();
        let publication = crate::build_host_resource_core(
            &source,
            directories.root.join("published"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                c4(b"Alice.c4p"),
                "Host",
            ),
        )
        .test_value();
        let valid_core = publication.core.clone();
        let hosted_path = publication.standalone_path.test_value();
        let mut removed_core = valid_core.clone();
        removed_core.id += 1;
        let mut scenario_core = valid_core.clone();
        scenario_core.id += 2;
        let mut nonloadable_core = valid_core.clone();
        nonloadable_core.id += 3;
        nonloadable_core.loadable = false;
        nonloadable_core.file_size = u32::MAX;
        nonloadable_core.file_crc = u32::MAX;

        let local_host = HostConfig::default();
        let local_snapshot = synthetic_join_snapshot(local_host.local_core, 8);
        let local_join_data = test_join_data(2, local_host.initial_status, local_snapshot);
        let local_work_path = directories.root.join("client-local");
        let mut local_state = ClientResourceState::new(
            &local_join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(local_work_path.clone()),
        )
        .test_value();
        let mut local_candidates = crate::ClientBootstrapLocalCandidates::default();
        local_candidates.extend_search_roots([directories.root.clone()]);
        local_state.retain_resource_resolver(
            crate::client_bootstrap::ClientBootstrapResolver::new(
                &local_candidates,
                local_work_path.clone(),
            ),
        );
        let mut local_info = clonk_engine::PlayerInfoControlData {
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(valid_core.clone()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let local_sources = local_state.load_authoritative_player_resources(&mut local_info);
        assert_eq!(local_sources, vec![(source.clone(), valid_core.clone())]);
        assert!(local_state.catalog.contains_resource(valid_core.id));
        let local_backend = local_state.backend.as_ref().test_value();
        assert_eq!(local_backend.core(valid_core.id), Some(&valid_core));
        let local_standalone = local_backend.path(valid_core.id).test_value();
        assert_ne!(local_standalone, source);
        assert_eq!(local_standalone.parent(), Some(local_work_path.as_path()));
        assert_eq!(
            fs::read(local_standalone).unwrap(),
            fs::read(&source).unwrap()
        );
        assert!(local_backend
            .catalog()
            .local_chunks(valid_core.id)
            .unwrap()
            .is_complete());

        let (address, listener) = bind_test_listener().await;
        let host_config = host_config!(resource_directory: Some(directories.host.clone()),
        resource_registrations: vec![crate::ResourceRegistration::from_core(
            &valid_core,
            true,
            false,
        )],
        resource_files: vec![HostedResourceFile {
            core: valid_core.clone(),
            path: hosted_path,
            ownership: crate::ResourceFileOwnership::Temporary,
            binary_compatible: true,
        }]);
        let host = start_host(listener, host_config).await.test_value();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .test_value();
        let mut client_events = client.take_event_receiver();

        let resource_player = |id: i32, flags: u16, core: clonk_engine::NetworkResourceCore| {
            clonk_engine::ControlPlayerInfoEntry {
                id,
                flags,
                resource: Some(core),
                ..Default::default()
            }
        };
        let info = clonk_engine::PlayerInfoControlData::new(
            1,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![
                resource_player(
                    1,
                    clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    valid_core.clone(),
                ),
                resource_player(
                    2,
                    clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE
                        | clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                    removed_core.clone(),
                ),
                resource_player(
                    3,
                    clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE
                        | clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE,
                    scenario_core,
                ),
                resource_player(
                    4,
                    clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    nonloadable_core,
                ),
            ],
            0,
        );
        let encoded = crate::encode_control_entry_payload(
            &clonk_engine::ControlPacket::PlayerInfo(info.clone()),
        )
        .test_value();
        host.submit_packet(ControlDelivery::Direct, encoded.clone())
            .await
            .test_value();

        let mut delivered = None;
        let mut completed = None;
        while delivered.is_none() || completed.is_none() {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Direct { data, .. }) => {
                    if let Ok(clonk_engine::ControlPacket::PlayerInfo(actual)) =
                        decode_control_entry_payload(&data)
                    {
                        delivered = Some(actual);
                    }
                }
                Some(ClientEvent::ResourceComplete {
                    resource_id,
                    core,
                    path,
                    local,
                }) if resource_id == valid_core.id => {
                    completed = Some((core, path, local));
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client disconnected while loading PlayerInfo resource: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }
        let delivered = delivered.test_value();
        assert_ne!(
            delivered.players[0].flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
            0
        );
        assert_ne!(
            delivered.players[1].flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
            0,
            "removed players return before LoadResource mutates their flags"
        );
        for player in &delivered.players[2..] {
            assert_eq!(
                player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                0
            );
            assert_eq!(player.resource, None);
        }
        let (completed_core, completed_path, local) = completed.test_value();
        assert_eq!(completed_core, valid_core);
        assert!(completed_path.is_file());
        assert!(!local);

        host.submit_packet(ControlDelivery::Direct, encoded)
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(clonk_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    break;
                }
                Some(ClientEvent::ResourceComplete { resource_id, .. })
                    if resource_id == valid_core.id =>
                {
                    panic!("an already-registered PlayerInfo resource restarted its download");
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("duplicate PlayerInfo disconnected client: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_resolves_authoritative_player_resource_before_direct_broadcast() {
        // The host executes CID_PlrInfo locally as a direct control before
        // peers consume it. HandlePlayerInfo calls LoadResources there, and
        // AddByCore first searches for an identical local file before falling
        // back to AddLoad. A later AddByFile of that path reuses the resolved
        // resource before allocating a new host ID (pristine 9ffa0a5d
        // src/C4Network2Players.cpp:245-260;
        // src/C4PlayerInfo.cpp:70-104,275-292;
        // src/C4Network2Res.cpp:1397-1417,1443-1516).
        let directories = SessionResourceDirectories::new();
        let local_root = directories.root.join("local");
        fs::create_dir_all(&local_root).test_value();
        let source = local_root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"host-local player".to_vec(), 1, false)
            .test_value();
        fs::write(&source, group.pack().unwrap()).test_value();
        let core = crate::build_host_resource_core(
            &source,
            directories.root.join("core"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                c4(b"Alice.c4p"),
                "Host",
            ),
        )
        .test_value()
        .core;

        let (address, listener) = bind_test_listener().await;
        let host_config = host_config!(resource_directory: Some(directories.host.clone()),
        local_resource_roots: vec![local_root]);
        let mut host = start_host(listener, host_config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .test_value();
        let mut client_events = client.take_event_receiver();
        let info = clonk_engine::PlayerInfoControlData::new(
            1,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            0,
        );
        host.submit_packet(
            ControlDelivery::Direct,
            crate::encode_control_entry_payload(&clonk_engine::ControlPacket::PlayerInfo(info))
                .unwrap(),
        )
        .await
        .test_value();

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ResourceComplete {
                    resource_id,
                    core: completed,
                    path,
                    local,
                }) if resource_id == core.id => {
                    assert_eq!(completed, core);
                    assert_eq!(path, source);
                    assert!(local);
                    break;
                }
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("host could not resolve local PlayerInfo resource: {error}");
                }
                Some(HostEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(clonk_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    panic!("host exposed PlayerInfo before its local resource completion");
                }
                Some(_) => {}
                None => panic!("host event stream ended"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(clonk_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    break;
                }
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("host could not expose local PlayerInfo: {error}");
                }
                Some(_) => {}
                None => panic!("host event stream ended"),
            }
        }

        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::ResourceComplete {
                    resource_id,
                    core: completed,
                    path,
                    local,
                }) if resource_id == core.id => {
                    assert_eq!(completed, core);
                    assert!(path.is_file());
                    assert!(!local);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("host could not serve its local PlayerInfo resource: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }

        let reused = host
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: source,
                wire_name: c4(b"Renamed.c4p"),
                group_maker: c4(b"Host maker"),
            })
            .await
            .test_value();
        assert_eq!(
            reused, core,
            "AddByFile reuses the locally resolved authoritative resource"
        );

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_resolves_local_player_resource_before_exposing_direct_control() {
        let directories = SessionResourceDirectories::new();
        let host_root = directories.root.join("host-local");
        let client_root = directories.root.join("client-local");
        fs::create_dir_all(&host_root).test_value();
        fs::create_dir_all(&client_root).test_value();
        let host_source = host_root.join("Alice.c4p");
        let client_source = client_root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"shared local player".to_vec(), 1, false)
            .test_value();
        let player_bytes = group.pack().test_value();
        fs::write(&host_source, &player_bytes).test_value();
        fs::write(&client_source, player_bytes).test_value();
        let core = crate::build_host_resource_core(
            &host_source,
            directories.root.join("core"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                c4(b"Alice.c4p"),
                "Host",
            ),
        )
        .test_value()
        .core;

        let (address, listener) = bind_test_listener().await;
        let host_config = host_config!(resource_directory: Some(directories.host.clone()),
        local_resource_roots: vec![host_root]);
        let host = start_host(listener, host_config).await.test_value();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_resource_roots([client_root]),
        )
        .await
        .test_value();
        let mut client_events = client.take_event_receiver();
        let info = clonk_engine::PlayerInfoControlData::new(
            1,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            0,
        );
        host.submit_packet(
            ControlDelivery::Direct,
            crate::encode_control_entry_payload(&clonk_engine::ControlPacket::PlayerInfo(info))
                .unwrap(),
        )
        .await
        .test_value();

        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::ResourceComplete {
                    resource_id,
                    core: completed,
                    path,
                    local,
                }) if resource_id == core.id => {
                    assert_eq!(completed, core);
                    assert_eq!(path, client_source);
                    assert!(local);
                    break;
                }
                Some(ClientEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(clonk_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    panic!("client exposed PlayerInfo before its local resource completion");
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client could not resolve local PlayerInfo resource: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(clonk_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client could not expose local PlayerInfo: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn default_client_config_transfers_a_cpp_resource_file_to_completion() {
        // C4Network2ResList handles Dis/Stat/Req/Data inside the network
        // session: OnStatus starts one request, SendChunk reads the standalone,
        // and OnChunk writes/refills until OnResComplete fires. ResList is
        // always initialized even when a caller does not override WorkPath
        // (src/C4Network2.cpp:358-362;
        // src/C4Network2Res.cpp:831-940,1017-1122,1546-1620).
        let directories = SessionResourceDirectories::new();
        let source = directories.host.join("Dynamic.c4d");
        fs::write(&source, b"local").test_value();
        let core = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 2,
        filename: c4(b"Dynamic.c4d"));
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = core.clone();
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![HostedResourceFile {
            core: core.clone(),
            path: source,
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: true,
        }];

        let (address, host) = start_test_host(host_config).await;
        let mut client = connect_test_player(address, "Alice").await;

        let mut progress = Vec::new();
        let completed_path = loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("resource transfer stalled")
                .test_value()
            {
                ClientEvent::ResourceComplete {
                    resource_id,
                    core: completed_core,
                    path,
                    local,
                } => {
                    assert_eq!(resource_id, core.id);
                    assert_eq!(completed_core, core);
                    assert!(!local);
                    break path;
                }
                ClientEvent::ResourceProgress {
                    resource_id,
                    present_percent,
                } => {
                    assert_eq!(resource_id, core.id);
                    progress.push(present_percent);
                }
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected during resource transfer: {reason:?}")
                }
                _ => continue,
            }
        };

        assert_eq!(progress, vec![33, 66, 100]);
        assert_eq!(fs::read(&completed_path).unwrap(), b"local");
        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_and_client_finish_a_derived_resource_without_redownloading_it() {
        // C4Player::Save calls Derive before replacing the file. The control
        // host then calls FinishDerive and every peer with a matching anonymous
        // resource adopts the new core with complete chunks
        // (src/C4Player.cpp:452-461; src/C4Network2Res.cpp:718-823,1584-1594).
        let directories = SessionResourceDirectories::new();
        let host_source = directories.host.join("Dynamic.c4d");
        fs::write(&host_source, b"local").test_value();
        let parent = network_core!(resource_type: crate::HostResourceType::Dynamic as u8,
        id: 7,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 2,
        filename: c4(b"Dynamic.c4d"));
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = parent.clone();
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_registrations =
            vec![crate::ResourceRegistration::from_core(&parent, true, false)];
        host_config.resource_files = vec![HostedResourceFile {
            core: parent.clone(),
            path: host_source.clone(),
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: true,
        }];

        let (address, mut host) = start_test_host(host_config).await;
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .test_value();

        let client_source = loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("parent resource transfer stalled")
                .test_value()
            {
                ClientEvent::ResourceComplete {
                    resource_id, path, ..
                } if resource_id == parent.id => break path,
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected while loading parent: {reason:?}")
                }
                _ => continue,
            }
        };

        let host_derivation = host
            .begin_resource_derive(
                parent.id,
                host_source.clone(),
                crate::ResourceFileOwnership::Persistent,
            )
            .await
            .test_value();
        let _client_derivation = client
            .begin_resource_derive(
                parent.id,
                client_source.clone(),
                crate::ResourceFileOwnership::Temporary,
            )
            .await
            .test_value();
        fs::write(&host_source, b"changed").test_value();
        fs::write(&client_source, b"changed").test_value();

        let derived = host
            .finish_resource_derive(host_derivation)
            .await
            .test_value();
        assert_ne!(derived.id, parent.id);
        assert_eq!(derived.derived_id, parent.id);
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host derive completion stalled")
                .test_value()
            {
                HostEvent::ResourceComplete {
                    resource_id,
                    core,
                    path,
                    local,
                } if resource_id == derived.id => {
                    assert_eq!(core, derived);
                    assert_eq!(path, host_source);
                    assert!(local);
                    break;
                }
                _ => continue,
            }
        }
        let completed_path = loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("derive announcement stalled")
                .test_value()
            {
                ClientEvent::ResourceComplete {
                    resource_id,
                    core,
                    path,
                    local,
                } if resource_id == derived.id => {
                    assert_eq!(core, derived);
                    assert!(!local);
                    break path;
                }
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected during derivation: {reason:?}")
                }
                _ => continue,
            }
        };
        assert_eq!(completed_path, client_source);
        assert_eq!(fs::read(completed_path).unwrap(), b"changed");

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_rejects_an_unmatched_required_nonloadable_system_before_lobby() {
        // InitClient reruns GameRes.InitNetwork after HandleJoinData and fails
        // before Control.InitNetwork, Players.Init, or DoLobby when a required
        // non-loadable System core has no contents-identical local candidate
        // (src/C4Network2.cpp:281-344; src/C4GameParameters.cpp:125-160;
        // src/C4Network2Res.cpp:441-493,1473-1516).
        let directories = SessionResourceDirectories::new();
        let system_path = directories.host.join("System.c4g");
        let mismatched_system_path = directories.client.join("System.c4g");
        fs::write(&system_path, b"host system").test_value();
        fs::write(&mismatched_system_path, b"different client system").test_value();
        let publication = crate::build_host_resource_core(
            &system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                c4(b"System.c4g"),
                "Test host",
            ),
        )
        .test_value();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 1,
        file_crc: 1,
        contents_crc: 1,
        filename: c4(b"Dynamic.c4d"));
        snapshot.parameters.scenario = network_core!(resource_type: 1,
        id: 8,
        loadable: true,
        file_size: 1,
        file_crc: 1,
        contents_crc: 1,
        filename: c4(b"Scenario.c4s"));
        snapshot
            .parameters
            .game_resources
            .push(publication.core.clone());
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![HostedResourceFile {
            core: publication.core,
            path: system_path,
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: false,
        }];

        let (address, host) = start_test_host(host_config).await;
        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(mismatched_system_path),
        )
        .await;
        host.shutdown().await.test_value();

        let error = result.expect_err("client must fail before entering the lobby");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("System.c4g") && message.contains("non-loadable")),
            "unexpected client bootstrap failure: {error:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cpp_interop_refuses_an_explicit_local_system_that_differs() {
        // An explicitly trusted path does not make a differing group the
        // host's group. C++ gates admission on ContentsCRC first: AddLoad
        // refuses the non-loadable core and the join dies with "System file
        // System.c4g differs from that used by the host!"
        // (src/C4Network2Res.cpp:1473-1507). It executes its process-local
        // Application.SystemGroup only *after* that check passes
        // (src/C4Application.cpp:127-134; src/C4Game.cpp:2764-2793), so
        // trusting a differing local group joins a round whose System scripts
        // disagree with the host's -- observed against the pinned oracle in
        // clonk-org/clonk-rs#1053.
        let directories = SessionResourceDirectories::new();
        let host_system_path = directories.host.join("System.c4g");
        let client_system_path = directories.client.join("System.c4g");
        fs::create_dir(&host_system_path).test_value();
        fs::create_dir(&client_system_path).test_value();
        fs::write(host_system_path.join("Host.c"), b"C++ host system").test_value();
        fs::write(client_system_path.join("Client.c"), b"Rust client system").test_value();
        let publication = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                2,
                c4(b"System.c4g"),
                "C++ host",
            ),
        )
        .test_value();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic.id = 3;
        snapshot.parameters.scenario.id = 0;
        snapshot
            .parameters
            .game_resources
            .push(publication.core.clone());
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![HostedResourceFile {
            core: publication.core.clone(),
            path: host_system_path,
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: false,
        }];

        let (address, host) = start_test_host(host_config).await;
        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_trusted_local_system_path(client_system_path.clone()),
        )
        .await;
        host.shutdown().await.test_value();

        let error = result.expect_err("a differing System must fail before the lobby");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("System.c4g") && message.contains("non-loadable")),
            "unexpected client bootstrap failure: {error:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_rejects_nonloadable_dynamic_when_game_resources_are_empty() {
        // HandleJoinData requires ResDynamic independently of GameRes. A
        // non-loadable dynamic core with no contents-identical local file
        // clears the client after control initialization but before DoLobby
        // (src/C4Network2.cpp:1574-1618).
        let directories = SessionResourceDirectories::new();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = network_core!(resource_type: 2,
        id: 7,
        loadable: false,
        file_size: u32::MAX,
        file_crc: u32::MAX,
        contents_crc: 1,
        filename: c4(b"Dynamic.c4d"));
        snapshot.parameters.scenario = network_core!(resource_type: 1,
        id: 8,
        loadable: true,
        file_size: 1,
        file_crc: 1,
        contents_crc: 1,
        filename: c4(b"Scenario.c4s"));
        assert!(snapshot.parameters.game_resources.is_empty());
        host_config.initial_join_snapshot = Some(snapshot);

        let (address, host) = start_test_host(host_config).await;
        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await;
        host.shutdown().await.test_value();

        let error = result.expect_err("missing non-loadable dynamic must abort bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Dynamic.c4d") && message.contains("non-loadable")),
            "unexpected client bootstrap failure: {error:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_accepts_a_contents_identical_local_nonloadable_system() {
        // SetByCore accepts a contents-identical local System even though its
        // non-loadable core has no transferable standalone; InitClient may
        // then continue into control/player initialization and the lobby
        // (src/C4Network2Res.cpp:441-493,1473-1516;
        // src/C4Network2.cpp:329-344).
        let directories = SessionResourceDirectories::new();
        let system_bytes = b"shared system";
        let host_system_path = directories.host.join("System.c4g");
        let client_system_path = directories.client.join("System.c4g");
        fs::write(&host_system_path, system_bytes).test_value();
        fs::write(&client_system_path, system_bytes).test_value();
        let publication = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                c4(b"System.c4g"),
                "Test host",
            ),
        )
        .test_value();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = network_core!(resource_type: 2,
        id: 7,
        loadable: true,
        file_size: 1,
        file_crc: 1,
        contents_crc: 1,
        filename: c4(b"Dynamic.c4d"));
        snapshot.parameters.scenario = network_core!(resource_type: 1,
        id: 8,
        loadable: true,
        file_size: 1,
        file_crc: 1,
        contents_crc: 1,
        filename: c4(b"Scenario.c4s"));
        snapshot
            .parameters
            .game_resources
            .push(publication.core.clone());
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![HostedResourceFile {
            core: publication.core,
            path: host_system_path,
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: false,
        }];
        let (address, host) = start_test_host(host_config).await;
        let client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(client_system_path),
        )
        .await
        .test_value();

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_search_roots_accept_contents_identical_nonloadable_definitions() {
        // SetByCore searches the executable roots for every core, not only
        // System. An over-limit Definitions resource remains non-loadable but
        // is accepted when a local Objects.c4d has the same contents CRC
        // (src/C4Network2Res.cpp:441-493,1443-1516;
        // src/C4GameParameters.cpp:125-160).
        let directories = SessionResourceDirectories::new();
        let system_bytes = b"shared system";
        let definitions_bytes = b"shared definitions";
        let host_system_path = directories.host.join("System.c4g");
        let client_system_path = directories.client.join("System.c4g");
        let host_definitions_path = directories.host.join("Objects.c4d");
        let client_definitions_path = directories.client.join("Objects.c4d");
        fs::write(&host_system_path, system_bytes).test_value();
        fs::write(&client_system_path, system_bytes).test_value();
        fs::write(&host_definitions_path, definitions_bytes).test_value();
        fs::write(&client_definitions_path, definitions_bytes).test_value();
        let system = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                c4(b"System.c4g"),
                "Test host",
            ),
        )
        .test_value();
        let mut definitions = crate::build_host_resource_core(
            &host_definitions_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                10,
                c4(b"Objects.c4d"),
                "Test host",
            ),
        )
        .test_value();
        definitions.core.resource_type = crate::HostResourceType::Definitions as u8;

        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.parameters.game_resources = vec![system.core.clone(), definitions.core.clone()];
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![
            HostedResourceFile {
                core: system.core,
                path: host_system_path,
                ownership: crate::ResourceFileOwnership::Persistent,
                binary_compatible: false,
            },
            HostedResourceFile {
                core: definitions.core,
                path: host_definitions_path,
                ownership: crate::ResourceFileOwnership::Persistent,
                binary_compatible: false,
            },
        ];

        let (address, host) = start_test_host(host_config).await;
        let client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(client_system_path)
                .with_local_resource_roots([directories.client.clone()]),
        )
        .await
        .test_value();

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_clears_an_unavailable_optional_player_resource_before_exposing_join_data() {
        // Player resource failure is nonfatal, but LoadResource clears
        // PIF_HasRes before HandleJoinData returns and before the parameters
        // become visible to the rest of the client
        // (src/C4PlayerInfo.cpp:275-292; src/C4Network2.cpp:1595-1622).
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.parameters.player_infos = crate::PlayerInfoListSnapshot {
            last_player_id: 1,
            clients: vec![crate::ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(nonloadable_core(3, 9, b"Host.c4p")),
                    ..Default::default()
                }],
            }],
        };
        host_config.initial_join_snapshot = Some(snapshot);

        let (address, host) = start_test_host(host_config).await;
        let mut client = connect_test_player(address, "Alice").await;

        let join_data = client.take_join_data().test_value();
        let player = &join_data.parameters.player_infos.clients[0].players[0];
        assert_eq!(
            player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
            0
        );
        assert_eq!(player.resource, None);

        shutdown_test_session(client, host).await;
    }

    static NEXT_RESOURCE_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct SessionResourceDirectories {
        root: std::path::PathBuf,
        host: std::path::PathBuf,
        client: std::path::PathBuf,
    }

    impl SessionResourceDirectories {
        fn new() -> Self {
            let unique = NEXT_RESOURCE_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "clonk-rust-session-resource-{}-{unique}",
                std::process::id()
            ));
            let host = root.join("host");
            let client = root.join("client");
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&host).test_value();
            fs::create_dir_all(&client).test_value();
            Self { root, host, client }
        }
    }

    impl Drop for SessionResourceDirectories {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// Upper bound for a single event wait. Generous so loaded parallel test
    /// runs do not trip it; a genuine failure still fails fast because the
    /// expected event never arrives at all.
    const EVENT_WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn direct_client_join_authenticates_the_embedded_host_author() {
        let payload = encode_control_entry_payload(&EngineControlPacket::ClientJoin(
            clonk_engine::ClientJoinControlData {
                core: clonk_engine::ClientCoreControlData {
                    client_id: 3,
                    ..Default::default()
                },
                by_client: 0,
            },
        ))
        .test_value();

        assert!(authenticated_single_control(&payload, 0).is_ok());
        assert!(authenticated_single_control(&payload, 3).is_err());
    }

    #[test]
    fn mesh_peer_cannot_author_host_membership_controls() {
        let remove = EngineControlPacket::ClientRemove(clonk_engine::ClientRemoveControlData {
            client_id: 3,
            reason: clonk_engine::LegacyCString::default(),
            by_client: 7,
        });
        let direct = encode_control_entry_payload(&remove).test_value();
        let authenticated = authenticated_single_control(&direct, 7).test_value();
        assert!(control_requires_host_ingress(&authenticated));

        let queued = encode_control_packet(&legacy_frame(7, 12, vec![remove])).test_value();
        assert!(validate_peer_control_packet(&queued, 7)
            .expect_err("peer membership control must be rejected")
            .contains("host-authority"));
    }

    #[test]
    fn mesh_peer_control_contribution_must_use_the_ingress_client_id() {
        let queued = encode_control_packet(&legacy_frame(
            8,
            12,
            vec![EngineControlPacket::PlayerControl(PlayerControlData {
                player: 1,
                command: 2,
                data: 3,
                by_client: 8,
            })],
        ))
        .test_value();

        assert!(validate_peer_control_packet(&queued, 8).is_ok());
        assert!(validate_peer_control_packet(&queued, 7).is_err());
    }

    #[test]
    fn cpp_typed_control_unpack_rejects_unknown_ids_before_peer_ingress() {
        let packet = ControlPacket::builder(7, 12).payload(vec![0x89, 0x31, 0xff]);

        assert!(validate_queued_control_authors(&packet).is_err());
        assert!(validate_peer_control_packet(&packet, 7).is_err());
        assert!(validate_peer_control_or_recovery(&packet, 7, Some(12)).is_err());
    }

    #[test]
    fn mesh_peer_recovery_may_relay_complete_and_partial_controls() {
        let partial = legacy_packet(8, 12, 0x21);
        assert!(validate_peer_control_packet(&partial, 7).is_err());
        assert!(validate_peer_control_or_recovery(&partial, 7, Some(12)).is_ok());
        assert!(validate_peer_control_or_recovery(&partial, 7, None).is_err());

        let complete = legacy_packet(BROADCAST_CLIENT_ID, 12, 0x31);
        assert!(validate_peer_control_packet(&complete, 7).is_err());
        assert!(validate_peer_control_or_recovery(&complete, 7, Some(12)).is_ok());
    }

    #[test]
    fn repeated_peer_recovery_keeps_the_earliest_outstanding_tick() {
        let mut recovery_from_tick = None;
        extend_peer_recovery_window(&mut recovery_from_tick, 12);
        extend_peer_recovery_window(&mut recovery_from_tick, 14);
        assert_eq!(recovery_from_tick, Some(12));

        let trailing = legacy_packet(8, 12, 0x21);
        assert!(validate_peer_control_or_recovery(&trailing, 7, recovery_from_tick).is_ok());
    }

    #[test]
    fn scenario_player_init_authenticates_the_selecting_client() {
        // PID_ControlPkt rejects a non-host packet whose embedded ByClient
        // differs from the authenticated connection (src/C4GameControlNetwork.cpp:478-490).
        let control =
            EngineControlPacket::InitScenarioPlayer(clonk_engine::InitScenarioPlayerControlData {
                team: 2,
                player: 4,
                by_client: 7,
            });
        assert_single_control_author(&control, 7, 3);
    }

    #[test]
    fn single_control_authentication_uses_control_set_by_client() {
        let control = crate::LegacyControlSet {
            value_type: 0,
            data: 1,
            by_client: 7,
        }
        .into_control_packet();
        assert_single_control_author(&control, 7, 8);
    }

    #[test]
    fn queued_control_set_authentication_uses_frame_client_id() {
        assert_queued_control_author(
            |by_client| {
                crate::LegacyControlSet {
                    value_type: 1,
                    data: 0,
                    by_client,
                }
                .into_control_packet()
            },
            "CID_Set",
        );
    }

    #[test]
    fn complete_queued_control_accepts_mixed_embedded_authors() {
        // PackCompleteCtrl marks the merged frame C4ClientIDAll and appends
        // each client's controls unchanged (src/C4GameControlNetwork.cpp:741-777).
        let packet = encode_control_packet(&legacy_frame(
            BROADCAST_CLIENT_ID,
            12,
            vec![
                EngineControlPacket::PlayerControl(PlayerControlData {
                    player: 1,
                    command: 2,
                    data: 3,
                    by_client: 0,
                }),
                EngineControlPacket::PlayerControl(PlayerControlData {
                    player: 4,
                    command: 5,
                    data: 6,
                    by_client: 7,
                }),
            ],
        ))
        .test_value();

        validate_queued_control_authors(&packet).test_value();
    }

    #[test]
    fn queued_vote_and_player_script_controls_authenticate_frame_author() {
        let controls = |by_client| {
            vec![
                EngineControlPacket::Vote(clonk_engine::VoteControlData {
                    vote_type: clonk_engine::VOTE_TYPE_KICK,
                    approve: true,
                    data: 3,
                    by_client,
                }),
                EngineControlPacket::VoteEnd(clonk_engine::VoteControlData {
                    vote_type: clonk_engine::VOTE_TYPE_KICK,
                    approve: true,
                    data: 3,
                    by_client,
                }),
                EngineControlPacket::InitScenarioPlayer(
                    clonk_engine::InitScenarioPlayerControlData {
                        team: 2,
                        player: 4,
                        by_client,
                    },
                ),
                EngineControlPacket::SurrenderPlayer(clonk_engine::SurrenderPlayerControlData {
                    player: 4,
                    by_client,
                }),
            ]
        };
        let packet = |controls| encode_control_packet(&legacy_frame(7, 12, controls)).test_value();

        validate_queued_control_authors(&packet(controls(7))).test_value();

        for (name, forged) in [
            "CID_Vote",
            "CID_VoteEnd",
            "CID_InitScenarioPlayer",
            "CID_SurrenderPlayer",
        ]
        .into_iter()
        .zip(controls(0))
        {
            let error = validate_queued_control_authors(&packet(vec![forged]))
                .expect_err("queued control may not forge the host author");
            assert!(error.contains(name), "{name}: {error}");
            assert!(error.contains("claimed author 0"), "{name}: {error}");
            assert!(
                error.contains("authenticated author is 7"),
                "{name}: {error}"
            );
        }
    }

    #[test]
    fn remove_player_control_cannot_forge_host_author() {
        let control = EngineControlPacket::RemovePlayer(clonk_engine::RemovePlayerControlData {
            player: 4,
            disconnected: false,
            by_client: 0,
        });
        assert_single_control_author(&control, 0, 7);

        let packet = encode_control_packet(&legacy_frame(7, 12, vec![control])).test_value();
        let error = validate_queued_control_authors(&packet)
            .expect_err("queued client may not forge host CID_RemovePlr");
        assert!(error.contains("queued CID_RemovePlr"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn single_script_control_authenticates_embedded_author() {
        let control = EngineControlPacket::Script(clonk_engine::ScriptControlData {
            target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: clonk_engine::ScriptStrictness::Strict3,
            script: c4(b"1+2"),
            by_client: 7,
        });
        assert_single_control_author(&control, 7, 8);
    }

    #[test]
    fn queued_script_control_cannot_forge_host_author() {
        assert_queued_control_author(
            |by_client| {
                EngineControlPacket::Script(clonk_engine::ScriptControlData {
                    target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                    strictness: clonk_engine::ScriptStrictness::Strict3,
                    script: c4(b"1+2"),
                    by_client,
                })
            },
            "CID_Script",
        );
    }

    #[test]
    fn single_message_board_answer_authenticates_embedded_author() {
        let control =
            EngineControlPacket::MessageBoardAnswer(clonk_engine::MessageBoardAnswerControlData {
                object: 42,
                answer: c4(b"answer"),
                player: 3,
                by_client: 7,
            });
        assert_single_control_author(&control, 7, 8);
    }

    #[test]
    fn single_message_control_authenticates_embedded_author() {
        let control = EngineControlPacket::Message(clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_PRIVATE,
            player: 3,
            to_player: 5,
            message: c4(b"secret"),
            by_client: 7,
        });
        assert_single_control_author(&control, 7, 8);
    }

    #[tokio::test]
    async fn lobby_team_messages_are_not_retained_for_future_clients() {
        // Team messages are filtered against the receiving client's team
        // (src/C4Control.cpp:1158-1189). Replaying their raw text to a client
        // that was not present at send time would widen that audience.
        let (outbound, _receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let control = EngineControlPacket::Message(clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_TEAM,
            player: -1,
            to_player: -1,
            message: c4(b"team secret"),
            by_client: HOST_CLIENT_ID as i32,
        });
        let data = encode_control_entry_payload(&control).test_value();

        broadcast_packet(ControlDelivery::Private, data, None, &mut state).await;

        assert!(state.lobby_chat_history.is_empty());
    }

    #[tokio::test]
    async fn lobby_system_messages_are_not_retained_as_user_chat() {
        // C++ treats host-authored system controls as network log output, not
        // user chat (src/C4Control.cpp:1238-1243). The late-join transcript
        // must not turn those one-shot notices into replayable conversation.
        let (outbound, _receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        let control = EngineControlPacket::Message(clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_SYSTEM,
            player: -1,
            to_player: -1,
            message: c4(b"network notice"),
            by_client: HOST_CLIENT_ID as i32,
        });
        let data = encode_control_entry_payload(&control).test_value();

        broadcast_packet(ControlDelivery::Private, data, None, &mut state).await;

        assert!(state.lobby_chat_history.is_empty());
    }

    #[tokio::test]
    async fn leaving_the_lobby_clears_retained_chat() {
        // C++ creates the bounded chat TextWindow with each lobby dialog, so
        // its contents end with that lobby's lifetime
        // (src/C4GameLobby.cpp:269-280).
        let (outbound, _receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        state.lobby_chat_history.push_back(b"old lobby".to_vec());
        let effects = state
            .status_barrier
            .change_status(state.status_barrier.status.with_state(NETWORK_STATE_GO));

        apply_barrier_effects(effects, &mut state).await;

        assert!(state.lobby_chat_history.is_empty());
    }

    #[tokio::test]
    async fn lobby_chat_history_uses_a_bounded_transport_budget() {
        // Borrow the lobby TextWindow's numeric 100/4096 ceilings as a
        // conservative raw-packet budget (src/C4GameLobby.cpp:277-280);
        // rendered entries/text bytes are not encoded control payloads.
        let (outbound, _receiver) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(7, outbound);
        for index in 0..=100 {
            let control = EngineControlPacket::Message(clonk_engine::MessageControlData {
                message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
                player: -1,
                to_player: -1,
                message: c4(format!("message {index}").into_bytes()),
                by_client: HOST_CLIENT_ID as i32,
            });
            let data = encode_control_entry_payload(&control).test_value();
            broadcast_packet(ControlDelivery::Private, data, None, &mut state).await;
        }

        assert_eq!(state.lobby_chat_history.len(), 100);
        let first = state
            .lobby_chat_history
            .front()
            .and_then(|data| decode_control_entry_payload(data).ok());
        assert!(matches!(
            first,
            Some(EngineControlPacket::Message(message))
                if message.message.as_bytes() == b"message 1"
        ));

        let mut newest = Vec::new();
        for index in 0_u8..20 {
            let mut text = vec![b'x'; 239];
            text.push(b'a' + index);
            let control = EngineControlPacket::Message(clonk_engine::MessageControlData {
                message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
                player: -1,
                to_player: -1,
                message: c4(text),
                by_client: HOST_CLIENT_ID as i32,
            });
            newest = encode_control_entry_payload(&control).test_value();
            broadcast_packet(ControlDelivery::Private, newest.clone(), None, &mut state).await;
        }

        assert!(state.lobby_chat_history.iter().map(Vec::len).sum::<usize>() <= 4096);
        assert_eq!(state.lobby_chat_history.back(), Some(&newest));
    }

    #[test]
    fn queued_message_board_answer_cannot_forge_host_author() {
        assert_queued_control_author(
            |by_client| {
                EngineControlPacket::MessageBoardAnswer(
                    clonk_engine::MessageBoardAnswerControlData {
                        object: 42,
                        answer: c4(b"answer"),
                        player: 3,
                        by_client,
                    },
                )
            },
            "CID_MessageBoardAnswer",
        );
    }

    #[test]
    fn single_custom_command_authenticates_embedded_author() {
        let control = EngineControlPacket::CustomCommand(clonk_engine::CustomCommandControlData {
            command: c4(b"push"),
            argument: c4(b"argument"),
            player: 3,
            by_client: 7,
        });
        assert_single_control_author(&control, 7, 8);
    }

    #[test]
    fn queued_custom_command_cannot_forge_host_author() {
        assert_queued_control_author(
            |by_client| {
                EngineControlPacket::CustomCommand(clonk_engine::CustomCommandControlData {
                    command: c4(b"push"),
                    argument: c4(b"argument"),
                    player: 3,
                    by_client,
                })
            },
            "CID_CustomCommand",
        );
    }

    #[test]
    fn em_move_object_control_authenticates_direct_and_queued_authors() {
        let control = |by_client| {
            EngineControlPacket::EmMoveObject(clonk_engine::EmMoveObjectControlData {
                action: clonk_engine::EMMO_SCRIPT,
                tx: -12,
                ty: 34,
                target_object: 42,
                objects: vec![7, 9],
                strictness: clonk_engine::ScriptStrictness::Strict2,
                script: c4(b"SetXDir(0)"),
                by_client,
            })
        };

        assert_single_control_author(&control(7), 7, 8);
        assert_queued_control_author(control, "CID_EMMoveObj");
    }

    #[test]
    fn em_draw_tool_control_authenticates_direct_and_queued_authors() {
        let control = |by_client| {
            EngineControlPacket::EmDrawTool(clonk_engine::EmDrawToolControlData {
                action: clonk_engine::EMDT_LINE,
                mode: 3,
                x: -12,
                y: 34,
                x2: 56,
                y2: -78,
                grade: 9,
                ift: true,
                material: c4(b"Earth"),
                texture: c4(b"Rough"),
                by_client,
            })
        };

        assert_single_control_author(&control(7), 7, 8);
        assert_queued_control_author(control, "CID_EMDrawTool");
    }

    #[test]
    fn em_drop_def_control_authenticates_direct_and_queued_authors() {
        let control = |by_client| {
            EngineControlPacket::EmDropDef(clonk_engine::EmDropDefControlData {
                id: *b"HUT2",
                x: -130,
                y: 130,
                by_client,
            })
        };

        assert_single_control_author(&control(7), 7, 8);
        assert_queued_control_author(control, "CID_EMDropDef");
    }

    #[test]
    fn internal_player_script_controls_authenticate_direct_and_queued_authors() {
        fn controls(by_client: i32) -> [EngineControlPacket; 5] {
            [
                EngineControlPacket::ActivateGameGoalMenu(
                    clonk_engine::ActivateGameGoalMenuControlData {
                        player: 3,
                        by_client,
                    },
                ),
                EngineControlPacket::ToggleHostility(clonk_engine::ToggleHostilityControlData {
                    opponent: 4,
                    player: 3,
                    by_client,
                }),
                EngineControlPacket::ActivateGameGoalRule(
                    clonk_engine::ActivateGameGoalRuleControlData {
                        object: 42,
                        player: 3,
                        by_client,
                    },
                ),
                EngineControlPacket::SetPlayerTeam(clonk_engine::SetPlayerTeamControlData {
                    team: 5,
                    player: 3,
                    by_client,
                }),
                EngineControlPacket::EliminatePlayer(clonk_engine::EliminatePlayerControlData {
                    player: 3,
                    by_client,
                }),
            ]
        }

        let names = [
            "CID_ActivateGameGoalMenu",
            "CID_ToggleHostility",
            "CID_ActivateGameGoalRule",
            "CID_SetPlayerTeam",
            "CID_EliminatePlayer",
        ];
        for (index, name) in names.into_iter().enumerate() {
            let control = |author| controls(author)[index].clone();
            assert_single_control_author(&control(7), 7, 8);
            assert_queued_control_author(control, name);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pending_connection_attempt_times_out() {
        // C4Network2IO::CheckTimeout closes unaccepted connections after
        // C4NetAcceptTimeout (src/C4Network2IO.cpp:1155-1170).
        let result = timeout(
            HANDSHAKE_TIMEOUT + Duration::from_secs(1),
            connect_client_from(
                pending::<Result<TcpStream, io::Error>>(),
                ClientConfig::new("Alice", ParticipantKind::Player),
            ),
        )
        .await;

        match result {
            Ok(Err(ClientError::Connect(error))) => {
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
                assert_eq!(error.to_string(), "connection attempt timed out");
            }
            other => panic!("expected bounded connection timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn client_connection_request_uses_target_compatibility_build() {
        // The C++ host accepts PID_Conn only when its packed Version equals
        // C4XVERBUILD. C4Network2Reference initializes and publishes that
        // exact value, so a Rust client must echo the target build rather than
        // its own (oracle-src-pinned src/C4Network2.cpp:1291-1299;
        // src/C4Network2Reference.cpp:79,100-102;
        // src/C4GameVersion.h:35-37).
        let (address, listener) = bind_test_listener().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            let mut transport = crate::ControlTransport::new(stream);
            match transport.read_message().await.test_value() {
                ControlMessage::ConnectionRequest(request) => request.build,
                other => panic!("expected client connection request, got {other:?}"),
            }
        });

        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_compatibility_build(CPP_COMPATIBILITY_BUILD),
        )
        .await;

        assert!(result.is_err(), "the probe server deliberately stops early");
        assert_eq!(server.await.unwrap(), CPP_COMPATIBILITY_BUILD);
    }

    #[tokio::test]
    async fn secondary_routes_use_target_compatibility_build_on_tcp_and_udp() {
        // Every C++ route is admitted by the same exact C4XVERBUILD check
        // (oracle-src-pinned src/C4Network2.cpp:1291-1299), including the
        // additional/reconnect PID_Conn sent by C4Network2IO.cpp:1611-1618.
        let local_core = compatibility_test_core(1, b"Alice");
        let host_core = compatibility_test_core(0, b"Host");
        let request_template = ClientHandshakeRequestTemplate::new(
            local_core.clone(),
            CPP_COMPATIBILITY_BUILD,
            clonk_engine::LegacyCString::default(),
        );

        let (address, listener) = bind_test_listener().await;
        let tcp_peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            read_compatibility_request(stream).await
        });
        let tcp_result = connect_secondary_tcp_route(
            address,
            request_template.clone(),
            host_core.clone(),
            41,
            crate::NetworkIoStatistics::default(),
        )
        .await;
        assert!(
            tcp_result.is_err(),
            "the probe peer deliberately stops early"
        );
        let tcp_request = tcp_peer.await.test_value();
        assert_eq!(tcp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(tcp_request.connection_id, 41);

        let local_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let mut peer_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let udp_task = tokio::spawn(connect_secondary_udp_route(
            local_hub.handle(),
            peer_hub.local_addr(),
            request_template,
            host_core,
            42,
        ));
        let udp_stream = timeout(EVENT_WAIT, peer_hub.accept())
            .await
            .unwrap()
            .test_value();
        let udp_request = read_compatibility_request(udp_stream).await;
        assert_eq!(udp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(udp_request.connection_id, 42);
        assert!(timeout(EVENT_WAIT, udp_task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        local_hub.shutdown().await.test_value();
        peer_hub.shutdown().await.test_value();
    }

    #[tokio::test]
    async fn outbound_mesh_routes_use_target_compatibility_build_on_tcp_and_udp() {
        // C++ sends the same C4XVERBUILD in every reciprocal PID_Conn
        // (oracle-src-pinned src/C4Network2IO.cpp:1611-1618); Rust peers in a
        // C++ game therefore keep the selected game build on mesh routes too.
        let alice = compatibility_test_core(1, b"Alice");
        let bob = compatibility_test_core(2, b"Bob");
        let request_template = ClientHandshakeRequestTemplate::new(
            alice,
            CPP_COMPATIBILITY_BUILD,
            clonk_engine::LegacyCString::default(),
        );
        let bob_id = ClientId::try_from(bob.client_id).test_value();

        let (address, listener) = bind_test_listener().await;
        let tcp_peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            read_compatibility_request(stream).await
        });
        let tcp_result = connect_mesh_tcp_route(
            bob_id,
            address,
            request_template.clone(),
            bob.clone(),
            51,
            crate::NetworkIoStatistics::default(),
        )
        .await;
        assert!(
            tcp_result.is_err(),
            "the probe peer deliberately stops early"
        );
        let tcp_request = tcp_peer.await.test_value();
        assert_eq!(tcp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(tcp_request.connection_id, 51);

        let local_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let mut peer_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let udp_task = tokio::spawn(connect_mesh_udp_route(
            local_hub.handle(),
            bob_id,
            peer_hub.local_addr(),
            request_template,
            bob,
            52,
        ));
        let udp_stream = timeout(EVENT_WAIT, peer_hub.accept())
            .await
            .unwrap()
            .test_value();
        let udp_request = read_compatibility_request(udp_stream).await;
        assert_eq!(udp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(udp_request.connection_id, 52);
        assert!(timeout(EVENT_WAIT, udp_task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        local_hub.shutdown().await.test_value();
        peer_hub.shutdown().await.test_value();
    }

    #[tokio::test]
    async fn inbound_mesh_routes_use_target_compatibility_build_on_tcp_and_udp() {
        // C++ applies its exact build check to existing clients as well as new
        // ones (oracle-src-pinned src/C4Network2.cpp:1286-1307), so accepted
        // mesh sockets must answer with the selected game's build.
        let alice = compatibility_test_core(1, b"Alice");
        let bob = compatibility_test_core(2, b"Bob");
        let request_template = ClientHandshakeRequestTemplate::new(
            alice,
            CPP_COMPATIBILITY_BUILD,
            clonk_engine::LegacyCString::default(),
        );
        let known_peers = BTreeMap::from([(bob.client_id, bob)]);

        let (address, listener) = bind_test_listener().await;
        let (peer_stream, accepted) = tokio::join!(TcpStream::connect(address), listener.accept());
        let peer_stream = peer_stream.test_value();
        let (accepted_stream, peer_addr) = accepted.test_value();
        let tcp_task = tokio::spawn(accept_mesh_tcp_route(
            accepted_stream,
            peer_addr,
            request_template.clone(),
            known_peers.clone(),
            61,
            crate::NetworkIoStatistics::default(),
        ));
        let tcp_request = read_compatibility_request(peer_stream).await;
        assert_eq!(tcp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(tcp_request.connection_id, 61);
        assert!(timeout(EVENT_WAIT, tcp_task)
            .await
            .unwrap()
            .unwrap()
            .is_err());

        let mut local_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let peer_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).test_value();
        let peer_handle = peer_hub.handle();
        let (peer_stream, accepted_stream) = tokio::join!(
            peer_handle.connect(local_hub.local_addr()),
            local_hub.accept()
        );
        let peer_stream = peer_stream.test_value();
        let accepted_stream = accepted_stream.test_value();
        let udp_task = tokio::spawn(accept_mesh_udp_route(
            accepted_stream,
            request_template,
            known_peers,
            62,
        ));
        let udp_request = read_compatibility_request(peer_stream).await;
        assert_eq!(udp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(udp_request.connection_id, 62);
        assert!(timeout(EVENT_WAIT, udp_task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        local_hub.shutdown().await.test_value();
        peer_hub.shutdown().await.test_value();
    }

    #[tokio::test]
    async fn simultaneous_open_mesh_route_uses_target_compatibility_build() {
        // Simultaneous-open still emits the ordinary C++ PID_Conn governed by
        // the exact build check (oracle-src-pinned src/C4Network2.cpp:1291-1299).
        let alice = compatibility_test_core(1, b"Alice");
        let bob = compatibility_test_core(2, b"Bob");
        let alice_id = ClientId::try_from(alice.client_id).test_value();
        let request_template = ClientHandshakeRequestTemplate::new(
            alice,
            CPP_COMPATIBILITY_BUILD,
            clonk_engine::LegacyCString::default(),
        );
        let (address, listener) = bind_test_listener().await;
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            read_compatibility_request(stream).await
        });
        let socket = tokio::net::TcpSocket::new_v4().test_value();
        socket
            .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .test_value();
        let result = connect_mesh_tcp_socket_route(
            ClientId::try_from(bob.client_id).test_value(),
            alice_id,
            socket,
            address,
            request_template,
            bob,
            71,
            Duration::ZERO,
            crate::NetworkIoStatistics::default(),
        )
        .await;
        assert!(result.is_err(), "the probe peer deliberately stops early");
        let request = peer.await.test_value();
        assert_eq!(request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(request.connection_id, 71);
    }

    #[test]
    fn client_dial_race_coalesces_a_duplicate_connect_address() {
        // ConnectWithSocket returns success without creating another
        // connection when GetConnectionByConnAddr finds the same protocol and
        // connect endpoint (src/C4Network2IO.cpp:228-240).
        let address = crate::NetworkAddress::new(
            crate::NetworkProtocol::Tcp,
            SocketAddr::from(([127, 0, 0, 1], 31_114)),
        );
        let race = ClientDialRace::new([address, address], true, None);

        assert_eq!(race.attempts.len(), 1);
        assert_eq!(race.attempts[0].index, 0, "the first attempt wins dedup");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_join_names_every_refused_address() {
        // Reference address lists put the IPv6 endpoints first, so reporting
        // only the lowest-indexed dial hides whatever went wrong on the
        // address the client actually needed (clonk-org/clonk-rs#109).
        //
        // Both ports sit below every platform's ephemeral range, so no
        // concurrent test in this binary can be bound to one of them. Dialling
        // a just-released ephemeral port instead would let a listener opened
        // elsewhere in the suite answer this join.
        let first_address = SocketAddr::from(([127, 0, 0, 1], 31_116));
        let second_address = SocketAddr::from(([127, 0, 0, 1], 31_117));

        let error = connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, first_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, second_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect_err("no listener remains on either address");

        // The refusal text is the platform's own, so only the endpoint labels
        // and their dial order are asserted here.
        let rendered = error.to_string();
        let first_label = rendered
            .find(&format!("TCP {first_address}: "))
            .unwrap_or_else(|| panic!("the first endpoint is missing from {rendered:?}"));
        let second_label = rendered
            .find(&format!("TCP {second_address}: "))
            .unwrap_or_else(|| panic!("the second endpoint is missing from {rendered:?}"));
        assert!(
            first_label < second_label,
            "the endpoints are out of dial order in {rendered:?}"
        );
        assert!(
            rendered.starts_with("failed to connect to host: TCP "),
            "the caption is not carried once by {rendered:?}"
        );
    }

    #[tokio::test]
    async fn an_expired_dial_race_times_out_every_open_address() {
        // The deadline abandons every attempt that is still open, so each of
        // them owes its own timeout; reporting one and dropping the rest
        // leaves those endpoints out of the join failure
        // (clonk-org/clonk-rs#109).
        let first = crate::NetworkAddress::new(
            crate::NetworkProtocol::Tcp,
            SocketAddr::from(([127, 0, 0, 1], 31_114)),
        );
        let second = crate::NetworkAddress::new(
            crate::NetworkProtocol::Tcp,
            SocketAddr::from(([127, 0, 0, 1], 31_115)),
        );
        let mut race = ClientDialRace::new([first, second], true, None);

        race.expire();

        let mut reported = Vec::new();
        while let Some((_, address, result)) = race.next().await {
            assert_eq!(
                result.err().map(|error| error.kind()),
                Some(std::io::ErrorKind::TimedOut)
            );
            reported.push(address);
        }
        assert_eq!(reported, vec![first, second]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exclusive_join_defers_welcome_until_the_active_route_fails() {
        // InitClient opens every prepared route together, but exclusive mode
        // permits only one outstanding PID_Conn. OnDisconn promotes the next
        // already-open socket (src/C4Network2IO.cpp:523-563,1223-1255).
        let first_listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let first_address = first_listener.local_addr().test_value();
        let second_listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let second_address = second_listener.local_addr().test_value();
        let client = tokio::spawn(connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, first_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, second_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));
        let (first, second) = timeout(EVENT_WAIT, async {
            let (first, second) = tokio::join!(first_listener.accept(), second_listener.accept());
            (first.test_value().0, second.test_value().0)
        })
        .await
        .test_value();

        let mut first_probe = [0_u8; 1];
        let mut second_probe = [0_u8; 1];
        let first_is_active = tokio::select! {
            ready = first.peek(&mut first_probe) => {
                assert_eq!(ready.unwrap(), 1);
                true
            }
            ready = second.peek(&mut second_probe) => {
                assert_eq!(ready.unwrap(), 1);
                false
            }
            _ = tokio::time::sleep(EVENT_WAIT) => panic!("neither route sent PID_Conn"),
        };
        let (mut active, mut deferred) = if first_is_active {
            (first, second)
        } else {
            (second, first)
        };
        let mut header_and_pid = [0_u8; 6];
        active.read_exact(&mut header_and_pid).await.test_value();
        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02, "active route sends PID_Conn");

        let mut deferred_probe = [0_u8; 1];
        assert!(
            timeout(
                Duration::from_millis(150),
                deferred.peek(&mut deferred_probe)
            )
            .await
            .is_err(),
            "the second open route must keep its welcome deferred"
        );

        drop(active);
        await_test(deferred.read_exact(&mut header_and_pid)).await;
        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02, "promoted route sends PID_Conn");

        client.abort();
        let _ = client.await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_races_prepared_tcp_addresses() {
        // InitClient launches every prepared reference address before waiting
        // for admission. Exclusive connection mode advances to another live
        // route when the first transport drops during Conn/ConnRe
        // (src/C4Network2.cpp:347-443; src/C4Network2IO.cpp:873-894).
        let stale = TcpListener::bind("127.0.0.1:0").await.test_value();
        let stale_address = stale.local_addr().test_value();
        let stale_route = tokio::spawn(async move {
            let (mut stream, _) = stale.accept().await.test_value();
            let mut header_and_pid = [0; 6];
            stream.read_exact(&mut header_and_pid).await.test_value();
            header_and_pid
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let live_address = listener.local_addr().test_value();
        let host = start_host(listener, HostConfig::default())
            .await
            .test_value();

        let mut client = connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, stale_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, live_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();

        assert_eq!(
            client
                .take_join_data()
                .expect("bootstrap JoinData remains available")
                .client_id,
            client.client_id() as i32
        );
        let stale_request = stale_route.await.test_value();
        assert_eq!(stale_request[0], 0xff);
        assert_eq!(stale_request[5], 0x02, "the first route reached PID_Conn");
        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_accepts_a_udp_only_address_list() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let udp_address = host.udp_local_addr().test_value();
        let mut client = connect_client_addresses(
            [crate::NetworkAddress::new(
                crate::NetworkProtocol::Udp,
                udp_address,
            )],
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();

        assert_eq!(
            client
                .take_join_data()
                .expect("bootstrap JoinData remains available")
                .client_id,
            client.client_id() as i32
        );
        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_retains_a_prepared_opposite_transport_route() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let tcp_address = listener.local_addr().test_value();
        let host = start_host(
            listener,
            host_config!(udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0)))),
        )
        .await
        .test_value();
        let udp_address = host.udp_local_addr().test_value();
        let client = connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, tcp_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Udp, udp_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .test_value();

        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 2 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert!(routes
            .iter()
            .all(|(_, client_id, _)| *client_id == client.client_id()));

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_surfaces_wrong_password_and_allows_retry() {
        // A negative ConnRe with WrongPassword set drives the outer password
        // prompt loop; ordinary admission failures remain terminal
        // (src/C4Network2.cpp:281-345,1448-1469).
        let secret = c4(b"correct horse");
        let host_config = host_config!(password: secret.clone());
        let (address, host) = start_test_host(host_config).await;

        let error = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player).with_password(c4(b"wrong")),
        )
        .await
        .expect_err("the first password is rejected");
        assert!(matches!(
            error,
            ClientError::WrongPassword { message } if message.as_bytes() == b"wrong password"
        ));

        let client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player).with_password(secret),
        )
        .await
        .test_value();
        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_keeps_non_password_rejection_terminal() {
        // HandleConnRe presents the peer's exact message text before closing
        // the rejected connection (src/C4Network2.cpp:1476-1485).
        let host_config = host_config!(allow_join: false);
        let (address, host) = start_test_host(host_config).await;

        let error = connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect_err("closed admission is not a password retry");
        assert_eq!(
            error.to_string(),
            "handshake rejected: the peer rejected the local connection: join denied"
        );
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_begin_go_acknowledgement_closes_admission_before_return() {
        let (address, host) = start_test_host(HostConfig::default()).await;
        let status = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);

        host.begin_go(status, false).await.test_value();
        let error = connect_client(
            address,
            ClientConfig::new("Late join", ParticipantKind::Player),
        )
        .await
        .expect_err("the acknowledged Go transition already closed admission");
        assert!(matches!(error, ClientError::Handshake(_)));
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_first_packet_is_cpp_connection_request_not_json() {
        // C4Network2IO sends PID_Conn through the ordinary C4NetIOTCP frame as
        // soon as the socket opens (src/C4Network2IO.cpp:478-525,1223-1252;
        // src/C4NetIO.cpp:1287-1323).
        let (addr, listener) = bind_test_listener().await;
        let client = tokio::spawn(connect_client_from(
            TcpStream::connect(addr),
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));
        let (mut peer, _) = listener.accept().await.test_value();
        let mut header_and_pid = [0; 6];
        peer.read_exact(&mut header_and_pid).await.test_value();

        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02);
        client.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_first_packet_is_cpp_connection_request_without_blocking_listener() {
        // An accepted C++ TCP socket sends its own PID_Conn immediately; the
        // listener/main loop does not wait for the peer's request first
        // (src/C4Network2IO.cpp:479-530,1223-1252).
        let (addr, host) = start_test_host(HostConfig::default()).await;
        let mut peer = TcpStream::connect(addr).await.test_value();
        let mut header_and_pid = [0; 6];
        timeout(Duration::from_secs(1), peer.read_exact(&mut header_and_pid))
            .await
            .test_value()
            .test_value();

        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_accepts_a_canonical_existing_client_connection_request() {
        // HandleConn selects an existing client before the new-client Join path;
        // CheckConn accepts status-only core differences and replies
        // "connection accepted" (src/C4Network2.cpp:1286-1334,1366-1380;
        // src/C4Client.cpp:58-70).
        let (addr, host) = start_test_host(HostConfig::default()).await;
        let client = connect_test_player(addr, "Alice").await;
        let stream = TcpStream::connect(addr).await.test_value();
        let mut transport = crate::ControlTransport::new(stream);

        assert!(matches!(
            timeout(EVENT_WAIT, transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = c4(b"Alice");
        transport
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                test_client_core(i32::try_from(client.client_id()).unwrap(), name, true),
                17,
                false,
            )))
            .await
            .test_value();

        let reply = await_test(transport.read_message()).await;
        let accepted_message = c4(b"connection accepted");
        assert_eq!(
            reply,
            ControlMessage::ConnectionReply(test_connection_reply(true, accepted_message, true,))
        );

        host.shutdown().await.test_value();
        client.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn synchronized_client_remove_closes_every_route_with_cpp_reason() {
        // DeleteClient closes every route with a negative PID_ConnRe carrying
        // the fixed "removing client" reason before removing the logical
        // network client (src/C4Network2Client.cpp:104-119,457-465).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut canonical, client_id) = raw_client_transport(addr, b"Alice").await;
        let mut secondary = raw_existing_client_transport(addr, client_id, 29, b"Alice").await;

        timeout(EVENT_WAIT, async {
            loop {
                if host.accepted_routes().await.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert_eq!(host.connected_clients().await, vec![client_id]);
        while host_events.try_recv().is_ok() {}

        let remove = EngineControlPacket::ClientRemove(clonk_engine::ClientRemoveControlData {
            client_id: i32::try_from(client_id).test_value(),
            reason: c4(b"voted out"),
            by_client: i32::try_from(HOST_CLIENT_ID).test_value(),
        });
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&remove).unwrap(),
        )
        .await
        .test_value();

        let close = ControlMessage::ConnectionReply(test_connection_reply(
            false,
            c4(b"removing client"),
            false,
        ));
        for route in [&mut canonical, &mut secondary] {
            assert!(raw_client_received_message(route, &close, EVENT_WAIT).await);
            match timeout(EVENT_WAIT, route.read_message()).await {
                Ok(Err(TransportError::Io(error)))
                    if error.kind() == io::ErrorKind::UnexpectedEof => {}
                other => panic!("removed route did not close after ConnRe: {other:?}"),
            }
        }

        assert!(host.accepted_routes().await.is_empty());
        assert!(host.connected_clients().await.is_empty());
        while let Ok(event) = host_events.try_recv() {
            assert!(
                !matches!(
                    event,
                    HostEvent::ClientConnectionFailed { client_id: failed }
                        if failed == client_id
                ),
                "synchronized removal was reported as a failed connection"
            );
        }
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn synchronized_remove_rejects_a_route_that_finishes_handshaking_late() {
        let (addr, host) = start_test_host(HostConfig::default()).await;
        let (mut canonical, client_id) = raw_client_transport(addr, b"Alice").await;

        let mut delayed = crate::ControlTransport::new(TcpStream::connect(addr).await.test_value());
        let admission =
            request_route(&mut delayed, i32::try_from(client_id).test_value(), 29).await;
        assert!(admission.ok);

        let remove = EngineControlPacket::ClientRemove(clonk_engine::ClientRemoveControlData {
            client_id: i32::try_from(client_id).test_value(),
            reason: c4(b"voted out"),
            by_client: i32::try_from(HOST_CLIENT_ID).test_value(),
        });
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&remove).unwrap(),
        )
        .await
        .test_value();

        let close = ControlMessage::ConnectionReply(test_connection_reply(
            false,
            c4(b"removing client"),
            false,
        ));
        assert!(raw_client_received_message(&mut canonical, &close, EVENT_WAIT).await);

        delayed
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                true,
                c4(b"connection accepted"),
                false,
            )))
            .await
            .test_value();
        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(deadline, delayed.read_message()).await {
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    let _ = delayed.send_message(ControlMessage::Pong(ping)).await;
                }
                Ok(Err(TransportError::Io(error)))
                    if error.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Ok(Ok(other)) => panic!("removed pending route received {other:?}"),
                Ok(Err(error)) => panic!("removed pending route failed unexpectedly: {error}"),
                Err(_) => panic!("removed pending route stayed connected"),
            }
        }

        assert!(host.accepted_routes().await.is_empty());
        assert!(host.connected_clients().await.is_empty());
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn last_route_loss_invalidates_pending_handshake_before_sync_remove() {
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();

        let mut alice = connect_test_player(addr, "Alice").await;
        let alice_id = alice.client_id();
        let mut alice_events = alice.take_event_receiver();
        activate_joined_client(&host, &mut host_events, alice_id).await;

        let mut beta = connect_test_player(addr, "Beta").await;
        let beta_id = beta.client_id();
        let mut beta_events = beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, beta_id).await;

        let running = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);
        host.change_status(running).await.test_value();
        for events in [&mut alice_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                    Some(ClientEvent::Status(status)) if status == running => break,
                    Some(_) => continue,
                    None => panic!("client event stream ended before initial Go"),
                }
            }
        }
        alice.submit_status_ack(running).await.test_value();
        beta.submit_status_ack(running).await.test_value();
        host.status_reached(running, running.target_tick)
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial Go committed"),
            }
        }

        let mut delayed = crate::ControlTransport::new(TcpStream::connect(addr).await.test_value());
        let admission = request_route(&mut delayed, i32::try_from(alice_id).test_value(), 31).await;
        assert!(admission.ok);

        let unreachable = running.with_target_tick(2);
        host.change_status(unreachable).await.test_value();
        for events in [&mut alice_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                    Some(ClientEvent::Status(status)) if status == unreachable => break,
                    Some(_) => continue,
                    None => panic!("client event stream ended before unreached Go"),
                }
            }
        }

        alice.shutdown().await.test_value();
        let mut connection_failed = false;
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ClientConnectionFailed { client_id }) if client_id == alice_id => {
                    connection_failed = true;
                }
                Some(HostEvent::ClientLeft { client_id }) if client_id == alice_id => {
                    assert!(
                        connection_failed,
                        "final route loss did not emit ClientConnectionFailed first"
                    );
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before Alice left"),
            }
        }

        delayed
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                true,
                c4(b"connection accepted"),
                false,
            )))
            .await
            .test_value();
        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(deadline, delayed.read_message()).await {
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    let _ = delayed.send_message(ControlMessage::Pong(ping)).await;
                }
                Ok(Err(TransportError::Io(error)))
                    if error.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Ok(Ok(other)) => panic!("invalidated pending route received {other:?}"),
                Ok(Err(error)) => panic!("invalidated pending route failed unexpectedly: {error}"),
                Err(_) => panic!("invalidated pending route stayed connected"),
            }
        }

        let mut new_route =
            crate::ControlTransport::new(TcpStream::connect(addr).await.test_value());
        let rejection =
            request_route(&mut new_route, i32::try_from(alice_id).test_value(), 32).await;
        assert!(!rejection.ok);
        assert_eq!(rejection.message.as_bytes(), b"removing client");

        assert_eq!(host.connected_clients().await, vec![beta_id]);
        assert!(host
            .accepted_routes()
            .await
            .iter()
            .all(|(_, client_id, _)| *client_id == beta_id));
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn secondary_route_from_a_different_peer_host_is_rejected() {
        let (addr, listener) = bind_test_listener().await;
        let host = start_host(
            listener,
            host_config!(udp_bind_address: Some("[::1]:0".parse().unwrap())),
        )
        .await
        .test_value();
        let client = connect_test_player(addr, "Alice").await;

        let udp_hub = crate::ReliableUdpSessionHub::bind("[::1]:0".parse().unwrap()).test_value();
        let stream = udp_hub
            .connect_owned(host.udp_local_addr().unwrap())
            .await
            .test_value();
        let mut transport = crate::ControlTransport::new(stream);
        loop {
            match await_test(transport.read_message()).await {
                ControlMessage::ConnectionRequest(_) => break,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected host connection request, got {other:?}"),
            }
        }
        let name = c4(b"Alice");
        transport
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                test_client_core(i32::try_from(client.client_id()).unwrap(), name, true),
                41,
                false,
            )))
            .await
            .test_value();

        let rejection = loop {
            match await_test(transport.read_message()).await {
                ControlMessage::ConnectionReply(reply) => break reply,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected host connection reply, got {other:?}"),
            }
        };
        assert!(!rejection.ok);
        assert_eq!(
            rejection.message.as_bytes(),
            b"secondary connection came from a different peer host"
        );
        assert_eq!(host.accepted_routes().await.len(), 1);

        drop(transport);
        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn different_peer_host_is_rejected_while_same_client_route_is_pending() {
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let tcp_address = listener.local_addr().test_value();
        let mut host = start_host(
            listener,
            host_config!(udp_bind_address: Some("[::1]:0".parse().unwrap())),
        )
        .await
        .test_value();
        let mut host_events = host.take_event_receiver();

        let mut pending_tcp =
            crate::ControlTransport::new(TcpStream::connect(tcp_address).await.test_value());
        let pending_reply = request_route(&mut pending_tcp, -1, 51).await;
        assert!(
            pending_reply.ok,
            "initial pending route was rejected: {:?}",
            pending_reply.message
        );
        // Do not send the reciprocal ConnRe: the newly assigned core and its
        // TCP peer now exist only in the serialized admission coordinator.
        let client_id = timeout(EVENT_WAIT, async {
            loop {
                match host_events.recv().await {
                    Some(HostEvent::Direct { data, .. }) => {
                        if let Ok(EngineControlPacket::ClientJoin(join)) =
                            decode_control_entry_payload(&data)
                        {
                            break ClientId::try_from(join.core.client_id).test_value();
                        }
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended before provisional ClientJoin"),
                }
            }
        })
        .await
        .test_value();

        let udp_hub = crate::ReliableUdpSessionHub::bind("[::1]:0".parse().unwrap()).test_value();
        let udp_stream = udp_hub
            .connect_owned(host.udp_local_addr().unwrap())
            .await
            .test_value();
        let mut different_host = crate::ControlTransport::new(udp_stream);
        let rejection = request_route(
            &mut different_host,
            i32::try_from(client_id).test_value(),
            52,
        )
        .await;
        assert!(!rejection.ok);
        assert_eq!(
            rejection.message.as_bytes(),
            b"secondary connection came from a different peer host"
        );
        assert!(host.accepted_routes().await.is_empty());

        drop(different_host);
        drop(pending_tcp);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn secondary_route_does_not_rejoin_replace_or_remove_the_logical_client() {
        // HandleConnRe records whether this is the client's first connection;
        // only that first connection runs OnClientConnect and its JoinData,
        // lobby, and resource setup (src/C4Network2.cpp:1479-1498,1734-1743,
        // 1768-1783).
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(
            listener,
            host_config!(resource_registrations: vec![crate::ResourceRegistration {
                resource_id: 3,
                chunk_count: 1,
                binary_compatible: true,
                loading: false,
            }]),
        )
        .await
        .test_value();
        let mut host_events = host.take_event_receiver();
        let mut canonical = connect_test_player(addr, "Alice").await;
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();
        while host_events.try_recv().is_ok() {}
        while canonical_events.try_recv().is_ok() {}

        let stream = TcpStream::connect(addr).await.test_value();
        let mut secondary = crate::ControlTransport::new(stream);
        let host_request = match secondary.read_message().await.test_value() {
            ControlMessage::ConnectionRequest(request) => request,
            other => panic!("expected host connection request, got {other:?}"),
        };
        let local_connection_id = host_request.connection_id;
        let remote_connection_id = 29;
        let name = c4(b"Alice");
        secondary
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                test_client_core(i32::try_from(canonical_id).unwrap(), name, true),
                remote_connection_id,
                false,
            )))
            .await
            .test_value();
        loop {
            match secondary.read_message().await.test_value() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected positive host connection reply, got {other:?}"),
            }
        }
        secondary
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                true,
                c4(b"connection accepted"),
                false,
            )))
            .await
            .test_value();

        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 2 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert!(routes.contains(&(local_connection_id, canonical_id, remote_connection_id,)));

        while let Ok(event) = host_events.try_recv() {
            assert!(
                !matches!(
                    event,
                    HostEvent::ClientJoined { client_id, .. } if client_id == canonical_id
                ),
                "secondary route emitted duplicate ClientJoined"
            );
        }

        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(quiet_deadline, secondary.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                Ok(Ok(message)) => {
                    panic!("secondary route received duplicate first-connect setup: {message:?}")
                }
                Ok(Err(error)) => panic!("secondary route closed unexpectedly: {error}"),
            }
        }

        let countdown = crate::LobbyCountdownPacket::new(7);
        host.submit_lobby_countdown(countdown).await.test_value();
        timeout(EVENT_WAIT, async {
            loop {
                match canonical_events.recv().await {
                    Some(ClientEvent::LobbyCountdown { packet }) if packet == countdown => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("canonical route disconnected unexpectedly: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("canonical event stream ended before lobby countdown"),
                }
            }
        })
        .await
        .test_value();

        // RemoveConn clears only the failed route. OnDisconnect removes the
        // logical client only when no message route remains
        // (src/C4Network2.cpp:1758-1783;
        // src/C4Network2Client.cpp:78-102).
        while host_events.try_recv().is_ok() {}
        drop(secondary);
        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 1 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert!(routes.iter().all(|(connection_id, client_id, _)| {
            *connection_id != local_connection_id && *client_id == canonical_id
        }));

        while let Ok(event) = host_events.try_recv() {
            match event {
                HostEvent::ClientLeft { client_id } if client_id == canonical_id => {
                    panic!("secondary disconnect emitted ClientLeft for the logical client")
                }
                HostEvent::ClientConnectionFailed { client_id } if client_id == canonical_id => {
                    panic!("secondary disconnect emitted ClientConnectionFailed")
                }
                HostEvent::SyncScheduled { controls, .. }
                    if controls.iter().any(|control| {
                        matches!(
                            control,
                            EngineControlPacket::ClientRemove(remove)
                                if remove.client_id == i32::try_from(canonical_id).unwrap()
                        )
                    }) =>
                {
                    panic!("secondary disconnect queued ClientRemove for the logical client")
                }
                _ => {}
            }
        }

        let after_disconnect = crate::LobbyCountdownPacket::new(6);
        host.submit_lobby_countdown(after_disconnect)
            .await
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                match canonical_events.recv().await {
                    Some(ClientEvent::LobbyCountdown { packet }) if packet == after_disconnect => {
                        break;
                    }
                    Some(ClientEvent::SyncScheduled { controls, .. })
                        if controls.iter().any(|control| {
                            matches!(
                                control,
                                EngineControlPacket::ClientRemove(remove)
                                    if remove.client_id == i32::try_from(canonical_id).unwrap()
                            )
                        }) =>
                    {
                        panic!("canonical client executed a secondary-route ClientRemove")
                    }
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("canonical route disconnected unexpectedly: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("canonical event stream ended after secondary disconnect"),
                }
            }
        })
        .await
        .test_value();

        host.shutdown().await.test_value();
        canonical.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn primary_route_loss_promotes_the_surviving_secondary() {
        // RemoveConn promotes the remaining data route to the message route;
        // OnDisconnect removes the logical client only when that fallback is
        // absent (src/C4Network2Client.cpp:78-102;
        // src/C4Network2.cpp:1758-1783).
        async fn connect_secondary(
            addr: SocketAddr,
            client_id: ClientId,
            remote_connection_id: u32,
        ) -> (crate::ControlTransport<TcpStream>, u32) {
            let stream = TcpStream::connect(addr).await.test_value();
            let mut transport = crate::ControlTransport::new(stream);
            let host_request = match transport.read_message().await.test_value() {
                ControlMessage::ConnectionRequest(request) => request,
                other => panic!("expected host connection request, got {other:?}"),
            };
            let name = c4(b"Alice");
            transport
                .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                    test_client_core(i32::try_from(client_id).unwrap(), name, true),
                    remote_connection_id,
                    false,
                )))
                .await
                .test_value();
            loop {
                match transport.read_message().await.test_value() {
                    ControlMessage::ConnectionReply(reply) if reply.ok => break,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .test_value();
                    }
                    other => panic!("expected positive host connection reply, got {other:?}"),
                }
            }
            transport
                .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                    true,
                    c4(b"connection accepted"),
                    true,
                )))
                .await
                .test_value();
            (transport, host_request.connection_id)
        }

        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut canonical = connect_test_player(addr, "Alice").await;
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();
        let remote_connection_id = 31;
        let (mut secondary, secondary_connection_id) =
            connect_secondary(addr, canonical_id, remote_connection_id).await;

        timeout(EVENT_WAIT, async {
            loop {
                if host.accepted_routes().await.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        while host_events.try_recv().is_ok() {}

        let dead_route_countdown = crate::LobbyCountdownPacket::new(9);
        host.submit_lobby_countdown(dead_route_countdown)
            .await
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                match canonical_events.recv().await {
                    Some(ClientEvent::LobbyCountdown { packet })
                        if packet == dead_route_countdown =>
                    {
                        break;
                    }
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("canonical route disconnected unexpectedly: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("canonical event stream ended before the test packet"),
                }
            }
        })
        .await
        .test_value();

        canonical.shutdown().await.test_value();
        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 1 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        assert_eq!(
            routes,
            vec![(secondary_connection_id, canonical_id, remote_connection_id,)]
        );

        // OnDisconn first removes the dead route (promoting the remaining
        // data route), then sends that dead route's exact packet backlog to the
        // same logical client through its new message route
        // (src/C4Network2.cpp:884-905;
        // src/C4Network2Client.cpp:90-102).
        let recovery = timeout(EVENT_WAIT, async {
            loop {
                match secondary.read_message().await {
                    Ok(ControlMessage::PostMortem(packet)) => break packet,
                    Ok(ControlMessage::Ping(ping)) => {
                        secondary
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .test_value();
                    }
                    Ok(_) => continue,
                    Err(error) => panic!("surviving route closed unexpectedly: {error}"),
                }
            }
        })
        .await
        .test_value();
        assert_eq!(recovery.connection_id, 0);
        assert!(recovery.packets.iter().any(|packet| {
            matches!(
                crate::transport::parse_complete_packet(packet),
                Ok(Some(ControlMessage::LobbyCountdown(packet))) if packet == dead_route_countdown
            )
        }));

        while let Ok(event) = host_events.try_recv() {
            match event {
                HostEvent::ClientLeft { client_id } if client_id == canonical_id => {
                    panic!("primary disconnect emitted ClientLeft despite a surviving route")
                }
                HostEvent::SyncScheduled { controls, .. }
                    if controls.iter().any(|control| {
                        matches!(
                            control,
                            EngineControlPacket::ClientRemove(remove)
                                if remove.client_id == i32::try_from(canonical_id).unwrap()
                        )
                    }) =>
                {
                    panic!("primary disconnect queued ClientRemove despite a surviving route")
                }
                _ => {}
            }
        }

        let countdown = crate::LobbyCountdownPacket::new(5);
        host.submit_lobby_countdown(countdown).await.test_value();
        timeout(EVENT_WAIT, async {
            loop {
                match secondary.read_message().await {
                    Ok(ControlMessage::LobbyCountdown(packet)) if packet == countdown => break,
                    Ok(ControlMessage::Ping(ping)) => {
                        secondary
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .test_value();
                    }
                    Ok(ControlMessage::Packet { data, .. })
                        if matches!(
                            decode_control_entry_payload(&data),
                            Ok(EngineControlPacket::ClientRemove(remove))
                                if remove.client_id == i32::try_from(canonical_id).unwrap()
                        ) =>
                    {
                        panic!("surviving route received ClientRemove")
                    }
                    Ok(_) => continue,
                    Err(error) => panic!("surviving route closed unexpectedly: {error}"),
                }
            }
        })
        .await
        .test_value();

        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_replays_a_dead_routes_post_mortem_suffix_once() {
        // OnDisconn retains the closed connection and its iInPacketCounter.
        // PID_PostMortem received over another route looks up the dead local
        // ConnID, dispatches only the consecutive suffix beginning at that
        // counter under the dead connection's CCore, and removes it afterward
        // (src/C4Network2IO.cpp:520-570,594-597,1036-1055,1351-1356).
        async fn connect_existing_route(
            addr: SocketAddr,
            client_id: ClientId,
            remote_connection_id: u32,
        ) -> (crate::ControlTransport<TcpStream>, u32) {
            let stream = TcpStream::connect(addr).await.test_value();
            let mut transport = crate::ControlTransport::new(stream);
            let host_request = match transport.read_message().await.test_value() {
                ControlMessage::ConnectionRequest(request) => request,
                other => panic!("expected host connection request, got {other:?}"),
            };
            let name = c4(b"Alice");
            transport
                .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                    test_client_core(i32::try_from(client_id).unwrap(), name, true),
                    remote_connection_id,
                    false,
                )))
                .await
                .test_value();
            loop {
                match transport.read_message().await.test_value() {
                    ControlMessage::ConnectionReply(reply) if reply.ok => break,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .test_value();
                    }
                    other => panic!("expected positive host connection reply, got {other:?}"),
                }
            }
            transport
                .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                    true,
                    c4(b"connection accepted"),
                    false,
                )))
                .await
                .test_value();
            (transport, host_request.connection_id)
        }

        async fn encode_nested(message: ControlMessage) -> Vec<u8> {
            let (writer, mut reader) = duplex(256);
            let mut transport = crate::ControlTransport::new(writer);
            transport.send_message(message).await.test_value();
            let mut header = [0; 5];
            reader.read_exact(&mut header).await.test_value();
            assert_eq!(header[0], 0xff);
            let length = u32::from_ne_bytes(header[1..].try_into().test_value()) as usize;
            let mut packet = vec![0; length];
            reader.read_exact(&mut packet).await.test_value();
            packet
        }

        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let canonical = connect_test_player(addr, "Alice").await;
        let client_id = canonical.client_id();
        let (mut dead_route, dead_connection_id) =
            connect_existing_route(addr, client_id, 29).await;
        let (mut surviving_route, _surviving_connection_id) =
            connect_existing_route(addr, client_id, 30).await;

        timeout(EVENT_WAIT, async {
            while host.accepted_routes().await.len() != 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        while host_events.try_recv().is_ok() {}

        for tick in [100, 101] {
            dead_route
                .send_message(ControlMessage::ActivationRequest { tick })
                .await
                .test_value();
        }
        let mut received_before_close = Vec::new();
        timeout(EVENT_WAIT, async {
            while received_before_close.len() != 2 {
                match host_events.recv().await {
                    Some(HostEvent::ActivationRequest {
                        client_id: source,
                        tick,
                        ..
                    }) if source == client_id => received_before_close.push(tick),
                    Some(_) => {}
                    None => panic!("host event stream ended before route close"),
                }
            }
        })
        .await
        .test_value();
        assert_eq!(received_before_close, vec![100, 101]);

        let recovery = crate::PostMortemPacket {
            connection_id: dead_connection_id,
            packet_counter: 4,
            packets: vec![
                encode_nested(ControlMessage::ActivationRequest { tick: 100 }).await,
                encode_nested(ControlMessage::ActivationRequest { tick: 101 }).await,
                encode_nested(ControlMessage::ActivationRequest { tick: 102 }).await,
                encode_nested(ControlMessage::ActivationRequest { tick: 103 }).await,
            ],
        };
        surviving_route
            .send_message(ControlMessage::PostMortem(recovery.clone()))
            .await
            .test_value();
        raw_client_ping_barrier(&mut surviving_route).await;
        timeout(EVENT_WAIT, async {
            while host.accepted_routes().await.len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        drop(dead_route);

        let mut recovered = Vec::new();
        timeout(EVENT_WAIT, async {
            while recovered.len() != 2 {
                match host_events.recv().await {
                    Some(HostEvent::ActivationRequest {
                        client_id: source,
                        tick,
                        ..
                    }) if source == client_id => recovered.push(tick),
                    Some(HostEvent::TransportError { error, .. }) => {
                        panic!("post-mortem recovery failed: {error}")
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended during recovery"),
                }
            }
        })
        .await
        .test_value();
        assert_eq!(recovered, vec![102, 103]);

        surviving_route
            .send_message(ControlMessage::PostMortem(recovery))
            .await
            .test_value();
        surviving_route
            .send_message(ControlMessage::ActivationRequest { tick: 104 })
            .await
            .test_value();
        timeout(EVENT_WAIT, async {
            loop {
                match host_events.recv().await {
                    Some(HostEvent::ActivationRequest {
                        client_id: source,
                        tick: 104,
                        ..
                    }) if source == client_id => break,
                    Some(HostEvent::ActivationRequest { tick, .. }) => {
                        panic!("retired dead route replayed packet {tick} twice")
                    }
                    Some(HostEvent::TransportError { error, .. }) => {
                        panic!("duplicate recovery was rejected noisily: {error}")
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended after recovery"),
                }
            }
        })
        .await
        .test_value();

        drop(surviving_route);
        canonical.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn nonresponsive_server_handshake_times_out() {
        // C4Network2IO::CheckTimeout closes connections which do not reach the
        // accepted state after C4NetAcceptTimeout (src/C4Network2IO.cpp:1155-1170).
        let (addr, listener) = bind_test_listener().await;
        let (connection, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
        let client_stream = connection.test_value();
        let (_server_stream, _) = accepted.test_value();

        let result = timeout(
            HANDSHAKE_TIMEOUT + Duration::from_secs(1),
            connect_client_from_inner(
                ready(Ok(client_stream)),
                ClientConfig::new("Alice", ParticipantKind::Player),
                Some(ConnectionLivenessState::new_test(0, 0)),
            ),
        )
        .await;

        match result {
            Ok(Err(ClientError::Handshake(message))) => {
                assert_eq!(message, "connection admission timed out");
            }
            other => panic!("expected bounded handshake timeout, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_emits_one_decodable_ready_packet_for_host_and_client() {
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, host_config!(max_players: 4))
            .await
            .test_value();

        let client = connect_test_player(addr, "Alice").await;
        let mut events = host.take_event_receiver();
        activate_joined_client(&host, &mut events, client.client_id()).await;

        client
            .submit_control(legacy_packet(1, 0, 0x12))
            .await
            .test_value();
        host.submit_local_control(legacy_packet(0, 0, 0x34))
            .await
            .test_value();

        let packet = wait_for_host_ready(&mut events, EVENT_WAIT).await;
        assert_eq!(packet.tick(), 0);
        assert_eq!(packet.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&packet), vec![0x34, 0x12]);

        shutdown_test_session(client, host).await;
    }

    fn take_queued_host_ready(events: &mut mpsc::Receiver<HostEvent>) -> Option<ControlPacket> {
        while let Ok(event) = events.try_recv() {
            match event {
                HostEvent::Ready { packet } => return Some(packet),
                HostEvent::TransportError { error, .. } => {
                    panic!("host transport failed while checking async control: {error}")
                }
                _ => {}
            }
        }
        None
    }

    fn take_queued_client_ready(events: &mut mpsc::Receiver<ClientEvent>) -> Option<ControlPacket> {
        while let Ok(event) = events.try_recv() {
            match event {
                ClientEvent::Ready { packet } => return Some(packet),
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected while checking async control: {reason:?}")
                }
                _ => {}
            }
        }
        None
    }

    async fn settle_paused_network() {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn async_host_forces_and_broadcasts_incomplete_tick_after_strict_budget() {
        let (address, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        config.initial_status.control_mode = 2;
        config.async_max_wait_frames = 2;
        config
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .control_rate = 2;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        // Keep one task runnable while live loopback setup completes. Without
        // this guard Tokio may auto-advance paused time to the dial timeout
        // before the OS socket becomes ready under a heavily loaded test run.
        let frozen_time_guard = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let mut client = connect_test_player(address, "Slow").await;
        let mut client_events = client.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        frozen_time_guard.abort();
        let _ = frozen_time_guard.await;

        host.control_tick_reached(
            0,
            2,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .test_value();
        host.set_join_allowed(true).await.test_value();

        // floor(2 * 2 * 1000 / 38) = 105ms. Native still waits at
        // equality and first permits the incomplete packet at 106ms.
        tokio::time::advance(Duration::from_millis(105)).await;
        host.set_join_allowed(true).await.test_value();
        settle_paused_network().await;
        assert!(take_queued_host_ready(&mut host_events).is_none());
        assert!(take_queued_client_ready(&mut client_events).is_none());

        tokio::time::advance(Duration::from_millis(1)).await;
        // This completion is a host-loop barrier. Because the deadline branch
        // is biased above commands, the expired tick is forced first.
        host.set_join_allowed(true).await.test_value();
        let host_ready = take_queued_host_ready(&mut host_events).test_value();
        assert_eq!(host_ready.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(host_ready.tick(), 0);
        assert_eq!(control_commands(&host_ready), vec![0xA0]);

        // The host event proves the exact paused-time boundary. Let live TCP
        // use wall-clock scheduling while checking delivery to the client.
        tokio::time::resume();
        let client_ready = wait_for_client_ready(&mut client_events, EVENT_WAIT).await;
        assert_eq!(client_ready, host_ready);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(start_paused = true)]
    async fn central_and_decentral_never_force_incomplete_ticks() {
        for mode in [0, 1] {
            let (address, listener) = bind_test_listener().await;
            let mut config = HostConfig::default();
            config.initial_status.control_mode = mode;
            config.async_max_wait_frames = 2;
            config
                .initial_join_snapshot
                .as_mut()
                .test_value()
                .parameters
                .control_rate = 2;
            let mut host = start_host(listener, config).await.test_value();
            let mut host_events = host.take_event_receiver();
            let mut client = connect_test_player(address, format!("Slow-{mode}")).await;
            let mut client_events = client.take_event_receiver();
            activate_joined_client(&host, &mut host_events, client.client_id()).await;

            host.control_tick_reached(
                0,
                2,
                DEFAULT_CONTROL_TARGET_FPS,
                tokio::time::Instant::now(),
            )
            .await
            .test_value();
            host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0 + mode))
                .await
                .test_value();
            host.set_join_allowed(true).await.test_value();
            tokio::time::advance(Duration::from_secs(1)).await;
            host.set_join_allowed(true).await.test_value();
            settle_paused_network().await;

            assert!(
                take_queued_host_ready(&mut host_events).is_none(),
                "mode {mode} forced an incomplete host tick"
            );
            assert!(
                take_queued_client_ready(&mut client_events).is_none(),
                "mode {mode} broadcast an incomplete complete packet"
            );

            shutdown_test_session(client, host).await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn async_mode_commit_uses_tick_reach_stamped_in_central_mode() {
        let (address, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        config.initial_status.control_mode = 1;
        config.async_max_wait_frames = 2;
        config
            .initial_join_snapshot
            .as_mut()
            .test_value()
            .parameters
            .control_rate = 2;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(address, "Slow").await;
        let mut client_events = client.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;

        host.control_tick_reached(
            0,
            2,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .test_value();
        host.set_join_allowed(true).await.test_value();
        tokio::time::advance(Duration::from_secs(1)).await;
        host.set_join_allowed(true).await.test_value();
        settle_paused_network().await;
        assert!(take_queued_host_ready(&mut host_events).is_none());

        let asynchronous = NetworkStatus::new(NETWORK_STATE_GO, 2, 0);
        host.change_status(asynchronous).await.test_value();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Status(status)) if status == asynchronous => break,
                Some(_) => continue,
                None => panic!("client event stream ended before async status"),
            }
        }
        client.submit_status_ack(asynchronous).await.test_value();
        host.status_reached(asynchronous, asynchronous.target_tick)
            .await
            .test_value();

        let mut ready = None;
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusCommitted(status)) if status == asynchronous => break,
                Some(HostEvent::Ready { packet }) => ready = Some(packet),
                Some(_) => continue,
                None => panic!("host event stream ended before async commit"),
            }
        }
        for _ in 0..256 {
            tokio::task::yield_now().await;
            ready = ready.or_else(|| take_queued_host_ready(&mut host_events));
            if ready.is_some() {
                break;
            }
        }
        let ready = ready.test_value();
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0xA0]);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn decentralized_host_and_two_clients_pack_the_same_ordered_tick() {
        // Every participant stores its own input, receives each other active
        // client's contribution through direct/forwarded broadcast, and runs
        // PackCompleteCtrl in client-ID order (pristine C++
        // src/C4GameControlNetwork.cpp:156-179,741-783).
        let (address, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut alpha = connect_test_player(address, "Alpha").await;
        let mut alpha_events = alpha.take_event_receiver();
        activate_joined_client(&host, &mut host_events, alpha.client_id()).await;
        let mut beta = connect_test_player(address, "Beta").await;
        let mut beta_events = beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, beta.client_id()).await;

        host.submit_local_control(legacy_packet(0, 0, 0x10))
            .await
            .test_value();
        alpha
            .submit_control(legacy_packet(alpha.client_id(), 0, 0x20))
            .await
            .test_value();
        beta.submit_control(legacy_packet(beta.client_id(), 0, 0x30))
            .await
            .test_value();

        let host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        let alpha_ready = wait_for_client_ready(&mut alpha_events, EVENT_WAIT).await;
        let beta_ready = wait_for_client_ready(&mut beta_events, EVENT_WAIT).await;
        assert_eq!(host_ready, alpha_ready);
        assert_eq!(host_ready, beta_ready);
        assert_eq!(control_commands(&host_ready), vec![0x10, 0x20, 0x30]);
        for events in [&mut alpha_events, &mut beta_events] {
            while let Ok(Some(event)) = timeout(Duration::from_millis(50), events.recv()).await {
                assert!(
                    !matches!(event, ClientEvent::Ready { .. }),
                    "one decentralized contribution emitted more than one complete tick"
                );
            }
        }

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn join_data_rebinds_local_client_and_host_emits_direct_join_first() {
        // Host Join inserts the canonical client before ConnRe/JoinData; the
        // client then rebinds its unknown local object to the assigned ID
        // (src/C4Network2.cpp:1395-1445,1574-1604;
        // src/C4Client.cpp:284-290,321-350).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(addr, "Alice").await;
        let client_id = client.client_id();
        let join_data = client.take_join_data().test_value();

        assert_eq!(join_data.client_id, i32::try_from(client_id).unwrap());
        assert_eq!(
            join_data
                .parameters
                .clients
                .clients
                .iter()
                .map(|core| core.client_id)
                .collect::<Vec<_>>(),
            vec![0, i32::try_from(client_id).unwrap()]
        );
        assert_eq!(
            join_data.parameters.clients.local_client_id,
            Some(i32::try_from(client_id).unwrap())
        );
        assert!(client.take_join_data().is_none());

        assert!(matches!(
            timeout(EVENT_WAIT, host_events.recv()).await.unwrap(),
            Some(HostEvent::Direct {
                delivery: ControlDelivery::Direct,
                ..
            })
        ));
        assert!(matches!(
            timeout(EVENT_WAIT, host_events.recv()).await.unwrap(),
            Some(HostEvent::ClientJoined {
                client_id: joined,
                ..
            }) if joined == client_id
        ));

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_sends_cpp_address_packets_immediately_after_join_data() {
        // SendJoinData writes PID_JoinData and then every known PID_Addr on the
        // accepted message connection before resource discovery begins
        // (src/C4Network2.cpp:1810-1850;
        // src/C4Network2Client.cpp:319-337,616-621).
        let (addr, listener) = bind_test_listener().await;
        let host = start_host(
            listener,
            host_config!(resource_registrations: vec![
                crate::ResourceRegistration {
                    resource_id: 3,
                    chunk_count: 1,
                    binary_compatible: true,
                    loading: false,
                },
                crate::ResourceRegistration {
                    resource_id: 4,
                    chunk_count: 2,
                    binary_compatible: true,
                    loading: false,
                },
            ]),
        )
        .await
        .test_value();
        let stream = TcpStream::connect(addr).await.test_value();
        let mut transport = crate::ControlTransport::new(stream);
        let name = c4(b"Alice");
        let request = test_connection_request(
            clonk_engine::ClientCoreControlData {
                client_id: -1,
                name: name.clone(),
                nick: name,
                ..Default::default()
            },
            0,
            false,
        );

        let bootstrap = run_client_connection_handshake(&mut transport, request)
            .await
            .test_value();
        assert_eq!(bootstrap.join_data.client_id, 1);

        let packet = await_test(transport.read_message()).await;
        match packet {
            ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                address:
                    crate::NetworkAddress {
                        protocol: crate::NetworkProtocol::Tcp,
                        endpoint,
                    },
            }) => assert_eq!(
                endpoint,
                format!("0.0.0.0:{}", addr.port()).parse().unwrap()
            ),
            other => panic!("expected host PID_Addr after JoinData, got {other:?}"),
        }
        loop {
            match await_test(transport.read_message()).await {
                ControlMessage::Address(crate::AddressPacket { client_id: 0, .. }) => continue,
                ControlMessage::Resource(ResourcePacket::Discover(discover)) => {
                    assert_eq!(discover.resource_ids, vec![4, 3]);
                    break;
                }
                other => panic!("expected PID_Addr* then PID_NetResDis, got {other:?}"),
            }
        }

        let client_address = crate::AddressPacket {
            client_id: 1,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "0.0.0.0:11112".parse().test_value(),
            ),
        };
        transport
            .send_message(ControlMessage::Address(client_address))
            .await
            .test_value();
        let mut saw_reannouncement = false;
        for _ in 0..8 {
            let message = await_test(transport.read_message()).await;
            if message == ControlMessage::Address(client_address) {
                saw_reannouncement = true;
                break;
            }
        }
        assert!(
            saw_reannouncement,
            "host did not re-announce the newly learned client address"
        );

        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_requests_the_join_tick_before_announcing_addresses() {
        // HandleJoinData initializes C4GameControlNetwork, whose Init sends
        // PID_ControlReq(start tick), before SendAddresses emits PID_Addr
        // (src/C4Network2.cpp:1603-1623;
        // src/C4GameControlNetwork.cpp:46-62).
        let (client_stream, host_stream) = duplex(512);
        let mut client = crate::ControlTransport::new(client_stream);
        let mut host = crate::ControlTransport::new(host_stream);
        let host_address = crate::NetworkAddress::new(
            crate::NetworkProtocol::Tcp,
            "192.0.2.4:11112".parse().test_value(),
        );

        send_client_post_join_packets(
            &mut client,
            17,
            vec![crate::AddressPacket {
                client_id: 0,
                address: host_address,
            }],
        )
        .await
        .test_value();

        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Request { from_tick: 17 }
        );
        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                address: host_address,
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_control_request_precedes_dynamic_failure_even_after_bad_game_resource() {
        // HandleJoinData initializes network control first, ignores the first
        // GameRes.InitNetwork failure, and only then treats Dynamic failure as
        // fatal (src/C4Network2.cpp:1603-1618).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.game_resources.push(nonloadable_core(
            crate::HostResourceType::System as u8,
            9,
            b"System.c4g",
        ));
        snapshot.dynamic = nonloadable_core(2, 7, b"Dynamic.c4d");
        let (address, server) = start_client_bootstrap_probe(snapshot).await;

        let result =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player)).await;
        let probe = server.await.test_value();
        let messages = probe.messages;

        let error = result.expect_err("missing non-loadable Dynamic must abort bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Dynamic.c4d") && message.contains("non-loadable")),
            "the ignored early GameRes failure masked Dynamic: {error:?}"
        );
        assert_eq!(
            messages,
            vec![
                ControlMessage::Request { from_tick: 0 },
                // The Rust port extension advertises retained-round support as
                // soon as the admitted port route exists. It must not disturb
                // the native control-before-resource ordering pinned here.
                ControlMessage::PortCapabilities(
                    crate::PortCapabilities::supported_without_voice(),
                ),
            ],
            "control initialization must precede Dynamic retrieval, but addresses must not"
        );
        assert!(
            probe.disconnected,
            "C4Network2::HandleJoinData calls Clear immediately after Dynamic bootstrap failure"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_announces_addresses_before_final_scenario_validation_failure() {
        // HandleJoinData accepts known addresses into the nonblocking output
        // buffer before outer InitClient calls Parameters.InitNetwork, whose
        // first required resource is Scenario
        // (src/C4Network2.cpp:1620-1622,329-331;
        // src/C4GameParameters.cpp:539-547). Fatal teardown may clear a still-
        // buffered PID_Addr before the peer observes it, so wire receipt is not
        // asserted (src/C4NetIO.cpp:1348-1399,1461-1472).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.scenario = nonloadable_core(1, 8, b"Scenario.c4s");
        let (address, server) = start_client_bootstrap_probe(snapshot).await;

        let result =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player)).await;
        let probe = server.await.test_value();
        let messages = probe.messages;

        let error = result.expect_err("missing non-loadable Scenario must abort bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Scenario.c4s") && message.contains("non-loadable")),
            "unexpected Scenario bootstrap failure: {error:?}"
        );
        assert_eq!(
            messages.first(),
            Some(&ControlMessage::Request { from_tick: 0 })
        );
        assert!(
            probe.disconnected,
            "C4Network2::InitClient failure must clear the admitted Scenario route"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_rechecks_failed_game_resource_after_announcing_addresses() {
        // The early GameRes result is ignored. After accepting addresses into
        // the nonblocking output buffer, outer Parameters.InitNetwork retries
        // GameRes after Scenario and makes the same missing non-loadable core
        // fatal
        // (src/C4Network2.cpp:1612-1622,329-331;
        // src/C4GameParameters.cpp:237-247,539-547). Fatal teardown may clear a
        // still-buffered PID_Addr before the peer observes it, so wire receipt
        // is not asserted (src/C4NetIO.cpp:1348-1399,1461-1472).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.game_resources.push(nonloadable_core(
            crate::HostResourceType::Definitions as u8,
            9,
            b"Objects.c4d",
        ));
        let (address, server) = start_client_bootstrap_probe(snapshot).await;

        let result =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player)).await;
        let probe = server.await.test_value();
        let messages = probe.messages;

        let error = result.expect_err("final GameRes retry must fail bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Objects.c4d") && message.contains("non-loadable")),
            "unexpected GameRes bootstrap failure: {error:?}"
        );
        assert_eq!(
            messages.first(),
            Some(&ControlMessage::Request { from_tick: 0 })
        );
        assert!(
            probe.disconnected,
            "C4Network2::InitClient failure must clear the admitted GameRes route"
        );
    }

    fn nonloadable_core(
        resource_type: u8,
        id: i32,
        filename: &[u8],
    ) -> clonk_engine::NetworkResourceCore {
        network_core!(resource_type,
        id,
        derived_id: -1,
        loadable: false,
        file_size: u32::MAX,
        file_crc: u32::MAX,
        contents_crc: 1,
        filename: c4(filename))
    }

    struct ClientBootstrapProbeResult {
        messages: Vec<ControlMessage>,
        disconnected: bool,
    }

    async fn start_client_bootstrap_probe(
        snapshot: HostJoinSnapshot,
    ) -> (
        SocketAddr,
        tokio::task::JoinHandle<ClientBootstrapProbeResult>,
    ) {
        let (address, listener) = bind_test_listener().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            let mut transport = crate::ControlTransport::new(stream);
            admit_and_send_test_join_data(&mut transport, |_| snapshot).await;

            let mut messages = Vec::new();
            let mut disconnected = false;
            while messages.len() < 4 {
                match timeout(Duration::from_millis(250), transport.read_message()).await {
                    Ok(Ok(message)) => messages.push(message),
                    Ok(Err(_)) => {
                        disconnected = true;
                        break;
                    }
                    Err(_) => break,
                }
            }
            ClientBootstrapProbeResult {
                messages,
                disconnected,
            }
        });
        (address, server)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_late_join_data_packet_is_ignored_without_dropping_the_link() {
        // `HandleJoinData` returns immediately unless the client is still in
        // GS_Init: it logs "unexpected join data received!" and drops the
        // packet. That early return is deliberately *before* every `Clear()`
        // path in the function, so a duplicate or late JoinData neither tears
        // the session down nor re-applies the client ID, control tick, status
        // or parameters over a live session
        // (7d43b47b src/C4Network2.cpp:1574-1592).
        //
        // Admission consumes the one valid JoinData, so anything reaching this
        // packet loop is by definition late. Re-applying one here would reset
        // the control clock and start barrier mid-game.
        let (host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop(512, 4, 4);
        let mut host = crate::ControlTransport::new(host_stream);

        let host_config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host_config.local_core, 8);
        host.send_message(ControlMessage::JoinData(Box::new(test_join_data(
            1,
            NetworkStatus::new(NETWORK_STATE_GO, 2, 4_242),
            snapshot,
        ))))
        .await
        .test_value();

        // The next ordinary packet still arrives and is still the *first*
        // event: the link survived (C++ logs rather than disconnecting) and
        // the late JoinData produced no event of its own.
        let status = NetworkStatus::new(NETWORK_STATE_PAUSE, 3, 17);
        host.send_message(ControlMessage::Status(status))
            .await
            .test_value();
        match timeout(EVENT_WAIT, event_rx.recv()).await {
            Ok(Some(ClientEvent::Status(received))) => assert_eq!(received, status),
            other => panic!("expected the Status following a dropped JoinData, got {other:?}"),
        }

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_accepts_exactly_one_join_data_after_a_lobby_restart_notice() {
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let (host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop_with_state(512, 4, 4, BTreeMap::new(), resource_state);
        let mut host = crate::ControlTransport::new(host_stream);
        let config = HostConfig::default();
        let mut first_snapshot = synthetic_join_snapshot(config.local_core.clone(), 8);
        first_snapshot.parameters.title = c4(b"Accepted restart");
        first_snapshot.parameters.clients.local_client_id = None;
        let first = test_join_data(1, config.initial_status, first_snapshot);
        let mut second = first.clone();
        second.parameters.title = c4(b"Unsolicited duplicate");

        host.send_message(ControlMessage::HostRestartLobby { restart_nonce: 42 })
            .await
            .test_value();
        host.send_message(ControlMessage::JoinData(Box::new(first.clone())))
            .await
            .test_value();
        host.send_message(ControlMessage::JoinData(Box::new(second)))
            .await
            .test_value();
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 0, -1);
        host.send_message(ControlMessage::Status(status))
            .await
            .test_value();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.test_value(),
            Some(ClientEvent::HostRestartLobby)
        ));
        loop {
            match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
                Some(ClientEvent::JoinData { join_data }) => {
                    assert_eq!(*join_data, first);
                    break;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before restarted JoinData"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
                Some(ClientEvent::Status(received)) => {
                    assert_eq!(received, status);
                    break;
                }
                Some(ClientEvent::JoinData { join_data }) => {
                    panic!("client accepted duplicate restart JoinData: {join_data:?}")
                }
                Some(_) => continue,
                None => panic!("client event stream ended after restarted JoinData"),
            }
        }

        let (completion, acknowledged) = oneshot::channel();
        command_tx
            .send(ClientCommand::AcknowledgeRoundRestart { completion })
            .await
            .test_value();
        acknowledged.await.test_value().test_value();
        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(deadline, host.read_message()).await {
                Ok(Ok(ControlMessage::RoundRestartAck { restart_nonce: 42 })) => break,
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => panic!("restart acknowledgement read failed: {error}"),
                Err(_) => panic!("client never sent its restart acknowledgement"),
            }
        }

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn restarted_client_pauses_resource_timer_until_delayed_ack() {
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let (host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop_with_state(512, 4, 8, BTreeMap::new(), resource_state);
        let mut host = crate::ControlTransport::new(host_stream);
        let config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(config.local_core, 8);
        snapshot.parameters.clients.local_client_id = None;
        snapshot.dynamic = network_core!(resource_type: 2,
        id: 4,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        contents_crc: 0x8bd6_88e8,
        chunk_size: 5,
        filename: c4(b"FreshDynamic.c4d"));
        let join_data = test_join_data(1, config.initial_status, snapshot);
        host.send_message(ControlMessage::HostRestartLobby { restart_nonce: 42 })
            .await
            .test_value();
        host.send_message(ControlMessage::JoinData(Box::new(join_data)))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.test_value(),
            Some(ClientEvent::HostRestartLobby)
        ));
        loop {
            match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
                Some(ClientEvent::JoinData { .. }) => break,
                Some(_) => continue,
                None => panic!("client event stream ended before restarted JoinData"),
            }
        }

        // The periodic resource timer must remain behind the same application
        // ACK fence as packets arriving from the host. The empty pre-restart
        // catalog makes the first post-JoinData tick deterministically want to
        // discover both fresh resources.
        tokio::time::advance(Duration::from_millis(crate::NETWORK_TIMER_INTERVAL_MS)).await;
        tokio::task::yield_now().await;
        let timer_deadline = tokio::time::Instant::now() + Duration::from_millis(1);
        while let Ok(Ok(message)) = timeout_at(timer_deadline, host.read_message()).await {
            assert!(
                !matches!(message, ControlMessage::Resource(_)),
                "the client resource timer ran before releasing the restart fence: {message:?}"
            );
        }

        let (completion, acknowledged) = oneshot::channel();
        command_tx
            .send(ClientCommand::AcknowledgeRoundRestart { completion })
            .await
            .test_value();
        acknowledged.await.test_value().test_value();
        let ack_deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(ack_deadline, host.read_message()).await {
                Ok(Ok(ControlMessage::RoundRestartAck { restart_nonce: 42 })) => break,
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => panic!("restart acknowledgement read failed: {error}"),
                Err(_) => panic!("client never sent its restart acknowledgement"),
            }
        }

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restarted_client_ignores_resource_status_until_ack_releases_round_fence() {
        let directories = SessionResourceDirectories::new();
        let resource_state = empty_client_resource_state(1, directories.client.clone());
        let (host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop_with_state(512, 4, 8, BTreeMap::new(), resource_state);
        let mut host = crate::ControlTransport::new(host_stream);
        let config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(config.local_core, 8);
        snapshot.parameters.clients.local_client_id = None;
        snapshot.dynamic = network_core!(resource_type: 2,
        id: 4,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        contents_crc: 0x8bd6_88e8,
        chunk_size: 5,
        filename: c4(b"FreshDynamic.c4d"));
        let join_data = test_join_data(1, config.initial_status, snapshot);
        let complete_status =
            ControlMessage::Resource(ResourcePacket::Status(crate::ResourceStatusPacket {
                resource_id: 4,
                chunks: crate::ResourceChunkAvailability {
                    chunk_count: 1,
                    ranges: vec![crate::ResourceChunkRange {
                        start: 0,
                        length: 1,
                    }],
                },
            }));

        host.send_message(ControlMessage::HostRestartLobby { restart_nonce: 42 })
            .await
            .test_value();
        host.send_message(ControlMessage::JoinData(Box::new(join_data)))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.test_value(),
            Some(ClientEvent::HostRestartLobby)
        ));
        loop {
            match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
                Some(ClientEvent::JoinData { .. }) => break,
                Some(_) => continue,
                None => panic!("client event stream ended before restarted JoinData"),
            }
        }

        // A fresh Status arriving while the app installs JoinData must not
        // schedule traffic that the host will quarantine before the ACK.
        host.send_message(complete_status.clone())
            .await
            .test_value();
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Ok(message)) = timeout_at(quiet_deadline, host.read_message()).await {
            assert!(
                !matches!(
                    message,
                    ControlMessage::Resource(ResourcePacket::Request(
                        crate::ResourceRequestPacket { resource_id: 4, .. }
                    ))
                ),
                "the client started fresh resource transfer before releasing the restart fence"
            );
        }

        let (completion, acknowledged) = oneshot::channel();
        command_tx
            .send(ClientCommand::AcknowledgeRoundRestart { completion })
            .await
            .test_value();
        acknowledged.await.test_value().test_value();
        let ack_deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(ack_deadline, host.read_message()).await {
                Ok(Ok(ControlMessage::RoundRestartAck { restart_nonce: 42 })) => break,
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => panic!("restart acknowledgement read failed: {error}"),
                Err(_) => panic!("client never sent its restart acknowledgement"),
            }
        }

        host.send_message(complete_status).await.test_value();
        let request_deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(request_deadline, host.read_message()).await {
                Ok(Ok(ControlMessage::Resource(ResourcePacket::Request(request))))
                    if request.resource_id == 4 =>
                {
                    assert_eq!(request.chunk, 0);
                    break;
                }
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => panic!("resource request read failed: {error}"),
                Err(_) => panic!("post-ACK resource status did not schedule a request"),
            }
        }

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ordinary_client_session_rejects_a_round_restart_ack_without_sending_it() {
        let (host_stream, command_tx, _event_rx, shutdown_tx, task) =
            start_test_client_loop(512, 4, 4);
        let mut host = crate::ControlTransport::new(host_stream);
        let (completion, acknowledged) = oneshot::channel();

        command_tx
            .send(ClientCommand::AcknowledgeRoundRestart { completion })
            .await
            .test_value();

        assert!(acknowledged.await.test_value().is_err());
        assert!(timeout(Duration::from_millis(50), host.read_message())
            .await
            .is_err());

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restarted_join_data_precedes_its_fresh_local_resource_completions() {
        let directories = SessionResourceDirectories::new();
        let local_system = directories.root.join("System.c4g");
        let local_dynamic = directories.root.join("fresh-dynamic.c4d");
        fs::write(&local_system, b"system").test_value();
        fs::write(&local_dynamic, b"local").test_value();
        let system = network_core!(resource_type: 5,
        id: 2,
        loadable: false,
        filename: c4(b"System.c4g"));
        let dynamic = network_core!(resource_type: 2,
        id: 77,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        contents_crc: 0x8bd6_88e8,
        chunk_size: 5,
        filename: c4(b"FreshDynamic.c4d"));
        let mut resource_state = empty_client_resource_state(1, directories.client.clone());
        resource_state
            .backend
            .as_mut()
            .test_value()
            .register_local_logical(system.clone(), &local_system)
            .test_value();
        resource_state
            .backend
            .as_mut()
            .test_value()
            .register_hosted_resource(
                dynamic.clone(),
                &local_dynamic,
                crate::ResourceFileOwnership::Persistent,
                true,
            )
            .test_value();
        assert!(resource_state
            .catalog
            .register(crate::ResourceRegistration::from_core(
                &system, false, false
            ),));
        assert!(resource_state
            .catalog
            .register(crate::ResourceRegistration::from_core(
                &dynamic, true, false
            ),));
        let (host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop_with_state(512, 4, 8, BTreeMap::new(), resource_state);
        let mut host = crate::ControlTransport::new(host_stream);
        let config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(config.local_core, 8);
        snapshot.dynamic = dynamic.clone();
        snapshot.parameters.scenario.id = 78;
        snapshot.parameters.game_resources.push(system.clone());
        snapshot.parameters.clients.local_client_id = None;
        let join_data = test_join_data(1, config.initial_status, snapshot);

        host.send_message(ControlMessage::HostRestartLobby { restart_nonce: 9 })
            .await
            .test_value();
        host.send_message(ControlMessage::JoinData(Box::new(join_data.clone())))
            .await
            .test_value();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.test_value(),
            Some(ClientEvent::HostRestartLobby)
        ));
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.test_value(),
            Some(ClientEvent::JoinData { join_data: received }) if *received == join_data
        ));
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.test_value(),
            Some(ClientEvent::ResourceComplete {
                resource_id: 2,
                core,
                path,
                local: true,
            }) if core == system && path == local_system
        ));
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.test_value(),
            Some(ClientEvent::ResourceComplete {
                resource_id: 77,
                core,
                path,
                local: true,
            }) if core == dynamic && path == local_dynamic
        ));

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_client_continues_the_cpp_ping_timer_after_bootstrap() {
        // C4Network2IO's 500 ms timer and strict one-second ping gate continue
        // on the accepted connection after JoinData
        // (src/C4Network2IO.cpp:605-617,1141-1151).
        let (host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop(512, 4, 4);
        let mut host = crate::ControlTransport::new(host_stream);

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1_500)).await;
        let ping = match host.read_message().await.test_value() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected accepted-session PID_Ping, got {other:?}"),
        };
        assert_eq!(ping.packet_counter, 0);
        tokio::time::advance(Duration::from_millis(37)).await;
        host.send_message(ControlMessage::Pong(ping))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::PingMeasured { round_trip_ms: 37 }))
        ));

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn established_host_link_accepts_cpp_tcp_sim_open_frame_without_disconnect() {
        let (mut host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop(512, 4, 4);

        // C++ body: packed client 7, TCP, [2001:db8::7]:11112.
        let payload = [
            0x14, 0x07, 0x01, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':',
            b'7', b']', b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ];
        host_stream
            .write_all(&tcp_frame(&payload))
            .await
            .test_value();

        let status = NetworkStatus::new(NETWORK_STATE_PAUSE, 3, 17);
        let mut host = crate::ControlTransport::new(host_stream);
        host.send_message(ControlMessage::Status(status))
            .await
            .test_value();
        match timeout(EVENT_WAIT, event_rx.recv()).await {
            Ok(Some(ClientEvent::Status(received))) => assert_eq!(received, status),
            Ok(Some(ClientEvent::Disconnected { reason })) => {
                panic!("PID_TCPSimOpen disconnected the established host link: {reason:?}")
            }
            other => panic!("status after PID_TCPSimOpen was not delivered: {other:?}"),
        }

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_host_connection_continues_the_cpp_ping_timer() {
        // The host's accepted connection remains on the same C4Network2IO
        // timer after mutual admission (src/C4Network2IO.cpp:605-617,
        // 1141-1177).
        let (host_stream, client_stream) = duplex(512);
        let mut client = crate::ControlTransport::new(client_stream);
        let (outbound_tx, mut host_rx, task) = start_test_host_route(host_stream, 1);

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1_500)).await;
        let ping = match client.read_message().await.test_value() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected host accepted-session PID_Ping, got {other:?}"),
        };
        client
            .send_message(ControlMessage::Pong(ping))
            .await
            .test_value();
        // The task mirrors its ping bookkeeping to the host loop so the
        // route can reproduce getPingTime/getLag: the dispatched probe, then
        // the measured pong. Neither is a ClientMessage.
        assert!(matches!(
            timeout(EVENT_WAIT, host_rx.recv()).await.unwrap(),
            Some(HostLoopMessage::ConnectionPing {
                connection_id: 3,
                client_id: 1,
                update: RoutePingUpdate::Dispatched,
            })
        ));
        assert!(matches!(
            timeout(EVENT_WAIT, host_rx.recv()).await.unwrap(),
            Some(HostLoopMessage::ConnectionPing {
                connection_id: 3,
                client_id: 1,
                update: RoutePingUpdate::Measured(_),
            })
        ));
        tokio::task::yield_now().await;
        assert!(host_rx.try_recv().is_err());

        drop(client);
        drop(outbound_tx);
        task.await.test_value();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_host_route_answers_ping_while_setup_is_delayed() {
        // C4Network2IO::Execute continues servicing every accepted socket
        // while C4Network2's main thread prepares SendJoinData
        // (src/C4Network2IO.cpp:611-623,1155-1191;
        // src/C4Network2.cpp:1107-1133,1836-1865).
        let (host_stream, client_stream) = duplex(4_096);
        let (admission_tx, mut admission_rx) = mpsc::channel(1);
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
        let mut route_tasks = tokio::task::JoinSet::new();
        let host_core = compatibility_test_core(0, b"Host");
        spawn_host_transport(
            &mut route_tasks,
            host_stream,
            "127.0.0.1:11112".parse().test_value(),
            crate::NetworkProtocol::Tcp,
            host_core,
            7,
            crate::NetworkIoStatistics::default(),
            admission_tx,
            host_tx,
            None,
        );
        let admission = tokio::spawn(async move {
            let request = admission_rx.recv().await.test_value();
            let mut assigned = request.request.core.clone();
            assigned.client_id = 1;
            request
                .decision_tx
                .send(AdmissionDecision::Accept {
                    peer_core: assigned,
                    before_reply: Vec::new(),
                    message: clonk_engine::LegacyCString::default(),
                })
                .test_value();
        });

        let mut client = crate::ControlTransport::new(client_stream);
        assert!(matches!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        client
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                compatibility_test_core(-1, b"Alice"),
                11,
                false,
            )))
            .await
            .test_value();
        client
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                true,
                clonk_engine::LegacyCString::default(),
                false,
            )))
            .await
            .test_value();
        assert!(matches!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(crate::ConnectionReply { ok: true, .. })
        ));

        let _delayed_setup = match timeout(EVENT_WAIT, host_rx.recv()).await.test_value() {
            Some(HostLoopMessage::ClientAccepted { setup_tx, .. }) => setup_tx,
            Some(other) => panic!("unexpected host route event: {other:?}"),
            None => panic!("host route stopped before acceptance"),
        };
        let ping = crate::PingPacket {
            sent_at: 17,
            packet_counter: 0,
        };
        client
            .send_message(ControlMessage::Ping(ping))
            .await
            .test_value();
        assert_eq!(
            timeout(Duration::from_millis(100), client.read_message())
                .await
                .expect("accepted host route stopped reading while setup was delayed")
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        route_tasks.abort_all();
        while route_tasks.join_next().await.is_some() {}
        admission.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_route_reads_inbound_while_its_socket_write_is_blocked() {
        // Native TCP services readable input independently of its pending
        // output buffer (oracle-src-pinned
        // src/C4NetIO.cpp:690-761,1345-1396).
        let (host_stream, peer_stream) = duplex(64);
        let mut peer = crate::ControlTransport::new(peer_stream);
        let (outbound_tx, mut host_rx, task) = start_test_host_route(host_stream, 7);
        outbound_tx
            .send(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: vec![0x55; 1_024 * 1_024],
            })
            .await
            .test_value();
        tokio::task::yield_now().await;
        let inbound = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 7);
        peer.send_message(ControlMessage::Status(inbound))
            .await
            .test_value();

        assert!(matches!(
            timeout(Duration::from_millis(50), host_rx.recv())
                .await
                .expect("blocked output prevented host full-duplex inbound progress"),
            Some(HostLoopMessage::ClientMessage {
                message: ControlMessage::Status(status),
                ..
            }) if status == inbound
        ));

        outbound_tx.retire();
        timeout(EVENT_WAIT, task).await.unwrap().test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_route_keeps_ping_live_beyond_the_old_event_capacity() {
        // C4InteractiveThread::PushEvent appends ordinary packets to an
        // uncapped FIFO, while Ping/Pong remains on the network thread. A
        // stopped main-thread consumer therefore cannot hide a later Ping
        // behind 128 earlier application events
        // (oracle-src-pinned src/C4InteractiveThread.cpp:70-100;
        // src/C4Packet2.cpp:51-73; src/C4Network2IO.cpp:1020-1038).
        const PACKET_COUNT: usize = 160;
        let (host_stream, peer_stream) = duplex(32 * 1_024);
        let mut peer = crate::ControlTransport::new(peer_stream);
        let (outbound_tx, mut host_rx, task) = start_test_host_route(host_stream, 7);

        for sequence in 0..PACKET_COUNT {
            peer.send_message(ControlMessage::Status(NetworkStatus::new(
                NETWORK_STATE_LOBBY,
                1,
                sequence as i32,
            )))
            .await
            .unwrap_or_else(|error| {
                panic!("host route closed while sending packet {sequence}: {error}")
            });
        }
        let ping = crate::PingPacket {
            sent_at: 0x1020_3040,
            packet_counter: PACKET_COUNT as u32,
        };
        peer.send_message(ControlMessage::Ping(ping))
            .await
            .test_value();
        assert_eq!(
            timeout(Duration::from_millis(100), peer.read_message())
                .await
                .expect("full host event queue hid a later Ping")
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        for expected in 0..PACKET_COUNT {
            let Some(HostLoopMessage::ClientMessage {
                message: ControlMessage::Status(status),
                ..
            }) = host_rx.recv().await
            else {
                panic!("host route lost an accepted application event");
            };
            assert_eq!(status.target_tick, expected as i32);
        }

        outbound_tx.retire();
        await_test(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_route_pong_does_not_overtake_accepted_output_frames() {
        // TCP Send appends complete frames to one OBuf under OCSec. A Pong
        // can join that buffer from the network thread, but cannot jump
        // application frames already accepted into it
        // (oracle-src-pinned src/C4NetIO.cpp:1284-1299,1345-1396;
        // src/C4Network2IO.cpp:1020-1029,1451-1491).
        let (host_stream, peer_stream) = duplex(32);
        let mut peer = crate::ControlTransport::new(peer_stream);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let addresses = (0..3)
            .map(|index| crate::AddressPacket {
                client_id: 0,
                address: crate::NetworkAddress::new(
                    crate::NetworkProtocol::Tcp,
                    SocketAddr::from(([192, 0, 2, 1], 11_112 + index)),
                ),
            })
            .collect::<Vec<_>>();
        for address in &addresses {
            outbound_tx
                .send(ControlMessage::Address(*address))
                .await
                .test_value();
        }
        let (host_tx, _host_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id: 7,
                transport: crate::ControlTransport::new(host_stream),
                outbound_rx,
                retire_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );
        let ping = crate::PingPacket {
            sent_at: 0x1020_3040,
            packet_counter: 0,
        };
        peer.send_message(ControlMessage::Ping(ping))
            .await
            .test_value();

        for address in addresses {
            assert_eq!(
                timeout(EVENT_WAIT, peer.read_message())
                    .await
                    .expect("accepted host output stalled")
                    .unwrap(),
                ControlMessage::Address(address)
            );
        }
        assert_eq!(
            timeout(EVENT_WAIT, peer.read_message())
                .await
                .expect("Pong did not follow accepted host output")
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        outbound_tx.retire();
        await_test(task).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_route_close_preempts_an_arbitrary_blocked_backlog() {
        // C4Network2Client::CloseConns sends one best-effort negative ConnRe
        // then immediately closes each connection; stale OBuf is not drained
        // first (oracle-src-pinned src/C4Network2Client.cpp:104-118;
        // src/C4NetIO.cpp:1458-1468).
        let (host_stream, _peer_stream) = duplex(1);
        let (outbound_tx, _host_rx, mut task) = start_test_host_route(host_stream, 7);
        for _ in 0..10_001 {
            outbound_tx
                .send(ControlMessage::Packet {
                    delivery: ControlDelivery::Direct,
                    data: vec![0x55; 1_024],
                })
                .await
                .test_value();
        }
        tokio::task::yield_now().await;
        outbound_tx
            .try_close(test_connection_reply(false, c4(b"removing client"), false))
            .test_value();

        timeout(Duration::from_millis(100), &mut task)
            .await
            .test_value()
            .test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn accepted_host_decodes_tcp_sim_open_and_keeps_the_connection() {
        let (host_stream, mut client_stream) = duplex(512);
        let (outbound_tx, mut host_rx, task) = start_test_host_route(host_stream, 7);

        // Packed client 7 plus a TCP IPv6 endpoint, matching the native
        // C4PacketTCPSimOpen binary layout.
        let tcp_sim_open = [
            0x14, 0x07, 0x01, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':',
            b'7', b']', b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ];
        client_stream
            .write_all(&tcp_frame(&tcp_sim_open))
            .await
            .test_value();

        assert!(matches!(
            timeout(EVENT_WAIT, host_rx.recv()).await.unwrap(),
            Some(HostLoopMessage::ClientMessage {
                connection_id: 3,
                client_id: 7,
                message: ControlMessage::TcpSimOpen(crate::TcpSimOpenPacket {
                    client_id: 7,
                    address: crate::NetworkAddress {
                        protocol: crate::NetworkProtocol::Tcp,
                        ..
                    },
                }),
                ..
            })
        ));

        let mut client = crate::ControlTransport::new(client_stream);
        let ping = crate::PingPacket {
            sent_at: 17,
            packet_counter: 0,
        };
        client
            .send_message(ControlMessage::Ping(ping))
            .await
            .test_value();
        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::Pong(ping),
            "the ignored packet must not terminate the accepted connection"
        );

        drop(client);
        drop(outbound_tx);
        task.await.test_value();
        assert!(matches!(
            host_rx.recv().await,
            Some(HostLoopMessage::ClientDisconnected {
                connection_id: 3,
                client_id: 7,
                next_inbound_packet: 1,
                ..
            })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_client_reports_league_results_and_keeps_the_connection() {
        let (mut host_stream, command_tx, mut event_rx, shutdown_tx, task) =
            start_test_client_loop(512, 4, 4);

        // Success, "OK", and zero result players, matching the native
        // C4PacketLeagueRoundResults binary layout.
        let league_results = [0x17, 0x01, b'O', b'K', 0x00, 0x00];
        host_stream
            .write_all(&tcp_frame(&league_results))
            .await
            .test_value();

        let event = timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .test_value()
            .test_value();
        let ClientEvent::LeagueRoundResults { packet } = event else {
            panic!("expected typed league round-results event, got {event:?}");
        };
        assert_eq!(
            packet,
            crate::LeagueRoundResultsPacket {
                success: true,
                result_string: c4(b"OK"),
                players: Vec::new(),
            }
        );

        let mut host = crate::ControlTransport::new(host_stream);
        let ping = crate::PingPacket {
            sent_at: 23,
            packet_counter: 0,
        };
        host.send_message(ControlMessage::Ping(ping))
            .await
            .test_value();
        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Pong(ping),
            "the typed packet must not terminate the accepted connection"
        );

        tokio::time::advance(Duration::from_millis(1_500)).await;
        let liveness_ping = match host.read_message().await.test_value() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected accepted-session PID_Ping, got {other:?}"),
        };
        assert_eq!(
            liveness_ping.packet_counter, 1,
            "the typed PID must advance the recoverable inbound counter"
        );
        host.send_message(ControlMessage::Pong(liveness_ping))
            .await
            .test_value();

        shutdown_tx.send(()).test_value();
        drop(command_tx);
        task.await.test_value();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn current_thread_client_services_ping_during_synchronous_resource_probe() {
        // C4Network2IO remains on C4InteractiveThread while HandleJoinData
        // probes local groups on the main thread. A single-thread async
        // embedder must preserve the same independence
        // (oracle-src-pinned src/C4Network2.cpp:1590-1639;
        // src/C4Network2IO.cpp:117-197; src/C4Packet2.cpp:51-73).
        let client_name = b"CurrentThreadBootstrap";
        let (probe_paused, resume_probe) = pause_client_resource_bootstrap_probe(client_name);
        let (address, listener) = bind_test_listener().await;
        let (ping_result_tx, ping_result_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            let mut transport = crate::ControlTransport::new(stream);
            admit_and_send_test_join_data(&mut transport, |host_core| {
                synthetic_join_snapshot(host_core.clone(), 8)
            })
            .await;
            assert_eq!(
                transport.read_message().await.unwrap(),
                ControlMessage::Request { from_tick: 0 }
            );

            let ping = crate::PingPacket {
                sent_at: 0x5060_7080,
                packet_counter: 0,
            };
            transport
                .send_message(ControlMessage::Ping(ping))
                .await
                .test_value();
            let serviced = timeout(Duration::from_millis(500), async {
                loop {
                    match transport.read_message().await.test_value() {
                        ControlMessage::Pong(reply) if reply == ping => break,
                        ControlMessage::Ping(probe) => {
                            transport
                                .send_message(ControlMessage::Pong(probe))
                                .await
                                .test_value();
                        }
                        _ => {}
                    }
                }
            })
            .await
            .is_ok();
            let _ = ping_result_tx.send(serviced);

            while let Ok(message) = transport.read_message().await {
                if let ControlMessage::Ping(probe) = message {
                    let _ = transport.send_message(ControlMessage::Pong(probe)).await;
                }
            }
        });

        let client_name = String::from_utf8(client_name.to_vec()).test_value();
        let client_thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .test_value()
                .block_on(async move {
                    let client = connect_client(
                        address,
                        ClientConfig::new(client_name, ParticipantKind::Player),
                    )
                    .await?;
                    client.shutdown().await
                })
        });

        probe_paused.await.test_value();
        let ping_serviced = ping_result_rx.await.test_value();
        resume_probe.send(()).test_value();
        let client_result = tokio::task::spawn_blocking(move || client_thread.join())
            .await
            .test_value()
            .test_value();
        client_result.test_value();
        server.await.test_value();

        assert!(
            ping_serviced,
            "synchronous resource probing blocked the accepted route's Ping/Pong task"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_primary_route_stays_live_and_fifo_during_post_join_bootstrap() {
        // HandleJoinData performs synchronous local resource probing on the
        // main thread, but C4Network2IO remains on C4InteractiveThread:
        // Ping/Pong is handled there and main-thread events append losslessly
        // even while the bootstrap consumer is stopped
        // (oracle-src-pinned src/C4Network2.cpp:1590-1639;
        // src/C4Packet2.cpp:51-73; src/C4InteractiveThread.cpp:70-100).
        const QUEUED_PACKETS: i32 = 96;
        let client_name = b"BootstrapRouteLiveness";
        let (bootstrap_paused, resume_bootstrap) = pause_client_post_join_bootstrap(client_name);
        let (address, listener) = bind_test_listener().await;
        let (probe_complete_tx, probe_complete_rx) = oneshot::channel();
        let (finish_server_tx, finish_server_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            let mut transport = crate::ControlTransport::new(stream);
            admit_and_send_test_join_data(&mut transport, |host_core| {
                synthetic_join_snapshot(host_core.clone(), 8)
            })
            .await;
            assert_eq!(
                transport.read_message().await.unwrap(),
                ControlMessage::Request { from_tick: 0 }
            );

            for countdown in 0..QUEUED_PACKETS {
                transport
                    .send_message(ControlMessage::LobbyCountdown(LobbyCountdownPacket::new(
                        countdown,
                    )))
                    .await
                    .test_value();
            }
            let ping = crate::PingPacket {
                sent_at: 0x1020_3040,
                packet_counter: 0,
            };
            transport
                .send_message(ControlMessage::Ping(ping))
                .await
                .test_value();
            timeout(Duration::from_millis(500), async {
                let mut saw_capabilities = false;
                loop {
                    match transport.read_message().await.test_value() {
                        ControlMessage::Pong(reply) if reply == ping => break,
                        ControlMessage::PortCapabilities(capabilities) => {
                            assert_eq!(
                                capabilities,
                                crate::PortCapabilities::supported_without_voice()
                            );
                            assert!(
                                !saw_capabilities,
                                "the primary port route advertised capabilities more than once"
                            );
                            saw_capabilities = true;
                        }
                        ControlMessage::Ping(probe) => {
                            transport
                                .send_message(ControlMessage::Pong(probe))
                                .await
                                .test_value();
                        }
                        other => panic!(
                            "client started post-bootstrap traffic before release: {other:?}"
                        ),
                    }
                }
                assert!(
                    saw_capabilities,
                    "the admitted port route did not advertise retained-round support"
                );
            })
            .await
            .test_value();
            let _ = probe_complete_tx.send(());
            let _ = finish_server_rx.await;
        });

        let config = ClientConfig::new(
            String::from_utf8(client_name.to_vec()).test_value(),
            ParticipantKind::Player,
        );
        let connect = tokio::spawn(connect_client(address, config));
        bootstrap_paused.await.test_value();
        timeout(Duration::from_secs(1), probe_complete_rx)
            .await
            .expect("bootstrap route did not answer the host's probe")
            .test_value();
        assert!(
            !connect.is_finished(),
            "the main client loop must not start before resource bootstrap completes"
        );
        resume_bootstrap.send(()).test_value();

        let mut client = connect.await.unwrap().test_value();
        for expected in 0..QUEUED_PACKETS {
            let event = timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("queued post-JoinData packet stalled")
                .test_value();
            let ClientEvent::LobbyCountdown { packet } = event else {
                panic!("expected queued lobby countdown, got {event:?}");
            };
            assert_eq!(
                packet.countdown(),
                expected,
                "post-JoinData packets must retain wire order across bootstrap"
            );
        }

        let _ = finish_server_tx.send(());
        server.await.test_value();
        client.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_announces_known_host_address_after_applying_join_data() {
        // HandleJoinData finishes by sending every address already known by
        // the client list. At this point the outgoing host ConnectAddr is
        // known, so it is re-announced as a host-owned PID_Addr
        // (src/C4Network2.cpp:1448-1499,1574-1623;
        // src/C4Network2Client.cpp:319-337,616-621).
        let (addr, listener) = bind_test_listener().await;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.test_value();
            let mut transport = crate::ControlTransport::new(stream);
            admit_and_send_test_join_data(&mut transport, |host_core| {
                synthetic_join_snapshot(host_core.clone(), 8)
            })
            .await;

            let control_request = await_test(transport.read_message()).await;
            // This Rust port extension is emitted once the production route is
            // admitted, before later post-JoinData address announcements.
            expect_control_wait_attribution_capability(&mut transport).await;
            let initial = await_test(transport.read_message()).await;
            let learned = crate::AddressPacket {
                client_id: 0,
                address: crate::NetworkAddress::new(
                    crate::NetworkProtocol::Tcp,
                    "198.51.100.7:11112".parse().test_value(),
                ),
            };
            transport
                .send_message(ControlMessage::Address(learned))
                .await
                .test_value();
            let mut echoed = None;
            for _ in 0..8 {
                let message = await_test(transport.read_message()).await;
                if message == ControlMessage::Address(learned) {
                    echoed = Some(message);
                    break;
                }
            }
            let echoed = echoed.test_value();
            (control_request, initial, learned, echoed)
        });

        let client = connect_test_player(addr, "Alice").await;
        let (control_request, packet, learned, echoed) = server.await.test_value();
        assert_eq!(control_request, ControlMessage::Request { from_tick: 0 });
        assert_eq!(
            packet,
            ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                address: crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, addr),
            })
        );
        assert_eq!(echoed, ControlMessage::Address(learned));

        client.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accepted_client_waits_for_a_fresh_published_join_snapshot() {
        // SendJoinData retains an accepted NCS_Joining client when no current
        // dynamic exists. OnGameSynchronized later publishes the fresh
        // dynamic and sends JoinData/Addr without re-running admission
        // (src/C4Network2.cpp:1099-1115,1768-1784,1820-1849).
        let (addr, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let mut client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        let mut needed = false;
        for _ in 0..4 {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::JoinDataNeeded {
                    client_id: 1,
                    current_control_tick: 0,
                }) => {
                    needed = true;
                    break;
                }
                Some(HostEvent::Direct { .. }) | Some(HostEvent::ClientJoined { .. }) => {}
                other => panic!("unexpected event while waiting for JoinData: {other:?}"),
            }
        }
        assert!(
            needed,
            "host did not retain the joining client for a dynamic"
        );
        assert!(timeout(Duration::from_millis(50), &mut client_task)
            .await
            .is_err());

        host.publish_join_snapshot(snapshot.clone())
            .await
            .test_value();
        let mut client = timeout(EVENT_WAIT, client_task)
            .await
            .test_value()
            .unwrap()
            .test_value();
        let join_data = client.take_join_data().test_value();
        assert_eq!(join_data.dynamic, snapshot.dynamic);
        assert_eq!(join_data.start_control_tick, snapshot.dynamic_tick);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn synchronized_runtime_dynamic_reaches_waiting_client_after_later_ticks_are_ready() {
        // C4Network2::OnGameSynchronized creates and sends the runtime dynamic
        // while C4GameControl::Execute is still executing the same ControlTick;
        // ControlTick advances only afterward (src/C4Network2.cpp:1099-1115,
        // 1820-1844,1945-1971; src/C4GameControl.cpp:274-330,363-366).
        let (addr, listener) = bind_test_listener().await;
        let directories = SessionResourceDirectories::new();
        let mut config = host_config!(
            initial_status: NetworkStatus::new(NETWORK_STATE_GO, 1, 0),
            resource_directory: Some(directories.host.clone()),
        );
        let parameters = config
            .initial_join_snapshot
            .as_ref()
            .test_value()
            .parameters
            .clone();
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let mut client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::JoinDataNeeded { client_id: 1, .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before JoinData was requested"),
            }
        }
        assert!(timeout(Duration::from_millis(50), &mut client_task)
            .await
            .is_err());

        for tick in 0..=2 {
            host.submit_local_control(legacy_packet(HOST_CLIENT_ID, tick, 0x31))
                .await
                .test_value();
            wait_for_host_ready_tick(&mut host_events, tick).await;
        }

        let dynamic = host
            .publish_runtime_dynamic(runtime_dynamic_for_session_test(), 0, parameters)
            .await
            .test_value();
        let mut client = timeout(EVENT_WAIT, client_task)
            .await
            .test_value()
            .unwrap()
            .test_value();
        let join_data = client.take_join_data().test_value();
        assert_eq!(join_data.dynamic, dynamic);
        assert_eq!(join_data.start_control_tick, 0);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delayed_join_data_is_followed_by_prior_lobby_chat() {
        // SendJoinData may wait for OnGameSynchronized to provide a dynamic
        // (src/C4Network2.cpp:1099-1115,1768-1784,1820-1849). The
        // presentation-only transcript extension must follow that delayed
        // JoinData just as it follows an immediately available one.
        let (addr, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::JoinDataNeeded { client_id: 1, .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before JoinData was requested"),
            }
        }

        let message = clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: c4(b"during delayed join"),
            by_client: HOST_CLIENT_ID as i32,
        };
        let data =
            encode_control_entry_payload(&EngineControlPacket::Message(message)).test_value();
        host.submit_packet(ControlDelivery::Private, data.clone())
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::Direct {
                    client_id: BROADCAST_CLIENT_ID,
                    delivery: ControlDelivery::Private,
                    data: received,
                }) if received == data => break,
                Some(_) => continue,
                None => panic!("host event stream ended before accepting lobby chat"),
            }
        }

        host.publish_join_snapshot(snapshot).await.test_value();
        let mut client = timeout(EVENT_WAIT, client_task)
            .await
            .test_value()
            .unwrap()
            .test_value();
        let mut client_events = client.take_event_receiver();
        let replayed = timeout(EVENT_WAIT, async {
            loop {
                match client_events.recv().await {
                    Some(ClientEvent::Direct {
                        delivery: ControlDelivery::Private,
                        data: received,
                    }) if received == data => break Some(received),
                    Some(_) => continue,
                    None => break None,
                }
            }
        })
        .await
        .ok()
        .flatten();

        shutdown_test_session(client, host).await;
        assert_eq!(replayed, Some(data));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_host_route_measures_ping_while_join_data_is_delayed() {
        // C4Network2IO::Execute keeps CheckTimeout/Ping running for every open
        // accepted connection while SendJoinData waits for a fresh dynamic
        // (src/C4Network2IO.cpp:611-623,1155-1191;
        // src/C4Network2.cpp:1107-1133,1836-1865).
        let (addr, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::JoinDataNeeded { client_id: 1, .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before JoinData was requested"),
            }
        }

        let ping_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let measured_ping = loop {
            let connections = host.runtime_connections().await.test_value();
            if let Some(ping_ms) = connections
                .iter()
                .find(|connection| connection.client_id == 1)
                .map(|connection| connection.ping_ms)
                .filter(|ping_ms| *ping_ms >= 0)
            {
                break Some(ping_ms);
            }
            if tokio::time::Instant::now() >= ping_deadline {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert!(
            measured_ping.is_some(),
            "accepted host route must service Ping/Pong before delayed JoinData"
        );
        assert!(
            !client_task.is_finished(),
            "client must still be waiting for the delayed JoinData"
        );

        host.publish_join_snapshot(snapshot).await.test_value();
        let client = timeout(EVENT_WAIT, client_task)
            .await
            .test_value()
            .unwrap()
            .test_value();

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(start_paused = true)]
    async fn runtime_dynamic_is_removed_on_the_next_game_execution_after_its_tick() {
        // C4Network2 removes an outdated dynamic while executing, rather than
        // in its control-tick notification path (src/C4Network2.cpp:679-696).
        // A normal game execution reaches that check only after ControlTick
        // advances (src/C4Game.cpp:776-782; src/C4GameControl.cpp:325-330).
        let directories = SessionResourceDirectories::new();
        let config = host_config!(resource_directory: Some(directories.host.clone()));
        let parameters = config
            .initial_join_snapshot
            .as_ref()
            .test_value()
            .parameters
            .clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let mut host = start_host(listener, config).await.test_value();
        let mut events = host.take_event_receiver();
        // Tokio intervals are initially ready. Drain that startup tick before
        // publishing a dynamic so this test isolates the control-tick handler
        // from a real timer pass.
        settle_paused_network().await;

        host.publish_runtime_dynamic(runtime_dynamic_for_session_test(), 0, parameters.clone())
            .await
            .test_value();
        host.control_tick_reached(
            0,
            1,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .test_value();
        assert!(
            host.remove_runtime_dynamic().await.unwrap(),
            "the dynamic must remain available at its exact control tick"
        );

        host.publish_runtime_dynamic(runtime_dynamic_for_session_test(), 0, parameters)
            .await
            .test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x34))
            .await
            .test_value();
        wait_for_host_ready_tick(&mut events, 0).await;
        host.control_tick_reached(
            1,
            1,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .test_value();
        host.execute(1).await.unwrap();
        assert!(
            !host.remove_runtime_dynamic().await.unwrap(),
            "the next game execution must remove a stale runtime dynamic"
        );

        host.shutdown().await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn stale_runtime_dynamic_expires_on_the_host_second_timer() {
        // C4Network2::OnSec1Timer invokes Execute, which removes a dynamic
        // after its control tick is stale (src/C4Network2.cpp:674-696). The
        // regular game loop is another Execute path; this test covers the
        // host's timer path specifically (src/C4Game.cpp:776-782).
        let directories = SessionResourceDirectories::new();
        let config = host_config!(resource_directory: Some(directories.host.clone()));
        let parameters = config
            .initial_join_snapshot
            .as_ref()
            .test_value()
            .parameters
            .clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.test_value();
        let mut host = start_host(listener, config).await.test_value();
        let mut events = host.take_event_receiver();
        settle_paused_network().await;

        host.publish_runtime_dynamic(runtime_dynamic_for_session_test(), 0, parameters)
            .await
            .test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x35))
            .await
            .test_value();
        wait_for_host_ready_tick(&mut events, 0).await;
        host.control_tick_reached(
            1,
            1,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .test_value();

        tokio::time::advance(Duration::from_secs(1)).await;
        settle_paused_network().await;
        assert!(
            !host.remove_runtime_dynamic().await.unwrap(),
            "the second timer must remove an already stale runtime dynamic"
        );

        host.shutdown().await.test_value();
    }

    #[test]
    fn stale_runtime_dynamic_waits_for_every_live_peer_to_report_complete_chunks() {
        let client_id = 7;
        let directories = SessionResourceDirectories::new();
        let dynamic_path = directories.host.join("DynFixture_2.c4s");
        fs::write(&dynamic_path, b"local").test_value();
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, outbound);
        let dynamic = network_core!(resource_type: crate::HostResourceType::Dynamic as u8,
        id: 4,
        loadable: true,
        file_size: 5,
        file_crc: 0x8bd6_88e8,
        chunk_size: 3,
        filename: c4(b"DynFixture_2.c4s"));
        let mut backend =
            crate::ResourceTransferBackend::new(HOST_CLIENT_ID as i32, directories.host.clone())
                .test_value();
        backend
            .register_hosted_resource(
                dynamic.clone(),
                dynamic_path,
                crate::ResourceFileOwnership::Persistent,
                true,
            )
            .test_value();
        state.resource_backend = Some(backend);
        state.join_snapshot.as_mut().test_value().dynamic = dynamic.clone();
        state.join_snapshot.as_mut().test_value().dynamic_tick = 0;
        state.dynamic_required_clients.insert(client_id);
        assert!(state
            .resource_catalog
            .register(crate::ResourceRegistration::from_core(
                &dynamic, true, false
            ),));
        state
            .coordinator
            .ingest(legacy_packet(HOST_CLIENT_ID, 0, 0x44))
            .test_value();
        state.game_control_tick = 1;

        assert!(
            !remove_stale_host_runtime_dynamic(&mut state),
            "absence of a retained peer's chunk status must pin the fresh dynamic"
        );

        let mut random = |_| 0;
        state
            .resource_backend
            .as_mut()
            .test_value()
            .on_packet(
                client_id as i32,
                &ResourcePacket::Status(crate::ResourceStatusPacket {
                    resource_id: dynamic.id,
                    chunks: crate::ResourceChunkAvailability {
                        chunk_count: 2,
                        ranges: vec![crate::ResourceChunkRange {
                            start: 0,
                            length: 1,
                        }],
                    },
                }),
                0,
                &mut random,
            )
            .test_value();
        assert!(
            !remove_stale_host_runtime_dynamic(&mut state),
            "a partial retained-peer download must pin the fresh dynamic"
        );

        state
            .resource_backend
            .as_mut()
            .test_value()
            .on_packet(
                client_id as i32,
                &ResourcePacket::Status(crate::ResourceStatusPacket {
                    resource_id: dynamic.id,
                    chunks: crate::ResourceChunkAvailability {
                        chunk_count: 2,
                        ranges: vec![crate::ResourceChunkRange {
                            start: 0,
                            length: 2,
                        }],
                    },
                }),
                0,
                &mut random,
            )
            .test_value();
        assert!(
            remove_stale_host_runtime_dynamic(&mut state),
            "the stale dynamic may be removed after every live peer reports completion"
        );
    }

    #[tokio::test]
    async fn late_join_runtime_dynamic_is_not_pinned_by_an_already_joined_peer() {
        let existing_client_id = 7;
        let pending_client_id = 8;
        let directories = SessionResourceDirectories::new();
        let (outbound, _outbound_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(existing_client_id, outbound);
        state.game_started = true;
        state.config.resource_directory = Some(directories.host.clone());
        state.resource_backend = Some(
            crate::ResourceTransferBackend::new(HOST_CLIENT_ID as i32, directories.host.clone())
                .test_value(),
        );
        let (pending_outbound, mut pending_outbound_rx) = HostOutboundSender::channel();
        let pending_core = compatibility_test_core(pending_client_id as i32, b"Pending");
        state.clients.insert(
            pending_client_id,
            ClientConnection {
                outbound: pending_outbound.clone(),
                core: pending_core.clone(),
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                join_data_sent: false,
                join_data_needed_emitted: true,
            },
        );
        state.accepted_routes.insert(
            2,
            AcceptedConnectionRoute {
                client_id: pending_client_id,
                remote_connection_id: 3,
                peer_addr: "127.0.0.1:11113".parse().test_value(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: RoutePingLag::default(),
                outbound: pending_outbound,
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: false,
            },
        );
        state
            .client_cores
            .insert(pending_core.client_id, pending_core);
        state.dynamic_required_clients.insert(existing_client_id);
        let parameters = state.join_snapshot.as_ref().test_value().parameters.clone();
        let published = publish_host_runtime_dynamic(
            runtime_dynamic_for_session_test(),
            0,
            parameters,
            &mut state,
        )
        .test_value();
        publish_pending_join_data(&mut state).await;
        assert!(matches!(
            pending_outbound_rx.try_recv().test_value(),
            HostOutboundMessage::Message(ControlMessage::JoinData(_))
        ));
        state
            .coordinator
            .ingest(legacy_packet(HOST_CLIENT_ID, 0, 0x45))
            .test_value();
        state.game_control_tick = 1;

        let chunk_count =
            crate::ResourceRegistration::from_core(&published.core, true, false).chunk_count;
        let mut random = |_| 0;
        state
            .resource_backend
            .as_mut()
            .test_value()
            .on_packet(
                pending_client_id as i32,
                &ResourcePacket::Status(crate::ResourceStatusPacket {
                    resource_id: published.core.id,
                    chunks: crate::ResourceChunkAvailability {
                        chunk_count,
                        ranges: vec![crate::ResourceChunkRange {
                            start: 0,
                            length: chunk_count,
                        }],
                    },
                }),
                0,
                &mut random,
            )
            .test_value();

        assert!(
            remove_stale_host_runtime_dynamic(&mut state),
            "a peer that received an earlier JoinData must not pin a late-join-only dynamic"
        );
    }

    #[test]
    fn shadow_catalog_reclaims_removed_runtime_dynamic_registration() {
        let mut catalog = crate::ResourceCatalog::new(HOST_CLIENT_ID as i32);
        let registration = crate::ResourceRegistration {
            resource_id: 41,
            chunk_count: 1,
            binary_compatible: true,
            loading: false,
        };
        assert!(catalog.register(registration));

        // Establish the same nonzero request epoch used by the live resource
        // timer, then mark the runtime resource through RemoveDynamic.
        advance_shadow_resource_catalog_timer(&mut catalog, 1);
        assert!(catalog.remove_resource(registration.resource_id));
        advance_shadow_resource_catalog_timer(
            &mut catalog,
            1 + crate::resource_catalog::RESOURCE_DELETE_TIME_SECONDS,
        );
        assert!(catalog.contains_resource(registration.resource_id));

        advance_shadow_resource_catalog_timer(
            &mut catalog,
            2 + crate::resource_catalog::RESOURCE_DELETE_TIME_SECONDS,
        );
        assert!(!catalog.contains_resource(registration.resource_id));
        assert!(
            catalog.register(registration),
            "the retired runtime ID must be reusable after delayed cleanup"
        );
    }

    #[test]
    fn authoritative_player_info_reuses_backend_resource_after_shadow_expiry() {
        // AddByCore returns an existing C4Network2Res before probing or
        // starting a second download. Rust's filesystem backend is the live
        // resource list after its allocation-only shadow entry expires
        // (src/C4Network2Res.cpp:1473-1516).
        let directories = SessionResourceDirectories::new();
        let source = directories.root.join("Returning.c4p");
        let mut group = MutableGroup::new("Returning.c4p");
        group
            .add_file_with_metadata("Player.txt", b"returning player".to_vec(), 1, false)
            .test_value();
        fs::write(&source, group.pack().unwrap()).test_value();
        let publication = crate::build_host_resource_core(
            &source,
            directories.root.join("published-returning"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                c4(b"Returning.c4p"),
                "Client",
            ),
        )
        .test_value();
        let core = publication.core;
        let hosted_path = publication.standalone_path.test_value();
        let hosted_ownership = publication.standalone_ownership.test_value();
        let mut state = empty_client_resource_state(7, directories.client.clone());
        state
            .backend
            .as_mut()
            .test_value()
            .register_hosted_resource(core.clone(), hosted_path, hosted_ownership, true)
            .test_value();
        assert!(!state.catalog.contains_resource(core.id));
        assert_eq!(
            state.backend.as_ref().test_value().core(core.id),
            Some(&core)
        );
        let mut info = clonk_engine::PlayerInfoControlData {
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(state
            .load_authoritative_player_resources(&mut info)
            .is_empty());

        assert_eq!(
            info.players[0].flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
            clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE
        );
        assert_eq!(info.players[0].resource, Some(core));
    }

    #[test]
    fn pending_join_data_excludes_clients_already_marked_for_removal() {
        let mut clients = BTreeMap::new();
        let mut receivers = Vec::new();
        for (client_id, join_data_sent) in [(7, false), (8, false), (9, true)] {
            let (outbound, receiver) = HostOutboundSender::channel();
            receivers.push(receiver);
            clients.insert(
                client_id,
                ClientConnection {
                    outbound,
                    core: clonk_engine::ClientCoreControlData {
                        client_id: client_id as i32,
                        ..Default::default()
                    },
                    peer_addr: "127.0.0.1:1111".parse().test_value(),
                    join_data_sent,
                    join_data_needed_emitted: false,
                },
            );
        }
        let removing_clients = BTreeSet::from([8]);

        assert_eq!(
            pending_join_data_client_ids(&clients, &removing_clients),
            vec![7],
            "a synchronized ClientRemove must suppress later JoinData publication"
        );
    }

    fn runtime_dynamic_for_session_test() -> crate::LiveNetworkDynamic {
        crate::compose_live_network_dynamic(crate::LiveNetworkDynamicSpec {
            group_filename: "DynRuntime.c4s".to_string(),
            maker: b"Host".to_vec(),
            parameters: b"[Parameters]\r\nControlRate=1\r\n".to_vec(),
            scenario: b"[Head]\r\nSaveGame=1\r\nNetworkGame=1\r\n".to_vec(),
            components: vec![crate::LiveNetworkDynamicComponent::File {
                name: "Game.txt".to_string(),
                payload: b"[Game]\r\nControlTick=0\r\n".to_vec(),
            }],
        })
        .test_value()
    }

    #[tokio::test(start_paused = true)]
    async fn chase_target_deadline_waits_exactly_five_seconds() {
        let deadline = tokio::time::Instant::now() + CHASE_TARGET_UPDATE_INTERVAL;
        let task = tokio::spawn(wait_for_chase_target_update(Some(deadline)));
        tokio::task::yield_now().await;

        tokio::time::advance(CHASE_TARGET_UPDATE_INTERVAL - Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(!task.is_finished());

        tokio::time::advance(Duration::from_millis(1)).await;
        task.await.test_value();
        assert_eq!(tokio::time::Instant::now(), deadline);
    }

    #[tokio::test(start_paused = true)]
    async fn chase_target_timer_arms_only_when_delayed_join_data_is_sent() {
        let (addr, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let mut client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::JoinDataNeeded { client_id: 1, .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before JoinData was requested"),
            }
        }

        tokio::time::advance(CHASE_TARGET_UPDATE_INTERVAL + Duration::from_secs(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert!(
            !client_task.is_finished(),
            "connection completed before the host published JoinData"
        );

        let chase_deadline = tokio::time::Instant::now() + CHASE_TARGET_UPDATE_INTERVAL;
        host.publish_join_snapshot(snapshot).await.test_value();
        let mut client = timeout(EVENT_WAIT, &mut client_task)
            .await
            .test_value()
            .unwrap()
            .test_value();
        let initial_status = client.take_join_data().test_value().status;
        let mut client_events = client.take_event_receiver();

        let remaining = chase_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(remaining > Duration::from_millis(1));
        tokio::time::advance(remaining - Duration::from_millis(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_no_queued_client_status(&mut client_events);

        tokio::time::advance(Duration::from_millis(1)).await;
        let status = wait_for_client_status(&mut client_events).await;
        assert_eq!(
            status,
            NetworkStatus {
                target_tick: 0,
                ..initial_status
            }
        );

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn direct_client_join_reaches_already_connected_clients_before_new_join_finishes() {
        // CtrlAdd executes CID_ClientJoin as direct control before the host
        // sends positive ConnRe, so every existing client learns the newcomer
        // before normal synchronized traffic continues
        // (src/C4Network2.cpp:1395-1445; src/C4Control.cpp:554-573).
        let (addr, host) = start_test_host(HostConfig::default()).await;
        let mut alpha = connect_test_player(addr, "Alpha").await;
        let mut alpha_events = alpha.take_event_receiver();
        let beta = connect_test_player(addr, "Beta").await;

        let data = loop {
            match timeout(EVENT_WAIT, alpha_events.recv()).await.test_value() {
                Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    data,
                }) => break data,
                Some(ClientEvent::Ready { .. }) => continue,
                other => panic!("expected direct ClientJoin for Beta, got {other:?}"),
            }
        };
        let clonk_engine::ControlPacket::ClientJoin(join) =
            decode_control_entry_payload(&data).test_value()
        else {
            panic!("direct packet was not ClientJoin");
        };
        assert_eq!(
            join.core.client_id,
            i32::try_from(beta.client_id()).unwrap()
        );
        assert_eq!(join.core.name.as_bytes(), b"Beta");

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn joining_client_receives_prior_lobby_chat_after_join_data() {
        // C++ sends lobby CID_Message as ephemeral CDT_Private controls only
        // to clients connected at that instant (src/C4MessageInput.cpp:423-425;
        // src/C4GameControlNetwork.cpp:225-237). Retaining those same raw
        // controls for post-JoinData replay fixes the presentation-only gap
        // without changing synchronized state or recipient-side filtering.
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let source = connect_test_player(addr, "Source").await;
        let source_id = source.client_id();
        let message = clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: c4(b"before join"),
            by_client: i32::try_from(source_id).test_value(),
        };
        let data =
            encode_control_entry_payload(&EngineControlPacket::Message(message)).test_value();

        source
            .submit_packet(ControlDelivery::Private, data.clone())
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Private,
                    data: received,
                }) if client_id == source_id && received == data => break,
                Some(_) => continue,
                None => panic!("host event stream ended before accepting lobby chat"),
            }
        }

        let mut client = connect_test_player(addr, "Late").await;
        let mut client_events = client.take_event_receiver();
        let replayed = timeout(EVENT_WAIT, async {
            loop {
                match client_events.recv().await {
                    Some(ClientEvent::Direct {
                        delivery: ControlDelivery::Private,
                        data: received,
                    }) if received == data => break Some(received),
                    Some(_) => continue,
                    None => break None,
                }
            }
        })
        .await
        .ok()
        .flatten();

        client.shutdown().await.test_value();
        source.shutdown().await.test_value();
        host.shutdown().await.test_value();
        assert_eq!(replayed, Some(data));
    }

    #[derive(Clone, Copy)]
    enum TestMeshTransport {
        Tcp,
        Udp,
    }

    async fn known_peer_mesh_keeps_host_forwarding_as_fallback(transport: TestMeshTransport) {
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mesh_bind = SocketAddr::from(([127, 0, 0, 1], 0));
        let config = |name| match transport {
            TestMeshTransport::Tcp => ClientConfig::new(name, ParticipantKind::Player)
                .with_mesh_tcp_bind_address(mesh_bind),
            TestMeshTransport::Udp => ClientConfig::new(name, ParticipantKind::Player)
                .with_mesh_udp_bind_address(mesh_bind),
        };
        let alpha = connect_client(addr, config("Alpha")).await.test_value();
        let mut beta = connect_client(addr, config("Beta")).await.test_value();

        // Each live PID_Addr invokes DoConnectAttempt. The leading wildcard
        // address is intentionally undialable and enters the native 10-second
        // backoff before the following interface address becomes eligible.
        // Invoke that already-recorded due attempt directly so this socket
        // integration test does not spend ten wall-clock seconds sleeping.
        timeout(EVENT_WAIT, async {
            loop {
                if alpha.mesh_address_count(beta.client_id()).await >= 2
                    && beta.mesh_address_count(alpha.client_id()).await >= 2
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        beta.force_mesh_attempt(alpha.client_id()).await;
        if matches!(transport, TestMeshTransport::Tcp) {
            tokio::task::yield_now().await;
        }

        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            let alpha_connected = alpha.mesh_peer_ids().await.contains(&beta.client_id());
            let beta_connected = beta.mesh_peer_ids().await.contains(&alpha.client_id());
            if alpha_connected && beta_connected {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{}",
                match transport {
                    TestMeshTransport::Tcp => {
                        "clients did not establish their direct known-peer route"
                    }
                    TestMeshTransport::Udp => {
                        "clients did not establish their direct known-peer UDP route"
                    }
                }
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(host.accepted_routes().await.len(), 2);

        let mut beta_events = beta.take_event_receiver();
        let alpha_id = alpha.client_id();
        let (command, command_data) = match transport {
            TestMeshTransport::Tcp => (0x44, 0x55),
            TestMeshTransport::Udp => (0x66, 0x77),
        };
        let data = encode_control_entry_payload(&EngineControlPacket::PlayerControl(
            PlayerControlData::new(
                i32::try_from(alpha_id).unwrap(),
                command,
                command_data,
                i32::try_from(alpha_id).unwrap(),
            ),
        ))
        .test_value();
        alpha
            .submit_packet(ControlDelivery::Direct, data.clone())
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.test_value() {
                Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if received == data => break,
                Some(_) => continue,
                None => panic!(
                    "beta event stream ended before direct {}mesh delivery",
                    if matches!(transport, TestMeshTransport::Udp) {
                        "UDP "
                    } else {
                        ""
                    }
                ),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if client_id == alpha_id && received == data => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == alpha_id => panic!(
                    "{}mesh fallback failed: {error}",
                    if matches!(transport, TestMeshTransport::Udp) {
                        "UDP "
                    } else {
                        ""
                    }
                ),
                Some(_) => continue,
                None => panic!(
                    "host event stream ended before {}mesh fallback",
                    if matches!(transport, TestMeshTransport::Udp) {
                        "UDP "
                    } else {
                        ""
                    }
                ),
            }
        }
        while let Ok(Some(event)) = timeout(Duration::from_millis(50), beta_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    ClientEvent::Direct {
                        delivery: ControlDelivery::Direct,
                        data: ref received,
                    } if *received == data
                ),
                "host fallback duplicated a directly delivered {}mesh packet",
                if matches!(transport, TestMeshTransport::Udp) {
                    "UDP "
                } else {
                    ""
                }
            );
        }

        if matches!(transport, TestMeshTransport::Udp) {
            timeout(EVENT_WAIT, async {
                while !alpha.voice_available() || !beta.voice_available() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .test_value();
            assert!(
                !host.voice_available(),
                "a TCP-only host is not a negotiated media relay"
            );
            let mut beta_voice = beta.take_voice_receiver();
            alpha
                .voice_sender()
                .try_send(crate::VoiceFrame::outbound(9, 17, 4, vec![0x3c; 164]).test_value())
                .test_value();
            let frame = await_test(beta_voice.recv()).await;
            assert_eq!(frame.client_id, alpha.client_id());
            assert_eq!(frame.player_id, 9);
        }

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn two_rust_clients_form_a_known_peer_tcp_mesh_and_keep_host_forwarding_as_fallback() {
        known_peer_mesh_keeps_host_forwarding_as_fallback(TestMeshTransport::Tcp).await;
    }

    #[tokio::test(start_paused = true)]
    async fn two_rust_clients_form_a_known_peer_udp_mesh_and_keep_host_forwarding_as_fallback() {
        known_peer_mesh_keeps_host_forwarding_as_fallback(TestMeshTransport::Udp).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connecting_without_pending_sync_emits_no_exec_sync_marker() {
        // PID_ExecSyncCtrl is emitted only when SyncControl is non-empty;
        // connection establishment is not a synchronization release
        // (src/C4GameControlNetwork.cpp:260-276).
        let (addr, listener) = bind_test_listener().await;
        let host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut client = connect_test_player(addr, "Alice").await;
        let mut events = client.take_event_receiver();

        assert!(timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err());

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_and_ack_round_trip_over_real_tcp() {
        // PID_Status is host-authored; a client answers with PID_StatusAck and
        // the host later broadcasts the final ACK
        // (src/C4Network2.cpp:1501-1534,1994-2012,2062-2077).
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(addr, "Alice").await;
        let client_id = client.client_id();
        let mut client_events = client.take_event_receiver();
        let status = NetworkStatus::new(NETWORK_STATE_GO, 1, 195_995);

        host.change_status(status).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusChanged(requested)) => {
                    assert_eq!(requested, status);
                    break;
                }
                Some(HostEvent::ClientJoined { .. }) | Some(HostEvent::Direct { .. }) => continue,
                other => panic!("expected host status request event, got {other:?}"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Status(received)) => {
                    assert_eq!(received, status);
                    break;
                }
                Some(ClientEvent::Ready { .. }) | Some(ClientEvent::Direct { .. }) => continue,
                other => panic!("expected client status event, got {other:?}"),
            }
        }

        client.submit_status_ack(status).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status: received,
                }) => {
                    assert_eq!((received_id, received), (client_id, status));
                    break;
                }
                Some(HostEvent::StatusChanged(_))
                | Some(HostEvent::ClientJoined { .. })
                | Some(HostEvent::Direct { .. }) => continue,
                other => panic!("expected host status ack event, got {other:?}"),
            }
        }

        assert!(timeout(Duration::from_millis(50), client_events.recv())
            .await
            .is_err());
        host.status_reached(status, status.target_tick)
            .await
            .test_value();
        match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
            Some(ClientEvent::StatusAck(received)) => assert_eq!(received, status),
            other => panic!("expected client final status ack, got {other:?}"),
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusCommitted(committed)) => {
                    assert_eq!(committed, status);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before status commit"),
            }
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(start_paused = true)]
    async fn host_chase_target_updates_only_chasing_clients_and_stops_after_ack() {
        let (addr, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let alpha_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alpha", ParticipantKind::Player),
        ));
        let beta_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Beta", ParticipantKind::Player),
        ));
        let mut waiting_clients = BTreeSet::new();
        while waiting_clients.len() < 2 {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::JoinDataNeeded { client_id, .. }) => {
                    waiting_clients.insert(client_id);
                }
                Some(_) => continue,
                None => panic!("host event stream ended before both clients needed JoinData"),
            }
        }

        let first_deadline = tokio::time::Instant::now() + CHASE_TARGET_UPDATE_INTERVAL;
        host.publish_join_snapshot(snapshot).await.test_value();
        let mut alpha = await_test(alpha_task).await.test_value();
        let mut beta = await_test(beta_task).await.test_value();
        let alpha_id = alpha.client_id();
        let initial_status = alpha.take_join_data().test_value().status;
        let mut alpha_events = alpha.take_event_receiver();
        let beta_id = beta.client_id();
        assert_eq!(beta.take_join_data().unwrap().status, initial_status);
        let mut beta_events = beta.take_event_receiver();

        beta.submit_status_ack(initial_status).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusAck { client_id, status })
                    if client_id == beta_id && status == initial_status =>
                {
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before Beta's status acknowledgement"),
            }
        }
        wait_for_client_status_ack(&mut beta_events, initial_status).await;

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x31))
            .await
            .test_value();
        wait_for_host_ready_tick(&mut host_events, 0).await;

        let remaining = first_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(remaining > Duration::from_millis(1));
        tokio::time::advance(remaining - Duration::from_millis(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_no_queued_client_status(&mut alpha_events);
        assert_no_queued_client_status(&mut beta_events);

        tokio::time::advance(Duration::from_millis(1)).await;
        let first_update = wait_for_client_status(&mut alpha_events).await;
        assert_eq!(
            first_update,
            NetworkStatus {
                target_tick: 1,
                ..initial_status
            }
        );
        let beta_barrier =
            ReadyCheckPacket::new(HOST_CLIENT_ID as i32, crate::ReadyCheckData::Other(101));
        host.submit_ready_check(beta_barrier).await.test_value();
        assert_no_client_status_through_ready_check(&mut beta_events, beta_barrier).await;

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0x32))
            .await
            .test_value();
        wait_for_host_ready_tick(&mut host_events, 1).await;
        let second_deadline = first_deadline + CHASE_TARGET_UPDATE_INTERVAL;
        let remaining = second_deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(remaining > Duration::from_millis(1));
        tokio::time::advance(remaining - Duration::from_millis(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_no_queued_client_status(&mut alpha_events);
        assert_no_queued_client_status(&mut beta_events);

        tokio::time::advance(Duration::from_millis(1)).await;
        let second_update = wait_for_client_status(&mut alpha_events).await;
        assert_eq!(
            second_update,
            NetworkStatus {
                target_tick: 2,
                ..initial_status
            }
        );
        let second_beta_barrier =
            ReadyCheckPacket::new(HOST_CLIENT_ID as i32, crate::ReadyCheckData::Other(102));
        host.submit_ready_check(second_beta_barrier)
            .await
            .test_value();
        assert_no_client_status_through_ready_check(&mut beta_events, second_beta_barrier).await;

        alpha.submit_status_ack(second_update).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusAck { client_id, status })
                    if client_id == alpha_id && status == second_update =>
                {
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before Alpha's status acknowledgement"),
            }
        }
        wait_for_client_status_ack(&mut alpha_events, second_update).await;

        tokio::time::advance(CHASE_TARGET_UPDATE_INTERVAL).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        let stopped_barrier =
            ReadyCheckPacket::new(HOST_CLIENT_ID as i32, crate::ReadyCheckData::Other(103));
        host.submit_ready_check(stopped_barrier).await.test_value();
        assert_no_client_status_through_ready_check(&mut alpha_events, stopped_barrier).await;
        assert_no_client_status_through_ready_check(&mut beta_events, stopped_barrier).await;

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn central_host_replays_contiguous_local_controls_after_decentral_commit() {
        // SetCtrlMode(CNM_Decentral) runs only after the final StatusAck and
        // resends the host's own stored controls from ControlTick until the
        // first gap (src/C4Network2.cpp:2062-2110;
        // src/C4GameControlNetwork.cpp:360-374).
        let (addr, listener) = bind_test_listener().await;
        let mut config = HostConfig::default();
        config.initial_status.control_mode = 1;
        let mut host = start_host(listener, config).await.test_value();
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"Alice").await;
        activate_joined_client(&host, &mut host_events, client_id).await;
        drain_raw_client(&mut client).await;
        acknowledge_raw_status(
            &mut client,
            &mut host_events,
            client_id,
            NetworkStatus::new(NETWORK_STATE_LOBBY, 1, -1),
        )
        .await;
        drain_raw_client(&mut client).await;

        let first = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let after_gap = legacy_packet(HOST_CLIENT_ID, 2, 0x13);
        host.submit_local_control(first.clone()).await.test_value();
        host.submit_local_control(after_gap.clone())
            .await
            .test_value();

        let decentral = NetworkStatus::new(NETWORK_STATE_GO, 0, 0);
        host.change_status(decentral).await.test_value();
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .test_value()
            {
                ControlMessage::Status(status) if status == decentral => break,
                _ => continue,
            }
        }
        client
            .send_message(ControlMessage::StatusAck(decentral))
            .await
            .test_value();
        host.status_reached(decentral, decentral.target_tick)
            .await
            .test_value();

        let mut saw_final_ack = false;
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .test_value()
            {
                ControlMessage::StatusAck(status) if status == decentral => {
                    saw_final_ack = true;
                }
                ControlMessage::Control(packet) if packet == first => {
                    assert!(saw_final_ack, "control replay preceded the final StatusAck");
                    break;
                }
                _ => continue,
            }
        }
        assert!(
            timeout(Duration::from_millis(100), async {
                loop {
                    if matches!(
                        client.read_message().await,
                        Ok(ControlMessage::Control(packet)) if packet == after_gap
                    ) {
                        break;
                    }
                }
            })
            .await
            .is_err(),
            "decentral replay crossed the first missing control tick"
        );

        drop(client);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn decentral_host_replays_complete_controls_after_central_commit() {
        // SetCtrlMode(CNM_Central) follows the final StatusAck and the host
        // resends stored complete C4ClientIDAll controls, not one participant's
        // contribution (src/C4Network2.cpp:2062-2110;
        // src/C4GameControlNetwork.cpp:360-374).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"Alice").await;
        activate_joined_client(&host, &mut host_events, client_id).await;
        drain_raw_client(&mut client).await;
        acknowledge_raw_status(
            &mut client,
            &mut host_events,
            client_id,
            NetworkStatus::new(NETWORK_STATE_LOBBY, 0, -1),
        )
        .await;
        drain_raw_client(&mut client).await;

        let host_packet = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        host.submit_local_control(host_packet.clone())
            .await
            .test_value();
        assert!(raw_client_received_control(&mut client, &host_packet, EVENT_WAIT,).await);
        let client_packet = legacy_packet(client_id, 0, 0x21);
        client
            .send_message(ControlMessage::Control(client_packet))
            .await
            .test_value();
        let complete = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(complete.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&complete), vec![0x11, 0x21]);
        drain_raw_client(&mut client).await;

        let central = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);
        host.change_status(central).await.test_value();
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .test_value()
            {
                ControlMessage::Status(status) if status == central => break,
                _ => continue,
            }
        }
        client
            .send_message(ControlMessage::StatusAck(central))
            .await
            .test_value();
        host.status_reached(central, central.target_tick)
            .await
            .test_value();

        let mut saw_final_ack = false;
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .test_value()
            {
                ControlMessage::StatusAck(status) if status == central => {
                    saw_final_ack = true;
                }
                ControlMessage::Control(packet) if packet == complete => {
                    assert!(saw_final_ack, "control replay preceded the final StatusAck");
                    break;
                }
                _ => continue,
            }
        }

        drop(client);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn higher_client_status_ack_retargets_real_tcp_barrier_before_commit() {
        // CheckStatusReached replaces a client's requested target with its
        // current control tick. HandleStatusAck must rebroadcast that higher
        // target before the barrier can commit
        // (src/C4Network2.cpp:1994-2012,2062-2077).
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(addr, "Alice").await;
        let client_id = client.client_id();
        let initial_status = client.take_join_data().test_value().status;
        let mut client_events = client.take_event_receiver();

        // Send the JoinData status acknowledgement first so the host advances
        // this client from Chasing to Ready before opening a fresh barrier.
        client.submit_status_ack(initial_status).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status,
                }) if received_id == client_id && status == initial_status => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::StatusAck(status)) if status == initial_status => break,
                Some(_) => continue,
                None => panic!("client event stream ended before initial status ack"),
            }
        }

        let requested = NetworkStatus::new(NETWORK_STATE_PAUSE, 1, 41);
        host.change_status(requested).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusChanged(status)) if status == requested => break,
                Some(_) => continue,
                None => panic!("host event stream ended before requested Pause event"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Status(status)) if status == requested => break,
                Some(_) => continue,
                None => panic!("client event stream ended before requested Pause"),
            }
        }

        let retargeted = requested.with_target_tick(44);
        client.submit_status_ack(retargeted).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status,
                }) if received_id == client_id && status == retargeted => break,
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusChanged(status)) if status == retargeted => break,
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted Pause event"),
            }
        }

        match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
            Some(ClientEvent::Status(status)) => assert_eq!(status, retargeted),
            other => panic!("expected retargeted Pause before final ack, got {other:?}"),
        }
        assert!(
            timeout(Duration::from_millis(50), async {
                loop {
                    match client_events.recv().await {
                        Some(ClientEvent::StatusAck(status)) => break Some(status),
                        Some(_) => continue,
                        None => break None,
                    }
                }
            })
            .await
            .is_err(),
            "retargeted barrier committed before the host reached tick 44"
        );

        host.status_reached(retargeted, retargeted.target_tick)
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::StatusAck(status)) => {
                    assert_eq!(status, retargeted);
                    break;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before final retargeted status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusCommitted(status)) => {
                    assert_eq!(status, retargeted);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted status commit"),
            }
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_controls_wait_for_status_barrier_and_keep_fifo_order() {
        // In running games, CDT_Sync packets accumulate in SyncControl and do
        // not execute until PID_ExecSyncCtrl is emitted after the status
        // barrier (src/C4GameControlNetwork.cpp:181-220,260-297,558-588).
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(addr, "Alice").await;
        let mut client_events = client.take_event_receiver();

        let running = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);
        host.change_status(running).await.test_value();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Status(status)) => {
                    assert_eq!(status, running);
                    break;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before initial Go"),
            }
        }
        client.submit_status_ack(running).await.test_value();
        host.status_reached(running, running.target_tick)
            .await
            .test_value();
        let mut host_running = false;
        let mut client_running = false;
        while !host_running || !client_running {
            if !host_running {
                match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                    Some(HostEvent::StatusCommitted(status)) => {
                        assert_eq!(status, running);
                        host_running = true;
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended before initial Go commit"),
                }
            }
            if !client_running {
                match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                    Some(ClientEvent::StatusAck(status)) => {
                        assert_eq!(status, running);
                        client_running = true;
                    }
                    Some(_) => {}
                    None => panic!("client event stream ended before initial Go ack"),
                }
            }
        }

        let first = EngineControlPacket::PlayerControl(PlayerControlData::new(0, 0x41, 0, 0));
        let second = EngineControlPacket::PlayerControl(PlayerControlData::new(0, 0x42, 0, 0));
        for control in [&first, &second] {
            host.submit_packet(
                ControlDelivery::Sync,
                encode_control_entry_payload(control).expect("encode sync control"),
            )
            .await
            .test_value();
        }

        let sync_status = loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Status(status)) => break status,
                Some(ClientEvent::SyncScheduled { .. }) => {
                    panic!("client released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before synchronization status"),
            }
        };
        assert_eq!(sync_status.state, NETWORK_STATE_GO);

        // A complete ordinary lockstep tick is not the C++ status barrier.
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x11))
            .await
            .test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x22))
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::Ready { .. }) => break,
                Some(HostEvent::SyncScheduled { .. }) => {
                    panic!("host released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Ready { .. }) => break,
                Some(ClientEvent::SyncScheduled { .. }) => {
                    panic!("client released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before ready"),
            }
        }

        client.submit_status_ack(sync_status).await.test_value();
        host.status_reached(sync_status, sync_status.target_tick)
            .await
            .test_value();
        let mut host_controls = None;
        let mut host_committed = false;
        while host_controls.is_none() || !host_committed {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(
                        i32::try_from(control_tick).ok(),
                        Some(sync_status.target_tick)
                    );
                    host_controls = Some(controls);
                }
                Some(HostEvent::StatusCommitted(status)) => {
                    assert_eq!(status, sync_status);
                    host_committed = true;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before sync release"),
            }
        }
        let mut client_controls = None;
        let mut client_committed = false;
        while client_controls.is_none() || !client_committed {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(
                        i32::try_from(control_tick).ok(),
                        Some(sync_status.target_tick)
                    );
                    client_controls = Some(controls);
                }
                Some(ClientEvent::StatusAck(status)) => {
                    assert_eq!(status, sync_status);
                    client_committed = true;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before sync release"),
            }
        }
        assert_eq!(host_controls, Some(vec![first.clone(), second.clone()]));
        assert_eq!(client_controls, Some(vec![first, second]));

        host.submit_exec_sync(2).await.test_value();
        // ExecSyncControl returns before sending PID_ExecSyncCtrl when the
        // synchronized queue is empty (src/C4GameControlNetwork.cpp:267-269,
        // 281-283). ReadyCheck is an ordered message-stream sentinel: seeing
        // it proves the preceding host command and every earlier wire packet
        // have been handled without mistaking a delayed StatusAck or ping for
        // an empty Sync release.
        let empty_release_barrier =
            ReadyCheckPacket::new(HOST_CLIENT_ID as i32, crate::ReadyCheckData::Other(0x5359));
        host.submit_ready_check(empty_release_barrier)
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::ReadyCheck { packet }) if packet == empty_release_barrier => {
                    break;
                }
                Some(ClientEvent::SyncScheduled { .. } | ClientEvent::ExecSync { .. }) => {
                    panic!("empty Sync release reached the client")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before empty-release barrier"),
            }
        }
        while let Ok(event) = host_events.try_recv() {
            assert!(
                !matches!(
                    event,
                    HostEvent::SyncScheduled { .. } | HostEvent::ExecSync { .. }
                ),
                "empty Sync release reached the host"
            );
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_control_executes_immediately_in_frozen_lobby() {
        // Lobby is frozen without a status round trip, so the host executes a
        // CDT_Sync control immediately and then emits PID_ExecSyncCtrl
        // (src/C4Network2.cpp:1982-1991;
        // src/C4GameControlNetwork.cpp:204-213).
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(addr, "Alice").await;
        let mut client_events = client.take_event_receiver();
        let control = EngineControlPacket::PlayerControl(PlayerControlData::new(0, 0x51, 0, 0));

        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&control).expect("encode lobby sync control"),
        )
        .await
        .test_value();

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(control_tick, 0);
                    assert_eq!(controls, vec![control.clone()]);
                    break;
                }
                Some(HostEvent::StatusAck { .. }) | Some(HostEvent::StatusCommitted(_)) => {
                    panic!("frozen lobby Sync must not open a status barrier")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before frozen sync"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(control_tick, 0);
                    assert_eq!(controls, vec![control]);
                    break;
                }
                Some(ClientEvent::Status(_)) | Some(ClientEvent::StatusAck(_)) => {
                    panic!("frozen lobby Sync must not open a status barrier")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before frozen sync"),
            }
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_matches_cpp_pid_control_source_id_semantics() {
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"spoof-check").await;
        activate_joined_client(&host, &mut host_events, client_id).await;
        drain_raw_client(&mut client).await;
        let spoofed = legacy_packet(HOST_CLIENT_ID, 0, 0x66);
        client
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&spoofed)
                    .test_value(),
            }))
            .await
            .test_value();
        raw_client_ping_barrier(&mut client).await;
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .test_value();
        let contribution = legacy_packet(client_id, 0, 0x22);
        client
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&contribution)
                    .test_value(),
            }))
            .await
            .test_value();

        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!(
                        "C++ accepts PID_Control independently of its source connection: {error}"
                    )
                }
                Some(HostEvent::Ready { packet }) => break packet,
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        };
        assert_eq!(
            control_commands(&ready),
            vec![0x66, 0x22],
            "HandleControl ignores iByClientID and retains the first contribution for a slot"
        );

        drop(client);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_silently_ignores_valid_control_from_an_unregistered_client() {
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"inactive-control").await;
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await {
                Ok(Some(HostEvent::ClientJoined {
                    client_id: joined, ..
                })) if joined == client_id => break,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("transport error before inactive control: {error}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before client join"),
                Err(_) => panic!("timed out waiting for client join"),
            }
        }

        for tick in 0..=1 {
            client
                .send_message(ControlMessage::Control(legacy_packet(
                    client_id,
                    tick,
                    0x60 + i32::try_from(tick).test_value(),
                )))
                .await
                .test_value();
        }
        raw_client_ping_barrier(&mut client).await;
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .test_value();

        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv()).await {
                Ok(Some(HostEvent::Ready { packet })) => break packet,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("valid inactive control became a transport error: {error}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before host-only control"),
                Err(_) => panic!("timed out waiting for host-only control"),
            }
        };
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0x11]);

        drop(client);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_typed_unpack_closes_only_the_malformed_control_route() {
        // PID_Control typed unpack includes its nested control list. A
        // compiler failure closes only pConn in release builds; the network
        // scheduler remains available for later clients
        // (src/C4GameControlNetwork.cpp:867-872;
        // src/C4Network2IO.cpp:822-835).
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"inactive-malformed").await;
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await {
                Ok(Some(HostEvent::ClientJoined {
                    client_id: joined, ..
                })) if joined == client_id => break,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before client join"),
                Err(_) => panic!("timed out waiting for client join"),
            }
        }

        client
            .send_message(ControlMessage::Control(
                ControlPacket::builder(client_id, 0).payload(vec![0x42]),
            ))
            .await
            .test_value();
        let mut connection_failed = false;
        let mut client_left = false;
        let mut diagnostic = false;
        timeout(EVENT_WAIT, async {
            while !connection_failed || !client_left || !diagnostic {
                match host_events.recv().await {
                    Some(HostEvent::ClientConnectionFailed { client_id: source })
                        if source == client_id =>
                    {
                        connection_failed = true;
                    }
                    Some(HostEvent::ClientLeft { client_id: source }) if source == client_id => {
                        client_left = true;
                    }
                    Some(HostEvent::RecoverableRouteDiagnostic {
                        client_id: Some(source),
                        error,
                    }) if source == client_id => {
                        assert!(error.contains("invalid complete control packet"));
                        assert!(error.contains("0x42"));
                        diagnostic = true;
                    }
                    Some(_) => continue,
                    None => panic!("host event stream ended before malformed-route cleanup"),
                }
            }
        })
        .await
        .test_value();

        drop(client);
        let (mut successor, _) = raw_client_transport(addr, b"after-malformed").await;
        raw_client_ping_barrier(&mut successor).await;
        drop(successor);
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forged_queued_control_set_author_does_not_consume_the_tick() {
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let client = connect_test_player(addr, "set-spoof-check").await;
        let client_id = client.client_id();
        activate_joined_client(&host, &mut host_events, client_id).await;
        let client_author = i32::try_from(client_id).test_value();
        let queued_set = |by_client| {
            encode_control_packet(&legacy_frame(
                client_id,
                0,
                vec![crate::LegacyControlSet {
                    value_type: 5,
                    data: 10_000,
                    by_client,
                }
                .into_control_packet()],
            ))
            .test_value()
        };

        client.submit_control(queued_set(0)).await.test_value();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .test_value();
        client
            .submit_control(queued_set(client_author))
            .await
            .test_value();

        let mut saw_rejection = false;
        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::TransportError {
                    client_id: Some(rejected_id),
                    error,
                }) if error.contains("queued CID_Set claimed author") => {
                    assert_eq!(rejected_id, client_id);
                    assert!(error.contains("claimed author 0"));
                    saw_rejection = true;
                }
                Some(HostEvent::Ready { packet }) => break packet,
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        };
        assert!(
            saw_rejection,
            "forged CID_Set contribution was not rejected"
        );
        let frame = decode_control_packet(&ready).test_value();
        let sets = frame
            .controls
            .iter()
            .filter_map(crate::LegacyControlSet::from_control_packet)
            .collect::<Vec<_>>();
        assert_eq!(
            sets,
            vec![crate::LegacyControlSet {
                value_type: 5,
                data: 10_000,
                by_client: client_author,
            }]
        );

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_contribution_does_not_consume_the_synchronized_tick() {
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, HostConfig::default())
            .await
            .test_value();
        let mut host_events = host.take_event_receiver();
        let client = connect_test_player(addr, "validation-check").await;
        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x22))
            .await
            .test_value();
        let valid_host = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let mut malformed_payload = valid_host.payload().to_vec();
        *malformed_payload.last_mut().test_value() = 0x7f;
        let malformed_host = ControlPacket::builder(HOST_CLIENT_ID, 0).payload(malformed_payload);
        host.submit_local_control(malformed_host).await.test_value();
        host.submit_local_control(valid_host).await.test_value();

        let mut saw_validation_error = false;
        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::TransportError { error, .. }) => {
                    assert!(error.contains("invalid control packet"));
                    assert!(error.contains("0x7f"));
                    saw_validation_error = true;
                }
                Some(HostEvent::Ready { packet }) => break packet,
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        };
        assert!(saw_validation_error, "malformed input was not diagnosed");
        assert_eq!(control_commands(&ready), vec![0x11, 0x22]);

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_sync_and_reconnect_smoke() {
        let (addr, listener) = bind_test_listener().await;
        let config = host_config!(max_players: 4);
        let mut host = start_host(listener, config.clone()).await.test_value();

        let mut client = connect_test_player(addr, "Alpha").await;

        let mut host_events = host.take_event_receiver();
        let mut client_events = client.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;

        submit_control_pair(&mut host, &client, 0, 0xAA, 0x11).await;

        let first_host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(first_host_ready.tick(), 0);

        let first_client_ready = wait_for_client_ready(&mut client_events, EVENT_WAIT).await;
        assert_eq!(first_client_ready.tick(), 0);

        client.shutdown().await.test_value();
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;
        let mut fresh_snapshot = config.initial_join_snapshot.test_value();
        fresh_snapshot.dynamic_tick = 1;
        host.publish_join_snapshot(fresh_snapshot)
            .await
            .test_value();

        let mut client_beta = connect_test_player(addr, "Beta").await;
        let mut client_beta_events = client_beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client_beta.client_id()).await;

        submit_control_pair(&mut host, &client_beta, 1, 0xBB, 0x22).await;

        let second_host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(second_host_ready.tick(), 1);

        let second_client_ready = wait_for_client_ready(&mut client_beta_events, EVENT_WAIT).await;
        assert_eq!(second_client_ready.tick(), 1);

        client_beta.shutdown().await.test_value();
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_continues_ready_after_client_disconnect() {
        let (addr, listener) = bind_test_listener().await;
        let mut host = start_host(listener, host_config!(max_players: 4))
            .await
            .test_value();

        let client = connect_test_player(addr, "Alpha").await;

        let mut host_events = host.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        submit_control_pair(&mut host, &client, 0, 0xA0, 0xB0).await;
        let ready0 = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready0.tick(), 0);
        assert_eq!(control_commands(&ready0), vec![0xA0, 0xB0]);

        let host_packet = legacy_packet(0, 1, 0xC0);
        host.submit_local_control(host_packet).await.test_value();

        client.shutdown().await.test_value();
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        let ready1 = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready1.tick(), 1);
        assert_eq!(control_commands(&ready1), vec![0xC0]);

        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn running_disconnect_executes_remove_at_the_retargeted_boundary() {
        // CtrlRemove is synchronized, but OnClientDisconnect immediately
        // retargets the unreached Go barrier to ControlTick. The removal then
        // executes before control packing resumes, so the disconnected
        // client's buffered contribution is no longer part of the batch
        // (src/C4Network2.cpp:1786-1807;
        // src/C4GameControlNetwork.cpp:260-297,329-345,741-783).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(addr, "Alpha").await;
        let mut client_events = client.take_event_receiver();
        let client_id = client.client_id();
        activate_joined_client(&host, &mut host_events, client_id).await;

        let running = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);
        host.change_status(running).await.test_value();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::Status(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("client event stream ended before Go"),
            }
        }
        client.submit_status_ack(running).await.test_value();
        host.status_reached(running, running.target_tick)
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before Go committed"),
            }
        }

        client
            .submit_control(legacy_packet(client_id, 0, 0xB0))
            .await
            .test_value();
        client.graceful_part().await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ClientLeft { client_id: left }) if left == client_id => break,
                Some(_) => continue,
                None => panic!("host event stream ended before client departure"),
            }
        }
        host.status_reached(running, running.target_tick)
            .await
            .test_value();

        let mut synchronized_remove = None;
        let mut committed = false;
        while synchronized_remove.is_none() || !committed {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::SyncScheduled {
                    control_tick: 0,
                    controls,
                }) => {
                    assert!(
                        synchronized_remove.replace(controls).is_none(),
                        "ClientRemove synchronized twice"
                    );
                }
                Some(HostEvent::StatusCommitted(status))
                    if status.state == NETWORK_STATE_GO && status.target_tick == 0 =>
                {
                    committed = true;
                }
                Some(_) => continue,
                None => panic!("host event stream ended during disconnect recovery"),
            }
        }
        let controls = synchronized_remove.test_value();
        let [EngineControlPacket::ClientRemove(remove)] = controls.as_slice() else {
            panic!("expected one synchronized ClientRemove, got {controls:?}");
        };
        assert_eq!(remove.client_id, i32::try_from(client_id).unwrap());
        assert_eq!(remove.by_client, i32::try_from(HOST_CLIENT_ID).unwrap());

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .test_value();
        let boundary = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(boundary.tick(), 0);
        assert_eq!(control_commands(&boundary), vec![0xA0]);

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0xA1))
            .await
            .test_value();
        let released = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(released.tick(), 1);
        assert_eq!(control_commands(&released), vec![0xA1]);

        host.shutdown().await.test_value();
    }

    #[test]
    fn unreached_pause_disconnect_retry_waits_for_runtime_local_reach() {
        let pause = NetworkStatus::new(NETWORK_STATE_PAUSE, 1, 12);
        let mut barrier = StatusBarrier::stable(NetworkStatus::new(NETWORK_STATE_GO, 1, 3));
        barrier.set_remote_state(7, RemoteBarrierState::Ready);
        barrier.change_status(pause);

        let retargeted = pause.with_target_tick(4);
        assert_eq!(
            retry_unreached_status_after_disconnect(&mut barrier, 4),
            vec![
                BarrierEffect::InvalidateReference,
                BarrierEffect::BroadcastStatus(retargeted),
                BarrierEffect::DriveControlTo(4),
            ]
        );
        assert_eq!(barrier.status, retargeted);
        assert_eq!(
            barrier.phase,
            BarrierPhase::Waiting {
                local_reached: false
            }
        );
        assert_eq!(barrier.remotes.get(&7), Some(&RemoteBarrierState::NotReady));
        assert_eq!(
            barrier.local_reached_for(retargeted, 4),
            vec![BarrierEffect::StopControl]
        );
        assert_eq!(
            barrier.remote_ack(7, retargeted),
            vec![
                BarrierEffect::ExecutePendingSyncControls(4),
                BarrierEffect::BroadcastStatusAck(retargeted),
            ]
        );
        assert_eq!(barrier.phase, BarrierPhase::Stable);
    }

    #[test]
    fn initial_go_disconnect_retargets_but_does_not_reach_before_game_initialization() {
        let initial_go = NetworkStatus::new(NETWORK_STATE_GO, 1, 12);
        let mut barrier = StatusBarrier::stable(NetworkStatus::new(NETWORK_STATE_LOBBY, 0, -1));
        barrier.change_status(initial_go);

        let retargeted = initial_go.with_target_tick(4);
        assert_eq!(
            retry_unreached_status_after_disconnect(&mut barrier, 4),
            vec![
                BarrierEffect::InvalidateReference,
                BarrierEffect::BroadcastStatus(retargeted),
                BarrierEffect::DriveControlTo(4),
            ]
        );
        assert_eq!(barrier.status, retargeted);
        assert_eq!(
            barrier.phase,
            BarrierPhase::Waiting {
                local_reached: false
            }
        );
        assert_eq!(
            barrier.local_reached(),
            vec![
                BarrierEffect::StopControl,
                BarrierEffect::ExecutePendingSyncControls(4),
                BarrierEffect::BroadcastStatusAck(retargeted),
                BarrierEffect::SetControlMode {
                    mode: 1,
                    from_tick: 4,
                },
                BarrierEffect::SweepUnjoinedPlayers,
                BarrierEffect::StartControl,
            ],
            "FinalInit reaches the authoritative retarget, not its stale prepared copy"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unreached_go_disconnect_retargets_and_releases_client_remove() {
        // OnClientDisconnect retries an unreached Go/Pause transition at the
        // host's current control tick. This lets the synchronized ClientRemove
        // execute even when the departed client was the only participant that
        // had not supplied the original target's preceding control
        // (src/C4Network2.cpp:1786-1807).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();

        let mut alpha = connect_test_player(addr, "Alpha").await;
        let alpha_id = alpha.client_id();
        let mut alpha_events = alpha.take_event_receiver();
        activate_joined_client(&host, &mut host_events, alpha_id).await;

        let mut beta = connect_test_player(addr, "Beta").await;
        let beta_id = beta.client_id();
        let mut beta_events = beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, beta_id).await;

        let running = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);
        host.change_status(running).await.test_value();
        for events in [&mut alpha_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                    Some(ClientEvent::Status(status)) if status == running => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("client disconnected before initial Go: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("client event stream ended before initial Go"),
                }
            }
        }
        alpha.submit_status_ack(running).await.test_value();
        beta.submit_status_ack(running).await.test_value();
        host.status_reached(running, running.target_tick)
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial Go committed"),
            }
        }

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .test_value();
        alpha
            .submit_control(legacy_packet(alpha_id, 0, 0xB0))
            .await
            .test_value();
        beta.submit_control(legacy_packet(beta_id, 0, 0xC0))
            .await
            .test_value();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0xA0, 0xB0, 0xC0]);

        let unreachable = running.with_target_tick(2);
        host.change_status(unreachable).await.test_value();
        for events in [&mut alpha_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                    Some(ClientEvent::Status(status)) if status == unreachable => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("client disconnected before unreached Go: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("client event stream ended before unreached Go"),
                }
            }
        }

        beta.submit_status_ack(unreachable).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusAck { client_id, status })
                    if client_id == beta_id && status == unreachable =>
                {
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before Beta acknowledged unreached Go"),
            }
        }
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0xA1))
            .await
            .test_value();
        beta.submit_control(legacy_packet(beta_id, 1, 0xC1))
            .await
            .test_value();

        alpha.shutdown().await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ClientLeft { client_id }) if client_id == alpha_id => break,
                Some(_) => continue,
                None => panic!("host event stream ended before Alpha departed"),
            }
        }

        let retargeted = unreachable.with_target_tick(1);
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.test_value() {
                Some(ClientEvent::Status(status)) if status == retargeted => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("Beta disconnected before the retry: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("Beta event stream ended before the retry"),
            }
        }

        beta.submit_status_ack(retargeted).await.test_value();
        host.status_reached(retargeted, retargeted.target_tick)
            .await
            .test_value();

        let mut released = None;
        let mut synchronized_remove = None;
        let mut committed = false;
        while released.is_none() || synchronized_remove.is_none() || !committed {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::Ready { packet }) if packet.tick() == 1 => {
                    assert!(released.replace(packet).is_none(), "tick released twice");
                }
                Some(HostEvent::SyncScheduled {
                    control_tick: 1,
                    controls,
                }) => {
                    assert!(
                        synchronized_remove.replace(controls).is_none(),
                        "ClientRemove synchronized twice"
                    );
                }
                Some(HostEvent::StatusCommitted(status)) if status == retargeted => {
                    committed = true;
                }
                Some(_) => continue,
                None => panic!("host event stream ended during disconnect recovery"),
            }
        }

        let released = released.test_value();
        assert_eq!(control_commands(&released), vec![0xA1, 0xC1]);
        let controls = synchronized_remove.test_value();
        let [EngineControlPacket::ClientRemove(remove)] = controls.as_slice() else {
            panic!("expected one synchronized ClientRemove, got {controls:?}");
        };
        assert_eq!(remove.client_id, i32::try_from(alpha_id).unwrap());
        assert_eq!(remove.by_client, i32::try_from(HOST_CLIENT_ID).unwrap());

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 2, 0xA2))
            .await
            .test_value();
        beta.submit_control(legacy_packet(beta_id, 2, 0xC2))
            .await
            .test_value();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 2);
        assert_eq!(control_commands(&ready), vec![0xA2, 0xC2]);

        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnect_broadcasts_host_authored_synchronized_client_remove() {
        // OnClientDisconnect calls C4ClientList::CtrlRemove, which broadcasts
        // a host-authored CDT_Sync ClientRemove and executes it at the frozen
        // synchronization boundary (src/C4Network2.cpp:1786-1802;
        // src/C4Client.cpp:293-303;
        // src/C4GameControlNetwork.cpp:181-220).
        let (addr, host) = start_test_host(HostConfig::default()).await;
        let alpha = connect_test_player(addr, "Alpha").await;
        let alpha_id = alpha.client_id();
        let mut beta = connect_test_player(addr, "Beta").await;
        let mut beta_events = beta.take_event_receiver();

        alpha.shutdown().await.test_value();
        let remove = loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.test_value() {
                Some(ClientEvent::SyncScheduled { controls, .. }) => {
                    let Some(EngineControlPacket::ClientRemove(remove)) =
                        controls.into_iter().next()
                    else {
                        continue;
                    };
                    break remove;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected unexpectedly: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended unexpectedly"),
            }
        };
        assert_eq!(remove.client_id, i32::try_from(alpha_id).unwrap());
        assert_eq!(remove.by_client, 0);
        // LoadResStr(IDS_MSG_DISCONNECTED) supplies the synchronized reason
        // verbatim (planet/System.c4g/LanguageUS.txt:831).
        assert_eq!(remove.reason.as_bytes(), b"disconnected");

        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_half_accepted_join_is_removed_from_existing_clients() {
        // Join creates/broadcasts ClientJoin before mutual ConnRe. If the
        // socket then fails, OnConnectFail routes the provisional client
        // through the same synchronized CtrlRemove path
        // (src/C4Network2.cpp:1395-1445,1745-1755;
        // src/C4Client.cpp:293-303).
        let (addr, host) = start_test_host(HostConfig::default()).await;
        let mut witness = connect_client(
            addr,
            ClientConfig::new("Witness", ParticipantKind::Observer),
        )
        .await
        .test_value();
        let mut witness_events = witness.take_event_receiver();

        let stream = TcpStream::connect(addr).await.test_value();
        let mut failed = crate::ControlTransport::new(stream);
        let _ = failed.read_message().await.test_value();
        let name = c4(b"HalfJoin");
        failed
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                clonk_engine::ClientCoreControlData {
                    client_id: -1,
                    name: name.clone(),
                    nick: name,
                    ..Default::default()
                },
                77,
                false,
            )))
            .await
            .test_value();
        loop {
            match failed.read_message().await.test_value() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    failed
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => continue,
            }
        }
        drop(failed);

        let mut provisional_id = None;
        loop {
            match timeout(EVENT_WAIT, witness_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::Direct { data, .. }) => {
                    if let Ok(EngineControlPacket::ClientJoin(join)) =
                        decode_control_entry_payload(&data)
                    {
                        provisional_id = Some(join.core.client_id);
                    }
                }
                Some(ClientEvent::SyncScheduled { controls, .. }) => {
                    if let Some(EngineControlPacket::ClientRemove(remove)) =
                        controls.into_iter().next()
                    {
                        assert_eq!(Some(remove.client_id), provisional_id);
                        assert_eq!(remove.by_client, 0);
                        break;
                    }
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected unexpectedly: {reason:?}")
                }
                Some(_) => {}
                None => panic!("witness event stream ended unexpectedly"),
            }
        }

        witness.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_admission_failure_retargets_unreached_go_and_executes_remove() {
        // A failed half-accepted runtime join has already broadcast ClientJoin
        // before its socket disappears. OnConnectFail queues the matching
        // ClientRemove and performs the same unreached Go/Pause retry as an
        // established-client disconnect (src/C4Network2.cpp:1745-1755,
        // 1786-1807; src/C4Client.cpp:293-303).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut witness = connect_test_player(addr, "Witness").await;
        let witness_id = witness.client_id();
        let mut witness_events = witness.take_event_receiver();
        activate_joined_client(&host, &mut host_events, witness_id).await;

        let running = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);
        host.change_status(running).await.test_value();
        loop {
            match timeout(EVENT_WAIT, witness_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::Status(status)) if status == running => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected before initial Go: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("witness event stream ended before initial Go"),
            }
        }
        witness.submit_status_ack(running).await.test_value();
        host.status_reached(running, running.target_tick)
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial Go committed"),
            }
        }

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .test_value();
        witness
            .submit_control(legacy_packet(witness_id, 0, 0xB0))
            .await
            .test_value();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0xA0, 0xB0]);

        let unreachable = running.with_target_tick(2);
        host.change_status(unreachable).await.test_value();
        loop {
            match timeout(EVENT_WAIT, witness_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::Status(status)) if status == unreachable => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected before unreached Go: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("witness event stream ended before unreached Go"),
            }
        }
        witness.submit_status_ack(unreachable).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::StatusAck { client_id, status })
                    if client_id == witness_id && status == unreachable =>
                {
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before unreached Go acknowledgement"),
            }
        }

        let stream = TcpStream::connect(addr).await.test_value();
        let mut failed = crate::ControlTransport::new(stream);
        assert!(matches!(
            failed.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = c4(b"HalfJoin");
        failed
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                clonk_engine::ClientCoreControlData {
                    client_id: -1,
                    name: name.clone(),
                    nick: name,
                    ..Default::default()
                },
                77,
                false,
            )))
            .await
            .test_value();
        loop {
            match failed.read_message().await.test_value() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    failed
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => continue,
            }
        }

        let provisional_id = loop {
            match timeout(EVENT_WAIT, witness_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::Direct { data, .. }) => {
                    if let Ok(EngineControlPacket::ClientJoin(join)) =
                        decode_control_entry_payload(&data)
                    {
                        break join.core.client_id;
                    }
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected before provisional ClientJoin: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("witness event stream ended before provisional ClientJoin"),
            }
        };
        drop(failed);

        let retargeted = unreachable.with_target_tick(1);
        loop {
            match timeout(EVENT_WAIT, witness_events.recv())
                .await
                .test_value()
            {
                Some(ClientEvent::Status(status)) if status == retargeted => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected before admission retry: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("witness event stream ended before admission retry"),
            }
        }
        witness.submit_status_ack(retargeted).await.test_value();
        host.status_reached(retargeted, retargeted.target_tick)
            .await
            .test_value();

        let mut synchronized_remove = None;
        let mut committed = false;
        let mut connection_failed = false;
        let mut diagnostic = false;
        while synchronized_remove.is_none() || !committed || !connection_failed || !diagnostic {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ClientConnectionFailed { client_id })
                    if i32::try_from(client_id).ok() == Some(provisional_id) =>
                {
                    connection_failed = true;
                }
                Some(HostEvent::RecoverableRouteDiagnostic {
                    client_id: Some(client_id),
                    error,
                }) if i32::try_from(client_id).ok() == Some(provisional_id) => {
                    assert!(error.contains("connection admission from"));
                    diagnostic = true;
                }
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if i32::try_from(client_id).ok() == Some(provisional_id) => {
                    panic!("provisional admission recovery became terminal: {error}");
                }
                Some(HostEvent::SyncScheduled {
                    control_tick: 1,
                    controls,
                }) => {
                    assert!(
                        synchronized_remove.replace(controls).is_none(),
                        "provisional ClientRemove synchronized twice"
                    );
                }
                Some(HostEvent::StatusCommitted(status)) if status == retargeted => {
                    committed = true;
                }
                Some(_) => continue,
                None => panic!("host event stream ended during admission recovery"),
            }
        }
        let controls = synchronized_remove.test_value();
        let [EngineControlPacket::ClientRemove(remove)] = controls.as_slice() else {
            panic!("expected one synchronized provisional ClientRemove, got {controls:?}");
        };
        assert_eq!(remove.client_id, provisional_id);
        assert_eq!(remove.by_client, i32::try_from(HOST_CLIENT_ID).unwrap());

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0xA1))
            .await
            .test_value();
        witness
            .submit_control(legacy_packet(witness_id, 1, 0xB1))
            .await
            .test_value();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 1);
        assert_eq!(control_commands(&ready), vec![0xA1, 0xB1]);

        witness.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_secondary_known_connection_keeps_the_canonical_client() {
        // OnConnectFail removes a half-accepted client only when that client has
        // no other connection. Losing a secondary route therefore leaves the
        // already-connected canonical client registered
        // (src/C4Network2.cpp:1366-1380,1745-1765).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut canonical = connect_test_player(addr, "Alice").await;
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();

        let stream = TcpStream::connect(addr).await.test_value();
        let mut secondary = crate::ControlTransport::new(stream);
        assert!(matches!(
            secondary.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = c4(b"Alice");
        secondary
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                test_client_core(i32::try_from(canonical_id).unwrap(), name, true),
                29,
                false,
            )))
            .await
            .test_value();
        loop {
            match secondary.read_message().await.test_value() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                _ => continue,
            }
        }
        drop(secondary);

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::SyncScheduled { controls, .. }) => assert!(
                    !controls.iter().any(|control| matches!(
                        control,
                        EngineControlPacket::ClientRemove(remove)
                            if remove.client_id == i32::try_from(canonical_id).unwrap()
                    )),
                    "secondary route failure queued ClientRemove for the canonical client"
                ),
                Some(HostEvent::ClientLeft { client_id }) if client_id == canonical_id => {
                    panic!("secondary route failure removed the canonical client")
                }
                Some(HostEvent::RecoverableRouteDiagnostic { client_id, error })
                    if error.contains("connection admission from") =>
                {
                    assert_eq!(client_id, Some(canonical_id));
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before secondary admission failed"),
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        while let Ok(Some(event)) = timeout_at(deadline, canonical_events.recv()).await {
            match event {
                ClientEvent::SyncScheduled { controls, .. }
                    if controls.iter().any(|control| {
                        matches!(
                            control,
                            EngineControlPacket::ClientRemove(remove)
                                if remove.client_id == i32::try_from(canonical_id).unwrap()
                        )
                    }) =>
                {
                    panic!("canonical client executed a secondary-route ClientRemove")
                }
                ClientEvent::Disconnected { reason } => {
                    panic!("canonical client disconnected unexpectedly: {reason:?}")
                }
                _ => {}
            }
        }

        canonical.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pre_admission_peer_close_is_recorded_without_a_route_diagnostic() {
        // Incoming TCP that closes before PID_Conn never associates a client.
        // C4NetIOTCP reports recv()==0 as "connection closed"; OnDisconn and
        // OnConnectFail log that at info. The GUI sink defaults to warn, so
        // MainDlg::OnLog never sees it while the log file still does
        // (src/C4NetIO.cpp:749; src/C4Network2IO.cpp:533-566;
        // src/C4Network2.cpp:1745-1747; src/C4Log.cpp:307).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut probe = crate::ControlTransport::new(stream);
        assert!(matches!(
            probe.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        drop(probe);

        let mut recorded = false;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        while let Ok(Some(event)) = timeout_at(deadline, host_events.recv()).await {
            match event {
                HostEvent::RecoverableRouteDiagnostic { error, .. } => {
                    panic!("pre-admission peer close reached the lobby diagnostic: {error}");
                }
                HostEvent::ClientConnectionFailed { client_id } => {
                    panic!("pre-admission peer close created a logical client {client_id}");
                }
                HostEvent::UnassociatedConnectionFailed { .. } => recorded = true,
                _ => {}
            }
        }
        assert!(recorded, "the closed socket left no record at all");

        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_client_starts_at_fresh_dynamic_tick_without_old_backlog() {
        let (addr, listener) = bind_test_listener().await;
        let config = host_config!(max_players: 4);
        let mut host = start_host(listener, config.clone()).await.test_value();

        let mut host_events = host.take_event_receiver();
        let client_alpha = connect_test_player(addr, "Alpha").await;
        activate_joined_client(&host, &mut host_events, client_alpha.client_id()).await;

        submit_control_pair(&mut host, &client_alpha, 0, 0xA1, 0xB2).await;
        let ready_packet = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready_packet.tick(), 0);

        // A runtime join receives a dynamic snapshot for the next control tick.
        // C++ sends no eager backlog after JoinData; Init requests exactly the
        // snapshot tick, so controls already represented by the dynamic must
        // not replay (src/C4Network2.cpp:1820-1850;
        // src/C4GameControlNetwork.cpp:46-62,531-555).
        client_alpha.shutdown().await.test_value();
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;
        let mut fresh_snapshot = config.initial_join_snapshot.test_value();
        fresh_snapshot.dynamic_tick = 1;
        host.publish_join_snapshot(fresh_snapshot)
            .await
            .test_value();

        let mut client_beta = connect_test_player(addr, "Beta").await;
        let mut beta_events = client_beta.take_event_receiver();
        assert!(timeout(Duration::from_millis(50), beta_events.recv())
            .await
            .is_err());
        activate_joined_client(&host, &mut host_events, client_beta.client_id()).await;

        submit_control_pair(&mut host, &client_beta, 1, 0xC3, 0xD4).await;
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 1);
        assert_eq!(control_commands(&ready), vec![0xC3, 0xD4]);
        assert_eq!(
            wait_for_client_ready(&mut beta_events, EVENT_WAIT).await,
            ready
        );

        client_beta.shutdown().await.test_value();
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_resends_backlog_when_requested() {
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop(512, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        let packet = legacy_packet(7, 42, 0xDE);
        command_tx
            .send(ClientCommand::SubmitControl(packet.clone()))
            .await
            .test_value();

        expect_control_wait_attribution_capability(&mut host_transport).await;

        match host_transport.read_message().await.test_value() {
            ControlMessage::Control(received) => {
                assert_eq!(received.client_id(), packet.client_id());
                assert_eq!(received.tick(), packet.tick());
                assert_eq!(received.payload(), packet.payload());
            }
            other => panic!("expected control packet, got {other:?}"),
        }

        // Ensure the client loop processed the send before issuing the request.
        while let Ok(Some(event)) = timeout(Duration::from_millis(20), event_rx.recv()).await {
            match event {
                ClientEvent::LocalAddressesChanged { .. }
                | ClientEvent::PingMeasured { .. }
                | ClientEvent::Ready { .. }
                | ClientEvent::Direct { .. }
                | ClientEvent::ExecSync { .. }
                | ClientEvent::Status(_)
                | ClientEvent::StatusAck(_)
                | ClientEvent::LobbyCountdown { .. }
                | ClientEvent::ReadyCheck { .. }
                | ClientEvent::ResourceAction(_)
                | ClientEvent::ResourceProgress { .. }
                | ClientEvent::ResourceComplete { .. }
                | ClientEvent::ResourceLoadFailed { .. }
                | ClientEvent::ResourceDeriveUnsupported { .. }
                | ClientEvent::LeagueRoundResults { .. }
                | ClientEvent::HostRestarting { .. }
                | ClientEvent::HostRestartLobby
                | ClientEvent::JoinData { .. }
                | ClientEvent::UnhandledPacket { .. }
                | ClientEvent::SyncScheduled { .. } => continue,
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected unexpectedly: {reason:?}");
                }
            }
        }

        host_transport
            .send_message(ControlMessage::Request { from_tick: 42 })
            .await
            .test_value();

        match host_transport.read_message().await.test_value() {
            ControlMessage::Control(resend) => {
                assert_eq!(resend.client_id(), packet.client_id());
                assert_eq!(resend.tick(), packet.tick());
                assert_eq!(resend.payload(), packet.payload());
            }
            other => panic!("expected resend control packet, got {other:?}"),
        }

        shutdown_tx.send(()).ok();
        client_handle.await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn central_client_repeats_missing_control_request_on_cpp_interval() {
        async fn next_request<S>(transport: &mut crate::ControlTransport<S>) -> ControlMessage
        where
            S: AsyncRead + AsyncWrite + Unpin,
        {
            loop {
                match transport.read_message().await.test_value() {
                    request @ ControlMessage::Request { .. } => return request,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .test_value();
                    }
                    other => panic!("expected control request, got {other:?}"),
                }
            }
        }

        let (host_stream, _command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop(512, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let status = NetworkStatus::new(NETWORK_STATE_GO, 1, 1);

        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .test_value();
        assert!(
            matches!(event_rx.recv().await, Some(ClientEvent::Status(value)) if value == status)
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        let future = legacy_packet(BROADCAST_CLIENT_ID, 1, 0x31);
        host_transport
            .send_message(ControlMessage::Control(future.clone()))
            .await
            .test_value();
        assert!(
            matches!(event_rx.recv().await, Some(ClientEvent::Ready { packet }) if packet == future)
        );

        tokio::time::advance(Duration::from_millis(998)).await;
        assert!(
            timeout(Duration::from_millis(1), next_request(&mut host_transport))
                .await
                .is_err()
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(
            next_request(&mut host_transport).await,
            ControlMessage::Request { from_tick: 0 }
        );

        tokio::time::advance(CONTROL_REQUEST_INTERVAL - Duration::from_millis(2)).await;
        assert!(
            timeout(Duration::from_millis(1), next_request(&mut host_transport))
                .await
                .is_err()
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(
            next_request(&mut host_transport).await,
            ControlMessage::Request { from_tick: 0 }
        );

        host_transport
            .send_message(ControlMessage::StatusAck(status))
            .await
            .test_value();
        loop {
            match event_rx.recv().await {
                Some(ClientEvent::StatusAck(value)) if value == status => break,
                Some(ClientEvent::PingMeasured { .. }) => continue,
                other => panic!("expected final status acknowledgement, got {other:?}"),
            }
        }
        tokio::time::advance(CONTROL_REQUEST_INTERVAL + Duration::from_secs(1)).await;
        assert!(
            timeout(Duration::from_millis(1), next_request(&mut host_transport))
                .await
                .is_err()
        );

        shutdown_tx.send(()).ok();
        client_loop.await.test_value();
    }

    #[tokio::test(start_paused = true)]
    async fn central_client_recovers_a_missing_due_tick_after_go_without_requesting_the_future() {
        async fn next_request<S>(transport: &mut crate::ControlTransport<S>) -> ControlMessage
        where
            S: AsyncRead + AsyncWrite + Unpin,
        {
            loop {
                match transport.read_message().await.test_value() {
                    request @ ControlMessage::Request { .. } => return request,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .test_value();
                    }
                    other => panic!("expected control request, got {other:?}"),
                }
            }
        }

        // C++ requests missing controls every two seconds (oracle-src-pinned
        // src/C4GameControlNetwork.h:31), then clears the finite startup target
        // when GO starts normal control execution (oracle-src-pinned
        // src/C4Network2.cpp:2101-2109; src/C4GameControlNetwork.h:144;
        // src/C4GameControlNetwork.cpp:329-337). Rust extends that liveness into
        // the asynchronous runtime worker, but only through the latest control
        // tick the application has actually reached.
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.register(1).test_value();
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop_with_state(512, 8, 8, BTreeMap::new(), resource_state);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let running = NetworkStatus::new(NETWORK_STATE_GO, 1, 0);

        host_transport
            .send_message(ControlMessage::Status(running))
            .await
            .test_value();
        assert!(matches!(
            event_rx.recv().await,
            Some(ClientEvent::Status(status)) if status == running
        ));
        command_tx
            .send(ClientCommand::SubmitStatusAck(running))
            .await
            .test_value();
        assert_eq!(
            host_transport.read_message().await.unwrap(),
            ControlMessage::StatusAck(running)
        );
        host_transport
            .send_message(ControlMessage::StatusAck(running))
            .await
            .test_value();
        assert!(matches!(
            event_rx.recv().await,
            Some(ClientEvent::StatusAck(status)) if status == running
        ));

        let local = legacy_packet(1, 0, 0x21);
        command_tx
            .send(ClientCommand::SubmitControl(local.clone()))
            .await
            .test_value();
        expect_control_wait_attribution_capability(&mut host_transport).await;
        assert_eq!(
            host_transport.read_message().await.unwrap(),
            ControlMessage::Control(local)
        );

        let reached_at = tokio::time::Instant::now();
        command_tx
            .send(ClientCommand::ControlTickReached {
                tick: 0,
                reached_at,
            })
            .await
            .test_value();
        tokio::time::advance(CONTROL_REQUEST_INTERVAL).await;
        assert_eq!(
            timeout(Duration::from_millis(1), next_request(&mut host_transport))
                .await
                .expect("missing due runtime control was not requested"),
            ControlMessage::Request { from_tick: 0 }
        );

        let recovered = legacy_packet(BROADCAST_CLIENT_ID, 0, 0x30);
        host_transport
            .send_message(ControlMessage::Control(recovered.clone()))
            .await
            .test_value();
        assert!(matches!(
            event_rx.recv().await,
            Some(ClientEvent::Ready { packet }) if packet == recovered
        ));

        tokio::time::advance(CONTROL_REQUEST_INTERVAL).await;
        assert!(
            timeout(Duration::from_millis(1), next_request(&mut host_transport))
                .await
                .is_err(),
            "recovery crossed the last runtime control tick the application reached"
        );

        shutdown_tx.send(()).ok();
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_graceful_part_sends_exact_cpp_removal_frame_before_close() {
        // C4Network2ClientList::DeleteClient asks CloseConns to send a negative
        // PID_ConnRe with "removing client" before closing the connection
        // (src/C4Network2Client.cpp:104-119,457-492).
        let (mut host_stream, command_tx, event_rx, shutdown_tx, join_handle) =
            start_test_client_loop(128, 1, 1);
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            control_wait_attribution: Default::default(),
            event_rx: Some(event_rx),
            voice_sender: crate::VoiceSender::new(mpsc::channel(1).0),
            voice_event_rx: Some(mpsc::channel(1).1),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            client_id: 1,
            join_data: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        handle.graceful_part().await.test_value();

        let mut bytes = Vec::new();
        host_stream.read_to_end(&mut bytes).await.test_value();
        assert_eq!(
            bytes,
            [
                0xff, 0x13, 0x00, 0x00, 0x00, 0x03, 0x00, b'r', b'e', b'm', b'o', b'v', b'i', b'n',
                b'g', b' ', b'c', b'l', b'i', b'e', b'n', b't', 0x00, 0x00,
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_treats_negative_post_admission_connre_as_host_removal() {
        // CloseConns sends the same negative PID_ConnRe on an already accepted
        // connection so the peer can report the removal reason before EOF
        // (src/C4Network2Client.cpp:104-119).
        let (host_stream, _command_tx, mut event_rx, _shutdown_tx, client_loop) =
            start_test_client_loop(128, 1, 1);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                false,
                c4(b"removing client"),
                false,
            )))
            .await
            .test_value();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Disconnected { reason: Some(reason) })
                if reason == "removing client"
        ));
        await_test(client_loop).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_still_rejects_positive_post_admission_connre_as_duplicate() {
        // A positive ConnRe only completes connection admission. Receiving a
        // second positive reply after admission is not the CloseConns removal
        // signal (src/C4Network2.cpp:1448-1474).
        let (host_stream, _command_tx, mut event_rx, _shutdown_tx, client_loop) =
            start_test_client_loop(128, 1, 1);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                true,
                c4(b"duplicate"),
                false,
            )))
            .await
            .test_value();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Disconnected { reason: Some(reason) })
                if reason == "host sent a duplicate connection reply"
        ));
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_reports_negative_post_admission_connre_once_before_eof() {
        // CloseConns writes the negative PID_ConnRe and immediately closes the
        // socket; the accepted connection must therefore report one removal,
        // not another disconnect when that close becomes EOF
        // (src/C4Network2Client.cpp:104-119,457-492).
        let (host_stream, client_stream) = duplex(128);
        let (_outbound_tx, mut host_rx, task) = start_test_host_route(host_stream, 7);
        let mut client_transport = crate::ControlTransport::new(client_stream);

        client_transport
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                false,
                c4(b"removing client"),
                false,
            )))
            .await
            .test_value();
        drop(client_transport);
        task.await.test_value();

        let mut messages = Vec::new();
        while let Some(message) = host_rx.recv().await {
            messages.push(message);
        }
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            messages.pop(),
            Some(HostLoopMessage::ClientDisconnected {
                connection_id: 3,
                client_id: 7,
                next_inbound_packet: 0,
                next_outbound_packet: _,
                post_mortem: None,
                reason: Some(reason),
            }) if reason == "removing client"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disconnected_connection_emits_cpp_post_mortem_backlog_for_peer_id() {
        // OnDisconn retains the closed connection; C4Network2 then builds one
        // recovery packet from its logged sends, identifying the dead socket
        // with iRemoteID so the peer can find its own local connection record
        // (src/C4Network2IO.cpp:520-570,1379-1396;
        // src/C4Network2.cpp:883-905).
        let (host_stream, client_stream) = duplex(256);
        let (outbound_tx, mut host_rx, task) = start_test_host_route(host_stream, 7);
        let mut client_transport = crate::ControlTransport::new(client_stream);
        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, -1);

        outbound_tx
            .send(ControlMessage::Status(status))
            .await
            .test_value();
        assert_eq!(
            client_transport.read_message().await.unwrap(),
            ControlMessage::Status(status)
        );
        drop(client_transport);
        task.await.test_value();

        let Some(HostLoopMessage::ClientDisconnected {
            connection_id: 3,
            client_id: 7,
            next_inbound_packet: 0,
            next_outbound_packet: _,
            post_mortem: Some(post_mortem),
            reason: None,
        }) = host_rx.recv().await
        else {
            panic!("expected recovery backlog for the disconnected route");
        };
        assert_eq!(post_mortem.connection_id, 5);
        assert_eq!(post_mortem.packet_counter, 1);
        assert_eq!(post_mortem.packets.len(), 1);
        assert_eq!(
            crate::transport::parse_complete_packet(&post_mortem.packets[0]).unwrap(),
            Some(ControlMessage::Status(status))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_host_route_retains_commands_already_accepted_by_its_queue() {
        let (host_stream, client_stream) = duplex(256);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
        let first = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 7);
        let second = NetworkStatus::new(NETWORK_STATE_PAUSE, 2, 8);
        outbound_tx
            .send(ControlMessage::Status(first))
            .await
            .test_value();
        outbound_tx
            .send(ControlMessage::Status(second))
            .await
            .test_value();
        drop(client_stream);

        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id: 7,
                transport: crate::ControlTransport::new(host_stream),
                outbound_rx,
                retire_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );
        task.await.test_value();
        let Some(HostLoopMessage::ClientDisconnected {
            post_mortem: Some(post_mortem),
            ..
        }) = host_rx.recv().await
        else {
            panic!("failed host route did not retain queued commands");
        };
        assert_eq!(post_mortem.connection_id, 5);
        assert_eq!(post_mortem.packet_counter, 2);
        assert_eq!(
            post_mortem
                .packets
                .iter()
                .map(|packet| crate::transport::parse_complete_packet(packet).unwrap())
                .collect::<Vec<_>>(),
            vec![
                Some(ControlMessage::Status(first)),
                Some(ControlMessage::Status(second)),
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_writer_failure_preserves_fifo_across_post_mortem_fallback() {
        // C4Network2::OnDisconn removes the failed message connection before
        // it creates and queues PID_PostMortem. Until that main-thread event,
        // C4Network2IOConnection::Send keeps accepting packets into the dead
        // connection's log, preventing fallback traffic from overtaking it
        // (oracle-src-pinned src/C4Network2.cpp:873-912;
        // src/C4Network2Client.cpp:90-102,121-124;
        // src/C4Network2IO.cpp:1451-1491).
        let client_id = 7;
        let (failed, failed_rx) = HostOutboundSender::channel();
        let retire_rx = failed.subscribe_retire();
        let (fallback, mut fallback_rx) = HostOutboundSender::channel();
        let mut state = host_state_with_test_route(client_id, failed.clone());
        let failed_route = state.accepted_routes.get_mut(&1).test_value();
        failed_route.remote_connection_id = 11;
        failed_route.protocol = crate::NetworkProtocol::Udp;
        state.accepted_routes.insert(
            2,
            AcceptedConnectionRoute {
                client_id,
                remote_connection_id: 12,
                peer_addr: "127.0.0.1:11112".parse().test_value(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: RoutePingLag::default(),
                outbound: fallback,
                voice_auth: crate::voice::VoiceRouteAuthentication::default(),
                peer_is_port: false,
            },
        );
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
        let route_task = tokio::spawn(
            ClientTask {
                local_connection_id: 1,
                remote_connection_id: 11,
                client_id,
                transport: crate::ControlTransport::new(FailingWriteStream),
                outbound_rx: failed_rx,
                retire_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );
        let first = NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 51);
        let second = NetworkStatus::new(NETWORK_STATE_PAUSE, 1, 52);

        assert!(try_send_host_message(
            &state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Status(first),
        ));
        timeout(EVENT_WAIT, async {
            loop {
                if failed.writer_channel_is_closed() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .test_value();
        failed.retire();
        assert!(try_send_host_message(
            &state,
            client_id,
            ConnectionTrafficClass::Message,
            ControlMessage::Status(second),
        ));

        let Some(HostLoopMessage::ClientDisconnected {
            connection_id,
            client_id: source_client_id,
            next_inbound_packet,
            next_outbound_packet,
            post_mortem,
            reason,
        }) = timeout(EVENT_WAIT, host_rx.recv()).await.test_value()
        else {
            panic!("failed host route did not publish its post-mortem");
        };
        handle_client_disconnected(
            connection_id,
            source_client_id,
            next_inbound_packet,
            next_outbound_packet,
            post_mortem,
            reason,
            &mut state,
        )
        .await;

        let mut logical_order = Vec::new();
        while logical_order.len() < 2 {
            match timeout(EVENT_WAIT, fallback_rx.recv())
                .await
                .expect("host fallback replay stalled")
                .test_value()
            {
                HostOutboundMessage::Message(ControlMessage::PostMortem(packet)) => {
                    logical_order.extend(packet.packets.into_iter().map(|packet| {
                        crate::transport::parse_complete_packet(&packet)
                            .test_value()
                            .test_value()
                    }));
                }
                HostOutboundMessage::Message(message) => logical_order.push(message),
                HostOutboundMessage::Raw(packet) => {
                    logical_order.push(
                        crate::transport::parse_complete_packet(&packet)
                            .test_value()
                            .test_value(),
                    );
                }
            }
        }
        assert_eq!(
            logical_order,
            vec![
                ControlMessage::Status(first),
                ControlMessage::Status(second),
            ]
        );

        route_task.await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn graceful_client_part_emits_one_host_departure_with_cpp_reason() {
        // DeleteClient closes the accepted peer with "removing client"; the
        // receiving network owns one disconnect notification even though EOF
        // follows the ConnRe frame (src/C4Network2Client.cpp:104-119,457-492).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let client = connect_test_player(addr, "Alpha").await;
        let client_id = client.client_id();

        client.graceful_part().await.test_value();

        let mut departures = 0;
        let mut saw_reason = false;
        while departures == 0 || !saw_reason {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ClientLeft { client_id: left }) if left == client_id => {
                    departures += 1;
                }
                Some(HostEvent::RecoverableRouteDiagnostic {
                    client_id: Some(source),
                    error,
                }) if source == client_id && error == "removing client" => {
                    saw_reason = true;
                }
                Some(_) => {}
                None => panic!("host event stream ended before graceful departure"),
            }
        }
        while let Ok(Some(event)) = timeout(Duration::from_millis(50), host_events.recv()).await {
            if matches!(event, HostEvent::ClientLeft { client_id: left } if left == client_id) {
                departures += 1;
            }
        }
        assert_eq!(departures, 1);

        host.shutdown().await.test_value();
    }

    /// The notice has to reach the app as its own event while the connection
    /// is still up, because the disconnect that follows carries no reason a
    /// client could act on.
    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_a_host_restart_notice_before_the_disconnect() {
        let (host_stream, _command_tx, mut event_rx, _shutdown_tx, client_loop) =
            start_test_client_loop(512, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::HostRestarting { rejoin_seconds: 30 })
            .await
            .test_value();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::HostRestarting { rejoin_seconds: 30 })
        ));

        drop(host_transport);
        let _ = timeout(EVENT_WAIT, client_loop).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_lobby_countdown_without_disconnecting() {
        // MainDlg receives every PID_LobbyCountdown and updates its local
        // countdown state; the packet does not close the connection
        // (src/C4GameLobby.cpp:392-418,695-701).
        let (host_stream, _command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop(512, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let packet = crate::LobbyCountdownPacket::new(5);

        host_transport
            .send_message(ControlMessage::LobbyCountdown(packet))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::LobbyCountdown { packet: received }) if received == packet
        ));

        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 0, 0);
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Status(received)) if received == status
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_surfaces_and_broadcasts_its_lobby_countdown() {
        // Countdown construction broadcasts the packet to clients while the
        // host applies the same packet directly to its local MainDlg
        // (src/C4GameLobby.cpp:1111-1131).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut client = connect_test_player(addr, "Alpha").await;
        let mut client_events = client.take_event_receiver();
        let packet = crate::LobbyCountdownPacket::new(5);

        host.submit_lobby_countdown(packet).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::LobbyCountdown { packet: received }) => {
                    assert_eq!(received, packet);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before lobby countdown"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.test_value() {
                Some(ClientEvent::LobbyCountdown { packet: received }) => {
                    assert_eq!(received, packet);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client disconnected during lobby countdown: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before lobby countdown"),
            }
        }

        shutdown_test_session(client, host).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_ready_check_without_disconnecting() {
        // Accepted PID_ReadyCheck packets are dispatched through
        // C4Network2::HandlePacket/HandleReadyCheck and do not close the
        // connection (src/C4Network2.cpp:949-953,1625-1707).
        let (host_stream, _command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop(512, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let packet = ReadyCheckPacket::new(0, crate::ReadyCheckData::Request);

        host_transport
            .send_message(ControlMessage::ReadyCheck(packet))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet: received }) if received == packet
        ));

        let status = NetworkStatus::new(NETWORK_STATE_LOBBY, 0, 0);
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Status(received)) if received == status
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ignores_nonhost_ready_request_without_disconnecting() {
        // HandleReadyCheck accepts a Request only when packet.Client resolves
        // to the host; a rejected request returns without closing the network
        // connection (src/C4Network2.cpp:1625-1646).
        let (host_stream, _command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop(512, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let rejected = ReadyCheckPacket::new(1, crate::ReadyCheckData::Request);

        host_transport
            .send_message(ControlMessage::ReadyCheck(rejected))
            .await
            .test_value();
        assert!(timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err());

        let accepted = ReadyCheckPacket::new(1, crate::ReadyCheckData::Ready);
        host_transport
            .send_message(ControlMessage::ReadyCheck(accepted))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet }) if packet == accepted
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_filters_nonhost_ready_request_buffered_during_join() {
        // Packets buffered until JoinData must still pass through the same
        // HandleReadyCheck host-request validation as live packets
        // (src/C4Network2.cpp:949-953,1625-1646).
        let rejected = ReadyCheckPacket::new(1, crate::ReadyCheckData::Request);
        let accepted = ReadyCheckPacket::new(0, crate::ReadyCheckData::Request);
        let mut resource_state = ClientResourceState::empty();
        resource_state.initial_ready_checks = vec![rejected, accepted];
        let (_host_stream, _command_tx, mut event_rx, shutdown_tx, client_loop) =
            start_test_client_loop_with_state(512, 8, 8, BTreeMap::new(), resource_state);

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet }) if packet == accepted
        ));
        assert!(timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err());

        shutdown_tx.send(()).ok();
        client_loop.await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_ignores_network_ready_request_but_relays_its_opaque_fanout_leg() {
        // HandleReadyCheck rejects every Request while this process is the
        // host. HandleFwdReq still relays the opaque packet to selected peers,
        // where a claimed host author is accepted; Ready/NotReady likewise
        // select packet.Client without checking the transport origin
        // (src/C4Network2IO.cpp:1077-1129;
        // src/C4Network2.cpp:1625-1654,1700-1703).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let alpha = connect_test_player(addr, "Alpha").await;
        let mut beta = connect_test_player(addr, "Beta").await;
        let mut beta_events = beta.take_event_receiver();
        let request = ReadyCheckPacket::new(HOST_CLIENT_ID as i32, crate::ReadyCheckData::Request);

        alpha.submit_ready_check(request).await.test_value();
        while let Ok(Some(event)) = timeout(Duration::from_millis(50), host_events.recv()).await {
            assert!(
                !matches!(event, HostEvent::ReadyCheck { packet } if packet == request),
                "host surfaced a network-origin ready request"
            );
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.test_value() {
                Some(ClientEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, request);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected during opaque request relay: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended before opaque request relay"),
            }
        }

        let spoofed_ready =
            ReadyCheckPacket::new(HOST_CLIENT_ID as i32, crate::ReadyCheckData::Ready);
        alpha.submit_ready_check(spoofed_ready).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, spoofed_ready);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before spoofed ready"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.test_value() {
                Some(ClientEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, spoofed_ready);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected during spoofed ready: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended before spoofed ready"),
            }
        }

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_relays_ready_check_unchanged_and_broadcasts_local_submission() {
        // Ready-check packets carry their claimed Client field through
        // BroadcastMsgToClients; HandleReadyCheck looks that client up without
        // comparing it to the transport origin (src/C4GameLobby.cpp:329-343,
        // 1072-1088; src/C4Network2.cpp:1625-1635).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let mut alpha = connect_test_player(addr, "Alpha").await;
        let mut alpha_events = alpha.take_event_receiver();
        let mut beta = connect_test_player(addr, "Beta").await;
        let mut beta_events = beta.take_event_receiver();
        let relayed = ReadyCheckPacket::new(0, crate::ReadyCheckData::Ready);

        alpha.submit_ready_check(relayed).await.test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, relayed);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before ready-check relay"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.test_value() {
                Some(ClientEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, relayed);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected during ready-check relay: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended before ready-check relay"),
            }
        }
        let host_duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(host_duplicate_deadline, host_events.recv()).await {
            assert!(
                !matches!(event, HostEvent::ReadyCheck { packet } if packet == relayed),
                "host handled one ready-check toggle twice"
            );
            assert!(
                !matches!(
                    event,
                    HostEvent::TransportError {
                        client_id: Some(client_id),
                        ..
                    } if client_id == alpha.client_id()
                ),
                "host rejected the ready-check forwarding leg"
            );
        }
        let beta_duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(beta_duplicate_deadline, beta_events.recv()).await {
            assert!(
                !matches!(event, ClientEvent::ReadyCheck { packet } if packet == relayed),
                "beta received one ready-check toggle twice"
            );
        }

        let local = ReadyCheckPacket::new(0, crate::ReadyCheckData::Request);
        host.submit_ready_check(local).await.test_value();
        for events in [&mut alpha_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.test_value() {
                    Some(ClientEvent::ReadyCheck { packet }) => {
                        assert_eq!(packet, local);
                        break;
                    }
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("client disconnected during host ready-check: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("client event stream ended before host ready-check"),
                }
            }
        }

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ready_check_updates_the_claimed_client_in_later_join_data() {
        // HandleReadyCheck mutates the C4Client selected by packet.Client;
        // later JoinData serializes that same Game.Clients registry
        // (src/C4Network2.cpp:1625-1635,1721-1729,1810-1850).
        let (addr, mut host) = start_test_host(HostConfig::default()).await;
        let mut host_events = host.take_event_receiver();
        let alpha = connect_test_player(addr, "Alpha").await;
        alpha
            .submit_ready_check(ReadyCheckPacket::new(0, crate::ReadyCheckData::Ready))
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.test_value() {
                Some(HostEvent::ReadyCheck { .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before ready-check"),
            }
        }

        let mut beta = connect_test_player(addr, "Beta").await;
        let join_data = beta.take_join_data().test_value();
        assert!(
            join_data
                .parameters
                .clients
                .clients
                .iter()
                .find(|client| client.client_id == 0)
                .expect("host remains in client registry")
                .lobby_ready
        );

        alpha.shutdown().await.test_value();
        beta.shutdown().await.test_value();
        host.shutdown().await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn direct_client_join_extends_the_address_owner_registry() {
        // CID_ClientJoin executes as direct control before later PID_Addr
        // propagation for that owner. The receiver must therefore admit the
        // new owner before handling its address packets
        // (src/C4Network2.cpp:1395-1445;
        // src/C4Network2Client.cpp:581-621).
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop_with_state(
                2048,
                8,
                8,
                BTreeMap::from([(0, Vec::new()), (1, Vec::new())]),
                ClientResourceState::empty(),
            );
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let name = c4(b"Beta");
        let direct = encode_control_entry_payload(&EngineControlPacket::ClientJoin(
            clonk_engine::ClientJoinControlData {
                core: clonk_engine::ClientCoreControlData {
                    client_id: 2,
                    name: name.clone(),
                    nick: name,
                    ..Default::default()
                },
                by_client: 0,
            },
        ))
        .test_value();
        host_transport
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: direct,
            })
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Direct { .. })
        ));

        let address = crate::AddressPacket {
            client_id: 2,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "198.51.100.22:11112".parse().test_value(),
            ),
        };
        host_transport
            .send_message(ControlMessage::Address(address))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Address(address)
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn synchronized_client_remove_changes_address_membership_at_exec_sync() {
        // CtrlRemove is delivered as CDT_Sync, so the client remains present
        // until PID_ExecSyncCtrl executes the queued removal
        // (src/C4Client.cpp:293-304;
        // src/C4GameControlNetwork.cpp:181-220,558-588).
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop_with_state(
                2048,
                8,
                8,
                BTreeMap::from([(0, Vec::new()), (1, Vec::new()), (2, Vec::new())]),
                ClientResourceState::empty(),
            );
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let remove = encode_control_entry_payload(&EngineControlPacket::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 2,
                reason: c4(b"left"),
                by_client: 0,
            },
        ))
        .test_value();
        host_transport
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Sync,
                data: remove,
            })
            .await
            .test_value();

        let before_execution = crate::AddressPacket {
            client_id: 2,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "198.51.100.22:11112".parse().test_value(),
            ),
        };
        host_transport
            .send_message(ControlMessage::Address(before_execution))
            .await
            .test_value();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Address(before_execution)
        );

        host_transport
            .send_message(ControlMessage::ExecSync { control_tick: 7 })
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::SyncScheduled {
                control_tick: 7,
                ..
            })
        ));

        host_transport
            .send_message(ControlMessage::Address(crate::AddressPacket {
                client_id: 2,
                address: crate::NetworkAddress::new(
                    crate::NetworkProtocol::Tcp,
                    "198.51.100.23:11112".parse().unwrap(),
                ),
            }))
            .await
            .test_value();
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err()
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn central_to_decentral_switch_fast_forwards_and_replays_own_controls() {
        // The final GO acknowledgement carries Game.Control.ControlTick, the
        // first unexecuted tick. SetCtrlMode then rebroadcasts contiguous own
        // control from that tick before resuming (pristine C++
        // src/C4Network2.cpp:2062-2110;
        // src/C4GameControlNetwork.cpp:360-374).
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.register(0).test_value();
        resource_state.control.register(1).test_value();
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop_with_state(4096, 8, 8, BTreeMap::new(), resource_state);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let live_tick = 137;

        let central_history = legacy_packet(BROADCAST_CLIENT_ID, live_tick - 1, 0x10);
        host_transport
            .send_message(ControlMessage::Control(central_history.clone()))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::Ready { packet })) if packet == central_history
        ));

        let local_packets = [
            legacy_packet(1, live_tick, 0x21),
            legacy_packet(1, live_tick + 1, 0x22),
            legacy_packet(1, live_tick + 3, 0x24),
        ];
        for packet in &local_packets {
            command_tx
                .send(ClientCommand::SubmitControl(packet.clone()))
                .await
                .test_value();
            if packet == &local_packets[0] {
                expect_control_wait_attribution_capability(&mut host_transport).await;
            }
            assert_eq!(
                timeout(EVENT_WAIT, host_transport.read_message())
                    .await
                    .unwrap()
                    .unwrap(),
                ControlMessage::Control(packet.clone())
            );
        }

        let decentral =
            NetworkStatus::new(NETWORK_STATE_GO, 0, i32::try_from(live_tick).test_value());
        host_transport
            .send_message(ControlMessage::StatusAck(decentral))
            .await
            .test_value();
        for packet in &local_packets[..2] {
            assert_eq!(
                timeout(EVENT_WAIT, host_transport.read_message())
                    .await
                    .unwrap()
                    .unwrap(),
                decentral_control_message(packet).unwrap()
            );
        }
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err(),
            "mode replay crossed the first missing control tick"
        );
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::StatusAck(status))) if status == decentral
        ));

        host_transport
            .send_message(ControlMessage::Control(legacy_packet(0, live_tick, 0x11)))
            .await
            .test_value();
        let aggregate = match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
            Some(ClientEvent::Ready { packet }) => packet,
            other => panic!("expected live decentralized aggregate, got {other:?}"),
        };
        assert_eq!(aggregate.tick(), live_tick);
        assert_eq!(control_commands(&aggregate), vec![0x11, 0x21]);

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[test]
    fn central_recovery_cursor_waits_for_the_first_missing_complete_tick() {
        let mut control = ClientControlState::central(42);
        control.set_status_target(NetworkStatus::new(NETWORK_STATE_GO, 1, 43));
        assert_eq!(control.recovery_tick(), Some(42));

        assert_eq!(
            control
                .accept_network(legacy_packet(BROADCAST_CLIENT_ID, 43, 0x31))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(control.recovery_tick(), Some(42));

        assert_eq!(
            control
                .accept_network(legacy_packet(BROADCAST_CLIENT_ID, 42, 0x30))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(control.expected_tick(), 44);
        assert_eq!(control.recovery_tick(), None);
    }

    #[test]
    fn client_recovery_target_exists_only_while_chasing_go_or_pause() {
        let mut control = ClientControlState::central(0);
        control.set_status_target(NetworkStatus::new(NETWORK_STATE_LOBBY, 1, 0));
        assert_eq!(control.recovery_tick(), None);

        control.set_status_target(NetworkStatus::new(NETWORK_STATE_PAUSE, 1, 0));
        assert_eq!(control.recovery_tick(), Some(0));
        control.clear_target();
        assert_eq!(control.recovery_tick(), None);
    }

    #[test]
    fn decentral_recovery_tick_is_derived_from_coordinator_missing_ranges() {
        let mut control = ClientControlState::central(0);
        control.register(0).test_value();
        control.register(1).test_value();
        control.change_mode(0, 0).test_value();
        control.set_status_target(NetworkStatus::new(NETWORK_STATE_GO, 0, 1));

        assert!(control
            .ingest_contribution(legacy_packet(0, 1, 0x11))
            .unwrap()
            .is_empty());
        assert!(control
            .coordinator
            .missing_ranges()
            .iter()
            .any(|range| range.from() == 0));
        assert_eq!(control.recovery_tick(), Some(0));
    }

    #[test]
    fn decentral_complete_recovery_advances_and_drains_future_ticks() {
        let mut control = ClientControlState::central(0);
        control.register(0).test_value();
        control.register(1).test_value();
        control.change_mode(0, 0).test_value();
        control.set_status_target(NetworkStatus::new(NETWORK_STATE_GO, 0, 1));
        assert!(control
            .ingest_contribution(legacy_packet(1, 0, 0x11))
            .unwrap()
            .is_empty());

        let future = legacy_packet(BROADCAST_CLIENT_ID, 1, 0x31);
        assert!(control.accept_network(future.clone()).unwrap().is_empty());
        let missing = legacy_packet(BROADCAST_CLIENT_ID, 0, 0x30);
        assert_eq!(
            control.accept_network(missing.clone()).unwrap(),
            vec![missing, future]
        );
        assert_eq!(control.expected_tick(), 2);
        assert_eq!(control.recovery_tick(), None);
    }

    #[test]
    fn active_client_recovery_waits_until_own_control_was_sent() {
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.register(1).test_value();
        resource_state
            .control
            .set_status_target(NetworkStatus::new(NETWORK_STATE_GO, 1, 0));
        let mut backlog = ControlBacklog::new(8);

        assert_eq!(
            eligible_client_recovery_tick(&resource_state, &backlog),
            None
        );
        backlog.record_packet(&legacy_packet(1, 0, 0x11));
        assert_eq!(
            eligible_client_recovery_tick(&resource_state, &backlog),
            Some(0)
        );
    }

    #[test]
    fn complete_replay_never_synthesizes_a_partial_tick() {
        let mut backlog = ControlBacklog::new(8);
        backlog.record_packet(&legacy_packet(1, 5, 0x11));
        let later_complete = legacy_packet(BROADCAST_CLIENT_ID, 6, 0x22);
        backlog.record_packet(&later_complete);
        assert!(contiguous_complete_controls(&backlog, 5)
            .unwrap()
            .is_empty());

        let complete = legacy_packet(BROADCAST_CLIENT_ID, 5, 0x33);
        backlog.record_packet(&complete);
        assert_eq!(
            contiguous_complete_controls(&backlog, 5).unwrap(),
            vec![complete, later_complete]
        );
    }

    #[test]
    fn central_mode_reentry_consumes_a_retained_future_complete_tick() {
        let mut control = ClientControlState::central(0);
        control.register(0).test_value();
        assert_eq!(
            control
                .accept_network(legacy_packet(BROADCAST_CLIENT_ID, 1, 0x21))
                .unwrap()
                .len(),
            1
        );
        control.change_mode(0, 0).test_value();
        assert_eq!(
            control
                .ingest_contribution(legacy_packet(0, 0, 0x11))
                .unwrap()
                .len(),
            1
        );
        control.change_mode(1, 1).test_value();

        assert_eq!(control.expected_tick(), 2);
        assert!(control
            .accept_network(legacy_packet(BROADCAST_CLIENT_ID, 1, 0x21))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn central_mode_switch_does_not_pack_buffered_partials() {
        let mut control = ClientControlState::central(0);
        control.register(0).test_value();
        control.change_mode(0, 0).test_value();
        assert!(control
            .ingest_contribution(legacy_packet(0, 1, 0x11))
            .unwrap()
            .is_empty());

        let (changed, ready) = control.change_mode(1, 1).test_value();
        assert!(changed);
        assert!(ready.is_empty());
        assert_eq!(control.expected_tick(), 1);

        let complete = legacy_packet(BROADCAST_CLIENT_ID, 1, 0x21);
        assert_eq!(
            control.accept_network(complete.clone()).unwrap(),
            vec![complete]
        );
        assert_eq!(control.expected_tick(), 2);
    }

    #[test]
    fn central_replay_does_not_repeat_a_locally_assembled_tick() {
        let mut control = ClientControlState::central(0);
        control.register(0).test_value();
        control.register(1).test_value();
        control.change_mode(0, 37).test_value();
        assert!(control
            .ingest_contribution(legacy_packet(0, 37, 0x11))
            .unwrap()
            .is_empty());
        assert_eq!(
            control
                .ingest_contribution(legacy_packet(1, 37, 0x21))
                .unwrap()
                .len(),
            1
        );
        control.change_mode(1, 37).test_value();

        assert!(control
            .accept_network(legacy_packet(BROADCAST_CLIENT_ID, 37, 0x31))
            .unwrap()
            .is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decentral_client_waits_for_every_active_contribution_before_ready() {
        // In CNM_Decentral every client broadcasts and stores its own
        // contribution, but CheckCompleteCtrl exposes only the locally packed
        // C4ClientIDAll packet after all active clients contributed. Packing is
        // in client-ID order (pristine C++ src/C4GameControlNetwork.cpp:156-179,
        // 679-718,741-783).
        let (host_stream, command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop(2048, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let decentral = NetworkStatus::new(NETWORK_STATE_GO, 0, 0);

        host_transport
            .send_message(ControlMessage::StatusAck(decentral))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::StatusAck(status))) if status == decentral
        ));

        for (client_id, name) in [(0, b"Host".as_slice()), (1, b"Local".as_slice())] {
            let name = c4(name);
            let join = EngineControlPacket::ClientJoin(clonk_engine::ClientJoinControlData {
                core: clonk_engine::ClientCoreControlData {
                    client_id,
                    activated: true,
                    observer: false,
                    name: name.clone(),
                    nick: name,
                    lobby_ready: false,
                },
                by_client: 0,
            });
            host_transport
                .send_message(ControlMessage::Packet {
                    delivery: ControlDelivery::Direct,
                    data: encode_control_entry_payload(&join).expect("encode client join"),
                })
                .await
                .test_value();
            assert!(matches!(
                timeout(EVENT_WAIT, event_rx.recv()).await,
                Ok(Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    ..
                }))
            ));
        }

        let host = legacy_packet(0, 0, 0x11);
        let local = legacy_packet(1, 0, 0x22);
        host_transport
            .send_message(ControlMessage::Control(host.clone()))
            .await
            .test_value();
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "one decentralized contribution must not execute"
        );

        command_tx
            .send(ClientCommand::SubmitControl(local.clone()))
            .await
            .test_value();
        let nested_packet = crate::transport::encode_complete_control_packet(&local).test_value();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .expect("local contribution send wait")
                .expect("read local contribution"),
            ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet,
            })
        );
        let aggregate = match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
            Some(ClientEvent::Ready { packet }) => packet,
            other => panic!("expected one aggregate ready event, got {other:?}"),
        };
        assert_eq!(aggregate.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&aggregate), vec![0x11, 0x22]);
        assert_eq!(
            aggregate
                .payload()
                .iter()
                .filter(|byte| **byte == 0xff)
                .count(),
            1,
            "the aggregate carries one C4Control list terminator"
        );

        for duplicate in [local, host] {
            host_transport
                .send_message(ControlMessage::Control(duplicate))
                .await
                .test_value();
        }
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "local echo and host retransmit must not execute the completed tick again"
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.test_value();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_emits_a_complete_tick_only_once_when_host_retransmits_it() {
        // A non-host in CNM_Central cannot pack per-client contributions and
        // waits for the host's C4ClientIDAll packet instead (pristine C++
        // src/C4GameControlNetwork.cpp:679-718,775-777).
        let (host_stream, _command_tx, mut event_rx, shutdown_tx, client_handle) =
            start_test_client_loop(512, 8, 8);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let central = NetworkStatus::new(NETWORK_STATE_GO, 1, 5);
        let complete = legacy_packet(BROADCAST_CLIENT_ID, 5, 0x44);

        host_transport
            .send_message(ControlMessage::StatusAck(central))
            .await
            .test_value();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::StatusAck(status))) if status == central
        ));
        host_transport
            .send_message(ControlMessage::Control(complete.clone()))
            .await
            .test_value();
        host_transport
            .send_message(ControlMessage::Control(complete.clone()))
            .await
            .test_value();

        match timeout(EVENT_WAIT, event_rx.recv()).await.test_value() {
            Some(ClientEvent::Ready { packet }) => assert_eq!(packet, complete),
            other => panic!("expected one ready event, got {other:?}"),
        }
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "a retransmitted complete packet must not execute twice"
        );

        shutdown_tx.send(()).ok();
        client_handle.await.test_value();
    }

    async fn submit_control_pair(
        host: &mut HostHandle,
        client: &ClientHandle,
        tick: Tick,
        host_command: i32,
        client_command: i32,
    ) {
        let host_packet = legacy_packet(0, tick, host_command);
        host.submit_local_control(host_packet).await.test_value();

        let client_packet = legacy_packet(client.client_id(), tick, client_command);
        client.submit_control(client_packet).await.test_value();
    }

    async fn activate_joined_client(
        host: &HostHandle,
        events: &mut mpsc::Receiver<HostEvent>,
        client_id: ClientId,
    ) {
        // Join assigns a deactivated client ID. C4Network2::ActivateClient
        // queues a host-authored CUT_Activate, and only execution of that
        // synchronized control changes active control-list membership
        // (src/C4Network2.cpp:1395-1406,1553-1571;
        // src/C4Control.cpp:578-606).
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::ClientJoined {
                    client_id: joined_id,
                    ..
                })) if joined_id == client_id => break,
                Ok(Some(HostEvent::TransportError {
                    client_id: Some(source),
                    ..
                })) if source != client_id => continue,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("transport error before client activation: {error}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before client join"),
                Err(_) => panic!("timed out waiting for client join"),
            }
        }

        let update = ClientUpdateControlData::new(
            CLIENT_UPDATE_ACTIVATE,
            i32::try_from(client_id).test_value(),
            1,
            i32::try_from(HOST_CLIENT_ID).test_value(),
        );
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&EngineControlPacket::ClientUpdate(update.clone()))
                .expect("encode activation control"),
        )
        .await
        .test_value();

        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::SyncScheduled { controls, .. }))
                    if controls == vec![EngineControlPacket::ClientUpdate(update.clone())] =>
                {
                    break;
                }
                Ok(Some(HostEvent::TransportError {
                    client_id: Some(source),
                    ..
                })) if source != client_id => continue,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("transport error while activating client: {error}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before client activation"),
                Err(_) => panic!("timed out waiting for client activation"),
            }
        }
    }

    fn legacy_packet(client_id: ClientId, tick: Tick, command: i32) -> ControlPacket {
        encode_control_packet(&legacy_frame(
            client_id,
            tick,
            vec![EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(client_id).unwrap_or(i32::MAX),
                command,
                data: command,
                by_client: i32::try_from(client_id).unwrap_or(i32::MAX),
            })],
        ))
        .test_value()
    }

    fn assert_no_queued_client_status(events: &mut mpsc::Receiver<ClientEvent>) {
        while let Ok(event) = events.try_recv() {
            match event {
                ClientEvent::Status(status) => {
                    panic!("received chase-target status before its deadline: {status:?}")
                }
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected while checking chase-target status: {reason:?}")
                }
                _ => {}
            }
        }
    }

    async fn wait_for_client_status(events: &mut mpsc::Receiver<ClientEvent>) -> NetworkStatus {
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(ClientEvent::Status(status))) => return status,
                Ok(Some(ClientEvent::Disconnected { reason })) => {
                    panic!("client disconnected before chase-target status: {reason:?}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("client event stream ended before chase-target status"),
                Err(_) => panic!("timed out waiting for chase-target status"),
            }
        }
    }

    async fn wait_for_client_status_ack(
        events: &mut mpsc::Receiver<ClientEvent>,
        expected: NetworkStatus,
    ) {
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(ClientEvent::StatusAck(status))) if status == expected => return,
                Ok(Some(ClientEvent::Disconnected { reason })) => {
                    panic!("client disconnected before status acknowledgement: {reason:?}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("client event stream ended before status acknowledgement"),
                Err(_) => panic!("timed out waiting for status acknowledgement"),
            }
        }
    }

    async fn wait_for_host_ready_tick(events: &mut mpsc::Receiver<HostEvent>, tick: Tick) {
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::Ready { packet })) if packet.tick() == tick => return,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("host transport failed before control tick {tick}: {error}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before control tick {tick}"),
                Err(_) => panic!("timed out waiting for host control tick {tick}"),
            }
        }
    }

    async fn assert_no_client_status_through_ready_check(
        events: &mut mpsc::Receiver<ClientEvent>,
        barrier: ReadyCheckPacket,
    ) {
        tokio::time::resume();
        let outcome = timeout(Duration::from_secs(1), async {
            loop {
                match events.recv().await {
                    Some(ClientEvent::ReadyCheck { packet }) if packet == barrier => return Ok(()),
                    Some(ClientEvent::Status(status)) => {
                        return Err(format!(
                            "non-chasing client received chase-target status: {status:?}"
                        ));
                    }
                    Some(ClientEvent::Disconnected { reason }) => {
                        return Err(format!(
                            "client disconnected before ready-check barrier: {reason:?}"
                        ));
                    }
                    Some(_) => continue,
                    None => {
                        return Err(
                            "client event stream ended before ready-check barrier".to_string()
                        );
                    }
                }
            }
        })
        .await;
        tokio::time::pause();

        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => panic!("{error}"),
            Err(_) => panic!("timed out waiting for ready-check barrier"),
        }
    }

    async fn raw_client_transport(
        address: SocketAddr,
        name: &[u8],
    ) -> (crate::ControlTransport<TcpStream>, ClientId) {
        let stream = TcpStream::connect(address).await.test_value();
        let mut transport = crate::ControlTransport::new(stream);
        let name = c4(name);
        let request = test_connection_request(test_client_core(-1, name, false), 0, false);
        let handshake = run_client_connection_handshake(&mut transport, request)
            .await
            .test_value();
        let client_id = ClientId::try_from(handshake.join_data.client_id).test_value();
        (transport, client_id)
    }

    async fn raw_existing_client_transport(
        address: SocketAddr,
        client_id: ClientId,
        remote_connection_id: u32,
        name: &[u8],
    ) -> crate::ControlTransport<TcpStream> {
        let stream = TcpStream::connect(address).await.test_value();
        let mut transport = crate::ControlTransport::new(stream);
        assert!(matches!(
            transport.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = c4(name);
        transport
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                test_client_core(i32::try_from(client_id).unwrap(), name, false),
                remote_connection_id,
                false,
            )))
            .await
            .test_value();
        loop {
            match transport.read_message().await.test_value() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected positive host connection reply, got {other:?}"),
            }
        }
        transport
            .send_message(ControlMessage::ConnectionReply(test_connection_reply(
                true,
                c4(b"connection accepted"),
                false,
            )))
            .await
            .test_value();
        transport
    }

    async fn request_route<S>(
        transport: &mut crate::ControlTransport<S>,
        client_id: i32,
        remote_connection_id: u32,
    ) -> crate::ConnectionReply
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        loop {
            match await_test(transport.read_message()).await {
                ControlMessage::ConnectionRequest(_) => break,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected host connection request, got {other:?}"),
            }
        }
        let name = c4(b"Alice");
        transport
            .send_message(ControlMessage::ConnectionRequest(test_connection_request(
                clonk_engine::ClientCoreControlData {
                    client_id,
                    activated: true,
                    observer: false,
                    name: name.clone(),
                    nick: name,
                    lobby_ready: true,
                },
                remote_connection_id,
                false,
            )))
            .await
            .test_value();
        loop {
            match await_test(transport.read_message()).await {
                ControlMessage::ConnectionReply(reply) => return reply,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .test_value();
                }
                other => panic!("expected host connection reply, got {other:?}"),
            }
        }
    }

    async fn drain_raw_client(transport: &mut crate::ControlTransport<TcpStream>) {
        while matches!(
            timeout(Duration::from_millis(20), transport.read_message()).await,
            Ok(Ok(_))
        ) {}
    }

    async fn raw_client_ping_barrier(transport: &mut crate::ControlTransport<TcpStream>) {
        let ping = crate::PingPacket {
            sent_at: 0x1020_3040,
            packet_counter: 0,
        };
        transport
            .send_message(ControlMessage::Ping(ping))
            .await
            .test_value();
        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(deadline, transport.read_message()).await {
                Ok(Ok(ControlMessage::Pong(received))) if received == ping => return,
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => panic!("ping barrier failed: {error}"),
                Err(_) => panic!("timed out waiting for ping barrier"),
            }
        }
    }

    async fn raw_client_received_message(
        transport: &mut crate::ControlTransport<TcpStream>,
        expected: &ControlMessage,
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        while let Ok(Ok(message)) = timeout_at(deadline, transport.read_message()).await {
            if &message == expected {
                return true;
            }
        }
        false
    }

    async fn raw_tcp_received_frame(
        stream: &mut TcpStream,
        expected: &[u8],
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            let mut header = [0_u8; 5];
            if !matches!(
                timeout_at(deadline, stream.read_exact(&mut header)).await,
                Ok(Ok(_))
            ) {
                return false;
            }
            assert_eq!(header[0], 0xff, "invalid TCP packet frame prefix");
            let size = u32::from_ne_bytes(header[1..].try_into().test_value()) as usize;
            let mut body = vec![0; size];
            if !matches!(
                timeout_at(deadline, stream.read_exact(&mut body)).await,
                Ok(Ok(_))
            ) {
                return false;
            }
            if body == expected {
                return true;
            }
        }
    }

    async fn acknowledge_raw_status(
        transport: &mut crate::ControlTransport<TcpStream>,
        events: &mut mpsc::Receiver<HostEvent>,
        client_id: ClientId,
        status: NetworkStatus,
    ) {
        transport
            .send_message(ControlMessage::StatusAck(status))
            .await
            .test_value();
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::StatusAck {
                    client_id: source,
                    status: received,
                })) if source == client_id && received == status => break,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before raw status acknowledgement"),
                Err(_) => panic!("timed out waiting for raw status acknowledgement"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, transport.read_message()).await {
                Ok(Ok(ControlMessage::StatusAck(received))) if received == status => break,
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => panic!("raw client failed while waiting for StatusAck: {error}"),
                Err(_) => panic!("timed out waiting for host StatusAck"),
            }
        }
    }

    async fn raw_client_received_control(
        transport: &mut crate::ControlTransport<TcpStream>,
        expected: &ControlPacket,
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        while let Ok(Ok(message)) = timeout_at(deadline, transport.read_message()).await {
            if message == ControlMessage::Control(expected.clone()) {
                return true;
            }
        }
        false
    }

    async fn raw_client_received_forward(
        transport: &mut crate::ControlTransport<TcpStream>,
        expected: &crate::ForwardPacket,
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        while let Ok(Ok(message)) = timeout_at(deadline, transport.read_message()).await {
            if message == ControlMessage::Forward(expected.clone()) {
                return true;
            }
        }
        false
    }

    async fn wait_for_host_error(
        events: &mut mpsc::Receiver<HostEvent>,
        source: ClientId,
    ) -> String {
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                })) if client_id == source => return error,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before forwarding error"),
                Err(_) => panic!("timed out waiting for forwarding error"),
            }
        }
    }

    fn control_commands(packet: &ControlPacket) -> Vec<i32> {
        decode_control_packet(packet)
            .test_value()
            .controls
            .into_iter()
            .map(|control| match control {
                EngineControlPacket::PlayerControl(control) => control.command,
                other => panic!("expected player control, got {other:?}"),
            })
            .collect()
    }

    async fn wait_for_host_ready(
        events: &mut mpsc::Receiver<HostEvent>,
        duration: Duration,
    ) -> ControlPacket {
        loop {
            match timeout(duration, events.recv()).await {
                Ok(Some(HostEvent::Ready { packet })) => break packet,
                Ok(Some(HostEvent::ClientJoined { .. })) => continue,
                // A departing client's closing socket can surface a transient
                // transport error; tolerate it like ClientLeft. A real failure
                // still trips the timeout because Ready never arrives.
                Ok(Some(HostEvent::ClientLeft { .. }))
                | Ok(Some(HostEvent::ClientConnectionFailed { .. }))
                | Ok(Some(HostEvent::RecoverableRouteDiagnostic { .. }))
                | Ok(Some(HostEvent::UnassociatedConnectionFailed { .. }))
                | Ok(Some(HostEvent::UnhandledPacket { .. }))
                | Ok(Some(HostEvent::TransportError { .. })) => continue,
                Ok(Some(HostEvent::Direct { .. }))
                | Ok(Some(HostEvent::JoinDataNeeded { .. }))
                | Ok(Some(HostEvent::ExecSync { .. }))
                | Ok(Some(HostEvent::ActivationRequest { .. }))
                | Ok(Some(HostEvent::PlayerInfoUpdate { .. }))
                | Ok(Some(HostEvent::LobbyCountdown { .. }))
                | Ok(Some(HostEvent::ReadyCheck { .. }))
                | Ok(Some(HostEvent::ResourceAction(_)))
                | Ok(Some(HostEvent::ResourceProgress { .. }))
                | Ok(Some(HostEvent::ResourceComplete { .. }))
                | Ok(Some(HostEvent::ResourceLoadFailed { .. }))
                | Ok(Some(HostEvent::ResourceDeriveUnsupported { .. }))
                | Ok(Some(HostEvent::RoundRestarted))
                | Ok(Some(HostEvent::StatusAck { .. }))
                | Ok(Some(HostEvent::StatusChanged(_)))
                | Ok(Some(HostEvent::SyncScheduled { .. }))
                | Ok(Some(HostEvent::LocalAddressesChanged { .. }))
                | Ok(Some(HostEvent::NetpuncherStateChanged { .. }))
                | Ok(Some(HostEvent::StatusCommitted(_))) => continue,
                Ok(Some(HostEvent::FatalError { error })) => {
                    panic!("host became fatal while waiting for ready tick: {error}")
                }
                Ok(None) => panic!("host event stream ended unexpectedly"),
                Err(_) => panic!("timed out waiting for host ready event"),
            }
        }
    }

    async fn wait_for_client_ready(
        events: &mut mpsc::Receiver<ClientEvent>,
        duration: Duration,
    ) -> ControlPacket {
        loop {
            match timeout(duration, events.recv()).await {
                Ok(Some(ClientEvent::Ready { packet })) => break packet,
                Ok(Some(ClientEvent::PingMeasured { .. })) => continue,
                Ok(Some(ClientEvent::LocalAddressesChanged { .. })) => continue,
                Ok(Some(ClientEvent::ExecSync { .. })) => continue,
                Ok(Some(ClientEvent::Direct { .. })) => continue,
                Ok(Some(ClientEvent::Status(_))) | Ok(Some(ClientEvent::StatusAck(_))) => continue,
                Ok(Some(ClientEvent::LobbyCountdown { .. })) => continue,
                Ok(Some(ClientEvent::ReadyCheck { .. })) => continue,
                Ok(Some(ClientEvent::ResourceAction(_))) => continue,
                Ok(Some(ClientEvent::ResourceProgress { .. }))
                | Ok(Some(ClientEvent::ResourceComplete { .. }))
                | Ok(Some(ClientEvent::ResourceLoadFailed { .. }))
                | Ok(Some(ClientEvent::ResourceDeriveUnsupported { .. })) => continue,
                Ok(Some(ClientEvent::LeagueRoundResults { .. })) => continue,
                Ok(Some(ClientEvent::HostRestarting { .. })) => continue,
                Ok(Some(ClientEvent::HostRestartLobby)) => continue,
                Ok(Some(ClientEvent::JoinData { .. })) => continue,
                Ok(Some(ClientEvent::UnhandledPacket { .. })) => continue,
                Ok(Some(ClientEvent::SyncScheduled { .. })) => continue,
                Ok(Some(ClientEvent::Disconnected { reason })) => {
                    panic!("client disconnected during test: {:?}", reason);
                }
                Ok(None) => panic!("client event stream ended unexpectedly"),
                Err(_) => panic!("timed out waiting for client ready event"),
            }
        }
    }

    async fn wait_for_client_departure(events: &mut mpsc::Receiver<HostEvent>, duration: Duration) {
        loop {
            match timeout(duration, events.recv()).await {
                Ok(Some(HostEvent::ClientLeft { .. })) => break,
                Ok(Some(HostEvent::TransportError { .. })) => break,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended unexpectedly"),
                Err(_) => panic!("timed out waiting for client departure"),
            }
        }
    }
}
