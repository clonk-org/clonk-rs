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
    use clonk_resources::{c4group_file_crc, MutableGroup};
    use std::fs;
    use std::future::{pending, ready};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt, ReadBuf};
    use tokio::net::UdpSocket;
    use tokio::time::{timeout, timeout_at};

    const CPP_COMPATIBILITY_BUILD: i32 = CURRENT_GAME_BUILD + 2;

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

    fn compatibility_test_core(client_id: i32, name: &[u8]) -> clonk_engine::ClientCoreControlData {
        let name = clonk_engine::LegacyCString::from_bytes(name.to_vec()).unwrap();
        clonk_engine::ClientCoreControlData {
            client_id,
            activated: true,
            observer: false,
            name: name.clone(),
            nick: name,
            lobby_ready: false,
        }
    }

    async fn read_compatibility_request<S>(stream: S) -> crate::ConnectionRequest
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut transport = crate::ControlTransport::new(stream);
        match transport.read_message().await.unwrap() {
            ControlMessage::ConnectionRequest(request) => request,
            other => panic!("expected client connection request, got {other:?}"),
        }
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

    fn tcp_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0xff];
        frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    async fn write_udp_session_payload(stream: &mut crate::ReliableUdpPeerStream, payload: &[u8]) {
        stream.write_all(&tcp_frame(payload)).await.unwrap();
        stream.flush().await.unwrap();
    }

    async fn read_udp_session_payload(stream: &mut crate::ReliableUdpPeerStream) -> Vec<u8> {
        let mut header = [0_u8; 5];
        stream.read_exact(&mut header).await.unwrap();
        assert_eq!(header[0], 0xff);
        let length = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).await.unwrap();
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
                self.released.lock().unwrap().push(self.requests);
            })
        }
    }

    impl crate::upnp::PortMappingBackend for RecordingPortMappingBackend {
        fn start(
            &self,
            requests: &[crate::upnp::PortMappingRequest],
        ) -> Box<dyn crate::upnp::ActivePortMappings> {
            self.started.lock().unwrap().push(requests.to_vec());
            Box::new(RecordingActivePortMappings {
                requests: requests.to_vec(),
                released: Arc::clone(&self.released),
            })
        }
    }

    #[test]
    fn upnp_mapping_requests_require_enablement_and_live_bound_transports() {
        let mut config = HostConfig {
            enable_upnp: true,
            configured_tcp_port: Some(31_112),
            configured_udp_port: Some(31_113),
            ..HostConfig::default()
        };
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
    fn l075_configured_zero_udp_port_disables_the_udp_binding() {
        let binding = HostUdpBinding::bind(&HostConfig {
            udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
            netpuncher_addresses: vec![SocketAddr::from(([127, 0, 0, 1], 11_115))],
            configured_udp_port: Some(0),
            ..HostConfig::default()
        });

        assert_eq!(binding.local_addr(), None);
        assert_eq!(binding.bind_error(), None);
    }

    #[tokio::test]
    async fn enabled_upnp_host_requests_tcp_udp_and_releases_on_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = HostConfig {
            enable_upnp: true,
            udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
            configured_tcp_port: Some(31_112),
            configured_udp_port: Some(31_113),
            ..HostConfig::default()
        };
        let udp_binding = HostUdpBinding::bind(&config);
        assert!(udp_binding.local_addr().is_some());
        let backend = RecordingPortMappingBackend::default();
        let host =
            start_host_with_udp_binding_and_backend(Some(listener), config, udp_binding, &backend)
                .await
                .unwrap();
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

        host.shutdown().await.unwrap();
        assert_eq!(&*backend.released.lock().unwrap(), &[expected]);
    }

    #[tokio::test]
    async fn host_session_requests_puncher_id_punches_and_reports_assigned_state() {
        let mut puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let puncher_address = puncher.local_addr();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                netpuncher_addresses: vec![puncher_address],
                configured_tcp_port: Some(31_112),
                configured_udp_port: Some(31_113),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let host_udp_address = host.udp_local_addr().unwrap();
        let mut puncher_stream = timeout(Duration::from_secs(2), puncher.accept())
            .await
            .unwrap()
            .unwrap();
        let observed_address = puncher_stream.peer_addr();

        let request = timeout(
            Duration::from_secs(2),
            read_udp_session_payload(&mut puncher_stream),
        )
        .await
        .unwrap();
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
        .unwrap();
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
        .unwrap();
        assert_eq!(game_ids.ipv4, assigned_id);
        assert_eq!(game_ids.ipv6, 0);
        assert_eq!(assigned_addresses, local_addresses);

        let target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
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
            .unwrap();
        assert_eq!(
            crate::canonical_reliable_udp_peer_address(source),
            host_udp_address
        );
        assert_eq!(length, 9);
        assert_eq!(raw_punch[0], 0x01);

        host.shutdown().await.unwrap();
        puncher.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn l082_live_host_initializes_one_netpuncher_per_resolved_family() {
        let mut ipv4_puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let mut ipv6_puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 0)))
                .unwrap();
        let ipv4_address = ipv4_puncher.local_addr();
        let ipv6_address = ipv6_puncher.local_addr();
        let ignored_same_family = SocketAddr::from(([127, 0, 0, 2], ipv4_address.port()));
        let listener = TcpListener::bind(SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], 0)))
            .await
            .unwrap();
        let host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([0_u16; 8], 0))),
                netpuncher_addresses: Vec::new(),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();

        host.init_netpunchers(vec![ipv4_address, ignored_same_family, ipv6_address])
            .await
            .unwrap();
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
                .unwrap();
            assert_eq!(
                crate::decode_netpuncher_packet(&payload).unwrap(),
                NetpuncherPacket::IdRequest
            );
        }

        host.shutdown().await.unwrap();
        ipv4_puncher.shutdown().await.unwrap();
        ipv6_puncher.shutdown().await.unwrap();
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
                    },
                },
            )
            .is_none());
        receiver
    }

    fn host_state_with_test_route(client_id: ClientId, outbound: HostOutboundSender) -> HostState {
        let config = HostConfig::default();
        let backlog_limit = config.backlog_limit;
        let mut coordinator = ControlCoordinator::with_start_tick(backlog_limit, config.start_tick);
        coordinator.register_client(HOST_CLIENT_ID).unwrap();
        let (event_tx, _event_rx) = mpsc::channel(1);
        let resource_resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &crate::ClientBootstrapLocalCandidates::default(),
            PathBuf::from("Network"),
        );
        let peer_core = compatibility_test_core(client_id as i32, b"Peer");

        HostState {
            coordinator,
            backlog: ControlBacklog::new(backlog_limit),
            client_performance: ClientPerformanceStats::new(backlog_limit),
            local_control_backlog: ControlBacklog::new(backlog_limit),
            scheduler: ResyncScheduler::new(config.resync_cooldown),
            clients: BTreeMap::from([(
                client_id,
                ClientConnection {
                    outbound: outbound.clone(),
                    core: peer_core.clone(),
                    peer_addr: "127.0.0.1:11112".parse().unwrap(),
                    join_data_sent: true,
                    join_data_needed_emitted: false,
                },
            )]),
            accepted_routes: BTreeMap::from([(
                1,
                AcceptedConnectionRoute {
                    client_id,
                    remote_connection_id: 2,
                    peer_addr: "127.0.0.1:11112".parse().unwrap(),
                    protocol: crate::NetworkProtocol::Tcp,
                    ping: RoutePingLag::default(),
                    outbound,
                },
            )]),
            control_send_time_epoch: 0,
            closed_routes: crate::post_mortem::ClosedConnectionRouter::default(),
            pending_sync: Vec::new(),
            status_barrier: StatusBarrier::stable(config.initial_status),
            last_chase_target_update: None,
            game_started: false,
            control_mode: config.initial_status.control_mode,
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
            "127.0.0.1:11113".parse().unwrap(),
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

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
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
            "127.0.0.1:11114".parse().unwrap(),
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
            .expect("first message route")
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
                peer_addr: "127.0.0.1:11113".parse().unwrap(),
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
                peer_addr: "127.0.0.1:11113".parse().unwrap(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: second_ping,
                outbound: second_outbound,
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
                peer_addr: "127.0.0.1:11112".parse().unwrap(),
                protocol: crate::NetworkProtocol::Udp,
                ping: RoutePingLag::default(),
                outbound: first_udp,
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
                peer_addr: "127.0.0.1:11113".parse().unwrap(),
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
                peer_addr: "127.0.0.1:11113".parse().unwrap(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: RoutePingLag::default(),
                outbound: second_tcp,
            },
        );
        let message = ControlMessage::Status(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 9,
        });

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
        routes.routes.get_mut(&1).unwrap().ping.record_pong(900);
        let _host_udp =
            add_test_route_queue(&mut routes, 2, HOST_CLIENT_ID, crate::NetworkProtocol::Udp);
        routes.routes.get_mut(&2).unwrap().ping.record_pong(40);
        let _peer = add_test_route_queue(&mut routes, 3, 7, crate::NetworkProtocol::Tcp);
        routes.routes.get_mut(&3).unwrap().ping.record_pong(100);

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
            .unwrap();
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
        let message = ControlMessage::Status(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 9,
        });

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
        let unencodable_tick = u32::try_from(i32::MAX).unwrap() + 1;
        state.coordinator =
            ControlCoordinator::with_start_tick(state.config.backlog_limit, unencodable_tick);
        state
            .coordinator
            .register_client(HOST_CLIENT_ID)
            .expect("register host at fatal test tick");
        let payload = legacy_packet(HOST_CLIENT_ID, 0, 0x71).payload().to_vec();

        ingest_control(
            ControlPacket::builder(HOST_CLIENT_ID, unencodable_tick).payload(payload),
            ControlIngress::Local,
            &mut state,
        )
        .await;

        match timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("fatal ready-tick event timeout")
        {
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

        match timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("fatal local-coordinator event timeout")
        {
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
            .insert(99, i32::try_from(client_id).unwrap());

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
        let delivered = ControlMessage::Status(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 9,
        });
        let live_core = compatibility_test_core(live_client_id as i32, b"Live");
        state.clients.insert(
            live_client_id,
            ClientConnection {
                outbound: live.clone(),
                core: live_core.clone(),
                peer_addr: "127.0.0.1:11113".parse().unwrap(),
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
                peer_addr: "127.0.0.1:11113".parse().unwrap(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: RoutePingLag::default(),
                outbound: live,
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
        let delivered = ControlMessage::Status(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 9,
        });
        routes.try_send_to(8, delivered.clone()).unwrap();
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
            match receiver.try_recv().expect("recovery request is queued") {
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
            .unwrap();
        assert_eq!(
            take_message(&mut host),
            ControlMessage::Request { from_tick: 17 }
        );
        assert!(peer.try_recv().is_err());

        send_client_recovery_request(&mut routes, 0, 18)
            .await
            .unwrap();
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
        match receiver
            .try_recv()
            .expect("resource command is queued on the selected route")
        {
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
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let puncher_address = puncher.local_addr();
        let config = ClientConfig::new("Client", ParticipantKind::Player)
            .with_mesh_udp_bind_address(SocketAddr::from(([127, 0, 0, 1], 0)))
            .with_mesh_punchers([ClientMeshPuncherConfig {
                address: puncher_address,
                game_id: 0x1122_3344,
            }]);

        let mut prepared = prepare_client_mesh(&config, false).await.unwrap();
        let local_address = prepared.udp_hub.as_ref().unwrap().local_addr();
        let mut puncher_stream = timeout(EVENT_WAIT, puncher.accept())
            .await
            .unwrap()
            .unwrap();
        let mut header = [0_u8; 5];
        timeout(EVENT_WAIT, puncher_stream.read_exact(&mut header))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(header[0], 0xff);
        let payload_len = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
        let mut payload = vec![0_u8; payload_len];
        puncher_stream.read_exact(&mut payload).await.unwrap();
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
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let handle = prepared.udp_hub.as_ref().unwrap().handle();
        let (game_stream, incoming_game_stream) =
            tokio::join!(handle.connect(game_peer.local_addr()), game_peer.accept());
        let game_stream = game_stream.unwrap();
        let incoming_game_stream = incoming_game_stream.unwrap();
        assert_eq!(incoming_game_stream.peer_addr(), local_address);
        drop(game_stream);
        drop(incoming_game_stream);

        prepared
            .puncher_init
            .lock()
            .expect("puncher initialization lock")
            .initializing = false;
        handle.close_puncher(puncher_address).await.unwrap();
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
            .unwrap();
        let mut reconnected = timeout(EVENT_WAIT, puncher.accept())
            .await
            .unwrap()
            .unwrap();
        assert!(timeout(
            Duration::from_millis(100),
            read_udp_session_payload(&mut reconnected),
        )
        .await
        .is_err());
        drop(reconnected);

        if let Some(hub) = prepared.udp_hub.take() {
            hub.shutdown().await.unwrap();
        }
        game_peer.shutdown().await.unwrap();
        puncher.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_unwraps_a_selected_forwarded_control() {
        // PID_Fwd dispatches its complete nested packet exactly once when the
        // local client matches the positive list (pristine C++
        // src/C4Network2IO.cpp:1026-1033).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.change_mode(0, 0).unwrap();
        resource_state.control.register(0).unwrap();
        resource_state.control.register(1).unwrap();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let local = legacy_packet(1, 0, 0x22);
        command_tx
            .send(ClientCommand::SubmitControl(local))
            .await
            .unwrap();
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
            .unwrap();

        let ready = match timeout(EVENT_WAIT, event_rx.recv()).await.unwrap() {
            Some(ClientEvent::Ready { packet }) => packet,
            other => panic!("expected forwarded aggregate, got {other:?}"),
        };
        assert_eq!(control_commands(&ready), vec![0x11, 0x22]);

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_selected_forwarded_league_results() {
        // PID_Fwd recursively enters HandlePacket. League results selected for
        // this client retain their typed host-only route and do not close the
        // connection (src/C4Network2IO.cpp:1037-1045;
        // src/C4Network2Players.cpp:392-419).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let league_results = vec![0x17, 0x01, b'O', b'K', 0x00, 0x00];

        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: league_results,
            }))
            .await
            .unwrap();
        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .unwrap()
            .expect("client event channel remains open");
        let ClientEvent::LeagueRoundResults { packet } = event else {
            panic!("expected typed forwarded league results, got {event:?}");
        };
        assert_eq!(
            packet,
            crate::LeagueRoundResultsPacket {
                success: true,
                result_string: clonk_engine::LegacyCString::from_bytes(b"OK".to_vec()).unwrap(),
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
            .unwrap();
        assert_eq!(
            host_transport.read_message().await.unwrap(),
            ControlMessage::Pong(ping)
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ignores_unselected_malformed_forward_and_bounds_recursion() {
        // DoFwdTo is evaluated before the nested packet is unpacked. A selected
        // recursive PID_Fwd is bounded instead of reproducing C++'s unbounded
        // recursive HandlePacket call (pristine C++
        // src/C4Network2IO.cpp:1026-1033,1626-1636).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));

        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![2],
                nested_packet: vec![0x40],
            }))
            .await
            .unwrap();
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
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
            .unwrap(),
        );
        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: recursive,
            }))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::Disconnected { reason: Some(reason) }))
                if reason == "recursive forwarding packet is not accepted"
        ));
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decentral_client_sends_cpp_forward_request_for_local_control() {
        // BroadcastMsgToClients excludes the directly connected host, records
        // no other direct peers in the negative list, and sends the complete
        // PID_Control inside PID_FwdReq (pristine C++
        // src/C4Network2Client.cpp:515-541; src/C4GameControlNetwork.cpp:156-174).
        let (client_stream, mut host_stream) = duplex(128);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.change_mode(0, 0).unwrap();
        resource_state.control.register(0).unwrap();
        resource_state.control.register(1).unwrap();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));

        command_tx
            .send(ClientCommand::SubmitControl(
                ControlPacket::builder(1, 0).payload(vec![0xff]),
            ))
            .await
            .unwrap();
        let mut bytes = vec![0; 64];
        let count = timeout(EVENT_WAIT, host_stream.read(&mut bytes))
            .await
            .expect("forward request send wait")
            .unwrap();
        bytes.truncate(count);
        assert_eq!(
            bytes,
            [0xff, 0x08, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x04, 0x40, 0x01, 0x00, 0xff,]
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_direct_and_private_packets_send_only_forward_request() {
        // CDT_Direct and CDT_Private exclude the host from the direct leg.
        // With no peer mesh, only the host FwdReq remains and its negative
        // list is empty (pristine C++ src/C4Network2Client.cpp:515-541;
        // src/C4GameControlNetwork.cpp:224-240).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        for delivery in [ControlDelivery::Direct, ControlDelivery::Private] {
            command_tx
                .send(ClientCommand::SubmitPacket {
                    delivery,
                    data: vec![0xaa, 0xbb],
                })
                .await
                .unwrap();
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
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ready_check_sends_raw_then_host_excluding_forward_request() {
        // ReadyCheck uses includeHost=true: the host receives the raw packet
        // first and is then excluded from the fallback FwdReq (pristine C++
        // src/C4Network2Client.cpp:515-541; src/C4GameLobby.cpp:329-343).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.host_peer_id = 7;
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let packet = ReadyCheckPacket {
            client_id: 12,
            data: crate::ReadyCheckData::Ready,
        };

        command_tx
            .send(ClientCommand::SubmitReadyCheck(packet))
            .await
            .unwrap();
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
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rust_client_direct_packet_reaches_rust_host_and_observer_once() {
        // The client now reaches the host only through FwdReq for direct
        // packets. Preserve Rust-host interoperability while the generic
        // opaque forwarding router remains separate work.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let source = connect_client(
            address,
            ClientConfig::new("Source", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut observer_a = connect_client(
            address,
            ClientConfig::new("Observer A", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut observer_a_events = observer_a.take_event_receiver();
        let mut observer_b = connect_client(
            address,
            ClientConfig::new("Observer B", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut observer_b_events = observer_b.take_event_receiver();
        let source_id = source.client_id();
        let data =
            encode_control_entry_payload(&EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(source_id).unwrap(),
                command: 0x22,
                data: 0x33,
                by_client: i32::try_from(source_id).unwrap(),
            }))
            .unwrap();

        source
            .submit_packet(ControlDelivery::Direct, data.clone())
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
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

        source.shutdown().await.unwrap();
        observer_a.shutdown().await.unwrap();
        observer_b.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_ignores_client_originated_league_results_and_keeps_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let (mut client, _) = raw_client_transport(address, b"Source").await;
        drain_raw_client(&mut client).await;

        client
            .send_message(ControlMessage::LeagueRoundResults(
                crate::LeagueRoundResultsPacket {
                    success: true,
                    result_string: clonk_engine::LegacyCString::from_bytes(b"OK".to_vec()).unwrap(),
                    players: Vec::new(),
                },
            ))
            .await
            .unwrap();
        let ping = crate::PingPacket {
            sent_at: 31,
            packet_counter: 0,
        };
        client
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();

        assert_eq!(
            timeout(EVENT_WAIT, client.read_message())
                .await
                .expect("host kept the accepted connection responsive")
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        drop(client);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_broadcasts_league_round_results_to_every_logical_client() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let (mut alice, _) = raw_client_transport(address, b"Alice").await;
        let (mut bob, _) = raw_client_transport(address, b"Bob").await;
        drain_raw_client(&mut alice).await;
        drain_raw_client(&mut bob).await;
        let packet = crate::LeagueRoundResultsPacket {
            success: true,
            result_string: clonk_engine::LegacyCString::from_bytes(b"Counted".to_vec()).unwrap(),
            players: vec![crate::LeagueRoundResultsPlayer {
                player_info_id: 17,
                total_playing_time: 900,
                settlement_score_old: 2,
                settlement_score_new: 4,
                league_score_new: 120,
                league_score_gain: 7,
                league_rank_new: 3,
                league_rank_symbol_new: 2,
                league_progress_data: clonk_engine::LegacyCString::from_bytes(b"p=2".to_vec())
                    .unwrap(),
                status: crate::LeagueRoundPlayerStatus::Won,
            }],
        };

        host.broadcast_league_round_results(packet.clone())
            .await
            .unwrap();

        let expected = ControlMessage::LeagueRoundResults(packet);
        assert!(raw_client_received_message(&mut alice, &expected, EVENT_WAIT).await);
        assert!(raw_client_received_message(&mut bob, &expected, EVENT_WAIT).await);

        drop(alice);
        drop(bob);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_shutdown_does_not_report_a_client_connection_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (client, client_id) = raw_client_transport(address, b"Alice").await;
        while host_events.try_recv().is_ok() {}

        host.shutdown().await.unwrap();

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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
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
            .unwrap();
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_control_request_falls_back_to_unregistered_partial_packet() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        drain_raw_client(&mut source).await;

        let partial = legacy_packet(source_id, 0, 0x22);
        source
            .send_message(ControlMessage::Control(partial.clone()))
            .await
            .unwrap();
        raw_client_ping_barrier(&mut source).await;
        source
            .send_message(ControlMessage::Request { from_tick: 0 })
            .await
            .unwrap();

        assert!(raw_client_received_control(&mut source, &partial, EVENT_WAIT).await);
        assert_ne!(partial.client_id(), BROADCAST_CLIENT_ID);

        drop(source);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_routes_forward_request_without_echoing_its_origin() {
        // HandleFwdReq excludes the requester from remote targets, sends the
        // nested packet directly when at most two remote clients remain, then
        // dispatches it locally when the negative list selects the host
        // (pristine C++ src/C4Network2IO.cpp:1066-1117).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        activate_joined_client(&host, &mut host_events, source_id).await;
        let (mut observer, _) = raw_client_transport(address, b"Observer").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer).await;

        let host_packet = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let source_packet = legacy_packet(source_id, 0, 0x22);
        host.submit_local_control(host_packet).await.unwrap();
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&source_packet)
                    .unwrap(),
            }))
            .await
            .unwrap();

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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_routes_forwarded_direct_control_and_checks_self_dispatch_author() {
        // HandleFwdReq relays the opaque ControlPkt before its independent
        // self leg applies C4GameControlNetwork's ByClient check
        // (src/C4Network2IO.cpp:1077-1128;
        // src/C4GameControlNetwork.cpp:477-492).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer_a, _) = raw_client_transport(address, b"Observer A").await;
        let (mut observer_b, _) = raw_client_transport(address, b"Observer B").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer_a).await;
        drain_raw_client(&mut observer_b).await;

        let direct_data =
            encode_control_entry_payload(&EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(source_id).unwrap(),
                command: 0x22,
                data: 0x33,
                by_client: i32::try_from(source_id).unwrap(),
            }))
            .unwrap();
        let mut nested_packet = vec![0x42, u8::from(ControlDelivery::Direct)];
        nested_packet.extend_from_slice(&direct_data);
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet,
            }))
            .await
            .unwrap();

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
            match timeout_at(host_deadline, host_events.recv()).await.unwrap() {
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

        let spoofed_data =
            encode_control_entry_payload(&EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(source_id).unwrap(),
                command: 0x44,
                data: 0x55,
                by_client: i32::try_from(source_id + 1).unwrap(),
            }))
            .unwrap();
        let mut spoofed_nested = vec![0x42, u8::from(ControlDelivery::Direct)];
        spoofed_nested.extend_from_slice(&spoofed_data);
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: spoofed_nested,
            }))
            .await
            .unwrap();

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
                .unwrap()
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_relays_forwarded_ready_check_opaquely_without_self_dispatch() {
        // A ReadyCheck can be selected for remote peers while the negative
        // list excludes the host. Its trailing bytes survive the direct relay
        // (src/C4Network2IO.cpp:1077-1128; src/C4GameLobby.cpp:329-343).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer, _) = raw_client_transport(address, b"Observer").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer).await;
        let mut observer = observer.into_inner();
        let ready = ReadyCheckPacket {
            client_id: i32::try_from(source_id).unwrap(),
            data: crate::ReadyCheckData::Ready,
        };
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
            .unwrap();
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_ignores_self_forwarded_client_league_results() {
        // HandleFwdReq relays first and then recursively handles the self leg.
        // A host recognizes league results but silently rejects them when they
        // originated from an ordinary client, without closing that client's
        // connection (src/C4Network2IO.cpp:1077-1129;
        // src/C4Network2Players.cpp:392-419).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
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
            .unwrap();
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
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, source.read_message()).await.unwrap() {
                Ok(ControlMessage::Pong(received)) if received == ping => break,
                Ok(_) => continue,
                Err(error) => panic!("connection closed after league-results forwarding: {error}"),
            }
        }

        drop(source);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_closes_only_the_source_route_for_a_malformed_nested_forward() {
        // Recursive forwarding sends selected nested bytes through the normal
        // packet unpacker. A compiler failure closes only pConn in release; the
        // network scheduler and unrelated routes remain live
        // (src/C4Network2IO.cpp:822-835,1041-1055,1088-1140).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
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
            .unwrap();
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
        .expect("source route was not closed after malformed forwarding");

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
        .expect("malformed source TCP route remained open");
        raw_client_ping_barrier(&mut witness).await;

        drop(source);
        drop(witness);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_uses_cpp_forward_wrapper_for_more_than_two_remote_targets() {
        // HandleFwdReq switches from direct nested sends to one positive-list
        // PID_Fwd broadcast when more than two remote client IDs are selected
        // (pristine C++ src/C4Network2IO.cpp:1083-1112).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
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
        let nested_packet = crate::transport::encode_complete_control_packet(&control).unwrap();
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: nested_packet.clone(),
            }))
            .await
            .unwrap();
        let expected = crate::ForwardPacket {
            negative_list: false,
            clients: vec![observer_c_id, observer_b_id, observer_a_id]
                .into_iter()
                .map(|client_id| i32::try_from(client_id).unwrap())
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn host_join_gate_returns_only_after_the_live_state_applies() {
        // C4Network2::AllowJoin mutates fAllowJoin before returning; callers
        // enter DoLobby only after that synchronous transition
        // (src/C4Network2.cpp:835-843; src/C4Game.cpp:3874-3880).
        let (command_tx, mut commands) = mpsc::channel(1);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let handle = HostHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle: tokio::spawn(async {}),
            udp_local_addr: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };
        let setter = tokio::spawn(async move { handle.set_join_allowed(true).await });

        let HostCommand::SetJoinAllowed {
            allowed,
            completion,
        } = commands.recv().await.expect("gate command")
        else {
            panic!("expected gate command");
        };
        assert!(allowed);
        assert!(!setter.is_finished(), "host state has not applied the gate");
        completion.send(()).expect("acknowledge applied gate");
        setter
            .await
            .expect("setter task")
            .expect("gate acknowledgement");
    }

    #[tokio::test]
    async fn host_begin_go_carries_status_and_admission_in_one_acknowledged_command() {
        let (command_tx, mut commands) = mpsc::channel(1);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let handle = HostHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle: tokio::spawn(async {}),
            udp_local_addr: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 41,
        };
        let starter = tokio::spawn(async move { handle.begin_go(status, false).await });

        let HostCommand::BeginGo {
            status: requested_status,
            join_allowed,
            completion,
        } = commands.recv().await.expect("atomic Go command")
        else {
            panic!("expected atomic Go command");
        };
        assert_eq!(requested_status, status);
        assert!(!join_allowed);
        assert!(
            !starter.is_finished(),
            "caller must wait until both host states have been applied"
        );
        completion.send(()).expect("acknowledge atomic transition");
        starter
            .await
            .expect("starter task")
            .expect("atomic transition acknowledgement");
    }

    #[tokio::test]
    async fn host_begin_go_reports_a_dropped_apply_acknowledgement() {
        let (command_tx, mut commands) = mpsc::channel(1);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let handle = HostHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle: tokio::spawn(async {}),
            udp_local_addr: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        let starter = tokio::spawn(async move { handle.begin_go(status, true).await });

        let HostCommand::BeginGo { completion, .. } =
            commands.recv().await.expect("atomic Go command")
        else {
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let reached_at = tokio::time::Instant::now();
        host.control_tick_reached(0, 1, DEFAULT_CONTROL_TARGET_FPS, reached_at)
            .await
            .unwrap();
        tokio::time::advance(Duration::from_millis(200)).await;
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .unwrap();
        host.control_tick_consumed(0, tokio::time::Instant::now(), vec![HOST_CLIENT_ID], false)
            .await
            .unwrap();
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn control_tick_consumed_public_handles_only_wait_for_enqueue() {
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 7,
        };

        let (host_command_tx, mut host_commands) = mpsc::channel(1);
        let (host_event_tx, host_event_rx) = mpsc::channel(1);
        host_event_tx
            .send(HostEvent::StatusCommitted(status))
            .await
            .unwrap();
        let (host_shutdown_tx, _host_shutdown_rx) = oneshot::channel();
        let host = HostHandle {
            command_tx: host_command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(host_event_rx),
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
        .expect("full undrained host event queue blocked enqueue-only API")
        .unwrap();
        assert!(matches!(
            host_commands.recv().await,
            Some(HostCommand::ControlTickConsumed { tick: 7, .. })
        ));

        let (client_command_tx, mut client_commands) = mpsc::channel(1);
        let (client_event_tx, client_event_rx) = mpsc::channel(1);
        client_event_tx
            .send(ClientEvent::Status(status))
            .await
            .unwrap();
        let (client_shutdown_tx, _client_shutdown_rx) = oneshot::channel();
        let client = ClientHandle {
            command_tx: client_command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(client_event_rx),
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
        .expect("full undrained client event queue blocked enqueue-only API")
        .unwrap();
        assert!(matches!(
            client_commands.recv().await,
            Some(ClientCommand::ControlTickConsumed { tick: 7, .. })
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_commands_are_serviced_while_route_events_remain_saturated() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
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
        .expect("route-event flood never reached the host loop");

        timeout(Duration::from_millis(250), host.set_join_allowed(false))
            .await
            .expect("unbounded route events starved an acknowledged host command")
            .unwrap();

        stop_tx.send_replace(true);
        flood.await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_commands_are_serviced_while_route_events_remain_saturated() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let host_event_drain =
            tokio::spawn(async move { while host_events.recv().await.is_some() {} });
        let mut client =
            connect_client(address, ClientConfig::new("Flood", ParticipantKind::Player))
                .await
                .unwrap();
        let mut events = client.take_event_receiver();
        let flood_data =
            encode_control_entry_payload(&EngineControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                command: 0x55,
                data: 0,
                by_client: HOST_CLIENT_ID as i32,
            }))
            .unwrap();
        let (saturated_tx, saturated_rx) = oneshot::channel();
        let event_drain = tokio::spawn(async move {
            let mut direct_count = 0;
            let mut saturated_tx = Some(saturated_tx);
            while let Some(event) = events.recv().await {
                if matches!(event, ClientEvent::Direct { .. }) {
                    direct_count += 1;
                    if direct_count == 64 {
                        let _ = saturated_tx.take().expect("one saturation signal").send(());
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
        timeout(EVENT_WAIT, saturated_rx)
            .await
            .expect("host-to-client route-event flood never became active")
            .unwrap();

        timeout(
            Duration::from_millis(250),
            client.runtime_client_states(0, false),
        )
        .await
        .expect("route events starved an acknowledged client command")
        .unwrap();

        flood.abort();
        let _ = flood.await;
        client.shutdown().await.unwrap();
        event_drain.await.unwrap();
        host.shutdown().await.unwrap();
        host_event_drain.await.unwrap();
    }

    #[tokio::test]
    async fn host_password_setter_returns_only_after_the_live_state_applies() {
        let (command_tx, mut commands) = mpsc::channel(1);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let handle = HostHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle: tokio::spawn(async {}),
            udp_local_addr: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };
        let secret = clonk_engine::LegacyCString::from_bytes(b"secret".to_vec()).unwrap();
        let setter = tokio::spawn(async move { handle.set_password(Some(secret)).await });

        let HostCommand::SetPassword {
            password,
            completion,
        } = commands.recv().await.expect("password command")
        else {
            panic!("expected password command");
        };
        assert_eq!(password.unwrap().as_bytes(), b"secret");
        assert!(
            !setter.is_finished(),
            "host state has not applied the password"
        );
        completion.send(()).expect("acknowledge applied password");
        setter
            .await
            .expect("setter task")
            .expect("password acknowledgement");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_handles_inspect_client_states_and_retire_the_live_tcp_route() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let client = connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();

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

        let host_connections = host.runtime_connections().await.unwrap();
        assert_eq!(host_connections.len(), 1);
        assert_eq!(host_connections[0].client_id, client.client_id());
        assert_eq!(host_connections[0].usage, "Data/Msg");
        assert_eq!(host_connections[0].protocol, crate::NetworkProtocol::Tcp);
        assert!(host_connections[0].peer_address.is_some());

        let client_connections = client.runtime_connections().await.unwrap();
        assert_eq!(client_connections.len(), 1);
        assert_eq!(client_connections[0].client_id, HOST_CLIENT_ID);
        assert_eq!(client_connections[0].usage, "Data/Msg");
        assert_eq!(client_connections[0].protocol, crate::NetworkProtocol::Tcp);
        assert!(client_connections[0].peer_address.is_some());

        let host_lobby_telemetry = host
            .lobby_client_telemetry(vec![client.client_id()])
            .await
            .unwrap();
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
            .unwrap();
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
            .unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                if host.runtime_connections().await.unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("host did not retire the client-closed route");

        client.shutdown().await.unwrap();

        let second_client =
            connect_client(address, ClientConfig::new("Bob", ParticipantKind::Player))
                .await
                .unwrap();
        let second_connections = host.runtime_connections().await.unwrap();
        assert_eq!(second_connections.len(), 1);
        assert_eq!(second_connections[0].client_id, second_client.client_id());
        host.disconnect_runtime_connection(second_connections[0].connection_id)
            .await
            .unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                if host.runtime_connections().await.unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("host did not retire its selected route");
        second_client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_queued_client_remove_projects_removing_before_sync_execution() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let config = HostConfig {
            initial_status: NetworkStatus {
                state: NETWORK_STATE_GO,
                control_mode: 0,
                target_tick: 0,
            },
            ..HostConfig::default()
        };
        let host = start_host(listener, config).await.unwrap();
        let client = connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let remove = encode_control_entry_payload(&EngineControlPacket::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: i32::try_from(client.client_id()).unwrap(),
                reason: clonk_engine::LegacyCString::from_bytes(b"removed".to_vec()).unwrap(),
                by_client: HOST_CLIENT_ID as i32,
            },
        ))
        .unwrap();
        host.submit_packet(ControlDelivery::Sync, remove)
            .await
            .unwrap();

        let states = host.runtime_client_states(0, false).await.unwrap();
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reliable_udp_client_completes_session_admission_and_control() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let udp_address = host
            .udp_local_addr()
            .expect("configured reliable-UDP listener");
        let mut host_events = host.take_event_receiver();
        let client = connect_udp_client(
            udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("reliable-UDP session admission");

        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x12))
            .await
            .unwrap();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x34))
            .await
            .unwrap();
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn l075_udp_only_host_completes_session_admission_and_control() {
        let config = HostConfig {
            udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
            configured_tcp_port: Some(0),
            ..HostConfig::default()
        };
        let udp_binding = HostUdpBinding::bind(&config);
        let udp_address = udp_binding
            .local_addr()
            .expect("configured reliable-UDP listener");
        let mut host = start_host_with_bindings(None, config, udp_binding)
            .await
            .expect("UDP-only host startup");
        let mut host_events = host.take_event_receiver();
        let client = connect_udp_client(
            udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("reliable-UDP session admission");

        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x12))
            .await
            .unwrap();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x34))
            .await
            .unwrap();
        let packet = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(control_commands(&packet), vec![0x34, 0x12]);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn occupied_udp_listener_falls_back_to_a_healthy_tcp_host() {
        let occupied = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let occupied_address = occupied.local_addr().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(occupied_address),
                ..HostConfig::default()
            },
        )
        .await
        .expect("TCP host survives optional UDP bind failure");
        assert_eq!(host.udp_local_addr(), None);
        let mut host_events = host.take_event_receiver();
        assert!(matches!(
            timeout(EVENT_WAIT, host_events.recv()).await,
            Ok(Some(HostEvent::TransportError {
                client_id: None,
                error,
            })) if error.contains("failed to start reliable-UDP listener")
        ));

        let client = connect_client(
            tcp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("TCP fallback remains connectable");
        assert_eq!(host.accepted_routes().await.len(), 1);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcp_accept_failure_keeps_existing_routes_and_retries_the_listener() {
        // A failed native Accept makes that scheduler pass report false, but
        // the no-op OnError leaves the TCP proc installed and the scheduler
        // thread keeps executing it (src/C4NetIO.cpp:610-625,1038-1053;
        // src/StdScheduler.cpp:160-191,229-244;
        // src/StdScheduler.h:95-98).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
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
        .expect("host did not report the injected TCP accept failure");

        raw_client_ping_barrier(&mut witness).await;
        let (mut successor, successor_id) =
            timeout(EVENT_WAIT, raw_client_transport(address, b"Successor"))
                .await
                .expect("host did not retry TCP accept after the injected failure");
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn occupied_client_udp_port_falls_back_to_a_healthy_tcp_route() {
        let occupied = tokio::net::UdpSocket::bind("[::]:0").await.unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();

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
        .expect("an unavailable configured UDP port does not suppress TCP admission");

        assert_eq!(host.accepted_routes().await.len(), 1);
        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_shutdown_releases_the_configured_shared_udp_port() {
        let reservation = tokio::net::UdpSocket::bind("[::]:0").await.unwrap();
        let configured_udp_port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let mut puncher =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let puncher_address = puncher.local_addr();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();

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
        .unwrap();
        let _puncher_stream = timeout(EVENT_WAIT, puncher.accept())
            .await
            .expect("client reaches the puncher")
            .unwrap();

        client.shutdown().await.unwrap();
        let rebound =
            tokio::net::UdpSocket::bind(SocketAddr::from(([0_u16; 8], configured_udp_port)))
                .await
                .expect("ClientHandle shutdown releases the shared UDP socket");
        drop(rebound);
        host.shutdown().await.unwrap();
        puncher.shutdown().await.unwrap();
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
                .unwrap();
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
                .unwrap();
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
        .unwrap();
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
        .unwrap();
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
        .unwrap();
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
        .expect("lost mesh resource source does not tear down the host route");
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
            .expect("an unavailable mesh source does not tear down the host route");

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
            Some(crate::ResourceTransferBackend::new(9, directories.client.clone()).unwrap());
        let core = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 77,
            loadable: true,
            file_size: 64,
            chunk_size: 1,
            filename: clonk_engine::LegacyCString::from_bytes(b"Swarm.bin".to_vec()).unwrap(),
            ..Default::default()
        };
        resource_state
            .backend
            .as_mut()
            .unwrap()
            .register_remote_loadable(core.clone())
            .unwrap();
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
                chunk_count: 64,
                ranges: vec![crate::ResourceChunkRange {
                    start: 0,
                    length: 64,
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
            .unwrap();
        }

        let mut outstanding = Vec::new();
        let mut fulfilled = None;
        for peer_id in 0_i32..=6 {
            let ResourcePacket::Request(request) =
                take_queued_resource(receivers.get_mut(&peer_id).unwrap())
            else {
                panic!("peer status did not schedule a resource request");
            };
            if peer_id == 1 {
                fulfilled = Some(request);
            } else {
                outstanding.push((peer_id, request.chunk));
            }
        }
        let fulfilled = fulfilled.unwrap();
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
        .unwrap();

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
        let backend = resource_state.backend.as_ref().unwrap();
        assert_eq!(backend.catalog().outstanding_load_count(core.id), 19);
        assert_eq!(outstanding.len(), 19);
        assert_eq!(
            outstanding
                .iter()
                .map(|(_, chunk)| *chunk)
                .collect::<BTreeSet<_>>()
                .len(),
            19
        );
        let mut per_peer = BTreeMap::<i32, usize>::new();
        for (peer_id, _) in &outstanding {
            *per_peer.entry(*peer_id).or_default() += 1;
        }
        let mut counts = per_peer.values().copied().collect::<Vec<_>>();
        counts.sort_unstable();
        assert_eq!(counts, vec![1, 3, 3, 3, 3, 3, 3]);
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
        .unwrap();
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
                            crate::transport::parse_complete_packet(&packet).unwrap()
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
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(tcp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
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
            .unwrap();
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
            .unwrap();
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
            .unwrap();
        udp.send_message(ControlMessage::Ping(udp_ping))
            .await
            .unwrap();
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
        .expect("failed UDP route did not close its outbound channel");
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 0,
        };
        routes
            .send_message(ControlMessage::StatusAck(status))
            .await
            .unwrap();
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
        .expect("failed UDP route was not removed");
        let mut recovered = Vec::new();
        timeout(EVENT_WAIT, async {
            while !recovered.contains(&ControlMessage::StatusAck(status)) {
                match tcp.read_message().await.unwrap() {
                    ControlMessage::Ping(packet) => tcp
                        .send_message(ControlMessage::Pong(packet))
                        .await
                        .unwrap(),
                    message => flatten_recovery(message, &mut recovered),
                }
            }
        })
        .await
        .expect("status was not recovered over the TCP fallback");

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
        .expect("failed TCP route did not close its outbound channel");
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
            .unwrap();
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
        .expect("failed TCP route was not removed");
        let expected = ControlMessage::Resource(ResourcePacket::Data(fallback_data));
        recovered.clear();
        timeout(EVENT_WAIT, async {
            while !recovered.contains(&expected) {
                match fallback_udp.read_message().await.unwrap() {
                    ControlMessage::Ping(packet) => fallback_udp
                        .send_message(ControlMessage::Pong(packet))
                        .await
                        .unwrap(),
                    message => flatten_recovery(message, &mut recovered),
                }
            }
        })
        .await
        .expect("resource data was not recovered over the UDP fallback");
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
                "[::]:0".parse().unwrap(),
            ),
        };
        udp.send_message(ControlMessage::Address(address))
            .await
            .unwrap();

        let (packet, ingress_peer_addr) = timeout(EVENT_WAIT, routes.read_packet())
            .await
            .unwrap()
            .unwrap();
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
            routes.try_send_to(7, message.clone()).unwrap();
        }
        assert!(!routes.routes[&1].outbound.is_closed());

        timeout(EVENT_WAIT, routes.shutdown())
            .await
            .expect("slow peer route did not shut down");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_address_burst_only_waits_for_cpp_output_buffer_acceptance() {
        // SendAddresses only appends each PID_Addr to the connection's OBuf;
        // it neither waits for the socket to drain nor fails the route on an
        // EWOULDBLOCK result (oracle-src-pinned
        // src/C4Network2Client.cpp:319-337,616-621;
        // src/C4NetIO.cpp:1345-1396).
        let (client_stream, _host_stream) = duplex(1);
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(client_stream),
            ConnectionLivenessState::new_accepted_system(),
        );
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
        .expect("address announcement waited for physical socket drainage")
        .unwrap();
        assert!(!routes.routes[&1].outbound.is_closed());

        timeout(EVENT_WAIT, routes.shutdown())
            .await
            .expect("backpressured address route did not shut down");
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
            .unwrap();
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
            .unwrap();
        assert_eq!(
            peer.read_message().await.unwrap(),
            ControlMessage::LobbyCountdown(countdown)
        );
        drop(peer);

        let event = timeout(EVENT_WAIT, routes.read_event())
            .await
            .unwrap()
            .unwrap();
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
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(tcp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        routes.add_route(
            2,
            12,
            crate::NetworkProtocol::Udp,
            None,
            crate::ControlTransport::new(udp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
        udp.send_message(ControlMessage::ConnectionRequest(
            crate::ConnectionRequest {
                core: clonk_engine::ClientCoreControlData::default(),
                build: CURRENT_GAME_BUILD,
                password: clonk_engine::LegacyCString::default(),
                connection_id: 99,
            },
        ))
        .await
        .unwrap();

        let event = timeout(EVENT_WAIT, routes.read_event())
            .await
            .expect("asymmetric UDP reconnect did not retire the stale route")
            .unwrap();
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
            .unwrap();
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
            .unwrap();
        let mut header = [0_u8; 5];
        encoded_stream.read_exact(&mut header).await.unwrap();
        let mut complete_packet =
            vec![0_u8; u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize];
        encoded_stream
            .read_exact(&mut complete_packet)
            .await
            .unwrap();

        udp.send_message(ControlMessage::PostMortem(crate::PostMortemPacket {
            connection_id: 1,
            packet_counter: 1,
            packets: vec![complete_packet],
        }))
        .await
        .unwrap();
        let (replayed, peer_addr) = timeout(EVENT_WAIT, routes.read_packet())
            .await
            .expect("post-mortem replay stalled")
            .unwrap();
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
                .unwrap()
            {
                ControlMessage::PostMortem(packet) => break packet,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                other => panic!("expected reciprocal post-mortem, got {other:?}"),
            }
        };
        assert_eq!(reciprocal.connection_id, 11);
        assert_eq!(reciprocal.packet_counter, 1);
        assert_eq!(reciprocal.packets.len(), 1);

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 0,
        };
        routes
            .send_message(ControlMessage::StatusAck(status))
            .await
            .unwrap();
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
            transport.send_message(message).await.unwrap();
            let mut header = [0_u8; 5];
            reader.read_exact(&mut header).await.unwrap();
            let mut body = vec![0; u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize];
            reader.read_exact(&mut body).await.unwrap();
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
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(host_client),
            ConnectionLivenessState::new_accepted_system(),
        );
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
            .unwrap();
        let mut header = [0_u8; 5];
        reader.read_exact(&mut header).await.unwrap();
        let mut complete_packet =
            vec![0; u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize];
        reader.read_exact(&mut complete_packet).await.unwrap();

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
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(client_stream),
            ConnectionLivenessState::new_accepted_system(),
        );
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

        timeout(EVENT_WAIT, routes.shutdown())
            .await
            .expect("route shutdown remained blocked behind socket output");
    }

    #[tokio::test]
    async fn host_handle_shutdown_bypasses_full_command_and_event_queues() {
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        let (command_tx, _command_rx) = mpsc::channel(1);
        command_tx.send(HostCommand::Shutdown).await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        event_tx
            .send(HostEvent::StatusCommitted(status))
            .await
            .unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join_handle = tokio::spawn(async move {
            let _ = event_tx.send(HostEvent::StatusCommitted(status)).await;
            let _ = shutdown_rx.await;
        });
        let handle = HostHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            udp_local_addr: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        timeout(EVENT_WAIT, handle.shutdown())
            .await
            .expect("host handle shutdown waited on a bounded queue")
            .unwrap();
    }

    #[tokio::test]
    async fn client_handle_shutdown_bypasses_full_command_and_event_queues() {
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        let (command_tx, _command_rx) = mpsc::channel(1);
        command_tx.send(ClientCommand::Shutdown).await.unwrap();
        let (event_tx, event_rx) = mpsc::channel(1);
        event_tx.send(ClientEvent::Status(status)).await.unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let join_handle = tokio::spawn(async move {
            let _ = event_tx.send(ClientEvent::Status(status)).await;
            let _ = shutdown_rx.await;
        });
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            client_id: 1,
            join_data: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        timeout(EVENT_WAIT, handle.shutdown())
            .await
            .expect("client handle shutdown waited on a bounded queue")
            .unwrap();
    }

    #[tokio::test]
    async fn failed_client_route_retains_commands_already_accepted_by_its_queue() {
        let (client_stream, peer_stream) = duplex(256);
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let (_retire_tx, retire_rx) = watch::channel(false);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let first = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 7,
        };
        let second = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 2,
            target_tick: 8,
        };
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Status(first)))
            .unwrap();
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Status(second)))
            .unwrap();
        drop(peer_stream);

        let task = tokio::spawn(run_client_route(
            1,
            11,
            None,
            crate::ControlTransport::new(client_stream),
            outbound_tx.clone(),
            outbound_rx,
            retire_rx,
            event_tx,
            ConnectionLivenessState::new_accepted_system(),
        ));
        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("failed route did not report its recovery backlog")
            .expect("failed route event channel closed");
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
        task.await.unwrap();
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
        let first = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 41,
        };
        let second = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 42,
        };

        routes
            .try_send_to(7, ControlMessage::Status(first))
            .unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                if routes.routes[&1].outbound.sender.is_closed() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("forced writer failure did not close the route queue");
        routes.routes[&1].outbound.retire();
        routes
            .try_send_to(7, ControlMessage::Status(second))
            .unwrap();
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
            match timeout(EVENT_WAIT, fallback.read_message())
                .await
                .expect("fallback replay stalled")
                .unwrap()
            {
                ControlMessage::PostMortem(packet) => {
                    logical_order.extend(packet.packets.into_iter().map(|packet| {
                        crate::transport::parse_complete_packet(&packet)
                            .unwrap()
                            .expect("post-mortem entry is typed")
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
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let (retire_tx, retire_rx) = watch::channel(false);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_client_route(
            1,
            11,
            None,
            crate::ControlTransport::new(client_stream),
            outbound_tx.clone(),
            outbound_rx,
            retire_rx,
            event_tx,
            ConnectionLivenessState::new_accepted_system(),
        ));
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: vec![0x55; 1_024 * 1_024],
            }))
            .unwrap();
        tokio::task::yield_now().await;
        let inbound = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 7,
        };
        peer.send_message(ControlMessage::Status(inbound))
            .await
            .unwrap();

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
        timeout(EVENT_WAIT, task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn retiring_client_route_cancels_an_inflight_write_into_post_mortem() {
        let (client_stream, mut peer_stream) = duplex(1);
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let (retire_tx, retire_rx) = watch::channel(false);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 9,
        };
        let task = tokio::spawn(run_client_route(
            1,
            11,
            None,
            crate::ControlTransport::new(client_stream),
            outbound_tx.clone(),
            outbound_rx,
            retire_rx,
            event_tx,
            ConnectionLivenessState::new_accepted_system(),
        ));
        outbound_tx
            .send(ClientRouteCommand::Message(ControlMessage::Status(status)))
            .unwrap();
        let mut first_wire_byte = [0_u8; 1];
        timeout(EVENT_WAIT, peer_stream.read_exact(&mut first_wire_byte))
            .await
            .expect("route write did not begin")
            .unwrap();
        retire_tx.send_replace(true);

        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("inflight route did not retire")
            .expect("route event channel closed");
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
        task.await.unwrap();
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
            .unwrap();
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
                .unwrap();
        }
        routes
            .event_tx
            .send(ClientRouteEvent::Packet {
                route_id: 2,
                peer_addr: Some(udp_peer_address),
                packet: crate::transport::InboundPacket::Empty,
            })
            .unwrap();
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
        let mut routes = ClientRouteManager::new();
        routes.add_route(
            1,
            11,
            crate::NetworkProtocol::Tcp,
            None,
            crate::ControlTransport::new(tcp_client),
            ConnectionLivenessState::new_accepted_system(),
        );
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
            .unwrap()
        {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected TCP route liveness Ping, got {other:?}"),
        };
        let udp_ping = match timeout(EVENT_WAIT, udp.read_message())
            .await
            .unwrap()
            .unwrap()
        {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected UDP route liveness Ping, got {other:?}"),
        };
        tcp.send_message(ControlMessage::Pong(tcp_ping))
            .await
            .unwrap();
        udp.send_message(ControlMessage::Pong(udp_ping))
            .await
            .unwrap();

        routes.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_client_handle_adds_udp_route_without_duplicate_join() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let udp_address = host.udp_local_addr().unwrap();
        let mut host_events = host.take_event_receiver();
        let client = connect_dual_client(
            tcp_address,
            udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("dual-route client admission");

        timeout(EVENT_WAIT, async {
            loop {
                if host.accepted_routes().await.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("host retained both client routes");

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

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 0,
        };
        client.submit_status_ack(status).await.unwrap();
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
        .expect("dual client message traffic reached the host");

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_client_reconnects_a_missing_tcp_route() {
        let host_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host_tcp_address = host_listener.local_addr().unwrap();
        let host = start_host(
            host_listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let host_udp_address = host.udp_local_addr().unwrap();
        let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        let (cut_first, cut_first_rx) = oneshot::channel();
        let proxy = tokio::spawn(async move {
            let (mut client, _) = proxy_listener.accept().await.unwrap();
            let mut host = TcpStream::connect(host_tcp_address).await.unwrap();
            let first =
                tokio::spawn(
                    async move { tokio::io::copy_bidirectional(&mut client, &mut host).await },
                );
            let _ = cut_first_rx.await;
            first.abort();
            let _ = first.await;

            let (mut client, _) = proxy_listener.accept().await.unwrap();
            let mut host = TcpStream::connect(host_tcp_address).await.unwrap();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut host).await;
        });
        let client = connect_dual_client(
            proxy_address,
            host_udp_address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let initial_routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 2 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dual routes were not established");
        let initial_ids = initial_routes
            .iter()
            .map(|(connection_id, _, _)| *connection_id)
            .collect::<BTreeSet<_>>();
        cut_first.send(()).unwrap();

        timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                let route_ids = routes
                    .iter()
                    .map(|(connection_id, _, _)| *connection_id)
                    .collect::<BTreeSet<_>>();
                if routes.len() == 2 && route_ids != initial_ids {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("missing TCP protocol was not reconnected");

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
        proxy.abort();
        let _ = proxy.await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_client_keeps_the_healthy_tcp_route_when_udp_is_unreachable() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let udp_blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let client = timeout(
            Duration::from_secs(2),
            connect_dual_client(
                tcp_address,
                udp_blackhole.local_addr().unwrap(),
                ClientConfig::new("Alice", ParticipantKind::Player),
            ),
        )
        .await
        .expect("optional reliable-UDP attempt stayed bounded")
        .expect("healthy TCP route remains usable");
        assert_eq!(host.accepted_routes().await.len(), 1);

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 0,
        };
        client.submit_status_ack(status).await.unwrap();
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
        .expect("TCP fallback carried client message traffic");

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_protocol_routes_prefer_udp_messages_and_tcp_resource_data() {
        let directories = SessionResourceDirectories::new();
        let source = directories.root.join("RouteSplit.c4d");
        let resource_bytes = b"resource data takes the TCP route";
        fs::write(&source, resource_bytes).unwrap();
        let publication = crate::build_host_resource_core(
            &source,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Definitions,
                7,
                clonk_engine::LegacyCString::from_bytes(b"RouteSplit.c4d".to_vec()).unwrap(),
                "",
            ),
        )
        .unwrap();
        let core = publication.core.clone();
        let hosted_path = publication
            .standalone_path
            .expect("loadable test resource has standalone bytes");
        let hosted_ownership = publication
            .standalone_ownership
            .expect("loadable test resource has standalone ownership");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                resource_directory: Some(directories.host.clone()),
                resource_registrations: vec![crate::ResourceRegistration::from_core(
                    &core, true, false,
                )],
                resource_files: vec![HostedResourceFile {
                    core: core.clone(),
                    path: hosted_path,
                    ownership: hosted_ownership,
                    binary_compatible: true,
                }],
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let udp_address = host
            .udp_local_addr()
            .expect("configured reliable-UDP listener");
        let mut host_events = host.take_event_receiver();
        let (mut tcp, client_id) = raw_client_transport(tcp_address, b"Alice").await;

        while host_events.try_recv().is_ok() {}
        let initial_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(initial_deadline, tcp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    tcp.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => panic!("TCP route closed while draining join setup: {error}"),
            }
        }

        let udp_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let udp_stream = timeout(EVENT_WAIT, udp_hub.connect_owned(udp_address))
            .await
            .expect("reliable-UDP connection attempt stalled")
            .unwrap();
        let mut udp = crate::ControlTransport::new(udp_stream);
        let host_request = loop {
            match timeout(EVENT_WAIT, udp.read_message())
                .await
                .expect("host UDP connection request stalled")
                .unwrap()
            {
                ControlMessage::ConnectionRequest(request) => break request,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                other => panic!("expected host UDP connection request, got {other:?}"),
            }
        };
        let remote_connection_id = 37;
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        udp.send_message(ControlMessage::ConnectionRequest(
            crate::ConnectionRequest {
                core: clonk_engine::ClientCoreControlData {
                    client_id: i32::try_from(client_id).unwrap(),
                    activated: true,
                    observer: false,
                    name: name.clone(),
                    nick: name,
                    lobby_ready: true,
                },
                build: CURRENT_GAME_BUILD,
                password: clonk_engine::LegacyCString::default(),
                connection_id: remote_connection_id,
            },
        ))
        .await
        .unwrap();
        loop {
            match timeout(EVENT_WAIT, udp.read_message())
                .await
                .expect("host UDP connection reply stalled")
                .unwrap()
            {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                other => panic!("expected positive host UDP connection reply, got {other:?}"),
            }
        }
        udp.send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
            ok: true,
            message: clonk_engine::LegacyCString::from_bytes(b"connection accepted".to_vec())
                .unwrap(),
            wrong_password: false,
        }))
        .await
        .unwrap();

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
        .expect("TCP and reliable-UDP routes were not both retained");
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
        loop {
            match timeout_at(quiet_deadline, udp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    udp.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                Ok(Ok(message)) => {
                    panic!(
                        "secondary reliable-UDP route received duplicate join setup: {message:?}"
                    )
                }
                Ok(Err(error)) => panic!("reliable-UDP route closed unexpectedly: {error}"),
            }
        }

        let countdown = crate::LobbyCountdownPacket::new(7);
        host.submit_lobby_countdown(countdown).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, udp.read_message())
                .await
                .expect("message traffic did not use reliable UDP")
                .unwrap()
            {
                ControlMessage::LobbyCountdown(packet) if packet == countdown => break,
                ControlMessage::Ping(ping) => {
                    udp.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                other => panic!("expected UDP lobby countdown, got {other:?}"),
            }
        }
        let tcp_quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(tcp_quiet_deadline, tcp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    tcp.send_message(ControlMessage::Pong(ping)).await.unwrap();
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
        .unwrap();
        loop {
            match timeout(EVENT_WAIT, tcp.read_message())
                .await
                .expect("resource data did not use TCP")
                .unwrap()
            {
                ControlMessage::Resource(ResourcePacket::Data(data))
                    if data.resource_id == core.id =>
                {
                    assert_eq!(data.chunk, 0);
                    assert_eq!(data.data, resource_bytes);
                    break;
                }
                ControlMessage::Ping(ping) => {
                    tcp.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                _ => {}
            }
        }
        let udp_quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(udp_quiet_deadline, udp.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    udp.send_message(ControlMessage::Pong(ping)).await.unwrap();
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
        host.shutdown().await.unwrap();
    }

    #[test]
    fn loading_resource_advertises_received_chunks_for_cpp_peer_sharing() {
        // SetLoad assigns szStandalone immediately, so IsBinaryCompatible is
        // true while the file is still loading. Discovery therefore receives
        // a status containing the currently present chunk ranges
        // (src/C4Network2Res.cpp:496-523,553-567,831-845,1557-1568).
        let host = HostConfig::default();
        let core = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 8,
            chunk_size: 4,
            ..Default::default()
        };
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            std::env::temp_dir(),
        )
        .unwrap();
        let mut state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            None,
        )
        .unwrap();

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
        snapshot.dynamic = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 1,
            chunk_size: 1,
            ..Default::default()
        };
        snapshot.parameters.scenario = clonk_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            chunk_size: 1,
            ..Default::default()
        };
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            std::env::temp_dir(),
        )
        .unwrap();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            None,
        )
        .unwrap();

        assert_eq!(state.catalog.discovery_packet().resource_ids, vec![8, 7]);
    }

    #[test]
    fn client_bootstrap_installs_an_exact_local_loadable_without_redownloading_it() {
        // SetByCore keeps a contents-identical binary-compatible local file;
        // AddByCore must not replace it with SetLoad or a Network temporary
        // (src/C4Network2Res.cpp:441-493,1473-1516).
        let directories = SessionResourceDirectories::new();
        let local_dynamic = directories.root.join("local-dynamic.c4d");
        fs::write(&local_dynamic, b"local").unwrap();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            contents_crc: 0x8bd6_88e8,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![local_dynamic.clone()]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .unwrap();

        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .unwrap();

        let backend = state.backend.expect("filesystem resource backend");
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
        fs::write(&local_dynamic, b"local").unwrap();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            contents_crc: 0x8bd6_88e8,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![local_dynamic.clone()]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .unwrap();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .unwrap();
        let (client_stream, _host_stream) = duplex(4096);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
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

        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("local resource completion event stalled")
            .expect("client event stream closed");
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

        shutdown_tx.send(()).unwrap();
        client_loop.await.unwrap();
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
        let core = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            derived_id: -1,
            loadable: true,
            file_size: 5,
            chunk_size: 5,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            directories.client.clone(),
        )
        .unwrap();
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
        .unwrap();
        let (client_stream, _host_stream) = duplex(4096);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
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

        let progress = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("buffered resource progress stalled")
            .expect("client event stream closed");
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
            .expect("client event stream closed");
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

        shutdown_tx
            .send(())
            .expect("bad resource chunk did not disconnect the client loop");
        client_loop.await.unwrap();
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
        fs::write(&local_dynamic, b"local").unwrap();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let dynamic = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            contents_crc: 0x8bd6_88e8,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = dynamic.clone();
        let scenario_id = snapshot.parameters.scenario.id;
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(dynamic.id, vec![local_dynamic]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .unwrap();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .unwrap();
        let (client_stream, host_stream) = duplex(4096);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = mpsc::channel(2);
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
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

        removal
            .await
            .expect("resource-removal task")
            .expect("registered dynamic removal");
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let message = timeout(EVENT_WAIT, host_transport.read_message())
            .await
            .expect("post-removal discovery stalled")
            .expect("post-removal discovery transport");
        let ControlMessage::Resource(ResourcePacket::Discover(discovery)) = message else {
            panic!("unexpected post-removal message: {message:?}");
        };
        assert_eq!(discovery.resource_ids, vec![scenario_id]);

        shutdown_tx.send(()).unwrap();
        client_loop.await.unwrap();
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
            .unwrap();
        fs::write(&player, group.pack().unwrap()).unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
        let request = |wire_name: &[u8], maker: &[u8]| crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: clonk_engine::LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
            group_maker: clonk_engine::LegacyCString::from_bytes(maker.to_vec()).unwrap(),
        };

        let original = state
            .publish_player_resource(request(b"First.c4p", b"First maker"))
            .unwrap();
        let reused = state
            .publish_player_resource(request(b"Second.c4p", b"Second maker"))
            .unwrap();

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
            .unwrap();
        fs::write(&player, group.pack().unwrap()).unwrap();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                clonk_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                "",
            ),
        )
        .unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
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
            .unwrap();
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        let reused = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: clonk_engine::LegacyCString::from_bytes(b"Renamed.c4p".to_vec())
                    .unwrap(),
                group_maker: clonk_engine::LegacyCString::from_bytes(b"Client maker".to_vec())
                    .unwrap(),
            })
            .unwrap();

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
            .unwrap();
        let contents_crc = player.contents_crc();
        let player_standalone = player.pack().unwrap();
        let mut mother = MutableGroup::new("Players.c4f");
        mother
            .add_child_with_metadata("Shared.c4p", player, 1, false)
            .unwrap();
        fs::write(&mother_path, mother.pack().unwrap()).unwrap();
        let nested_player = mother_path.join("Shared.c4p");
        let core = clonk_engine::NetworkResourceCore {
            resource_type: crate::HostResourceType::Player as u8,
            id: 1 << 16,
            derived_id: -1,
            loadable: true,
            file_size: player_standalone.len() as u32,
            file_crc: c4group_file_crc(&player_standalone),
            chunk_size: 100 * 1024,
            contents_crc,
            filename: clonk_engine::LegacyCString::from_bytes(b"Players.c4f/Shared.c4p".to_vec())
                .unwrap(),
            ..Default::default()
        };
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![nested_player.clone()]);
        let resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        );
        let resource = resolver
            .resolve(crate::ClientBootstrapResourceRole::Player, &core)
            .unwrap();
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
        fs::create_dir(&player).unwrap();
        fs::write(player.join("Player.txt"), b"player core").unwrap();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                clonk_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                "Host maker",
            ),
        )
        .unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
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
            .unwrap();
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        let published = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: clonk_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                group_maker: clonk_engine::LegacyCString::from_bytes(b"Client maker".to_vec())
                    .unwrap(),
            })
            .unwrap();

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
            .unwrap();
        fs::write(&player, group.pack().unwrap()).unwrap();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                clonk_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                "",
            ),
        )
        .unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
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
                wire_name: clonk_engine::LegacyCString::from_bytes(b"Renamed.c4p".to_vec())
                    .unwrap(),
                group_maker: clonk_engine::LegacyCString::from_bytes(b"Client maker".to_vec())
                    .unwrap(),
            })
            .unwrap();

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
            .unwrap();
        group
            .add_file_with_metadata("Portrait.png", b"portrait".to_vec(), 2, false)
            .unwrap();
        let original = group.pack().unwrap();
        fs::write(&player, &original).unwrap();
        let request = crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: clonk_engine::LegacyCString::from_bytes(b"Players.c4f/Alice.c4p".to_vec())
                .unwrap(),
            group_maker: clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
        };
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };

        let direct_directory = directories.root.join("direct");
        let mut direct_state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(direct_directory),
        )
        .unwrap();
        let direct_core = direct_state
            .publish_player_resource(request.clone())
            .unwrap();
        assert_eq!(direct_core.id, 7 << 16);
        assert!(direct_state.catalog.contains_resource(direct_core.id));
        let direct_backend = direct_state.backend.as_ref().unwrap();
        assert_eq!(direct_backend.core(direct_core.id), Some(&direct_core));
        assert!(direct_backend.path(direct_core.id).unwrap().is_file());

        let (client_stream, host_stream) = duplex(4096);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(4);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let loop_directory = directories.root.join("loop");
        let resource_state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(loop_directory),
        )
        .unwrap();
        let join_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            client_id: 7,
            join_data: Some(join_data),
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        let core = handle.publish_player_resource(request).await.unwrap();
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
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap()
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
                        .unwrap();
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
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap()
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
                        .unwrap();
                }
                _ => {}
            }
        }

        handle.shutdown().await.unwrap();
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
            .unwrap();
        fs::write(&initial_player, initial_group.pack().unwrap()).unwrap();
        let initial_wire =
            clonk_engine::LegacyCString::from_bytes(b"HostInitial.c4p".to_vec()).unwrap();
        let maker = clonk_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
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
            .unwrap();
        let initial_core = initial_publication.core.clone();

        let player = directories.root.join("HostRuntime.c4p");
        let mut group = MutableGroup::new("HostRuntime.c4p");
        group
            .add_file_with_metadata("Player.txt", b"host runtime player".to_vec(), 1, false)
            .unwrap();
        let original = group.pack().unwrap();
        fs::write(&player, &original).unwrap();
        let publication = crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: clonk_engine::LegacyCString::from_bytes(b"HostRuntime.c4p".to_vec())
                .unwrap(),
            group_maker: maker,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(
            listener,
            HostConfig {
                resource_registrations: vec![initial_publication.registration],
                resource_directory: Some(directories.host.clone()),
                resource_files: vec![initial_publication.resource_file],
                player_resource_sources: vec![(initial_player, initial_core.clone())],
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let stream = TcpStream::connect(address).await.unwrap();
        let mut peer = crate::ControlTransport::new(stream);
        let peer_name = clonk_engine::LegacyCString::from_bytes(b"Peer".to_vec()).unwrap();
        run_client_connection_handshake(
            &mut peer,
            crate::ConnectionRequest {
                core: clonk_engine::ClientCoreControlData {
                    client_id: -1,
                    name: peer_name.clone(),
                    nick: peer_name,
                    ..Default::default()
                },
                build: CURRENT_GAME_BUILD,
                password: clonk_engine::LegacyCString::default(),
                connection_id: 0,
            },
        )
        .await
        .expect("peer joins before runtime publication");

        assert_eq!(
            host.publish_player_resource(initial_request).await.unwrap(),
            initial_core,
            "an InitHost player source reuses its existing core"
        );
        let core = host
            .publish_player_resource(publication.clone())
            .await
            .unwrap();
        let reused = host.publish_player_resource(publication).await.unwrap();
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
        .unwrap();
        loop {
            match timeout(EVENT_WAIT, peer.read_message())
                .await
                .expect("host runtime resource discovery stalled")
                .unwrap()
            {
                ControlMessage::Resource(ResourcePacket::Status(status))
                    if status.resource_id == core.id =>
                {
                    assert_eq!(status.chunks.ranges[0].start, 0);
                    break;
                }
                ControlMessage::Ping(ping) => {
                    peer.send_message(ControlMessage::Pong(ping)).await.unwrap();
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
        .unwrap();
        loop {
            match timeout(EVENT_WAIT, peer.read_message())
                .await
                .expect("host runtime resource chunk stalled")
                .unwrap()
            {
                ControlMessage::Resource(ResourcePacket::Data(data))
                    if data.resource_id == core.id =>
                {
                    assert_eq!(data.chunk, 0);
                    assert!(!data.data.is_empty());
                    break;
                }
                ControlMessage::Ping(ping) => {
                    peer.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                _ => {}
            }
        }

        host.shutdown().await.unwrap();
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
            .unwrap();
        fs::write(&source, group.pack().unwrap()).unwrap();
        let publication = crate::build_host_resource_core(
            &source,
            directories.root.join("published"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                clonk_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec()).unwrap(),
                "Host",
            ),
        )
        .unwrap();
        let valid_core = publication.core.clone();
        let hosted_path = publication.standalone_path.unwrap();
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
        let local_join_data = JoinDataEnvelope {
            client_id: 2,
            start_control_tick: local_snapshot.dynamic_tick,
            status: local_host.initial_status,
            dynamic: local_snapshot.dynamic,
            parameters: local_snapshot.parameters,
        };
        let local_work_path = directories.root.join("client-local");
        let mut local_state = ClientResourceState::new(
            &local_join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(local_work_path.clone()),
        )
        .unwrap();
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
        let local_backend = local_state.backend.as_ref().unwrap();
        assert_eq!(local_backend.core(valid_core.id), Some(&valid_core));
        let local_standalone = local_backend.path(valid_core.id).unwrap();
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host_config = HostConfig {
            resource_directory: Some(directories.host.clone()),
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
            }],
            ..HostConfig::default()
        };
        let host = start_host(listener, host_config).await.unwrap();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .unwrap();
        let mut client_events = client.take_event_receiver();

        let resource_player = |id: i32, flags: u16, core: clonk_engine::NetworkResourceCore| {
            clonk_engine::ControlPlayerInfoEntry {
                id,
                flags,
                resource: Some(core),
                ..Default::default()
            }
        };
        let info = clonk_engine::PlayerInfoControlData {
            client_id: 1,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![
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
            by_client: 0,
        };
        let encoded = crate::encode_control_entry_payload(
            &clonk_engine::ControlPacket::PlayerInfo(info.clone()),
        )
        .unwrap();
        host.submit_packet(ControlDelivery::Direct, encoded.clone())
            .await
            .unwrap();

        let mut delivered = None;
        let mut completed = None;
        while delivered.is_none() || completed.is_none() {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
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
        let delivered = delivered.unwrap();
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
        let (completed_core, completed_path, local) = completed.unwrap();
        assert_eq!(completed_core, valid_core);
        assert!(completed_path.is_file());
        assert!(!local);

        host.submit_packet(ControlDelivery::Direct, encoded)
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
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
        fs::create_dir_all(&local_root).unwrap();
        let source = local_root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"host-local player".to_vec(), 1, false)
            .unwrap();
        fs::write(&source, group.pack().unwrap()).unwrap();
        let core = crate::build_host_resource_core(
            &source,
            directories.root.join("core"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                clonk_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec()).unwrap(),
                "Host",
            ),
        )
        .unwrap()
        .core;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host_config = HostConfig {
            resource_directory: Some(directories.host.clone()),
            local_resource_roots: vec![local_root],
            ..HostConfig::default()
        };
        let mut host = start_host(listener, host_config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .unwrap();
        let mut client_events = client.take_event_receiver();
        let info = clonk_engine::PlayerInfoControlData {
            client_id: 1,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            by_client: 0,
        };
        host.submit_packet(
            ControlDelivery::Direct,
            crate::encode_control_entry_payload(&clonk_engine::ControlPacket::PlayerInfo(info))
                .unwrap(),
        )
        .await
        .unwrap();

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
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
                wire_name: clonk_engine::LegacyCString::from_bytes(b"Renamed.c4p".to_vec())
                    .unwrap(),
                group_maker: clonk_engine::LegacyCString::from_bytes(b"Host maker".to_vec())
                    .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(
            reused, core,
            "AddByFile reuses the locally resolved authoritative resource"
        );

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_resolves_local_player_resource_before_exposing_direct_control() {
        let directories = SessionResourceDirectories::new();
        let host_root = directories.root.join("host-local");
        let client_root = directories.root.join("client-local");
        fs::create_dir_all(&host_root).unwrap();
        fs::create_dir_all(&client_root).unwrap();
        let host_source = host_root.join("Alice.c4p");
        let client_source = client_root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"shared local player".to_vec(), 1, false)
            .unwrap();
        let player_bytes = group.pack().unwrap();
        fs::write(&host_source, &player_bytes).unwrap();
        fs::write(&client_source, player_bytes).unwrap();
        let core = crate::build_host_resource_core(
            &host_source,
            directories.root.join("core"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                clonk_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec()).unwrap(),
                "Host",
            ),
        )
        .unwrap()
        .core;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host_config = HostConfig {
            resource_directory: Some(directories.host.clone()),
            local_resource_roots: vec![host_root],
            ..HostConfig::default()
        };
        let host = start_host(listener, host_config).await.unwrap();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_resource_roots([client_root]),
        )
        .await
        .unwrap();
        let mut client_events = client.take_event_receiver();
        let info = clonk_engine::PlayerInfoControlData {
            client_id: 1,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            by_client: 0,
        };
        host.submit_packet(
            ControlDelivery::Direct,
            crate::encode_control_entry_payload(&clonk_engine::ControlPacket::PlayerInfo(info))
                .unwrap(),
        )
        .await
        .unwrap();

        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
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
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
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
        fs::write(&source, b"local").unwrap();
        let core = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let mut client =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();

        let mut progress = Vec::new();
        let completed_path = loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("resource transfer stalled")
                .expect("client event stream closed")
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
        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_and_client_finish_a_derived_resource_without_redownloading_it() {
        // C4Player::Save calls Derive before replacing the file. The control
        // host then calls FinishDerive and every peer with a matching anonymous
        // resource adopts the new core with complete chunks
        // (src/C4Player.cpp:452-461; src/C4Network2Res.cpp:718-823,1584-1594).
        let directories = SessionResourceDirectories::new();
        let host_source = directories.host.join("Dynamic.c4d");
        fs::write(&host_source, b"local").unwrap();
        let parent = clonk_engine::NetworkResourceCore {
            resource_type: crate::HostResourceType::Dynamic as u8,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, host_config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .unwrap();

        let client_source = loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("parent resource transfer stalled")
                .expect("client event stream closed")
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
            .unwrap();
        let _client_derivation = client
            .begin_resource_derive(
                parent.id,
                client_source.clone(),
                crate::ResourceFileOwnership::Temporary,
            )
            .await
            .unwrap();
        fs::write(&host_source, b"changed").unwrap();
        fs::write(&client_source, b"changed").unwrap();

        let derived = host.finish_resource_derive(host_derivation).await.unwrap();
        assert_ne!(derived.id, parent.id);
        assert_eq!(derived.derived_id, parent.id);
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host derive completion stalled")
                .expect("host event stream closed")
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
                .expect("client event stream closed")
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
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
        fs::write(&system_path, b"host system").unwrap();
        fs::write(&mismatched_system_path, b"different client system").unwrap();
        let publication = crate::build_host_resource_core(
            &system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                clonk_engine::LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.parameters.scenario = clonk_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
            ..Default::default()
        };
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(mismatched_system_path),
        )
        .await;
        host.shutdown().await.unwrap();

        let error = result.expect_err("client must fail before entering the lobby");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("System.c4g") && message.contains("non-loadable")),
            "unexpected client bootstrap failure: {error:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cpp_interop_trusts_the_explicit_local_system_across_builds() {
        // C++ publishes System as non-loadable and normally gates it by
        // ContentsCRC (src/C4Network2Res.cpp:441-493,1458-1461). Once admitted,
        // however, it executes the process-local Application.SystemGroup
        // (src/C4Application.cpp:127-134; src/C4Game.cpp:2764-2793). A Rust
        // client therefore maps the C++ core to its explicitly trusted local
        // System while retaining exact checks for every transferable resource.
        let directories = SessionResourceDirectories::new();
        let host_system_path = directories.host.join("System.c4g");
        let client_system_path = directories.client.join("System.c4g");
        fs::create_dir(&host_system_path).unwrap();
        fs::create_dir(&client_system_path).unwrap();
        fs::write(host_system_path.join("Host.c"), b"C++ host system").unwrap();
        fs::write(client_system_path.join("Client.c"), b"Rust client system").unwrap();
        let publication = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                2,
                clonk_engine::LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
                "C++ host",
            ),
        )
        .unwrap();
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_trusted_local_system_path(client_system_path.clone()),
        )
        .await
        .expect("a trusted Rust System permits C++ cross-build bootstrap");

        assert_eq!(
            client.take_join_data().unwrap().status.state,
            NETWORK_STATE_LOBBY
        );
        let mut events = client.take_event_receiver();
        loop {
            match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
                Some(ClientEvent::ResourceComplete {
                    resource_id: 2,
                    core,
                    path,
                    local,
                }) => {
                    assert_eq!(core, publication.core);
                    assert_eq!(path, client_system_path);
                    assert!(local);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("trusted System join disconnected: {reason:?}")
                }
                Some(_) => {}
                None => panic!("client event stream closed before System completion"),
            }
        }

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
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
        snapshot.dynamic = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: false,
            file_size: u32::MAX,
            file_crc: u32::MAX,
            contents_crc: 1,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.parameters.scenario = clonk_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
            ..Default::default()
        };
        assert!(snapshot.parameters.game_resources.is_empty());
        host_config.initial_join_snapshot = Some(snapshot);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await;
        host.shutdown().await.unwrap();

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
        fs::write(&host_system_path, system_bytes).unwrap();
        fs::write(&client_system_path, system_bytes).unwrap();
        let publication = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                clonk_engine::LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.parameters.scenario = clonk_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
            ..Default::default()
        };
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(client_system_path),
        )
        .await
        .expect("contents-identical local System permits client bootstrap");

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
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
        fs::write(&host_system_path, system_bytes).unwrap();
        fs::write(&client_system_path, system_bytes).unwrap();
        fs::write(&host_definitions_path, definitions_bytes).unwrap();
        fs::write(&client_definitions_path, definitions_bytes).unwrap();
        let system = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                clonk_engine::LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
        let mut definitions = crate::build_host_resource_core(
            &host_definitions_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                10,
                clonk_engine::LegacyCString::from_bytes(b"Objects.c4d".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(client_system_path)
                .with_local_resource_roots([directories.client.clone()]),
        )
        .await
        .expect("contents-identical non-loadable definitions permit bootstrap");

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let mut client =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .expect("an unavailable player resource must not abort the join");

        let join_data = client.take_join_data().expect("initial JoinData");
        let player = &join_data.parameters.player_infos.clients[0].players[0];
        assert_eq!(
            player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
            0
        );
        assert_eq!(player.resource, None);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
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
            fs::create_dir_all(&host).unwrap();
            fs::create_dir_all(&client).unwrap();
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
        .expect("encode ClientJoin");

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
        let direct = encode_control_entry_payload(&remove).expect("encode ClientRemove");
        let authenticated =
            authenticated_single_control(&direct, 7).expect("embedded peer author matches");
        assert!(control_requires_host_ingress(&authenticated));

        let queued = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![remove],
        })
        .expect("encode queued ClientRemove");
        assert!(validate_peer_control_packet(&queued, 7)
            .expect_err("peer membership control must be rejected")
            .contains("host-authority"));
    }

    #[test]
    fn mesh_peer_control_contribution_must_use_the_ingress_client_id() {
        let queued = encode_control_packet(&LegacyControlFrame {
            client_id: 8,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![EngineControlPacket::PlayerControl(PlayerControlData {
                player: 1,
                command: 2,
                data: 3,
                by_client: 8,
            })],
        })
        .expect("encode peer contribution");

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
        let payload = encode_control_entry_payload(&EngineControlPacket::InitScenarioPlayer(
            clonk_engine::InitScenarioPlayerControlData {
                team: 2,
                player: 4,
                by_client: 7,
            },
        ))
        .expect("encode InitScenarioPlayer");

        assert!(authenticated_single_control(&payload, 7).is_ok());
        assert!(authenticated_single_control(&payload, 3).is_err());
    }

    #[test]
    fn single_control_authentication_uses_control_set_by_client() {
        let control = crate::LegacyControlSet {
            value_type: 0,
            data: 1,
            by_client: 7,
        }
        .into_control_packet();
        let payload = encode_control_entry_payload(&control).expect("encode CID_Set");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error = authenticated_single_control(&payload, 8).expect_err("reject spoofed author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn queued_control_set_authentication_uses_frame_client_id() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![crate::LegacyControlSet {
                    value_type: 1,
                    data: 0,
                    by_client,
                }
                .into_control_packet()],
            })
            .expect("encode queued CID_Set")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_Set");
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
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
        let packet = |controls| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls,
            })
            .expect("encode queued controls")
        };

        validate_queued_control_authors(&packet(controls(7))).expect("matching queued authors");

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
        let payload = encode_control_entry_payload(&control).expect("encode CID_RemovePlr");
        assert_eq!(
            authenticated_single_control(&payload, 0).expect("host author matches"),
            control
        );
        assert!(authenticated_single_control(&payload, 7).is_err());

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control],
        })
        .expect("encode queued CID_RemovePlr");
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
            script: clonk_engine::LegacyCString::from_bytes(b"1+2".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 7,
        });
        let payload = encode_control_entry_payload(&control).expect("encode CID_Script");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error =
            authenticated_single_control(&payload, 8).expect_err("reject spoofed script author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn queued_script_control_cannot_forge_host_author() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![EngineControlPacket::Script(
                    clonk_engine::ScriptControlData {
                        target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                        strictness: clonk_engine::ScriptStrictness::Strict3,
                        script: clonk_engine::LegacyCString::from_bytes(b"1+2".to_vec())
                            .expect("fixture is NUL-free"),
                        by_client,
                    },
                )],
            })
            .expect("encode queued CID_Script")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_Script");
        assert!(error.contains("queued CID_Script"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn single_message_board_answer_authenticates_embedded_author() {
        let control =
            EngineControlPacket::MessageBoardAnswer(clonk_engine::MessageBoardAnswerControlData {
                object: 42,
                answer: clonk_engine::LegacyCString::from_bytes(b"answer".to_vec())
                    .expect("fixture is NUL-free"),
                player: 3,
                by_client: 7,
            });
        let payload =
            encode_control_entry_payload(&control).expect("encode CID_MessageBoardAnswer");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error = authenticated_single_control(&payload, 8)
            .expect_err("reject spoofed message-board answer author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn single_message_control_authenticates_embedded_author() {
        let control = EngineControlPacket::Message(clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_PRIVATE,
            player: 3,
            to_player: 5,
            message: clonk_engine::LegacyCString::from_bytes(b"secret".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 7,
        });
        let payload = encode_control_entry_payload(&control).expect("encode CID_Message");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error =
            authenticated_single_control(&payload, 8).expect_err("reject spoofed message author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
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
            message: clonk_engine::LegacyCString::from_bytes(b"team secret".to_vec())
                .expect("fixture is NUL-free"),
            by_client: HOST_CLIENT_ID as i32,
        });
        let data = encode_control_entry_payload(&control).expect("encode CID_Message");

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
            message: clonk_engine::LegacyCString::from_bytes(b"network notice".to_vec())
                .expect("fixture is NUL-free"),
            by_client: HOST_CLIENT_ID as i32,
        });
        let data = encode_control_entry_payload(&control).expect("encode CID_Message");

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
        let effects = state.status_barrier.change_status(NetworkStatus {
            state: NETWORK_STATE_GO,
            ..state.status_barrier.status
        });

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
                message: clonk_engine::LegacyCString::from_bytes(
                    format!("message {index}").into_bytes(),
                )
                .expect("fixture is NUL-free"),
                by_client: HOST_CLIENT_ID as i32,
            });
            let data = encode_control_entry_payload(&control).expect("encode CID_Message");
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
                message: clonk_engine::LegacyCString::from_bytes(text)
                    .expect("fixture is NUL-free"),
                by_client: HOST_CLIENT_ID as i32,
            });
            newest = encode_control_entry_payload(&control).expect("encode CID_Message");
            broadcast_packet(ControlDelivery::Private, newest.clone(), None, &mut state).await;
        }

        assert!(state.lobby_chat_history.iter().map(Vec::len).sum::<usize>() <= 4096);
        assert_eq!(state.lobby_chat_history.back(), Some(&newest));
    }

    #[test]
    fn queued_message_board_answer_cannot_forge_host_author() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![EngineControlPacket::MessageBoardAnswer(
                    clonk_engine::MessageBoardAnswerControlData {
                        object: 42,
                        answer: clonk_engine::LegacyCString::from_bytes(b"answer".to_vec())
                            .expect("fixture is NUL-free"),
                        player: 3,
                        by_client,
                    },
                )],
            })
            .expect("encode queued CID_MessageBoardAnswer")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_MessageBoardAnswer");
        assert!(error.contains("queued CID_MessageBoardAnswer"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn single_custom_command_authenticates_embedded_author() {
        let control = EngineControlPacket::CustomCommand(clonk_engine::CustomCommandControlData {
            command: clonk_engine::LegacyCString::from_bytes(b"push".to_vec())
                .expect("fixture is NUL-free"),
            argument: clonk_engine::LegacyCString::from_bytes(b"argument".to_vec())
                .expect("fixture is NUL-free"),
            player: 3,
            by_client: 7,
        });
        let payload = encode_control_entry_payload(&control).expect("encode CID_CustomCommand");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error = authenticated_single_control(&payload, 8)
            .expect_err("reject spoofed custom-command author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn queued_custom_command_cannot_forge_host_author() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![EngineControlPacket::CustomCommand(
                    clonk_engine::CustomCommandControlData {
                        command: clonk_engine::LegacyCString::from_bytes(b"push".to_vec())
                            .expect("fixture is NUL-free"),
                        argument: clonk_engine::LegacyCString::from_bytes(b"argument".to_vec())
                            .expect("fixture is NUL-free"),
                        player: 3,
                        by_client,
                    },
                )],
            })
            .expect("encode queued CID_CustomCommand")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_CustomCommand");
        assert!(error.contains("queued CID_CustomCommand"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
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
                script: clonk_engine::LegacyCString::from_bytes(b"SetXDir(0)".to_vec())
                    .expect("fixture is NUL-free"),
                by_client,
            })
        };

        let direct = control(7);
        let payload = encode_control_entry_payload(&direct).expect("encode CID_EMMoveObj");
        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching direct author"),
            direct
        );
        let direct_error = authenticated_single_control(&payload, 8)
            .expect_err("direct editor control may not spoof its author");
        assert!(direct_error.contains("claimed author 7"));
        assert!(direct_error.contains("authenticated author is 8"));

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(7)],
        })
        .expect("encode queued CID_EMMoveObj");
        validate_queued_control_authors(&packet).expect("matching queued author");

        let forged_packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(0)],
        })
        .expect("encode forged queued CID_EMMoveObj");
        let queued_error = validate_queued_control_authors(&forged_packet)
            .expect_err("queued editor control may not forge the host author");
        assert!(queued_error.contains("queued CID_EMMoveObj"));
        assert!(queued_error.contains("claimed author 0"));
        assert!(queued_error.contains("authenticated author is 7"));
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
                material: clonk_engine::LegacyCString::from_bytes(b"Earth".to_vec())
                    .expect("fixture is NUL-free"),
                texture: clonk_engine::LegacyCString::from_bytes(b"Rough".to_vec())
                    .expect("fixture is NUL-free"),
                by_client,
            })
        };

        let direct = control(7);
        let payload = encode_control_entry_payload(&direct).expect("encode CID_EMDrawTool");
        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching direct author"),
            direct
        );
        let direct_error = authenticated_single_control(&payload, 8)
            .expect_err("direct editor draw control may not spoof its author");
        assert!(direct_error.contains("claimed author 7"));
        assert!(direct_error.contains("authenticated author is 8"));

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(7)],
        })
        .expect("encode queued CID_EMDrawTool");
        validate_queued_control_authors(&packet).expect("matching queued author");

        let forged_packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(0)],
        })
        .expect("encode forged queued CID_EMDrawTool");
        let queued_error = validate_queued_control_authors(&forged_packet)
            .expect_err("queued editor draw control may not forge the host author");
        assert!(queued_error.contains("queued CID_EMDrawTool"));
        assert!(queued_error.contains("claimed author 0"));
        assert!(queued_error.contains("authenticated author is 7"));
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

        let direct = control(7);
        let payload = encode_control_entry_payload(&direct).expect("encode CID_EMDropDef");
        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching direct author"),
            direct
        );
        let direct_error = authenticated_single_control(&payload, 8)
            .expect_err("direct editor drop control may not spoof its author");
        assert!(direct_error.contains("claimed author 7"));
        assert!(direct_error.contains("authenticated author is 8"));

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(7)],
        })
        .expect("encode queued CID_EMDropDef");
        validate_queued_control_authors(&packet).expect("matching queued author");

        let forged_packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(0)],
        })
        .expect("encode forged queued CID_EMDropDef");
        let queued_error = validate_queued_control_authors(&forged_packet)
            .expect_err("queued editor drop control may not forge the host author");
        assert!(queued_error.contains("queued CID_EMDropDef"));
        assert!(queued_error.contains("claimed author 0"));
        assert!(queued_error.contains("authenticated author is 7"));
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
        for (name, control) in names.into_iter().zip(controls(7)) {
            let payload = encode_control_entry_payload(&control).expect("encode direct control");
            assert_eq!(
                authenticated_single_control(&payload, 7).expect("matching direct author"),
                control
            );
            let direct_error = authenticated_single_control(&payload, 8)
                .expect_err("direct author spoof must fail");
            assert!(
                direct_error.contains("claimed author 7"),
                "{name}: {direct_error}"
            );

            let packet = encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![control],
            })
            .expect("encode queued control");
            validate_queued_control_authors(&packet).expect("matching queued author");

            let forged = controls(0)
                .into_iter()
                .zip(names)
                .find_map(|(candidate, candidate_name)| {
                    (candidate_name == name).then_some(candidate)
                })
                .expect("fixture name exists");
            let forged_packet = encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![forged],
            })
            .expect("encode forged queued control");
            let queued_error = validate_queued_control_authors(&forged_packet)
                .expect_err("queued author spoof must fail");
            assert!(queued_error.contains(name), "{name}: {queued_error}");
            assert!(
                queued_error.contains("claimed author 0"),
                "{name}: {queued_error}"
            );
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            match transport.read_message().await.unwrap() {
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let tcp_peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
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
        let tcp_request = tcp_peer.await.unwrap();
        assert_eq!(tcp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(tcp_request.connection_id, 41);

        let local_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let mut peer_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
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
            .unwrap();
        let udp_request = read_compatibility_request(udp_stream).await;
        assert_eq!(udp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(udp_request.connection_id, 42);
        assert!(timeout(EVENT_WAIT, udp_task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        local_hub.shutdown().await.unwrap();
        peer_hub.shutdown().await.unwrap();
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
        let bob_id = ClientId::try_from(bob.client_id).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let tcp_peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
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
        let tcp_request = tcp_peer.await.unwrap();
        assert_eq!(tcp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(tcp_request.connection_id, 51);

        let local_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let mut peer_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
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
            .unwrap();
        let udp_request = read_compatibility_request(udp_stream).await;
        assert_eq!(udp_request.build, CPP_COMPATIBILITY_BUILD);
        assert_eq!(udp_request.connection_id, 52);
        assert!(timeout(EVENT_WAIT, udp_task)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        local_hub.shutdown().await.unwrap();
        peer_hub.shutdown().await.unwrap();
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

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (peer_stream, accepted) = tokio::join!(TcpStream::connect(address), listener.accept());
        let peer_stream = peer_stream.unwrap();
        let (accepted_stream, peer_addr) = accepted.unwrap();
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
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let peer_hub =
            crate::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let peer_handle = peer_hub.handle();
        let (peer_stream, accepted_stream) = tokio::join!(
            peer_handle.connect(local_hub.local_addr()),
            local_hub.accept()
        );
        let peer_stream = peer_stream.unwrap();
        let accepted_stream = accepted_stream.unwrap();
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
        local_hub.shutdown().await.unwrap();
        peer_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn simultaneous_open_mesh_route_uses_target_compatibility_build() {
        // Simultaneous-open still emits the ordinary C++ PID_Conn governed by
        // the exact build check (oracle-src-pinned src/C4Network2.cpp:1291-1299).
        let alice = compatibility_test_core(1, b"Alice");
        let bob = compatibility_test_core(2, b"Bob");
        let alice_id = ClientId::try_from(alice.client_id).unwrap();
        let request_template = ClientHandshakeRequestTemplate::new(
            alice,
            CPP_COMPATIBILITY_BUILD,
            clonk_engine::LegacyCString::default(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            read_compatibility_request(stream).await
        });
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
        let result = connect_mesh_tcp_socket_route(
            ClientId::try_from(bob.client_id).unwrap(),
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
        let request = peer.await.unwrap();
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
    async fn exclusive_join_defers_welcome_until_the_active_route_fails() {
        // InitClient opens every prepared route together, but exclusive mode
        // permits only one outstanding PID_Conn. OnDisconn promotes the next
        // already-open socket (src/C4Network2IO.cpp:523-563,1223-1255).
        let first_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let first_address = first_listener.local_addr().unwrap();
        let second_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let second_address = second_listener.local_addr().unwrap();
        let client = tokio::spawn(connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, first_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, second_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));
        let (first, second) = timeout(EVENT_WAIT, async {
            let (first, second) = tokio::join!(first_listener.accept(), second_listener.accept());
            (first.unwrap().0, second.unwrap().0)
        })
        .await
        .expect("both transport sockets open together");

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
        active.read_exact(&mut header_and_pid).await.unwrap();
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
        timeout(EVENT_WAIT, deferred.read_exact(&mut header_and_pid))
            .await
            .expect("active failure promotes the deferred route")
            .unwrap();
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
        let stale = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stale_address = stale.local_addr().unwrap();
        let stale_route = tokio::spawn(async move {
            let (mut stream, _) = stale.accept().await.unwrap();
            let mut header_and_pid = [0; 6];
            stream.read_exact(&mut header_and_pid).await.unwrap();
            header_and_pid
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live_address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();

        let mut client = connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, stale_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, live_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("the live route wins the transport race");

        assert_eq!(
            client
                .take_join_data()
                .expect("bootstrap JoinData remains available")
                .client_id,
            client.client_id() as i32
        );
        let stale_request = stale_route.await.unwrap();
        assert_eq!(stale_request[0], 0xff);
        assert_eq!(stale_request[5], 0x02, "the first route reached PID_Conn");
        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_accepts_a_udp_only_address_list() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let udp_address = host
            .udp_local_addr()
            .expect("configured reliable-UDP listener");
        let mut client = connect_client_addresses(
            [crate::NetworkAddress::new(
                crate::NetworkProtocol::Udp,
                udp_address,
            )],
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("the UDP route completes admission");

        assert_eq!(
            client
                .take_join_data()
                .expect("bootstrap JoinData remains available")
                .client_id,
            client.client_id() as i32
        );
        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_retains_a_prepared_opposite_transport_route() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let udp_address = host
            .udp_local_addr()
            .expect("configured reliable-UDP listener");
        let client = connect_client_addresses(
            [
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, tcp_address),
                crate::NetworkAddress::new(crate::NetworkProtocol::Udp, udp_address),
            ],
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("one prepared transport completes primary admission");

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
        .expect("the prepared opposite transport was not retained");
        assert!(routes
            .iter()
            .all(|(_, client_id, _)| *client_id == client.client_id()));

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_surfaces_wrong_password_and_allows_retry() {
        // A negative ConnRe with WrongPassword set drives the outer password
        // prompt loop; ordinary admission failures remain terminal
        // (src/C4Network2.cpp:281-345,1448-1469).
        let secret = clonk_engine::LegacyCString::from_bytes(b"correct horse".to_vec()).unwrap();
        let host_config = HostConfig {
            password: secret.clone(),
            ..HostConfig::default()
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();

        let error = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_password(clonk_engine::LegacyCString::from_bytes(b"wrong".to_vec()).unwrap()),
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
        .expect("a fresh attempt with the replacement password succeeds");
        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_join_flow_keeps_non_password_rejection_terminal() {
        // HandleConnRe presents the peer's exact message text before closing
        // the rejected connection (src/C4Network2.cpp:1476-1485).
        let host_config = HostConfig {
            allow_join: false,
            ..HostConfig::default()
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();

        let error = connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect_err("closed admission is not a password retry");
        assert_eq!(
            error.to_string(),
            "handshake rejected: the peer rejected the local connection: join denied"
        );
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_begin_go_acknowledgement_closes_admission_before_return() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };

        host.begin_go(status, false).await.unwrap();
        let error = connect_client(
            address,
            ClientConfig::new("Late join", ParticipantKind::Player),
        )
        .await
        .expect_err("the acknowledged Go transition already closed admission");
        assert!(matches!(error, ClientError::Handshake(_)));
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_first_packet_is_cpp_connection_request_not_json() {
        // C4Network2IO sends PID_Conn through the ordinary C4NetIOTCP frame as
        // soon as the socket opens (src/C4Network2IO.cpp:478-525,1223-1252;
        // src/C4NetIO.cpp:1287-1323).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(connect_client_from(
            TcpStream::connect(addr),
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut header_and_pid = [0; 6];
        peer.read_exact(&mut header_and_pid).await.unwrap();

        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02);
        client.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_first_packet_is_cpp_connection_request_without_blocking_listener() {
        // An accepted C++ TCP socket sends its own PID_Conn immediately; the
        // listener/main loop does not wait for the peer's request first
        // (src/C4Network2IO.cpp:479-530,1223-1252).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut peer = TcpStream::connect(addr).await.unwrap();
        let mut header_and_pid = [0; 6];
        timeout(Duration::from_secs(1), peer.read_exact(&mut header_and_pid))
            .await
            .expect("host must not wait for a client JSON/request prefix")
            .unwrap();

        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_accepts_a_canonical_existing_client_connection_request() {
        // HandleConn selects an existing client before the new-client Join path;
        // CheckConn accepts status-only core differences and replies
        // "connection accepted" (src/C4Network2.cpp:1286-1334,1366-1380;
        // src/C4Client.cpp:58-70).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = crate::ControlTransport::new(stream);

        assert!(matches!(
            timeout(EVENT_WAIT, transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        transport
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id: i32::try_from(client.client_id()).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: 17,
                },
            ))
            .await
            .unwrap();

        let reply = timeout(EVENT_WAIT, transport.read_message())
            .await
            .expect("host existing-client admission stalled")
            .unwrap();
        let accepted_message =
            clonk_engine::LegacyCString::from_bytes(b"connection accepted".to_vec()).unwrap();
        assert_eq!(
            reply,
            ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: accepted_message,
                wrong_password: false,
            })
        );

        host.shutdown().await.unwrap();
        client.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn synchronized_client_remove_closes_every_route_with_cpp_reason() {
        // DeleteClient closes every route with a negative PID_ConnRe carrying
        // the fixed "removing client" reason before removing the logical
        // network client (src/C4Network2Client.cpp:104-119,457-465).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
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
        .expect("both routes were not retained");
        assert_eq!(host.connected_clients().await, vec![client_id]);
        while host_events.try_recv().is_ok() {}

        let remove = EngineControlPacket::ClientRemove(clonk_engine::ClientRemoveControlData {
            client_id: i32::try_from(client_id).unwrap(),
            reason: clonk_engine::LegacyCString::from_bytes(b"voted out".to_vec()).unwrap(),
            by_client: i32::try_from(HOST_CLIENT_ID).unwrap(),
        });
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&remove).unwrap(),
        )
        .await
        .unwrap();

        let close = ControlMessage::ConnectionReply(crate::ConnectionReply {
            ok: false,
            message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec()).unwrap(),
            wrong_password: false,
        });
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn synchronized_remove_rejects_a_route_that_finishes_handshaking_late() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let (mut canonical, client_id) = raw_client_transport(addr, b"Alice").await;

        let mut delayed = crate::ControlTransport::new(TcpStream::connect(addr).await.unwrap());
        let admission = request_route(&mut delayed, i32::try_from(client_id).unwrap(), 29).await;
        assert!(admission.ok);

        let remove = EngineControlPacket::ClientRemove(clonk_engine::ClientRemoveControlData {
            client_id: i32::try_from(client_id).unwrap(),
            reason: clonk_engine::LegacyCString::from_bytes(b"voted out".to_vec()).unwrap(),
            by_client: i32::try_from(HOST_CLIENT_ID).unwrap(),
        });
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&remove).unwrap(),
        )
        .await
        .unwrap();

        let close = ControlMessage::ConnectionReply(crate::ConnectionReply {
            ok: false,
            message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec()).unwrap(),
            wrong_password: false,
        });
        assert!(raw_client_received_message(&mut canonical, &close, EVENT_WAIT).await);

        delayed
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: clonk_engine::LegacyCString::from_bytes(b"connection accepted".to_vec())
                    .unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn last_route_loss_invalidates_pending_handshake_before_sync_remove() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();

        let mut alice = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let alice_id = alice.client_id();
        let mut alice_events = alice.take_event_receiver();
        activate_joined_client(&host, &mut host_events, alice_id).await;

        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let beta_id = beta.client_id();
        let mut beta_events = beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, beta_id).await;

        let running = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(running).await.unwrap();
        for events in [&mut alice_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
                    Some(ClientEvent::Status(status)) if status == running => break,
                    Some(_) => continue,
                    None => panic!("client event stream ended before initial Go"),
                }
            }
        }
        alice.submit_status_ack(running).await.unwrap();
        beta.submit_status_ack(running).await.unwrap();
        host.status_reached(running, running.target_tick)
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial Go committed"),
            }
        }

        let mut delayed = crate::ControlTransport::new(TcpStream::connect(addr).await.unwrap());
        let admission = request_route(&mut delayed, i32::try_from(alice_id).unwrap(), 31).await;
        assert!(admission.ok);

        let unreachable = NetworkStatus {
            target_tick: 2,
            ..running
        };
        host.change_status(unreachable).await.unwrap();
        for events in [&mut alice_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
                    Some(ClientEvent::Status(status)) if status == unreachable => break,
                    Some(_) => continue,
                    None => panic!("client event stream ended before unreached Go"),
                }
            }
        }

        alice.shutdown().await.unwrap();
        let mut connection_failed = false;
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: clonk_engine::LegacyCString::from_bytes(b"connection accepted".to_vec())
                    .unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();
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

        let mut new_route = crate::ControlTransport::new(TcpStream::connect(addr).await.unwrap());
        let rejection = request_route(&mut new_route, i32::try_from(alice_id).unwrap(), 32).await;
        assert!(!rejection.ok);
        assert_eq!(rejection.message.as_bytes(), b"removing client");

        assert_eq!(host.connected_clients().await, vec![beta_id]);
        assert!(host
            .accepted_routes()
            .await
            .iter()
            .all(|(_, client_id, _)| *client_id == beta_id));
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn secondary_route_from_a_different_peer_host_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some("[::1]:0".parse().unwrap()),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();

        let udp_hub = crate::ReliableUdpSessionHub::bind("[::1]:0".parse().unwrap()).unwrap();
        let stream = udp_hub
            .connect_owned(host.udp_local_addr().unwrap())
            .await
            .unwrap();
        let mut transport = crate::ControlTransport::new(stream);
        loop {
            match timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("host connection request stalled")
                .unwrap()
            {
                ControlMessage::ConnectionRequest(_) => break,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                other => panic!("expected host connection request, got {other:?}"),
            }
        }
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        transport
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id: i32::try_from(client.client_id()).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: 41,
                },
            ))
            .await
            .unwrap();

        let rejection = loop {
            match timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("host secondary-route decision stalled")
                .unwrap()
            {
                ControlMessage::ConnectionReply(reply) => break reply,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
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
        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn different_peer_host_is_rejected_while_same_client_route_is_pending() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp_address = listener.local_addr().unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some("[::1]:0".parse().unwrap()),
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let mut host_events = host.take_event_receiver();

        let mut pending_tcp =
            crate::ControlTransport::new(TcpStream::connect(tcp_address).await.unwrap());
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
                            break ClientId::try_from(join.core.client_id).unwrap();
                        }
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended before provisional ClientJoin"),
                }
            }
        })
        .await
        .expect("provisional ClientJoin was not emitted");

        let udp_hub = crate::ReliableUdpSessionHub::bind("[::1]:0".parse().unwrap()).unwrap();
        let udp_stream = udp_hub
            .connect_owned(host.udp_local_addr().unwrap())
            .await
            .unwrap();
        let mut different_host = crate::ControlTransport::new(udp_stream);
        let rejection =
            request_route(&mut different_host, i32::try_from(client_id).unwrap(), 52).await;
        assert!(!rejection.ok);
        assert_eq!(
            rejection.message.as_bytes(),
            b"secondary connection came from a different peer host"
        );
        assert!(host.accepted_routes().await.is_empty());

        drop(different_host);
        drop(pending_tcp);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn secondary_route_does_not_rejoin_replace_or_remove_the_logical_client() {
        // HandleConnRe records whether this is the client's first connection;
        // only that first connection runs OnClientConnect and its JoinData,
        // lobby, and resource setup (src/C4Network2.cpp:1479-1498,1734-1743,
        // 1768-1783).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                resource_registrations: vec![crate::ResourceRegistration {
                    resource_id: 3,
                    chunk_count: 1,
                    binary_compatible: true,
                    loading: false,
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut host_events = host.take_event_receiver();
        let mut canonical =
            connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();
        while host_events.try_recv().is_ok() {}
        while canonical_events.try_recv().is_ok() {}

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut secondary = crate::ControlTransport::new(stream);
        let host_request = match secondary.read_message().await.unwrap() {
            ControlMessage::ConnectionRequest(request) => request,
            other => panic!("expected host connection request, got {other:?}"),
        };
        let local_connection_id = host_request.connection_id;
        let remote_connection_id = 29;
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        secondary
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id: i32::try_from(canonical_id).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: remote_connection_id,
                },
            ))
            .await
            .unwrap();
        loop {
            match secondary.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                other => panic!("expected positive host connection reply, got {other:?}"),
            }
        }
        secondary
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: clonk_engine::LegacyCString::from_bytes(b"connection accepted".to_vec())
                    .unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();

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
        .expect("secondary accepted route was not retained");
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
                        .unwrap();
                }
                Ok(Ok(message)) => {
                    panic!("secondary route received duplicate first-connect setup: {message:?}")
                }
                Ok(Err(error)) => panic!("secondary route closed unexpectedly: {error}"),
            }
        }

        let countdown = crate::LobbyCountdownPacket::new(7);
        host.submit_lobby_countdown(countdown).await.unwrap();
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
        .expect("secondary route replaced the logical client's primary sender");

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
        .expect("secondary route was not removed");
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
        host.submit_lobby_countdown(after_disconnect).await.unwrap();
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
        .expect("primary route stopped receiving after secondary disconnect");

        host.shutdown().await.unwrap();
        canonical.shutdown().await.unwrap();
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
            let stream = TcpStream::connect(addr).await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_request = match transport.read_message().await.unwrap() {
                ControlMessage::ConnectionRequest(request) => request,
                other => panic!("expected host connection request, got {other:?}"),
            };
            let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
            transport
                .send_message(ControlMessage::ConnectionRequest(
                    crate::ConnectionRequest {
                        core: clonk_engine::ClientCoreControlData {
                            client_id: i32::try_from(client_id).unwrap(),
                            activated: true,
                            observer: false,
                            name: name.clone(),
                            nick: name,
                            lobby_ready: true,
                        },
                        build: CURRENT_GAME_BUILD,
                        password: clonk_engine::LegacyCString::default(),
                        connection_id: remote_connection_id,
                    },
                ))
                .await
                .unwrap();
            loop {
                match transport.read_message().await.unwrap() {
                    ControlMessage::ConnectionReply(reply) if reply.ok => break,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
                    }
                    other => panic!("expected positive host connection reply, got {other:?}"),
                }
            }
            transport
                .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                    ok: true,
                    message: clonk_engine::LegacyCString::from_bytes(
                        b"connection accepted".to_vec(),
                    )
                    .unwrap(),
                    wrong_password: false,
                }))
                .await
                .unwrap();
            (transport, host_request.connection_id)
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut canonical =
            connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();
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
        .expect("secondary route was not accepted");
        while host_events.try_recv().is_ok() {}

        let dead_route_countdown = crate::LobbyCountdownPacket::new(9);
        host.submit_lobby_countdown(dead_route_countdown)
            .await
            .unwrap();
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
        .expect("dead route did not receive the recoverable test packet");

        canonical.shutdown().await.unwrap();
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
        .expect("primary route was not removed");
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
                            .unwrap();
                    }
                    Ok(_) => continue,
                    Err(error) => panic!("surviving route closed unexpectedly: {error}"),
                }
            }
        })
        .await
        .expect("dead route backlog was not rerouted over the promoted survivor");
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
        host.submit_lobby_countdown(countdown).await.unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                match secondary.read_message().await {
                    Ok(ControlMessage::LobbyCountdown(packet)) if packet == countdown => break,
                    Ok(ControlMessage::Ping(ping)) => {
                        secondary
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
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
        .expect("host traffic did not use the promoted secondary route");

        host.shutdown().await.unwrap();
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
            let stream = TcpStream::connect(addr).await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_request = match transport.read_message().await.unwrap() {
                ControlMessage::ConnectionRequest(request) => request,
                other => panic!("expected host connection request, got {other:?}"),
            };
            let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
            transport
                .send_message(ControlMessage::ConnectionRequest(
                    crate::ConnectionRequest {
                        core: clonk_engine::ClientCoreControlData {
                            client_id: i32::try_from(client_id).unwrap(),
                            activated: true,
                            observer: false,
                            name: name.clone(),
                            nick: name,
                            lobby_ready: true,
                        },
                        build: CURRENT_GAME_BUILD,
                        password: clonk_engine::LegacyCString::default(),
                        connection_id: remote_connection_id,
                    },
                ))
                .await
                .unwrap();
            loop {
                match transport.read_message().await.unwrap() {
                    ControlMessage::ConnectionReply(reply) if reply.ok => break,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
                    }
                    other => panic!("expected positive host connection reply, got {other:?}"),
                }
            }
            transport
                .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                    ok: true,
                    message: clonk_engine::LegacyCString::from_bytes(
                        b"connection accepted".to_vec(),
                    )
                    .unwrap(),
                    wrong_password: false,
                }))
                .await
                .unwrap();
            (transport, host_request.connection_id)
        }

        async fn encode_nested(message: ControlMessage) -> Vec<u8> {
            let (writer, mut reader) = duplex(256);
            let mut transport = crate::ControlTransport::new(writer);
            transport.send_message(message).await.unwrap();
            let mut header = [0; 5];
            reader.read_exact(&mut header).await.unwrap();
            assert_eq!(header[0], 0xff);
            let length = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
            let mut packet = vec![0; length];
            reader.read_exact(&mut packet).await.unwrap();
            packet
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let canonical = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
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
        .expect("additional routes were not accepted");
        while host_events.try_recv().is_ok() {}

        for tick in [100, 101] {
            dead_route
                .send_message(ControlMessage::ActivationRequest { tick })
                .await
                .unwrap();
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
        .expect("host did not dispatch the pre-close packets");
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
            .unwrap();
        raw_client_ping_barrier(&mut surviving_route).await;
        timeout(EVENT_WAIT, async {
            while host.accepted_routes().await.len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("post-mortem did not retire the referenced live route");
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
        .expect("host did not dispatch the recovered suffix");
        assert_eq!(recovered, vec![102, 103]);

        surviving_route
            .send_message(ControlMessage::PostMortem(recovery))
            .await
            .unwrap();
        surviving_route
            .send_message(ControlMessage::ActivationRequest { tick: 104 })
            .await
            .unwrap();
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
        .expect("host did not process the duplicate-recovery barrier");

        drop(surviving_route);
        canonical.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn nonresponsive_server_handshake_times_out() {
        // C4Network2IO::CheckTimeout closes connections which do not reach the
        // accepted state after C4NetAcceptTimeout (src/C4Network2IO.cpp:1155-1170).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let (connection, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
        let client_stream = connection.expect("connect client socket");
        let (_server_stream, _) = accepted.expect("accept client socket");

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
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(
            listener,
            HostConfig {
                max_players: 4,
                ..Default::default()
            },
        )
        .await
        .expect("start host");

        let client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut events = host.take_event_receiver();
        activate_joined_client(&host, &mut events, client.client_id()).await;

        client
            .submit_control(legacy_packet(1, 0, 0x12))
            .await
            .expect("submit client control");
        host.submit_local_control(legacy_packet(0, 0, 0x34))
            .await
            .expect("submit host control");

        let packet = wait_for_host_ready(&mut events, EVENT_WAIT).await;
        assert_eq!(packet.tick(), 0);
        assert_eq!(packet.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&packet), vec![0x34, 0x12]);

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        config.initial_status.control_mode = 2;
        config.async_max_wait_frames = 2;
        config
            .initial_join_snapshot
            .as_mut()
            .expect("default JoinData")
            .parameters
            .control_rate = 2;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        // Keep one task runnable while live loopback setup completes. Without
        // this guard Tokio may auto-advance paused time to the dial timeout
        // before the OS socket becomes ready under a heavily loaded test run.
        let frozen_time_guard = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let mut client =
            connect_client(address, ClientConfig::new("Slow", ParticipantKind::Player))
                .await
                .unwrap();
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
        .unwrap();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .unwrap();
        host.set_join_allowed(true).await.unwrap();

        // floor(2 * 2 * 1000 / 38) = 105ms. Native still waits at
        // equality and first permits the incomplete packet at 106ms.
        tokio::time::advance(Duration::from_millis(105)).await;
        host.set_join_allowed(true).await.unwrap();
        settle_paused_network().await;
        assert!(take_queued_host_ready(&mut host_events).is_none());
        assert!(take_queued_client_ready(&mut client_events).is_none());

        tokio::time::advance(Duration::from_millis(1)).await;
        // This completion is a host-loop barrier. Because the deadline branch
        // is biased above commands, the expired tick is forced first.
        host.set_join_allowed(true).await.unwrap();
        let host_ready = take_queued_host_ready(&mut host_events)
            .expect("async host did not force the expired tick");
        assert_eq!(host_ready.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(host_ready.tick(), 0);
        assert_eq!(control_commands(&host_ready), vec![0xA0]);

        // The host event proves the exact paused-time boundary. Let live TCP
        // use wall-clock scheduling while checking delivery to the client.
        tokio::time::resume();
        let client_ready = wait_for_client_ready(&mut client_events, EVENT_WAIT).await;
        assert_eq!(client_ready, host_ready);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn central_and_decentral_never_force_incomplete_ticks() {
        for mode in [0, 1] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let mut config = HostConfig::default();
            config.initial_status.control_mode = mode;
            config.async_max_wait_frames = 2;
            config
                .initial_join_snapshot
                .as_mut()
                .expect("default JoinData")
                .parameters
                .control_rate = 2;
            let mut host = start_host(listener, config).await.unwrap();
            let mut host_events = host.take_event_receiver();
            let mut client = connect_client(
                address,
                ClientConfig::new(format!("Slow-{mode}"), ParticipantKind::Player),
            )
            .await
            .unwrap();
            let mut client_events = client.take_event_receiver();
            activate_joined_client(&host, &mut host_events, client.client_id()).await;

            host.control_tick_reached(
                0,
                2,
                DEFAULT_CONTROL_TARGET_FPS,
                tokio::time::Instant::now(),
            )
            .await
            .unwrap();
            host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0 + mode))
                .await
                .unwrap();
            host.set_join_allowed(true).await.unwrap();
            tokio::time::advance(Duration::from_secs(1)).await;
            host.set_join_allowed(true).await.unwrap();
            settle_paused_network().await;

            assert!(
                take_queued_host_ready(&mut host_events).is_none(),
                "mode {mode} forced an incomplete host tick"
            );
            assert!(
                take_queued_client_ready(&mut client_events).is_none(),
                "mode {mode} broadcast an incomplete complete packet"
            );

            client.shutdown().await.unwrap();
            host.shutdown().await.unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn async_mode_commit_uses_tick_reach_stamped_in_central_mode() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        config.initial_status.control_mode = 1;
        config.async_max_wait_frames = 2;
        config
            .initial_join_snapshot
            .as_mut()
            .expect("default JoinData")
            .parameters
            .control_rate = 2;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client =
            connect_client(address, ClientConfig::new("Slow", ParticipantKind::Player))
                .await
                .unwrap();
        let mut client_events = client.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;

        host.control_tick_reached(
            0,
            2,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .unwrap();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .unwrap();
        host.set_join_allowed(true).await.unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        host.set_join_allowed(true).await.unwrap();
        settle_paused_network().await;
        assert!(take_queued_host_ready(&mut host_events).is_none());

        let asynchronous = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 0,
        };
        host.change_status(asynchronous).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::Status(status)) if status == asynchronous => break,
                Some(_) => continue,
                None => panic!("client event stream ended before async status"),
            }
        }
        client.submit_status_ack(asynchronous).await.unwrap();
        host.status_reached(asynchronous, asynchronous.target_tick)
            .await
            .unwrap();

        let mut ready = None;
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
        let ready = ready.expect("expired pre-async tick reach did not force after mode commit");
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0xA0]);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn decentralized_host_and_two_clients_pack_the_same_ordered_tick() {
        // Every participant stores its own input, receives each other active
        // client's contribution through direct/forwarded broadcast, and runs
        // PackCompleteCtrl in client-ID order (pristine C++
        // src/C4GameControlNetwork.cpp:156-179,741-783).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut alpha =
            connect_client(address, ClientConfig::new("Alpha", ParticipantKind::Player))
                .await
                .unwrap();
        let mut alpha_events = alpha.take_event_receiver();
        activate_joined_client(&host, &mut host_events, alpha.client_id()).await;
        let mut beta = connect_client(address, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta_events = beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, beta.client_id()).await;

        host.submit_local_control(legacy_packet(0, 0, 0x10))
            .await
            .unwrap();
        alpha
            .submit_control(legacy_packet(alpha.client_id(), 0, 0x20))
            .await
            .unwrap();
        beta.submit_control(legacy_packet(beta.client_id(), 0, 0x30))
            .await
            .unwrap();

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

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn join_data_rebinds_local_client_and_host_emits_direct_join_first() {
        // Host Join inserts the canonical client before ConnRe/JoinData; the
        // client then rebinds its unknown local object to the assigned ID
        // (src/C4Network2.cpp:1395-1445,1574-1604;
        // src/C4Client.cpp:284-290,321-350).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let client_id = client.client_id();
        let join_data = client.take_join_data().expect("bootstrap is retained once");

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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_sends_cpp_address_packets_immediately_after_join_data() {
        // SendJoinData writes PID_JoinData and then every known PID_Addr on the
        // accepted message connection before resource discovery begins
        // (src/C4Network2.cpp:1810-1850;
        // src/C4Network2Client.cpp:319-337,616-621).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(
            listener,
            HostConfig {
                resource_registrations: vec![
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
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = crate::ControlTransport::new(stream);
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        let request = crate::ConnectionRequest {
            core: clonk_engine::ClientCoreControlData {
                client_id: -1,
                name: name.clone(),
                nick: name,
                ..Default::default()
            },
            build: CURRENT_GAME_BUILD,
            password: clonk_engine::LegacyCString::default(),
            connection_id: 0,
        };

        let bootstrap = run_client_connection_handshake(&mut transport, request)
            .await
            .expect("binary admission and JoinData");
        assert_eq!(bootstrap.join_data.client_id, 1);

        let packet = timeout(EVENT_WAIT, transport.read_message())
            .await
            .expect("host must follow JoinData with PID_Addr")
            .unwrap();
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
            match timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("resource discovery follows JoinData addresses")
                .unwrap()
            {
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
                "0.0.0.0:11112".parse().unwrap(),
            ),
        };
        transport
            .send_message(ControlMessage::Address(client_address))
            .await
            .unwrap();
        let mut saw_reannouncement = false;
        for _ in 0..8 {
            let message = timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("host address propagation stalled")
                .unwrap();
            if message == ControlMessage::Address(client_address) {
                saw_reannouncement = true;
                break;
            }
        }
        assert!(
            saw_reannouncement,
            "host did not re-announce the newly learned client address"
        );

        host.shutdown().await.unwrap();
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
            "192.0.2.4:11112".parse().unwrap(),
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
        .unwrap();

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
        let probe = server.await.unwrap();
        let messages = probe.messages;

        let error = result.expect_err("missing non-loadable Dynamic must abort bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Dynamic.c4d") && message.contains("non-loadable")),
            "the ignored early GameRes failure masked Dynamic: {error:?}"
        );
        assert_eq!(
            messages,
            vec![ControlMessage::Request { from_tick: 0 }],
            "control initialization must precede Dynamic retrieval, but addresses must not"
        );
        assert!(
            probe.disconnected,
            "C4Network2::HandleJoinData calls Clear immediately after Dynamic bootstrap failure"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_announces_addresses_before_final_scenario_validation_failure() {
        // HandleJoinData sends known addresses before outer InitClient calls
        // Parameters.InitNetwork, whose first required resource is Scenario
        // (src/C4Network2.cpp:1620-1622,329-331;
        // src/C4GameParameters.cpp:539-547).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.scenario = nonloadable_core(1, 8, b"Scenario.c4s");
        let (address, server) = start_client_bootstrap_probe(snapshot).await;

        let result =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player)).await;
        let probe = server.await.unwrap();
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
            matches!(messages.get(1), Some(ControlMessage::Address(packet)) if
            packet.client_id == 0 && packet.address.endpoint == address)
        );
        assert!(
            probe.disconnected,
            "C4Network2::InitClient failure must clear the admitted Scenario route"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_rechecks_failed_game_resource_after_announcing_addresses() {
        // The early GameRes result is ignored. After addresses, the outer
        // Parameters.InitNetwork retries GameRes after Scenario and makes the
        // same missing non-loadable core fatal
        // (src/C4Network2.cpp:1612-1622,329-331;
        // src/C4GameParameters.cpp:237-247,539-547).
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
        let probe = server.await.unwrap();
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
            matches!(messages.get(1), Some(ControlMessage::Address(packet)) if
            packet.client_id == 0 && packet.address.endpoint == address)
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
        clonk_engine::NetworkResourceCore {
            resource_type,
            id,
            derived_id: -1,
            loadable: false,
            file_size: u32::MAX,
            file_crc: u32::MAX,
            contents_crc: 1,
            filename: clonk_engine::LegacyCString::from_bytes(filename.to_vec()).unwrap(),
            ..Default::default()
        }
    }

    struct ClientBootstrapProbeResult {
        messages: Vec<ControlMessage>,
        disconnected: bool,
    }

    async fn start_client_bootstrap_probe(
        mut snapshot: HostJoinSnapshot,
    ) -> (
        SocketAddr,
        tokio::task::JoinHandle<ClientBootstrapProbeResult>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_name = clonk_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
            let host_core = clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: host_name.clone(),
                nick: host_name,
                ..Default::default()
            };
            let request = crate::ConnectionRequest {
                core: host_core.clone(),
                build: CURRENT_GAME_BUILD,
                password: clonk_engine::LegacyCString::default(),
                connection_id: 9,
            };
            let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(1);
            let admission = tokio::spawn(async move {
                let request = admission_rx.recv().await.unwrap();
                let mut assigned = request.request.core.clone();
                assigned.client_id = 1;
                request
                    .decision_tx
                    .send(AdmissionDecision::Accept {
                        peer_core: assigned.clone(),
                        before_reply: Vec::new(),
                        message: clonk_engine::LegacyCString::from_bytes(b"join accepted".to_vec())
                            .unwrap(),
                    })
                    .unwrap();
                assigned
            });
            run_host_connection_handshake(&mut transport, request, &admission_tx)
                .await
                .unwrap();
            let assigned = admission.await.unwrap();
            snapshot.parameters.clients =
                JoinClientRegistrySnapshot::new(vec![host_core, assigned.clone()]);
            transport
                .send_message(ControlMessage::JoinData(Box::new(JoinDataEnvelope {
                    client_id: assigned.client_id,
                    start_control_tick: snapshot.dynamic_tick,
                    status: NetworkStatus {
                        state: NETWORK_STATE_LOBBY,
                        control_mode: 0,
                        target_tick: -1,
                    },
                    dynamic: snapshot.dynamic,
                    parameters: snapshot.parameters,
                })))
                .await
                .unwrap();

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

    #[tokio::test(start_paused = true)]
    async fn accepted_client_continues_the_cpp_ping_timer_after_bootstrap() {
        // C4Network2IO's 500 ms timer and strict one-second ping gate continue
        // on the accepted connection after JoinData
        // (src/C4Network2IO.cpp:605-617,1141-1151).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1_500)).await;
        let ping = match host.read_message().await.unwrap() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected accepted-session PID_Ping, got {other:?}"),
        };
        assert_eq!(ping.packet_counter, 0);
        tokio::time::advance(Duration::from_millis(37)).await;
        host.send_message(ControlMessage::Pong(ping)).await.unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::PingMeasured { round_trip_ms: 37 }))
        ));

        shutdown_tx.send(()).unwrap();
        drop(command_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn established_host_link_accepts_cpp_tcp_sim_open_frame_without_disconnect() {
        let (client_stream, mut host_stream) = duplex(512);
        let (command_tx, command_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        // C++ body: packed client 7, TCP, [2001:db8::7]:11112.
        let payload = [
            0x14, 0x07, 0x01, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':',
            b'7', b']', b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ];
        host_stream.write_all(&tcp_frame(&payload)).await.unwrap();

        let status = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 3,
            target_tick: 17,
        };
        let mut host = crate::ControlTransport::new(host_stream);
        host.send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        match timeout(EVENT_WAIT, event_rx.recv()).await {
            Ok(Some(ClientEvent::Status(received))) => assert_eq!(received, status),
            Ok(Some(ClientEvent::Disconnected { reason })) => {
                panic!("PID_TCPSimOpen disconnected the established host link: {reason:?}")
            }
            other => panic!("status after PID_TCPSimOpen was not delivered: {other:?}"),
        }

        shutdown_tx.send(()).unwrap();
        drop(command_tx);
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_host_connection_continues_the_cpp_ping_timer() {
        // The host's accepted connection remains on the same C4Network2IO
        // timer after mutual admission (src/C4Network2IO.cpp:605-617,
        // 1141-1177).
        let (host_stream, client_stream) = duplex(512);
        let mut client = crate::ControlTransport::new(client_stream);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id: 1,
                transport: crate::ControlTransport::new(host_stream),
                outbound_rx,
                retire_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1_500)).await;
        let ping = match client.read_message().await.unwrap() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected host accepted-session PID_Ping, got {other:?}"),
        };
        client
            .send_message(ControlMessage::Pong(ping))
            .await
            .unwrap();
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
        task.await.unwrap();
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
            "127.0.0.1:11112".parse().unwrap(),
            crate::NetworkProtocol::Tcp,
            host_core,
            7,
            crate::NetworkIoStatistics::default(),
            admission_tx,
            host_tx,
        );
        let admission = tokio::spawn(async move {
            let request = admission_rx.recv().await.unwrap();
            let mut assigned = request.request.core.clone();
            assigned.client_id = 1;
            request
                .decision_tx
                .send(AdmissionDecision::Accept {
                    peer_core: assigned,
                    before_reply: Vec::new(),
                    message: clonk_engine::LegacyCString::default(),
                })
                .unwrap();
        });

        let mut client = crate::ControlTransport::new(client_stream);
        assert!(matches!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        client
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: compatibility_test_core(-1, b"Alice"),
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: 11,
                },
            ))
            .await
            .unwrap();
        client
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: clonk_engine::LegacyCString::default(),
                wrong_password: false,
            }))
            .await
            .unwrap();
        assert!(matches!(
            client.read_message().await.unwrap(),
            ControlMessage::ConnectionReply(crate::ConnectionReply { ok: true, .. })
        ));

        let _delayed_setup = match timeout(EVENT_WAIT, host_rx.recv()).await.unwrap() {
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
            .unwrap();
        assert_eq!(
            timeout(Duration::from_millis(100), client.read_message())
                .await
                .expect("accepted host route stopped reading while setup was delayed")
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        route_tasks.abort_all();
        while route_tasks.join_next().await.is_some() {}
        admission.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_route_reads_inbound_while_its_socket_write_is_blocked() {
        // Native TCP services readable input independently of its pending
        // output buffer (oracle-src-pinned
        // src/C4NetIO.cpp:690-761,1345-1396).
        let (host_stream, peer_stream) = duplex(64);
        let mut peer = crate::ControlTransport::new(peer_stream);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
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
        outbound_tx
            .send(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: vec![0x55; 1_024 * 1_024],
            })
            .await
            .unwrap();
        tokio::task::yield_now().await;
        let inbound = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 7,
        };
        peer.send_message(ControlMessage::Status(inbound))
            .await
            .unwrap();

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
        timeout(EVENT_WAIT, task).await.unwrap().unwrap();
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
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
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

        for sequence in 0..PACKET_COUNT {
            peer.send_message(ControlMessage::Status(NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 1,
                target_tick: sequence as i32,
            }))
            .await
            .unwrap_or_else(|error| {
                panic!("host route closed while sending packet {sequence}: {error}")
            });
        }
        let ping = crate::PingPacket {
            sent_at: 0x1020_3040,
            packet_counter: PACKET_COUNT as u32,
        };
        peer.send_message(ControlMessage::Ping(ping)).await.unwrap();
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
        timeout(EVENT_WAIT, task)
            .await
            .expect("saturated host route did not shut down")
            .unwrap();
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
                .unwrap();
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
        peer.send_message(ControlMessage::Ping(ping)).await.unwrap();

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
        timeout(EVENT_WAIT, task)
            .await
            .expect("FIFO host route did not shut down")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_route_close_preempts_an_arbitrary_blocked_backlog() {
        // C4Network2Client::CloseConns sends one best-effort negative ConnRe
        // then immediately closes each connection; stale OBuf is not drained
        // first (oracle-src-pinned src/C4Network2Client.cpp:104-118;
        // src/C4NetIO.cpp:1458-1468).
        let (host_stream, _peer_stream) = duplex(1);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let (host_tx, _host_rx) = mpsc::unbounded_channel();
        let mut task = tokio::spawn(
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
        for _ in 0..10_001 {
            outbound_tx
                .send(ControlMessage::Packet {
                    delivery: ControlDelivery::Direct,
                    data: vec![0x55; 1_024],
                })
                .await
                .unwrap();
        }
        tokio::task::yield_now().await;
        outbound_tx
            .try_close(crate::ConnectionReply {
                ok: false,
                message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec())
                    .unwrap(),
                wrong_password: false,
            })
            .unwrap();

        timeout(Duration::from_millis(100), &mut task)
            .await
            .expect("controlled close waited behind the stale outbound backlog")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn accepted_host_decodes_tcp_sim_open_and_keeps_the_connection() {
        let (host_stream, mut client_stream) = duplex(512);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
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

        // Packed client 7 plus a TCP IPv6 endpoint, matching the native
        // C4PacketTCPSimOpen binary layout.
        let tcp_sim_open = [
            0x14, 0x07, 0x01, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':',
            b'7', b']', b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ];
        client_stream
            .write_all(&tcp_frame(&tcp_sim_open))
            .await
            .unwrap();

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
            .unwrap();
        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::Pong(ping),
            "the ignored packet must not terminate the accepted connection"
        );

        drop(client);
        drop(outbound_tx);
        task.await.unwrap();
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
        let (client_stream, mut host_stream) = duplex(512);
        let (command_tx, command_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        // Success, "OK", and zero result players, matching the native
        // C4PacketLeagueRoundResults binary layout.
        let league_results = [0x17, 0x01, b'O', b'K', 0x00, 0x00];
        host_stream
            .write_all(&tcp_frame(&league_results))
            .await
            .unwrap();

        let event = timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .unwrap()
            .expect("client event channel remains open");
        let ClientEvent::LeagueRoundResults { packet } = event else {
            panic!("expected typed league round-results event, got {event:?}");
        };
        assert_eq!(
            packet,
            crate::LeagueRoundResultsPacket {
                success: true,
                result_string: clonk_engine::LegacyCString::from_bytes(b"OK".to_vec()).unwrap(),
                players: Vec::new(),
            }
        );

        let mut host = crate::ControlTransport::new(host_stream);
        let ping = crate::PingPacket {
            sent_at: 23,
            packet_counter: 0,
        };
        host.send_message(ControlMessage::Ping(ping)).await.unwrap();
        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Pong(ping),
            "the typed packet must not terminate the accepted connection"
        );

        tokio::time::advance(Duration::from_millis(1_500)).await;
        let liveness_ping = match host.read_message().await.unwrap() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected accepted-session PID_Ping, got {other:?}"),
        };
        assert_eq!(
            liveness_ping.packet_counter, 1,
            "the typed PID must advance the recoverable inbound counter"
        );
        host.send_message(ControlMessage::Pong(liveness_ping))
            .await
            .unwrap();

        shutdown_tx.send(()).unwrap();
        drop(command_tx);
        task.await.unwrap();
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ping_result_tx, ping_result_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_name = clonk_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
            let host_core = clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: host_name.clone(),
                nick: host_name,
                ..Default::default()
            };
            let request = crate::ConnectionRequest {
                core: host_core.clone(),
                build: CURRENT_GAME_BUILD,
                password: clonk_engine::LegacyCString::default(),
                connection_id: 9,
            };
            let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(1);
            let admission = tokio::spawn(async move {
                let request = admission_rx.recv().await.unwrap();
                let mut assigned = request.request.core.clone();
                assigned.client_id = 1;
                request
                    .decision_tx
                    .send(AdmissionDecision::Accept {
                        peer_core: assigned.clone(),
                        before_reply: Vec::new(),
                        message: clonk_engine::LegacyCString::from_bytes(b"join accepted".to_vec())
                            .unwrap(),
                    })
                    .unwrap();
                assigned
            });
            run_host_connection_handshake(&mut transport, request, &admission_tx)
                .await
                .unwrap();
            let assigned = admission.await.unwrap();
            let mut snapshot = synthetic_join_snapshot(host_core.clone(), 8);
            snapshot.parameters.clients =
                JoinClientRegistrySnapshot::new(vec![host_core, assigned.clone()]);
            transport
                .send_message(ControlMessage::JoinData(Box::new(JoinDataEnvelope {
                    client_id: assigned.client_id,
                    start_control_tick: snapshot.dynamic_tick,
                    status: NetworkStatus {
                        state: NETWORK_STATE_LOBBY,
                        control_mode: 0,
                        target_tick: -1,
                    },
                    dynamic: snapshot.dynamic,
                    parameters: snapshot.parameters,
                })))
                .await
                .unwrap();
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
                .unwrap();
            let serviced = timeout(Duration::from_millis(500), async {
                loop {
                    match transport.read_message().await.unwrap() {
                        ControlMessage::Pong(reply) if reply == ping => break,
                        ControlMessage::Ping(probe) => {
                            transport
                                .send_message(ControlMessage::Pong(probe))
                                .await
                                .unwrap();
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

        let client_name = String::from_utf8(client_name.to_vec()).unwrap();
        let client_thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let client = connect_client(
                        address,
                        ClientConfig::new(client_name, ParticipantKind::Player),
                    )
                    .await?;
                    client.shutdown().await
                })
        });

        probe_paused
            .await
            .expect("client did not enter synchronous resource probing");
        let ping_serviced = ping_result_rx
            .await
            .expect("probe server stopped before reporting Ping service");
        resume_probe
            .send(())
            .expect("resource bootstrap worker disappeared");
        let client_result = tokio::task::spawn_blocking(move || client_thread.join())
            .await
            .unwrap()
            .expect("current-thread client runtime panicked");
        client_result.expect("client should complete bootstrap after probe release");
        server.await.unwrap();

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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (probe_complete_tx, probe_complete_rx) = oneshot::channel();
        let (finish_server_tx, finish_server_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_name = clonk_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
            let host_core = clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: host_name.clone(),
                nick: host_name,
                ..Default::default()
            };
            let request = crate::ConnectionRequest {
                core: host_core.clone(),
                build: CURRENT_GAME_BUILD,
                password: clonk_engine::LegacyCString::default(),
                connection_id: 9,
            };
            let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(1);
            let admission = tokio::spawn(async move {
                let request = admission_rx.recv().await.unwrap();
                let mut assigned = request.request.core.clone();
                assigned.client_id = 1;
                request
                    .decision_tx
                    .send(AdmissionDecision::Accept {
                        peer_core: assigned.clone(),
                        before_reply: Vec::new(),
                        message: clonk_engine::LegacyCString::from_bytes(b"join accepted".to_vec())
                            .unwrap(),
                    })
                    .unwrap();
                assigned
            });
            run_host_connection_handshake(&mut transport, request, &admission_tx)
                .await
                .unwrap();
            let assigned = admission.await.unwrap();
            let mut snapshot = synthetic_join_snapshot(host_core.clone(), 8);
            snapshot.parameters.clients =
                JoinClientRegistrySnapshot::new(vec![host_core, assigned.clone()]);
            transport
                .send_message(ControlMessage::JoinData(Box::new(JoinDataEnvelope {
                    client_id: assigned.client_id,
                    start_control_tick: snapshot.dynamic_tick,
                    status: NetworkStatus {
                        state: NETWORK_STATE_LOBBY,
                        control_mode: 0,
                        target_tick: -1,
                    },
                    dynamic: snapshot.dynamic,
                    parameters: snapshot.parameters,
                })))
                .await
                .unwrap();
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
                    .unwrap();
            }
            let ping = crate::PingPacket {
                sent_at: 0x1020_3040,
                packet_counter: 0,
            };
            transport
                .send_message(ControlMessage::Ping(ping))
                .await
                .unwrap();
            timeout(Duration::from_millis(500), async {
                loop {
                    match transport.read_message().await.unwrap() {
                        ControlMessage::Pong(reply) if reply == ping => break,
                        ControlMessage::Ping(probe) => {
                            transport
                                .send_message(ControlMessage::Pong(probe))
                                .await
                                .unwrap();
                        }
                        other => panic!(
                            "client started post-bootstrap traffic before release: {other:?}"
                        ),
                    }
                }
            })
            .await
            .expect("accepted primary route did not service Ping during bootstrap");
            let _ = probe_complete_tx.send(());
            let _ = finish_server_rx.await;
        });

        let config = ClientConfig::new(
            String::from_utf8(client_name.to_vec()).unwrap(),
            ParticipantKind::Player,
        );
        let connect = tokio::spawn(connect_client(address, config));
        bootstrap_paused
            .await
            .expect("client did not reach the post-JoinData bootstrap pause");
        timeout(Duration::from_secs(1), probe_complete_rx)
            .await
            .expect("bootstrap route did not answer the host's probe")
            .expect("probe server stopped before observing Pong");
        assert!(
            !connect.is_finished(),
            "the main client loop must not start before resource bootstrap completes"
        );
        resume_bootstrap
            .send(())
            .expect("paused client bootstrap disappeared");

        let mut client = connect.await.unwrap().unwrap();
        for expected in 0..QUEUED_PACKETS {
            let event = timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("queued post-JoinData packet stalled")
                .expect("client event stream ended");
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
        server.await.unwrap();
        client.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_announces_known_host_address_after_applying_join_data() {
        // HandleJoinData finishes by sending every address already known by
        // the client list. At this point the outgoing host ConnectAddr is
        // known, so it is re-announced as a host-owned PID_Addr
        // (src/C4Network2.cpp:1448-1499,1574-1623;
        // src/C4Network2Client.cpp:319-337,616-621).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_name = clonk_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
            let host_core = clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: host_name.clone(),
                nick: host_name,
                ..Default::default()
            };
            let request = crate::ConnectionRequest {
                core: host_core.clone(),
                build: CURRENT_GAME_BUILD,
                password: clonk_engine::LegacyCString::default(),
                connection_id: 9,
            };
            let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(1);
            let admission = tokio::spawn(async move {
                let request = admission_rx.recv().await.unwrap();
                let mut assigned = request.request.core.clone();
                assigned.client_id = 1;
                request
                    .decision_tx
                    .send(AdmissionDecision::Accept {
                        peer_core: assigned.clone(),
                        before_reply: Vec::new(),
                        message: clonk_engine::LegacyCString::from_bytes(b"join accepted".to_vec())
                            .unwrap(),
                    })
                    .unwrap();
                assigned
            });
            run_host_connection_handshake(&mut transport, request, &admission_tx)
                .await
                .unwrap();
            let assigned = admission.await.unwrap();
            let mut snapshot = synthetic_join_snapshot(host_core.clone(), 8);
            snapshot.parameters.clients =
                JoinClientRegistrySnapshot::new(vec![host_core, assigned.clone()]);
            transport
                .send_message(ControlMessage::JoinData(Box::new(JoinDataEnvelope {
                    client_id: assigned.client_id,
                    start_control_tick: snapshot.dynamic_tick,
                    status: NetworkStatus {
                        state: NETWORK_STATE_LOBBY,
                        control_mode: 0,
                        target_tick: -1,
                    },
                    dynamic: snapshot.dynamic,
                    parameters: snapshot.parameters,
                })))
                .await
                .unwrap();

            let control_request = timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("client must request its JoinData control tick")
                .unwrap();
            let initial = timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("client must announce addresses after JoinData")
                .unwrap();
            let learned = crate::AddressPacket {
                client_id: 0,
                address: crate::NetworkAddress::new(
                    crate::NetworkProtocol::Tcp,
                    "198.51.100.7:11112".parse().unwrap(),
                ),
            };
            transport
                .send_message(ControlMessage::Address(learned))
                .await
                .unwrap();
            let mut echoed = None;
            for _ in 0..8 {
                let message = timeout(EVENT_WAIT, transport.read_message())
                    .await
                    .expect("client must re-announce a newly learned address")
                    .unwrap();
                if message == ControlMessage::Address(learned) {
                    echoed = Some(message);
                    break;
                }
            }
            let echoed = echoed.expect("client never re-announced the newly learned address");
            (control_request, initial, learned, echoed)
        });

        let client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let (control_request, packet, learned, echoed) = server.await.unwrap();
        assert_eq!(control_request, ControlMessage::Request { from_tick: 0 });
        assert_eq!(
            packet,
            ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                address: crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, addr),
            })
        );
        assert_eq!(echoed, ControlMessage::Address(learned));

        client.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accepted_client_waits_for_a_fresh_published_join_snapshot() {
        // SendJoinData retains an accepted NCS_Joining client when no current
        // dynamic exists. OnGameSynchronized later publishes the fresh
        // dynamic and sends JoinData/Addr without re-running admission
        // (src/C4Network2.cpp:1099-1115,1768-1784,1820-1849).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        let mut needed = false;
        for _ in 0..4 {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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

        host.publish_join_snapshot(snapshot.clone()).await.unwrap();
        let mut client = timeout(EVENT_WAIT, client_task)
            .await
            .expect("published JoinData did not release the client")
            .unwrap()
            .unwrap();
        let join_data = client.take_join_data().unwrap();
        assert_eq!(join_data.dynamic, snapshot.dynamic);
        assert_eq!(join_data.start_control_tick, snapshot.dynamic_tick);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delayed_join_data_is_followed_by_prior_lobby_chat() {
        // SendJoinData may wait for OnGameSynchronized to provide a dynamic
        // (src/C4Network2.cpp:1099-1115,1768-1784,1820-1849). The
        // presentation-only transcript extension must follow that delayed
        // JoinData just as it follows an immediately available one.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::JoinDataNeeded { client_id: 1, .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before JoinData was requested"),
            }
        }

        let message = clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"during delayed join".to_vec())
                .unwrap(),
            by_client: HOST_CLIENT_ID as i32,
        };
        let data = encode_control_entry_payload(&EngineControlPacket::Message(message)).unwrap();
        host.submit_packet(ControlDelivery::Private, data.clone())
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct {
                    client_id: BROADCAST_CLIENT_ID,
                    delivery: ControlDelivery::Private,
                    data: received,
                }) if received == data => break,
                Some(_) => continue,
                None => panic!("host event stream ended before accepting lobby chat"),
            }
        }

        host.publish_join_snapshot(snapshot).await.unwrap();
        let mut client = timeout(EVENT_WAIT, client_task)
            .await
            .expect("published JoinData did not release the client")
            .unwrap()
            .unwrap();
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
        assert_eq!(replayed, Some(data));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_host_route_measures_ping_while_join_data_is_delayed() {
        // C4Network2IO::Execute keeps CheckTimeout/Ping running for every open
        // accepted connection while SendJoinData waits for a fresh dynamic
        // (src/C4Network2IO.cpp:611-623,1155-1191;
        // src/C4Network2.cpp:1107-1133,1836-1865).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::JoinDataNeeded { client_id: 1, .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before JoinData was requested"),
            }
        }

        let ping_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let measured_ping = loop {
            let connections = host.runtime_connections().await.unwrap();
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

        host.publish_join_snapshot(snapshot).await.unwrap();
        let client = timeout(EVENT_WAIT, client_task)
            .await
            .expect("published JoinData did not release the live client")
            .unwrap()
            .unwrap();

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_dynamic_expires_only_after_the_host_advances_past_its_tick() {
        let directories = SessionResourceDirectories::new();
        let config = HostConfig {
            resource_directory: Some(directories.host.clone()),
            ..HostConfig::default()
        };
        let parameters = config
            .initial_join_snapshot
            .as_ref()
            .unwrap()
            .parameters
            .clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut host = start_host(listener, config).await.unwrap();
        let mut events = host.take_event_receiver();

        host.publish_runtime_dynamic(runtime_dynamic_for_session_test(), 0, parameters.clone())
            .await
            .unwrap();
        host.control_tick_reached(
            0,
            1,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .unwrap();
        assert!(
            host.remove_runtime_dynamic().await.unwrap(),
            "the dynamic must remain available at its exact control tick"
        );

        host.publish_runtime_dynamic(runtime_dynamic_for_session_test(), 0, parameters)
            .await
            .unwrap();
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x34))
            .await
            .unwrap();
        wait_for_host_ready_tick(&mut events, 0).await;
        host.control_tick_reached(
            1,
            1,
            DEFAULT_CONTROL_TARGET_FPS,
            tokio::time::Instant::now(),
        )
        .await
        .unwrap();
        assert!(
            !host.remove_runtime_dynamic().await.unwrap(),
            "the host must automatically remove a dynamic once current tick is greater"
        );

        host.shutdown().await.unwrap();
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
                    peer_addr: "127.0.0.1:1111".parse().unwrap(),
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
        .unwrap()
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
        task.await.unwrap();
        assert_eq!(tokio::time::Instant::now(), deadline);
    }

    #[tokio::test(start_paused = true)]
    async fn chase_target_timer_arms_only_when_delayed_join_data_is_sent() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
        host.publish_join_snapshot(snapshot).await.unwrap();
        let mut client = timeout(EVENT_WAIT, &mut client_task)
            .await
            .expect("published JoinData did not release the client")
            .unwrap()
            .unwrap();
        let initial_status = client.take_join_data().unwrap().status;
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn direct_client_join_reaches_already_connected_clients_before_new_join_finishes() {
        // CtrlAdd executes CID_ClientJoin as direct control before the host
        // sends positive ConnRe, so every existing client learns the newcomer
        // before normal synchronized traffic continues
        // (src/C4Network2.cpp:1395-1445; src/C4Control.cpp:554-573).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut alpha_events = alpha.take_event_receiver();
        let beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();

        let data = loop {
            match timeout(EVENT_WAIT, alpha_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    data,
                }) => break data,
                Some(ClientEvent::Ready { .. }) => continue,
                other => panic!("expected direct ClientJoin for Beta, got {other:?}"),
            }
        };
        let clonk_engine::ControlPacket::ClientJoin(join) =
            decode_control_entry_payload(&data).unwrap()
        else {
            panic!("direct packet was not ClientJoin");
        };
        assert_eq!(
            join.core.client_id,
            i32::try_from(beta.client_id()).unwrap()
        );
        assert_eq!(join.core.name.as_bytes(), b"Beta");

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn joining_client_receives_prior_lobby_chat_after_join_data() {
        // C++ sends lobby CID_Message as ephemeral CDT_Private controls only
        // to clients connected at that instant (src/C4MessageInput.cpp:423-425;
        // src/C4GameControlNetwork.cpp:225-237). Retaining those same raw
        // controls for post-JoinData replay fixes the presentation-only gap
        // without changing synchronized state or recipient-side filtering.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let source = connect_client(addr, ClientConfig::new("Source", ParticipantKind::Player))
            .await
            .unwrap();
        let source_id = source.client_id();
        let message = clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"before join".to_vec()).unwrap(),
            by_client: i32::try_from(source_id).unwrap(),
        };
        let data = encode_control_entry_payload(&EngineControlPacket::Message(message)).unwrap();

        source
            .submit_packet(ControlDelivery::Private, data.clone())
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Private,
                    data: received,
                }) if client_id == source_id && received == data => break,
                Some(_) => continue,
                None => panic!("host event stream ended before accepting lobby chat"),
            }
        }

        let mut client = connect_client(addr, ClientConfig::new("Late", ParticipantKind::Player))
            .await
            .unwrap();
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

        client.shutdown().await.unwrap();
        source.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
        assert_eq!(replayed, Some(data));
    }

    #[tokio::test(start_paused = true)]
    async fn two_rust_clients_form_a_known_peer_tcp_mesh_and_keep_host_forwarding_as_fallback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mesh_bind = SocketAddr::from(([127, 0, 0, 1], 0));
        let alpha = connect_client(
            addr,
            ClientConfig::new("Alpha", ParticipantKind::Player)
                .with_mesh_tcp_bind_address(mesh_bind),
        )
        .await
        .unwrap();
        let mut beta = connect_client(
            addr,
            ClientConfig::new("Beta", ParticipantKind::Player)
                .with_mesh_tcp_bind_address(mesh_bind),
        )
        .await
        .unwrap();

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
        .expect("host did not propagate both clients' mesh addresses");
        beta.force_mesh_attempt(alpha.client_id()).await;
        tokio::task::yield_now().await;

        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            let alpha_connected = alpha.mesh_peer_ids().await.contains(&beta.client_id());
            let beta_connected = beta.mesh_peer_ids().await.contains(&alpha.client_id());
            if alpha_connected && beta_connected {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "clients did not establish their direct known-peer route"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(host.accepted_routes().await.len(), 2);

        let mut beta_events = beta.take_event_receiver();
        let alpha_id = alpha.client_id();
        let data =
            encode_control_entry_payload(&EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(alpha_id).unwrap(),
                command: 0x44,
                data: 0x55,
                by_client: i32::try_from(alpha_id).unwrap(),
            }))
            .unwrap();
        alpha
            .submit_packet(ControlDelivery::Direct, data.clone())
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if received == data => break,
                Some(_) => continue,
                None => panic!("beta event stream ended before direct mesh delivery"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if client_id == alpha_id && received == data => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == alpha_id => panic!("mesh fallback failed: {error}"),
                Some(_) => continue,
                None => panic!("host event stream ended before mesh fallback"),
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
                "host fallback duplicated a directly delivered mesh packet"
            );
        }

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn two_rust_clients_form_a_known_peer_udp_mesh_and_keep_host_forwarding_as_fallback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mesh_bind = SocketAddr::from(([127, 0, 0, 1], 0));
        let alpha = connect_client(
            addr,
            ClientConfig::new("Alpha", ParticipantKind::Player)
                .with_mesh_udp_bind_address(mesh_bind),
        )
        .await
        .unwrap();
        let mut beta = connect_client(
            addr,
            ClientConfig::new("Beta", ParticipantKind::Player)
                .with_mesh_udp_bind_address(mesh_bind),
        )
        .await
        .unwrap();

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
        .expect("host did not propagate both clients' UDP mesh addresses");
        beta.force_mesh_attempt(alpha.client_id()).await;

        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            let alpha_connected = alpha.mesh_peer_ids().await.contains(&beta.client_id());
            let beta_connected = beta.mesh_peer_ids().await.contains(&alpha.client_id());
            if alpha_connected && beta_connected {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "clients did not establish their direct known-peer UDP route"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(host.accepted_routes().await.len(), 2);

        let mut beta_events = beta.take_event_receiver();
        let alpha_id = alpha.client_id();
        let data =
            encode_control_entry_payload(&EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(alpha_id).unwrap(),
                command: 0x66,
                data: 0x77,
                by_client: i32::try_from(alpha_id).unwrap(),
            }))
            .unwrap();
        alpha
            .submit_packet(ControlDelivery::Direct, data.clone())
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if received == data => break,
                Some(_) => continue,
                None => panic!("beta event stream ended before direct UDP mesh delivery"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if client_id == alpha_id && received == data => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == alpha_id => panic!("UDP mesh fallback failed: {error}"),
                Some(_) => continue,
                None => panic!("host event stream ended before UDP mesh fallback"),
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
                "host fallback duplicated a directly delivered UDP mesh packet"
            );
        }

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connecting_without_pending_sync_emits_no_exec_sync_marker() {
        // PID_ExecSyncCtrl is emitted only when SyncControl is non-empty;
        // connection establishment is not a synchronization release
        // (src/C4GameControlNetwork.cpp:260-276).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut events = client.take_event_receiver();

        assert!(timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err());

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_and_ack_round_trip_over_real_tcp() {
        // PID_Status is host-authored; a client answers with PID_StatusAck and
        // the host later broadcasts the final ACK
        // (src/C4Network2.cpp:1501-1534,1994-2012,2062-2077).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let client_id = client.client_id();
        let mut client_events = client.take_event_receiver();
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 195_995,
        };

        host.change_status(status).await.expect("broadcast status");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host status request wait")
            {
                Some(HostEvent::StatusChanged(requested)) => {
                    assert_eq!(requested, status);
                    break;
                }
                Some(HostEvent::ClientJoined { .. }) | Some(HostEvent::Direct { .. }) => continue,
                other => panic!("expected host status request event, got {other:?}"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client status wait")
            {
                Some(ClientEvent::Status(received)) => {
                    assert_eq!(received, status);
                    break;
                }
                Some(ClientEvent::Ready { .. }) | Some(ClientEvent::Direct { .. }) => continue,
                other => panic!("expected client status event, got {other:?}"),
            }
        }

        client
            .submit_status_ack(status)
            .await
            .expect("submit status ack");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host status ack wait")
            {
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
            .expect("host reached status target");
        match timeout(EVENT_WAIT, client_events.recv())
            .await
            .expect("client final status ack wait")
        {
            Some(ClientEvent::StatusAck(received)) => assert_eq!(received, status),
            other => panic!("expected client final status ack, got {other:?}"),
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host status commit wait")
            {
                Some(HostEvent::StatusCommitted(committed)) => {
                    assert_eq!(committed, status);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before status commit"),
            }
        }

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(start_paused = true)]
    async fn host_chase_target_updates_only_chasing_clients_and_stops_after_ack() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.unwrap();
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
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::JoinDataNeeded { client_id, .. }) => {
                    waiting_clients.insert(client_id);
                }
                Some(_) => continue,
                None => panic!("host event stream ended before both clients needed JoinData"),
            }
        }

        let first_deadline = tokio::time::Instant::now() + CHASE_TARGET_UPDATE_INTERVAL;
        host.publish_join_snapshot(snapshot).await.unwrap();
        let mut alpha = timeout(EVENT_WAIT, alpha_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut beta = timeout(EVENT_WAIT, beta_task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let alpha_id = alpha.client_id();
        let initial_status = alpha.take_join_data().unwrap().status;
        let mut alpha_events = alpha.take_event_receiver();
        let beta_id = beta.client_id();
        assert_eq!(beta.take_join_data().unwrap().status, initial_status);
        let mut beta_events = beta.take_event_receiver();

        beta.submit_status_ack(initial_status).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
            .unwrap();
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
        let beta_barrier = ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: crate::ReadyCheckData::Other(101),
        };
        host.submit_ready_check(beta_barrier).await.unwrap();
        assert_no_client_status_through_ready_check(&mut beta_events, beta_barrier).await;

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0x32))
            .await
            .unwrap();
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
        let second_beta_barrier = ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: crate::ReadyCheckData::Other(102),
        };
        host.submit_ready_check(second_beta_barrier).await.unwrap();
        assert_no_client_status_through_ready_check(&mut beta_events, second_beta_barrier).await;

        alpha.submit_status_ack(second_update).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
        let stopped_barrier = ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: crate::ReadyCheckData::Other(103),
        };
        host.submit_ready_check(stopped_barrier).await.unwrap();
        assert_no_client_status_through_ready_check(&mut alpha_events, stopped_barrier).await;
        assert_no_client_status_through_ready_check(&mut beta_events, stopped_barrier).await;

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn central_host_replays_contiguous_local_controls_after_decentral_commit() {
        // SetCtrlMode(CNM_Decentral) runs only after the final StatusAck and
        // resends the host's own stored controls from ControlTick until the
        // first gap (src/C4Network2.cpp:2062-2110;
        // src/C4GameControlNetwork.cpp:360-374).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        config.initial_status.control_mode = 1;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"Alice").await;
        activate_joined_client(&host, &mut host_events, client_id).await;
        drain_raw_client(&mut client).await;
        acknowledge_raw_status(
            &mut client,
            &mut host_events,
            client_id,
            NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 1,
                target_tick: -1,
            },
        )
        .await;
        drain_raw_client(&mut client).await;

        let first = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let after_gap = legacy_packet(HOST_CLIENT_ID, 2, 0x13);
        host.submit_local_control(first.clone()).await.unwrap();
        host.submit_local_control(after_gap.clone()).await.unwrap();

        let decentral = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 0,
        };
        host.change_status(decentral).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .unwrap()
            {
                ControlMessage::Status(status) if status == decentral => break,
                _ => continue,
            }
        }
        client
            .send_message(ControlMessage::StatusAck(decentral))
            .await
            .unwrap();
        host.status_reached(decentral, decentral.target_tick)
            .await
            .unwrap();

        let mut saw_final_ack = false;
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .unwrap()
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn decentral_host_replays_complete_controls_after_central_commit() {
        // SetCtrlMode(CNM_Central) follows the final StatusAck and the host
        // resends stored complete C4ClientIDAll controls, not one participant's
        // contribution (src/C4Network2.cpp:2062-2110;
        // src/C4GameControlNetwork.cpp:360-374).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"Alice").await;
        activate_joined_client(&host, &mut host_events, client_id).await;
        drain_raw_client(&mut client).await;
        acknowledge_raw_status(
            &mut client,
            &mut host_events,
            client_id,
            NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 0,
                target_tick: -1,
            },
        )
        .await;
        drain_raw_client(&mut client).await;

        let host_packet = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        host.submit_local_control(host_packet.clone())
            .await
            .unwrap();
        assert!(raw_client_received_control(&mut client, &host_packet, EVENT_WAIT,).await);
        let client_packet = legacy_packet(client_id, 0, 0x21);
        client
            .send_message(ControlMessage::Control(client_packet))
            .await
            .unwrap();
        let complete = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(complete.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&complete), vec![0x11, 0x21]);
        drain_raw_client(&mut client).await;

        let central = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(central).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .unwrap()
            {
                ControlMessage::Status(status) if status == central => break,
                _ => continue,
            }
        }
        client
            .send_message(ControlMessage::StatusAck(central))
            .await
            .unwrap();
        host.status_reached(central, central.target_tick)
            .await
            .unwrap();

        let mut saw_final_ack = false;
        loop {
            match timeout(EVENT_WAIT, client.read_message())
                .await
                .unwrap()
                .unwrap()
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
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn higher_client_status_ack_retargets_real_tcp_barrier_before_commit() {
        // CheckStatusReached replaces a client's requested target with its
        // current control tick. HandleStatusAck must rebroadcast that higher
        // target before the barrier can commit
        // (src/C4Network2.cpp:1994-2012,2062-2077).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let client_id = client.client_id();
        let initial_status = client.take_join_data().expect("client JoinData").status;
        let mut client_events = client.take_event_receiver();

        // Send the JoinData status acknowledgement first so the host advances
        // this client from Chasing to Ready before opening a fresh barrier.
        client
            .submit_status_ack(initial_status)
            .await
            .expect("acknowledge JoinData status");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host initial status ack wait")
            {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status,
                }) if received_id == client_id && status == initial_status => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client initial status ack wait")
            {
                Some(ClientEvent::StatusAck(status)) if status == initial_status => break,
                Some(_) => continue,
                None => panic!("client event stream ended before initial status ack"),
            }
        }

        let requested = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 41,
        };
        host.change_status(requested)
            .await
            .expect("broadcast requested Pause");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host requested Pause event wait")
            {
                Some(HostEvent::StatusChanged(status)) if status == requested => break,
                Some(_) => continue,
                None => panic!("host event stream ended before requested Pause event"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client requested Pause wait")
            {
                Some(ClientEvent::Status(status)) if status == requested => break,
                Some(_) => continue,
                None => panic!("client event stream ended before requested Pause"),
            }
        }

        let retargeted = NetworkStatus {
            target_tick: 44,
            ..requested
        };
        client
            .submit_status_ack(retargeted)
            .await
            .expect("submit retargeted Pause acknowledgement");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host retargeted status ack wait")
            {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status,
                }) if received_id == client_id && status == retargeted => break,
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host retargeted Pause event wait")
            {
                Some(HostEvent::StatusChanged(status)) if status == retargeted => break,
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted Pause event"),
            }
        }

        match timeout(EVENT_WAIT, client_events.recv())
            .await
            .expect("client retargeted Pause wait")
        {
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
            .expect("host reached retargeted Pause");
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client final retargeted status ack wait")
            {
                Some(ClientEvent::StatusAck(status)) => {
                    assert_eq!(status, retargeted);
                    break;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before final retargeted status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host retargeted status commit wait")
            {
                Some(HostEvent::StatusCommitted(status)) => {
                    assert_eq!(status, retargeted);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted status commit"),
            }
        }

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_controls_wait_for_status_barrier_and_keep_fifo_order() {
        // In running games, CDT_Sync packets accumulate in SyncControl and do
        // not execute until PID_ExecSyncCtrl is emitted after the status
        // barrier (src/C4GameControlNetwork.cpp:181-220,260-297,558-588).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut client_events = client.take_event_receiver();

        let running = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(running)
            .await
            .expect("enter running status");
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("initial Go status wait")
            {
                Some(ClientEvent::Status(status)) => {
                    assert_eq!(status, running);
                    break;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before initial Go"),
            }
        }
        client
            .submit_status_ack(running)
            .await
            .expect("acknowledge initial Go");
        host.status_reached(running, running.target_tick)
            .await
            .expect("host reached initial Go");
        let mut host_running = false;
        let mut client_running = false;
        while !host_running || !client_running {
            if !host_running {
                match timeout(EVENT_WAIT, host_events.recv())
                    .await
                    .expect("host initial Go commit wait")
                {
                    Some(HostEvent::StatusCommitted(status)) => {
                        assert_eq!(status, running);
                        host_running = true;
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended before initial Go commit"),
                }
            }
            if !client_running {
                match timeout(EVENT_WAIT, client_events.recv())
                    .await
                    .expect("client initial Go ack wait")
                {
                    Some(ClientEvent::StatusAck(status)) => {
                        assert_eq!(status, running);
                        client_running = true;
                    }
                    Some(_) => {}
                    None => panic!("client event stream ended before initial Go ack"),
                }
            }
        }

        let first = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 0x41,
            data: 0,
            by_client: 0,
        });
        let second = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 0x42,
            data: 0,
            by_client: 0,
        });
        for control in [&first, &second] {
            host.submit_packet(
                ControlDelivery::Sync,
                encode_control_entry_payload(control).expect("encode sync control"),
            )
            .await
            .expect("submit sync control");
        }

        let sync_status = loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client synchronization status wait")
            {
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
            .expect("submit client tick");
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x22))
            .await
            .expect("submit host tick");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host ready wait")
            {
                Some(HostEvent::Ready { .. }) => break,
                Some(HostEvent::SyncScheduled { .. }) => {
                    panic!("host released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client ready wait")
            {
                Some(ClientEvent::Ready { .. }) => break,
                Some(ClientEvent::SyncScheduled { .. }) => {
                    panic!("client released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before ready"),
            }
        }

        client
            .submit_status_ack(sync_status)
            .await
            .expect("acknowledge synchronization status");
        host.status_reached(sync_status, sync_status.target_tick)
            .await
            .expect("host reached synchronization target");
        let mut host_controls = None;
        let mut host_committed = false;
        while host_controls.is_none() || !host_committed {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host sync release wait")
            {
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
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client sync release wait")
            {
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

        host.submit_exec_sync(2)
            .await
            .expect("empty sync release is accepted");
        assert!(timeout(Duration::from_millis(50), host_events.recv())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(50), client_events.recv())
            .await
            .is_err());

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_control_executes_immediately_in_frozen_lobby() {
        // Lobby is frozen without a status round trip, so the host executes a
        // CDT_Sync control immediately and then emits PID_ExecSyncCtrl
        // (src/C4Network2.cpp:1982-1991;
        // src/C4GameControlNetwork.cpp:204-213).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut client_events = client.take_event_receiver();
        let control = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 0x51,
            data: 0,
            by_client: 0,
        });

        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&control).expect("encode lobby sync control"),
        )
        .await
        .expect("submit lobby sync control");

        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host frozen sync wait")
            {
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
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client frozen sync wait")
            {
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

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_matches_cpp_pid_control_source_id_semantics() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"spoof-check").await;
        activate_joined_client(&host, &mut host_events, client_id).await;
        drain_raw_client(&mut client).await;
        let spoofed = legacy_packet(HOST_CLIENT_ID, 0, 0x66);
        client
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&spoofed).unwrap(),
            }))
            .await
            .expect("submit spoofed host control");
        raw_client_ping_barrier(&mut client).await;
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .expect("submit real host control");
        let contribution = legacy_packet(client_id, 0, 0x22);
        client
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&contribution)
                    .unwrap(),
            }))
            .await
            .expect("submit real client control");

        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host event wait")
            {
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
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_silently_ignores_valid_control_from_an_unregistered_client() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
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
                    0x60 + i32::try_from(tick).unwrap(),
                )))
                .await
                .expect("submit inactive control");
        }
        raw_client_ping_barrier(&mut client).await;
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .expect("submit host control");

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
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_typed_unpack_closes_only_the_malformed_control_route() {
        // PID_Control typed unpack includes its nested control list. A
        // compiler failure closes only pConn in release builds; the network
        // scheduler remains available for later clients
        // (src/C4GameControlNetwork.cpp:867-872;
        // src/C4Network2IO.cpp:822-835).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
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
            .expect("submit malformed inactive control");
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
        .expect("timed out waiting for malformed-route cleanup");

        drop(client);
        let (mut successor, _) = raw_client_transport(addr, b"after-malformed").await;
        raw_client_ping_barrier(&mut successor).await;
        drop(successor);
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forged_queued_control_set_author_does_not_consume_the_tick() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let client = connect_client(
            addr,
            ClientConfig::new("set-spoof-check", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        let client_id = client.client_id();
        activate_joined_client(&host, &mut host_events, client_id).await;
        let client_author = i32::try_from(client_id).expect("test client id fits i32");
        let queued_set = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id,
                tick: 0,
                timestamp_ms: 0,
                controls: vec![crate::LegacyControlSet {
                    value_type: 5,
                    data: 10_000,
                    by_client,
                }
                .into_control_packet()],
            })
            .expect("encode queued CID_Set")
        };

        client
            .submit_control(queued_set(0))
            .await
            .expect("submit forged host-authored Set");
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .expect("submit host contribution");
        client
            .submit_control(queued_set(client_author))
            .await
            .expect("replace with authenticated Set");

        let mut saw_rejection = false;
        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host event wait")
            {
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
        let frame = decode_control_packet(&ready).expect("ready packet remains decodable");
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

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_contribution_does_not_consume_the_synchronized_tick() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let client = connect_client(
            addr,
            ClientConfig::new("validation-check", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x22))
            .await
            .expect("submit valid client control");
        let valid_host = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let mut malformed_payload = valid_host.payload().to_vec();
        *malformed_payload.last_mut().expect("control terminator") = 0x7f;
        let malformed_host = ControlPacket::builder(HOST_CLIENT_ID, 0).payload(malformed_payload);
        host.submit_local_control(malformed_host)
            .await
            .expect("submit malformed host control");
        host.submit_local_control(valid_host)
            .await
            .expect("replace malformed host control");

        let mut saw_validation_error = false;
        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host event wait")
            {
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

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_sync_and_reconnect_smoke() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let config = HostConfig {
            max_players: 4,
            ..Default::default()
        };
        let mut host = start_host(listener, config.clone())
            .await
            .expect("start host");

        let mut client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .expect("connect client");

        let mut host_events = host.take_event_receiver();
        let mut client_events = client.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;

        submit_control_pair(&mut host, &client, 0, 0xAA, 0x11).await;

        let first_host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(first_host_ready.tick(), 0);

        let first_client_ready = wait_for_client_ready(&mut client_events, EVENT_WAIT).await;
        assert_eq!(first_client_ready.tick(), 0);

        client.shutdown().await.expect("client shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;
        let mut fresh_snapshot = config.initial_join_snapshot.unwrap();
        fresh_snapshot.dynamic_tick = 1;
        host.publish_join_snapshot(fresh_snapshot)
            .await
            .expect("publish runtime-join dynamic");

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect second client");
        let mut client_beta_events = client_beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client_beta.client_id()).await;

        submit_control_pair(&mut host, &client_beta, 1, 0xBB, 0x22).await;

        let second_host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(second_host_ready.tick(), 1);

        let second_client_ready = wait_for_client_ready(&mut client_beta_events, EVENT_WAIT).await;
        assert_eq!(second_client_ready.tick(), 1);

        client_beta
            .shutdown()
            .await
            .expect("second client shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_continues_ready_after_client_disconnect() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(
            listener,
            HostConfig {
                max_players: 4,
                ..Default::default()
            },
        )
        .await
        .expect("start host");

        let client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .expect("connect client");

        let mut host_events = host.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        submit_control_pair(&mut host, &client, 0, 0xA0, 0xB0).await;
        let ready0 = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready0.tick(), 0);
        assert_eq!(control_commands(&ready0), vec![0xA0, 0xB0]);

        let host_packet = legacy_packet(0, 1, 0xC0);
        host.submit_local_control(host_packet)
            .await
            .expect("host submit control");

        client.shutdown().await.expect("client shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        let ready1 = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready1.tick(), 1);
        assert_eq!(control_commands(&ready1), vec![0xC0]);

        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn running_disconnect_executes_remove_at_the_retargeted_boundary() {
        // CtrlRemove is synchronized, but OnClientDisconnect immediately
        // retargets the unreached Go barrier to ControlTick. The removal then
        // executes before control packing resumes, so the disconnected
        // client's buffered contribution is no longer part of the batch
        // (src/C4Network2.cpp:1786-1807;
        // src/C4GameControlNetwork.cpp:260-297,329-345,741-783).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut client_events = client.take_event_receiver();
        let client_id = client.client_id();
        activate_joined_client(&host, &mut host_events, client_id).await;

        let running = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(running).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::Status(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("client event stream ended before Go"),
            }
        }
        client.submit_status_ack(running).await.unwrap();
        host.status_reached(running, running.target_tick)
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before Go committed"),
            }
        }

        client
            .submit_control(legacy_packet(client_id, 0, 0xB0))
            .await
            .unwrap();
        client.graceful_part().await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ClientLeft { client_id: left }) if left == client_id => break,
                Some(_) => continue,
                None => panic!("host event stream ended before client departure"),
            }
        }
        host.status_reached(running, running.target_tick)
            .await
            .unwrap();

        let mut synchronized_remove = None;
        let mut committed = false;
        while synchronized_remove.is_none() || !committed {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
        let controls = synchronized_remove.unwrap();
        let [EngineControlPacket::ClientRemove(remove)] = controls.as_slice() else {
            panic!("expected one synchronized ClientRemove, got {controls:?}");
        };
        assert_eq!(remove.client_id, i32::try_from(client_id).unwrap());
        assert_eq!(remove.by_client, i32::try_from(HOST_CLIENT_ID).unwrap());

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .unwrap();
        let boundary = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(boundary.tick(), 0);
        assert_eq!(control_commands(&boundary), vec![0xA0]);

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0xA1))
            .await
            .unwrap();
        let released = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(released.tick(), 1);
        assert_eq!(control_commands(&released), vec![0xA1]);

        host.shutdown().await.unwrap();
    }

    #[test]
    fn unreached_pause_disconnect_retry_waits_for_runtime_local_reach() {
        let pause = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 12,
        };
        let mut barrier = StatusBarrier::stable(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 3,
        });
        barrier.set_remote_state(7, RemoteBarrierState::Ready);
        barrier.change_status(pause);

        let retargeted = NetworkStatus {
            target_tick: 4,
            ..pause
        };
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
        let initial_go = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 12,
        };
        let mut barrier = StatusBarrier::stable(NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: -1,
        });
        barrier.change_status(initial_go);

        let retargeted = NetworkStatus {
            target_tick: 4,
            ..initial_go
        };
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();

        let mut alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let alpha_id = alpha.client_id();
        let mut alpha_events = alpha.take_event_receiver();
        activate_joined_client(&host, &mut host_events, alpha_id).await;

        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let beta_id = beta.client_id();
        let mut beta_events = beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, beta_id).await;

        let running = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(running).await.unwrap();
        for events in [&mut alpha_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
                    Some(ClientEvent::Status(status)) if status == running => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("client disconnected before initial Go: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("client event stream ended before initial Go"),
                }
            }
        }
        alpha.submit_status_ack(running).await.unwrap();
        beta.submit_status_ack(running).await.unwrap();
        host.status_reached(running, running.target_tick)
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial Go committed"),
            }
        }

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .unwrap();
        alpha
            .submit_control(legacy_packet(alpha_id, 0, 0xB0))
            .await
            .unwrap();
        beta.submit_control(legacy_packet(beta_id, 0, 0xC0))
            .await
            .unwrap();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0xA0, 0xB0, 0xC0]);

        let unreachable = NetworkStatus {
            target_tick: 2,
            ..running
        };
        host.change_status(unreachable).await.unwrap();
        for events in [&mut alpha_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
                    Some(ClientEvent::Status(status)) if status == unreachable => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("client disconnected before unreached Go: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("client event stream ended before unreached Go"),
                }
            }
        }

        beta.submit_status_ack(unreachable).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
            .unwrap();
        beta.submit_control(legacy_packet(beta_id, 1, 0xC1))
            .await
            .unwrap();

        alpha.shutdown().await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ClientLeft { client_id }) if client_id == alpha_id => break,
                Some(_) => continue,
                None => panic!("host event stream ended before Alpha departed"),
            }
        }

        let retargeted = NetworkStatus {
            target_tick: 1,
            ..unreachable
        };
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
                Some(ClientEvent::Status(status)) if status == retargeted => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("Beta disconnected before the retry: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("Beta event stream ended before the retry"),
            }
        }

        beta.submit_status_ack(retargeted).await.unwrap();
        host.status_reached(retargeted, retargeted.target_tick)
            .await
            .unwrap();

        let mut released = None;
        let mut synchronized_remove = None;
        let mut committed = false;
        while released.is_none() || synchronized_remove.is_none() || !committed {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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

        let released = released.unwrap();
        assert_eq!(control_commands(&released), vec![0xA1, 0xC1]);
        let controls = synchronized_remove.unwrap();
        let [EngineControlPacket::ClientRemove(remove)] = controls.as_slice() else {
            panic!("expected one synchronized ClientRemove, got {controls:?}");
        };
        assert_eq!(remove.client_id, i32::try_from(alpha_id).unwrap());
        assert_eq!(remove.by_client, i32::try_from(HOST_CLIENT_ID).unwrap());

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 2, 0xA2))
            .await
            .unwrap();
        beta.submit_control(legacy_packet(beta_id, 2, 0xC2))
            .await
            .unwrap();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 2);
        assert_eq!(control_commands(&ready), vec![0xA2, 0xC2]);

        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnect_broadcasts_host_authored_synchronized_client_remove() {
        // OnClientDisconnect calls C4ClientList::CtrlRemove, which broadcasts
        // a host-authored CDT_Sync ClientRemove and executes it at the frozen
        // synchronization boundary (src/C4Network2.cpp:1786-1802;
        // src/C4Client.cpp:293-303;
        // src/C4GameControlNetwork.cpp:181-220).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let alpha_id = alpha.client_id();
        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta_events = beta.take_event_receiver();

        alpha.shutdown().await.unwrap();
        let remove = loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
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

        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_half_accepted_join_is_removed_from_existing_clients() {
        // Join creates/broadcasts ClientJoin before mutual ConnRe. If the
        // socket then fails, OnConnectFail routes the provisional client
        // through the same synchronized CtrlRemove path
        // (src/C4Network2.cpp:1395-1445,1745-1755;
        // src/C4Client.cpp:293-303).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut witness = connect_client(
            addr,
            ClientConfig::new("Witness", ParticipantKind::Observer),
        )
        .await
        .unwrap();
        let mut witness_events = witness.take_event_receiver();

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut failed = crate::ControlTransport::new(stream);
        let _ = failed.read_message().await.unwrap();
        let name = clonk_engine::LegacyCString::from_bytes(b"HalfJoin".to_vec()).unwrap();
        failed
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id: -1,
                        name: name.clone(),
                        nick: name,
                        ..Default::default()
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: 77,
                },
            ))
            .await
            .unwrap();
        loop {
            match failed.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    failed
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }
        drop(failed);

        let mut provisional_id = None;
        loop {
            match timeout(EVENT_WAIT, witness_events.recv()).await.unwrap() {
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

        witness.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_admission_failure_retargets_unreached_go_and_executes_remove() {
        // A failed half-accepted runtime join has already broadcast ClientJoin
        // before its socket disappears. OnConnectFail queues the matching
        // ClientRemove and performs the same unreached Go/Pause retry as an
        // established-client disconnect (src/C4Network2.cpp:1745-1755,
        // 1786-1807; src/C4Client.cpp:293-303).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut witness =
            connect_client(addr, ClientConfig::new("Witness", ParticipantKind::Player))
                .await
                .unwrap();
        let witness_id = witness.client_id();
        let mut witness_events = witness.take_event_receiver();
        activate_joined_client(&host, &mut host_events, witness_id).await;

        let running = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(running).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, witness_events.recv()).await.unwrap() {
                Some(ClientEvent::Status(status)) if status == running => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected before initial Go: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("witness event stream ended before initial Go"),
            }
        }
        witness.submit_status_ack(running).await.unwrap();
        host.status_reached(running, running.target_tick)
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial Go committed"),
            }
        }

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .unwrap();
        witness
            .submit_control(legacy_packet(witness_id, 0, 0xB0))
            .await
            .unwrap();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 0);
        assert_eq!(control_commands(&ready), vec![0xA0, 0xB0]);

        let unreachable = NetworkStatus {
            target_tick: 2,
            ..running
        };
        host.change_status(unreachable).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, witness_events.recv()).await.unwrap() {
                Some(ClientEvent::Status(status)) if status == unreachable => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected before unreached Go: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("witness event stream ended before unreached Go"),
            }
        }
        witness.submit_status_ack(unreachable).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::StatusAck { client_id, status })
                    if client_id == witness_id && status == unreachable =>
                {
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before unreached Go acknowledgement"),
            }
        }

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut failed = crate::ControlTransport::new(stream);
        assert!(matches!(
            failed.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = clonk_engine::LegacyCString::from_bytes(b"HalfJoin".to_vec()).unwrap();
        failed
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id: -1,
                        name: name.clone(),
                        nick: name,
                        ..Default::default()
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: 77,
                },
            ))
            .await
            .unwrap();
        loop {
            match failed.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    failed
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }

        let provisional_id = loop {
            match timeout(EVENT_WAIT, witness_events.recv()).await.unwrap() {
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

        let retargeted = NetworkStatus {
            target_tick: 1,
            ..unreachable
        };
        loop {
            match timeout(EVENT_WAIT, witness_events.recv()).await.unwrap() {
                Some(ClientEvent::Status(status)) if status == retargeted => break,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected before admission retry: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("witness event stream ended before admission retry"),
            }
        }
        witness.submit_status_ack(retargeted).await.unwrap();
        host.status_reached(retargeted, retargeted.target_tick)
            .await
            .unwrap();

        let mut synchronized_remove = None;
        let mut committed = false;
        let mut connection_failed = false;
        let mut diagnostic = false;
        while synchronized_remove.is_none() || !committed || !connection_failed || !diagnostic {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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
        let controls = synchronized_remove.unwrap();
        let [EngineControlPacket::ClientRemove(remove)] = controls.as_slice() else {
            panic!("expected one synchronized provisional ClientRemove, got {controls:?}");
        };
        assert_eq!(remove.client_id, provisional_id);
        assert_eq!(remove.by_client, i32::try_from(HOST_CLIENT_ID).unwrap());

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0xA1))
            .await
            .unwrap();
        witness
            .submit_control(legacy_packet(witness_id, 1, 0xB1))
            .await
            .unwrap();
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 1);
        assert_eq!(control_commands(&ready), vec![0xA1, 0xB1]);

        witness.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_secondary_known_connection_keeps_the_canonical_client() {
        // OnConnectFail removes a half-accepted client only when that client has
        // no other connection. Losing a secondary route therefore leaves the
        // already-connected canonical client registered
        // (src/C4Network2.cpp:1366-1380,1745-1765).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut canonical =
            connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut secondary = crate::ControlTransport::new(stream);
        assert!(matches!(
            secondary.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        secondary
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id: i32::try_from(canonical_id).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: 29,
                },
            ))
            .await
            .unwrap();
        loop {
            match secondary.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }
        drop(secondary);

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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

        canonical.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_client_starts_at_fresh_dynamic_tick_without_old_backlog() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let config = HostConfig {
            max_players: 4,
            ..Default::default()
        };
        let mut host = start_host(listener, config.clone())
            .await
            .expect("start host");

        let mut host_events = host.take_event_receiver();
        let client_alpha =
            connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
                .await
                .expect("connect alpha client");
        activate_joined_client(&host, &mut host_events, client_alpha.client_id()).await;

        submit_control_pair(&mut host, &client_alpha, 0, 0xA1, 0xB2).await;
        let ready_packet = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready_packet.tick(), 0);

        // A runtime join receives a dynamic snapshot for the next control tick.
        // C++ sends no eager backlog after JoinData; Init requests exactly the
        // snapshot tick, so controls already represented by the dynamic must
        // not replay (src/C4Network2.cpp:1820-1850;
        // src/C4GameControlNetwork.cpp:46-62,531-555).
        client_alpha.shutdown().await.expect("alpha shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;
        let mut fresh_snapshot = config.initial_join_snapshot.unwrap();
        fresh_snapshot.dynamic_tick = 1;
        host.publish_join_snapshot(fresh_snapshot)
            .await
            .expect("publish fresh dynamic");

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect beta client");
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

        client_beta.shutdown().await.expect("beta shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_resends_backlog_when_requested() {
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let client_handle = tokio::spawn(super::run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        let packet = legacy_packet(7, 42, 0xDE);
        command_tx
            .send(ClientCommand::SubmitControl(packet.clone()))
            .await
            .expect("submit control");

        match host_transport
            .read_message()
            .await
            .expect("receive control")
        {
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
            .expect("send request");

        match host_transport.read_message().await.expect("receive resend") {
            ControlMessage::Control(resend) => {
                assert_eq!(resend.client_id(), packet.client_id());
                assert_eq!(resend.tick(), packet.tick());
                assert_eq!(resend.payload(), packet.payload());
            }
            other => panic!("expected resend control packet, got {other:?}"),
        }

        shutdown_tx.send(()).ok();
        client_handle.await.expect("client loop exited");
    }

    #[tokio::test(start_paused = true)]
    async fn central_client_repeats_missing_control_request_on_cpp_interval() {
        async fn next_request<S>(transport: &mut crate::ControlTransport<S>) -> ControlMessage
        where
            S: AsyncRead + AsyncWrite + Unpin,
        {
            loop {
                match transport.read_message().await.unwrap() {
                    request @ ControlMessage::Request { .. } => return request,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
                    }
                    other => panic!("expected control request, got {other:?}"),
                }
            }
        }

        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 1,
        };

        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        assert!(
            matches!(event_rx.recv().await, Some(ClientEvent::Status(value)) if value == status)
        );

        tokio::time::advance(Duration::from_secs(1)).await;
        let future = legacy_packet(BROADCAST_CLIENT_ID, 1, 0x31);
        host_transport
            .send_message(ControlMessage::Control(future.clone()))
            .await
            .unwrap();
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
            .unwrap();
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
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_graceful_part_sends_exact_cpp_removal_frame_before_close() {
        // C4Network2ClientList::DeleteClient asks CloseConns to send a negative
        // PID_ConnRe with "removing client" before closing the connection
        // (src/C4Network2Client.cpp:104-119,457-492).
        let (client_stream, mut host_stream) = duplex(128);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let handle = ClientHandle {
            command_tx,
            control_send_time: test_control_send_time_snapshot(),
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle: tokio::spawn(run_client_loop(
                crate::ControlTransport::new(client_stream),
                command_rx,
                event_tx,
                shutdown_rx,
            )),
            client_id: 1,
            join_data: None,
            io_statistics: crate::NetworkIoStatistics::new(0),
        };

        handle.graceful_part().await.expect("graceful client part");

        let mut bytes = Vec::new();
        host_stream.read_to_end(&mut bytes).await.unwrap();
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
        let (client_stream, host_stream) = duplex(128);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: false,
                message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec())
                    .unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Disconnected { reason: Some(reason) })
                if reason == "removing client"
        ));
        timeout(EVENT_WAIT, client_loop)
            .await
            .expect("client loop did not close after host removal")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_still_rejects_positive_post_admission_connre_as_duplicate() {
        // A positive ConnRe only completes connection admission. Receiving a
        // second positive reply after admission is not the CloseConns removal
        // signal (src/C4Network2.cpp:1448-1474).
        let (client_stream, host_stream) = duplex(128);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: clonk_engine::LegacyCString::from_bytes(b"duplicate".to_vec()).unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Disconnected { reason: Some(reason) })
                if reason == "host sent a duplicate connection reply"
        ));
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_reports_negative_post_admission_connre_once_before_eof() {
        // CloseConns writes the negative PID_ConnRe and immediately closes the
        // socket; the accepted connection must therefore report one removal,
        // not another disconnect when that close becomes EOF
        // (src/C4Network2Client.cpp:104-119,457-492).
        let (host_stream, client_stream) = duplex(128);
        let (_outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = _outbound_tx.subscribe_retire();
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
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
        let mut client_transport = crate::ControlTransport::new(client_stream);

        client_transport
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: false,
                message: clonk_engine::LegacyCString::from_bytes(b"removing client".to_vec())
                    .unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();
        drop(client_transport);
        task.await.unwrap();

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
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel();
        let retire_rx = outbound_tx.subscribe_retire();
        let (host_tx, mut host_rx) = mpsc::unbounded_channel();
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
        let mut client_transport = crate::ControlTransport::new(client_stream);
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: -1,
        };

        outbound_tx
            .send(ControlMessage::Status(status))
            .await
            .unwrap();
        assert_eq!(
            client_transport.read_message().await.unwrap(),
            ControlMessage::Status(status)
        );
        drop(client_transport);
        task.await.unwrap();

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
        let first = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 7,
        };
        let second = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 2,
            target_tick: 8,
        };
        outbound_tx
            .send(ControlMessage::Status(first))
            .await
            .unwrap();
        outbound_tx
            .send(ControlMessage::Status(second))
            .await
            .unwrap();
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
        task.await.unwrap();
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
        let failed_route = state.accepted_routes.get_mut(&1).unwrap();
        failed_route.remote_connection_id = 11;
        failed_route.protocol = crate::NetworkProtocol::Udp;
        state.accepted_routes.insert(
            2,
            AcceptedConnectionRoute {
                client_id,
                remote_connection_id: 12,
                peer_addr: "127.0.0.1:11112".parse().unwrap(),
                protocol: crate::NetworkProtocol::Tcp,
                ping: RoutePingLag::default(),
                outbound: fallback,
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
        let first = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 51,
        };
        let second = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 52,
        };

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
        .expect("forced host writer failure did not close its route queue");
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
        }) = timeout(EVENT_WAIT, host_rx.recv())
            .await
            .expect("failed host route did not report disconnection")
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
                .expect("host fallback route closed")
            {
                HostOutboundMessage::Message(ControlMessage::PostMortem(packet)) => {
                    logical_order.extend(packet.packets.into_iter().map(|packet| {
                        crate::transport::parse_complete_packet(&packet)
                            .unwrap()
                            .expect("post-mortem entry is typed")
                    }));
                }
                HostOutboundMessage::Message(message) => logical_order.push(message),
                HostOutboundMessage::Raw(packet) => {
                    logical_order.push(
                        crate::transport::parse_complete_packet(&packet)
                            .unwrap()
                            .expect("raw fallback entry is typed"),
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

        route_task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn graceful_client_part_emits_one_host_departure_with_cpp_reason() {
        // DeleteClient closes the accepted peer with "removing client"; the
        // receiving network owns one disconnect notification even though EOF
        // follows the ConnRe frame (src/C4Network2Client.cpp:104-119,457-492).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let client_id = client.client_id();

        client.graceful_part().await.unwrap();

        let mut departures = 0;
        let mut saw_reason = false;
        while departures == 0 || !saw_reason {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
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

        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_lobby_countdown_without_disconnecting() {
        // MainDlg receives every PID_LobbyCountdown and updates its local
        // countdown state; the packet does not close the connection
        // (src/C4GameLobby.cpp:392-418,695-701).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let packet = crate::LobbyCountdownPacket::new(5);

        host_transport
            .send_message(ControlMessage::LobbyCountdown(packet))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::LobbyCountdown { packet: received }) if received == packet
        ));

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Status(received)) if received == status
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_surfaces_and_broadcasts_its_lobby_countdown() {
        // Countdown construction broadcasts the packet to clients while the
        // host applies the same packet directly to its local MainDlg
        // (src/C4GameLobby.cpp:1111-1131).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut client_events = client.take_event_receiver();
        let packet = crate::LobbyCountdownPacket::new(5);

        host.submit_lobby_countdown(packet).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::LobbyCountdown { packet: received }) => {
                    assert_eq!(received, packet);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before lobby countdown"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
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

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_ready_check_without_disconnecting() {
        // Accepted PID_ReadyCheck packets are dispatched through
        // C4Network2::HandlePacket/HandleReadyCheck and do not close the
        // connection (src/C4Network2.cpp:949-953,1625-1707).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let packet = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Request,
        };

        host_transport
            .send_message(ControlMessage::ReadyCheck(packet))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet: received }) if received == packet
        ));

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Status(received)) if received == status
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ignores_nonhost_ready_request_without_disconnecting() {
        // HandleReadyCheck accepts a Request only when packet.Client resolves
        // to the host; a rejected request returns without closing the network
        // connection (src/C4Network2.cpp:1625-1646).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let rejected = ReadyCheckPacket {
            client_id: 1,
            data: crate::ReadyCheckData::Request,
        };

        host_transport
            .send_message(ControlMessage::ReadyCheck(rejected))
            .await
            .unwrap();
        assert!(timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err());

        let accepted = ReadyCheckPacket {
            client_id: 1,
            data: crate::ReadyCheckData::Ready,
        };
        host_transport
            .send_message(ControlMessage::ReadyCheck(accepted))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet }) if packet == accepted
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_filters_nonhost_ready_request_buffered_during_join() {
        // Packets buffered until JoinData must still pass through the same
        // HandleReadyCheck host-request validation as live packets
        // (src/C4Network2.cpp:949-953,1625-1646).
        let (client_stream, _host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let rejected = ReadyCheckPacket {
            client_id: 1,
            data: crate::ReadyCheckData::Request,
        };
        let accepted = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Request,
        };
        let mut resource_state = ClientResourceState::empty();
        resource_state.initial_ready_checks = vec![rejected, accepted];
        let client_loop = tokio::spawn(run_client_loop_with_addresses(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet }) if packet == accepted
        ));
        assert!(timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err());

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_ignores_network_ready_request_but_relays_its_opaque_fanout_leg() {
        // HandleReadyCheck rejects every Request while this process is the
        // host. HandleFwdReq still relays the opaque packet to selected peers,
        // where a claimed host author is accepted; Ready/NotReady likewise
        // select packet.Client without checking the transport origin
        // (src/C4Network2IO.cpp:1077-1129;
        // src/C4Network2.cpp:1625-1654,1700-1703).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta_events = beta.take_event_receiver();
        let request = ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: crate::ReadyCheckData::Request,
        };

        alpha.submit_ready_check(request).await.unwrap();
        while let Ok(Some(event)) = timeout(Duration::from_millis(50), host_events.recv()).await {
            assert!(
                !matches!(event, HostEvent::ReadyCheck { packet } if packet == request),
                "host surfaced a network-origin ready request"
            );
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
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

        let spoofed_ready = ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: crate::ReadyCheckData::Ready,
        };
        alpha.submit_ready_check(spoofed_ready).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, spoofed_ready);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before spoofed ready"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
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

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_relays_ready_check_unchanged_and_broadcasts_local_submission() {
        // Ready-check packets carry their claimed Client field through
        // BroadcastMsgToClients; HandleReadyCheck looks that client up without
        // comparing it to the transport origin (src/C4GameLobby.cpp:329-343,
        // 1072-1088; src/C4Network2.cpp:1625-1635).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut alpha_events = alpha.take_event_receiver();
        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta_events = beta.take_event_receiver();
        let relayed = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Ready,
        };

        alpha.submit_ready_check(relayed).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, relayed);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before ready-check relay"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
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

        let local = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Request,
        };
        host.submit_ready_check(local).await.unwrap();
        for events in [&mut alpha_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
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

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ready_check_updates_the_claimed_client_in_later_join_data() {
        // HandleReadyCheck mutates the C4Client selected by packet.Client;
        // later JoinData serializes that same Game.Clients registry
        // (src/C4Network2.cpp:1625-1635,1721-1729,1810-1850).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        alpha
            .submit_ready_check(ReadyCheckPacket {
                client_id: 0,
                data: crate::ReadyCheckData::Ready,
            })
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ReadyCheck { .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before ready-check"),
            }
        }

        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let join_data = beta.take_join_data().expect("beta receives JoinData");
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

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn direct_client_join_extends_the_address_owner_registry() {
        // CID_ClientJoin executes as direct control before later PID_Addr
        // propagation for that owner. The receiver must therefore admit the
        // new owner before handling its address packets
        // (src/C4Network2.cpp:1395-1445;
        // src/C4Network2Client.cpp:581-621).
        let (client_stream, host_stream) = duplex(2048);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::from([(0, Vec::new()), (1, Vec::new())]),
            ClientResourceState::empty(),
        ));
        let name = clonk_engine::LegacyCString::from_bytes(b"Beta".to_vec()).unwrap();
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
        .unwrap();
        host_transport
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: direct,
            })
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Direct { .. })
        ));

        let address = crate::AddressPacket {
            client_id: 2,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "198.51.100.22:11112".parse().unwrap(),
            ),
        };
        host_transport
            .send_message(ControlMessage::Address(address))
            .await
            .unwrap();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Address(address)
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn synchronized_client_remove_changes_address_membership_at_exec_sync() {
        // CtrlRemove is delivered as CDT_Sync, so the client remains present
        // until PID_ExecSyncCtrl executes the queued removal
        // (src/C4Client.cpp:293-304;
        // src/C4GameControlNetwork.cpp:181-220,558-588).
        let (client_stream, host_stream) = duplex(2048);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::from([(0, Vec::new()), (1, Vec::new()), (2, Vec::new())]),
            ClientResourceState::empty(),
        ));
        let remove = encode_control_entry_payload(&EngineControlPacket::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 2,
                reason: clonk_engine::LegacyCString::from_bytes(b"left".to_vec()).unwrap(),
                by_client: 0,
            },
        ))
        .unwrap();
        host_transport
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Sync,
                data: remove,
            })
            .await
            .unwrap();

        let before_execution = crate::AddressPacket {
            client_id: 2,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "198.51.100.22:11112".parse().unwrap(),
            ),
        };
        host_transport
            .send_message(ControlMessage::Address(before_execution))
            .await
            .unwrap();
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
            .unwrap();
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
            .unwrap();
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err()
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn central_to_decentral_switch_fast_forwards_and_replays_own_controls() {
        // The final GO acknowledgement carries Game.Control.ControlTick, the
        // first unexecuted tick. SetCtrlMode then rebroadcasts contiguous own
        // control from that tick before resuming (pristine C++
        // src/C4Network2.cpp:2062-2110;
        // src/C4GameControlNetwork.cpp:360-374).
        let (client_stream, host_stream) = duplex(4096);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.register(0).unwrap();
        resource_state.control.register(1).unwrap();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let live_tick = 137;

        let central_history = legacy_packet(BROADCAST_CLIENT_ID, live_tick - 1, 0x10);
        host_transport
            .send_message(ControlMessage::Control(central_history.clone()))
            .await
            .unwrap();
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
                .unwrap();
            assert_eq!(
                timeout(EVENT_WAIT, host_transport.read_message())
                    .await
                    .unwrap()
                    .unwrap(),
                ControlMessage::Control(packet.clone())
            );
        }

        let decentral = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: i32::try_from(live_tick).unwrap(),
        };
        host_transport
            .send_message(ControlMessage::StatusAck(decentral))
            .await
            .unwrap();
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
            .unwrap();
        let aggregate = match timeout(EVENT_WAIT, event_rx.recv()).await.unwrap() {
            Some(ClientEvent::Ready { packet }) => packet,
            other => panic!("expected live decentralized aggregate, got {other:?}"),
        };
        assert_eq!(aggregate.tick(), live_tick);
        assert_eq!(control_commands(&aggregate), vec![0x11, 0x21]);

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[test]
    fn central_recovery_cursor_waits_for_the_first_missing_complete_tick() {
        let mut control = ClientControlState::central(42);
        control.set_status_target(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 43,
        });
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
        control.set_status_target(NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 0,
        });
        assert_eq!(control.recovery_tick(), None);

        control.set_status_target(NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 0,
        });
        assert_eq!(control.recovery_tick(), Some(0));
        control.clear_target();
        assert_eq!(control.recovery_tick(), None);
    }

    #[test]
    fn decentral_recovery_tick_is_derived_from_coordinator_missing_ranges() {
        let mut control = ClientControlState::central(0);
        control.register(0).unwrap();
        control.register(1).unwrap();
        control.change_mode(0, 0).unwrap();
        control.set_status_target(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 1,
        });

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
        control.register(0).unwrap();
        control.register(1).unwrap();
        control.change_mode(0, 0).unwrap();
        control.set_status_target(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 1,
        });
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
        resource_state.control.register(1).unwrap();
        resource_state.control.set_status_target(NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        });
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
        control.register(0).unwrap();
        assert_eq!(
            control
                .accept_network(legacy_packet(BROADCAST_CLIENT_ID, 1, 0x21))
                .unwrap()
                .len(),
            1
        );
        control.change_mode(0, 0).unwrap();
        assert_eq!(
            control
                .ingest_contribution(legacy_packet(0, 0, 0x11))
                .unwrap()
                .len(),
            1
        );
        control.change_mode(1, 1).unwrap();

        assert_eq!(control.expected_tick(), 2);
        assert!(control
            .accept_network(legacy_packet(BROADCAST_CLIENT_ID, 1, 0x21))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn central_mode_switch_does_not_pack_buffered_partials() {
        let mut control = ClientControlState::central(0);
        control.register(0).unwrap();
        control.change_mode(0, 0).unwrap();
        assert!(control
            .ingest_contribution(legacy_packet(0, 1, 0x11))
            .unwrap()
            .is_empty());

        let (changed, ready) = control.change_mode(1, 1).unwrap();
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
        control.register(0).unwrap();
        control.register(1).unwrap();
        control.change_mode(0, 37).unwrap();
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
        control.change_mode(1, 37).unwrap();

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
        let (client_stream, host_stream) = duplex(2048);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(super::run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let decentral = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 0,
        };

        host_transport
            .send_message(ControlMessage::StatusAck(decentral))
            .await
            .expect("send decentralized status");
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::StatusAck(status))) if status == decentral
        ));

        for (client_id, name) in [(0, b"Host".as_slice()), (1, b"Local".as_slice())] {
            let name = clonk_engine::LegacyCString::from_bytes(name.to_vec()).unwrap();
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
                .expect("send active client join");
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
            .expect("send host contribution");
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "one decentralized contribution must not execute"
        );

        command_tx
            .send(ClientCommand::SubmitControl(local.clone()))
            .await
            .expect("submit local contribution");
        let nested_packet = crate::transport::encode_complete_control_packet(&local).unwrap();
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
        let aggregate = match timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("aggregate wait")
        {
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
                .expect("echo duplicate contribution");
        }
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "local echo and host retransmit must not execute the completed tick again"
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.expect("client loop exited");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_emits_a_complete_tick_only_once_when_host_retransmits_it() {
        // A non-host in CNM_Central cannot pack per-client contributions and
        // waits for the host's C4ClientIDAll packet instead (pristine C++
        // src/C4GameControlNetwork.cpp:679-718,775-777).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(super::run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let central = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 5,
        };
        let complete = legacy_packet(BROADCAST_CLIENT_ID, 5, 0x44);

        host_transport
            .send_message(ControlMessage::StatusAck(central))
            .await
            .expect("send central status");
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::StatusAck(status))) if status == central
        ));
        host_transport
            .send_message(ControlMessage::Control(complete.clone()))
            .await
            .expect("send complete tick");
        host_transport
            .send_message(ControlMessage::Control(complete.clone()))
            .await
            .expect("retransmit complete tick");

        match timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("ready wait")
        {
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
        client_handle.await.expect("client loop exited");
    }

    async fn submit_control_pair(
        host: &mut HostHandle,
        client: &ClientHandle,
        tick: Tick,
        host_command: i32,
        client_command: i32,
    ) {
        let host_packet = legacy_packet(0, tick, host_command);
        host.submit_local_control(host_packet)
            .await
            .expect("host submit control");

        let client_packet = legacy_packet(client.client_id(), tick, client_command);
        client
            .submit_control(client_packet)
            .await
            .expect("client submit control");
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

        let update = ClientUpdateControlData {
            update_type: CLIENT_UPDATE_ACTIVATE,
            client_id: i32::try_from(client_id).expect("test client ID fits i32"),
            data: 1,
            by_client: i32::try_from(HOST_CLIENT_ID).expect("host client ID fits i32"),
        };
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&EngineControlPacket::ClientUpdate(update.clone()))
                .expect("encode activation control"),
        )
        .await
        .expect("submit activation control");

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
        encode_control_packet(&LegacyControlFrame {
            client_id,
            tick,
            timestamp_ms: 0,
            controls: vec![EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(client_id).unwrap_or(i32::MAX),
                command,
                data: command,
                by_client: i32::try_from(client_id).unwrap_or(i32::MAX),
            })],
        })
        .expect("test legacy control encodes")
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
        let stream = TcpStream::connect(address).await.unwrap();
        let mut transport = crate::ControlTransport::new(stream);
        let name = clonk_engine::LegacyCString::from_bytes(name.to_vec()).unwrap();
        let request = crate::ConnectionRequest {
            core: clonk_engine::ClientCoreControlData {
                client_id: -1,
                activated: true,
                observer: false,
                name: name.clone(),
                nick: name,
                lobby_ready: false,
            },
            build: CURRENT_GAME_BUILD,
            password: clonk_engine::LegacyCString::default(),
            connection_id: 0,
        };
        let handshake = run_client_connection_handshake(&mut transport, request)
            .await
            .unwrap();
        let client_id = ClientId::try_from(handshake.join_data.client_id).unwrap();
        (transport, client_id)
    }

    async fn raw_existing_client_transport(
        address: SocketAddr,
        client_id: ClientId,
        remote_connection_id: u32,
        name: &[u8],
    ) -> crate::ControlTransport<TcpStream> {
        let stream = TcpStream::connect(address).await.unwrap();
        let mut transport = crate::ControlTransport::new(stream);
        assert!(matches!(
            transport.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = clonk_engine::LegacyCString::from_bytes(name.to_vec()).unwrap();
        transport
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id: i32::try_from(client_id).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: false,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: remote_connection_id,
                },
            ))
            .await
            .unwrap();
        loop {
            match transport.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                other => panic!("expected positive host connection reply, got {other:?}"),
            }
        }
        transport
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: clonk_engine::LegacyCString::from_bytes(b"connection accepted".to_vec())
                    .unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();
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
            match timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("host connection request stalled")
                .unwrap()
            {
                ControlMessage::ConnectionRequest(_) => break,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                other => panic!("expected host connection request, got {other:?}"),
            }
        }
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        transport
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: clonk_engine::ClientCoreControlData {
                        client_id,
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: clonk_engine::LegacyCString::default(),
                    connection_id: remote_connection_id,
                },
            ))
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("host route-admission decision stalled")
                .unwrap()
            {
                ControlMessage::ConnectionReply(reply) => return reply,
                ControlMessage::Ping(ping) => {
                    transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
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
            .unwrap();
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
            let size = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
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
            .expect("send raw status acknowledgement");
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
            .expect("complete control decodes")
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
