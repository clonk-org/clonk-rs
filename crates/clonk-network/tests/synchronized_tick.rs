use std::time::Duration;

use clonk_engine::{
    ClientRemoveControlData, ClientUpdateControlData, ControlPacket as EngineControlPacket,
    ControlPlayerInfoEntry, LegacyCString, PlayerControlData, SynchronizeControlData,
    CLIENT_UPDATE_ACTIVATE, CLIENT_UPDATE_SET_OBSERVER,
};
use clonk_network::{
    connect_client, decode_control_entry_payload, decode_control_packet,
    encode_control_entry_payload, encode_control_packet, ClientConfig, ClientEvent,
    ControlDelivery, ControlPacket, HostConfig, HostEvent, LegacyControlFrame, ParticipantKind,
    PlayerInfoUpdateRequest, BROADCAST_CLIENT_ID,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{timeout, Instant};

const EVENT_WAIT: Duration = Duration::from_secs(2);
const QUIET_WINDOW: Duration = Duration::from_millis(100);

#[tokio::test(flavor = "multi_thread")]
async fn synchronize_retains_its_position_in_a_live_ready_control_list() {
    // C4Control executes ID packets in list order, and PackCompleteCtrl
    // appends each contributing list without reordering its entries
    // (pristine 9ffa0a5d src/C4Control.cpp:73-109;
    // src/C4GameControlNetwork.cpp:741-769).
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host listener");
    let mut host = clonk_network::start_host(listener, HostConfig::default())
        .await
        .expect("start host session");
    let mut events = host.take_event_receiver();
    let before = player_control(0, 2, 10, 0);
    let synchronize = EngineControlPacket::Synchronize(SynchronizeControlData {
        save_player_files: false,
        sync_clearance: true,
        by_client: 0,
    });
    let after = player_control(0, 5, 20, 0);
    let packet = encode_control_packet(&LegacyControlFrame {
        client_id: 0,
        tick: 0,
        timestamp_ms: 0,
        controls: vec![before.clone(), synchronize.clone(), after.clone()],
    })
    .expect("encode ordered host controls");

    host.submit_local_control(packet)
        .await
        .expect("submit host tick");

    let ready = wait_for_host_ready(&mut events).await;
    assert_eq!(
        decode_control_packet(&ready)
            .expect("live Ready control decodes")
            .controls,
        vec![before, synchronize, after]
    );

    host.shutdown().await.expect("shut down host session");
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronized_tick_waits_for_host_and_client_then_broadcasts_one_decodable_aggregate() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host listener");
    let address = listener.local_addr().expect("host listener address");
    let mut host = clonk_network::start_host(listener, HostConfig::default())
        .await
        .expect("start host session");
    let mut client = connect_client(
        address,
        ClientConfig::new("tick-client", ParticipantKind::Player),
    )
    .await
    .expect("connect client session");
    let client_id = client.client_id();
    let mut host_events = host.take_event_receiver();
    let mut client_events = client.take_event_receiver();

    wait_for_join(&mut host_events, client_id).await;
    activate_client(&host, &mut host_events, client_id).await;

    let host_control = player_control(0, 2, 10, 0);
    let client_control = player_control(1, 5, 20, client_id as i32);
    let client_packet = legacy_packet(client_id, client_control.clone());

    // C4GameControlNetwork::PackCompleteCtrl waits until every registered
    // client has this tick before packing (`src/C4GameControlNetwork.cpp:741-759`).
    client
        .submit_control(client_packet.clone())
        .await
        .expect("submit client tick zero first");
    assert_no_host_ready(&mut host_events, QUIET_WINDOW).await;
    assert_no_client_ready(&mut client_events, QUIET_WINDOW).await;

    host.submit_local_control(legacy_packet(0, host_control.clone()))
        .await
        .expect("submit host tick zero second");

    let host_aggregate = wait_for_host_ready(&mut host_events).await;
    let client_aggregate = wait_for_client_ready(&mut client_events).await;
    assert_eq!(host_aggregate, client_aggregate);
    assert_eq!(host_aggregate.client_id(), BROADCAST_CLIENT_ID);
    assert_eq!(host_aggregate.tick(), 0);

    // PackCompleteCtrl appends controls in client-ID order, so host 0 must
    // precede client 1 (`src/C4GameControlNetwork.cpp:760-774`). Decoding the
    // aggregate as one legacy frame must consume it fully; duplicated envelope
    // bytes or trailing data are regressions in the synchronized-tick wire path.
    let decoded = decode_control_packet(&host_aggregate)
        .expect("aggregate decodes without a duplicated envelope or trailing data");
    assert_eq!(decoded.tick, 0);
    assert_eq!(decoded.controls, vec![host_control, client_control]);

    // CheckCompleteCtrl advances iControlReady after installing one complete
    // packet (`src/C4GameControlNetwork.cpp:679-718`); a repeated contribution
    // for tick zero must not publish a second aggregate.
    client
        .submit_control(client_packet)
        .await
        .expect("submit duplicate client tick zero");
    assert_no_host_ready(&mut host_events, QUIET_WINDOW).await;
    assert_no_client_ready(&mut client_events, QUIET_WINDOW).await;

    client.shutdown().await.expect("shut down client session");
    host.shutdown().await.expect("shut down host session");
}

#[tokio::test(flavor = "multi_thread")]
async fn inactive_joined_client_does_not_block_host_lockstep() {
    // The admitted client core is deactivated until CUT_Activate executes.
    // C4GameControlNetwork waits only for activated control clients, so an
    // inactive lobby join cannot prevent the host from completing its tick
    // (pristine 9ffa0a5d src/C4Network2.cpp:1395-1406;
    // src/C4Control.cpp:588-606; src/C4GameControlNetwork.cpp:741-769).
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host listener");
    let address = listener.local_addr().expect("host listener address");
    let mut host = clonk_network::start_host(listener, HostConfig::default())
        .await
        .expect("start host session");
    let client = connect_client(
        address,
        ClientConfig::new("inactive-client", ParticipantKind::Player),
    )
    .await
    .expect("connect inactive client session");
    let client_id = client.client_id();
    let mut host_events = host.take_event_receiver();
    wait_for_join(&mut host_events, client_id).await;

    let host_control = player_control(0, 2, 10, 0);
    host.submit_local_control(legacy_packet(0, host_control.clone()))
        .await
        .expect("submit host tick while peer is inactive");

    let aggregate = wait_for_host_ready(&mut host_events).await;
    assert_eq!(
        decode_control_packet(&aggregate)
            .expect("host-only aggregate decodes")
            .controls,
        vec![host_control]
    );

    client.shutdown().await.expect("shut down client session");
    host.shutdown().await.expect("shut down host session");
}

#[tokio::test(flavor = "multi_thread")]
async fn deactivation_observer_and_remove_release_waiting_host_controls() {
    // Host-authored CUT_Activate(false), CUT_SetObserver, and ClientRemove all
    // remove the peer from the active control-client list. Removing a missing
    // contributor immediately lets PackCompleteCtrl publish the host's queued
    // tick (pristine 9ffa0a5d src/C4Control.cpp:578-618,637-687;
    // src/C4GameControlNetwork.cpp:318-326,741-769).
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host listener");
    let address = listener.local_addr().expect("host listener address");
    let mut host = clonk_network::start_host(listener, HostConfig::default())
        .await
        .expect("start host session");
    let client = connect_client(
        address,
        ClientConfig::new("membership-client", ParticipantKind::Player),
    )
    .await
    .expect("connect client session");
    let client_id = client.client_id();
    let mut host_events = host.take_event_receiver();
    wait_for_join(&mut host_events, client_id).await;

    for (tick, control) in [
        (
            0,
            EngineControlPacket::ClientUpdate(ClientUpdateControlData {
                update_type: CLIENT_UPDATE_ACTIVATE,
                client_id: client_id as i32,
                data: 0,
                by_client: 0,
            }),
        ),
        (
            1,
            EngineControlPacket::ClientUpdate(ClientUpdateControlData {
                update_type: CLIENT_UPDATE_SET_OBSERVER,
                client_id: client_id as i32,
                data: 0,
                by_client: 0,
            }),
        ),
        (
            2,
            EngineControlPacket::ClientRemove(ClientRemoveControlData {
                client_id: client_id as i32,
                reason: LegacyCString::from_bytes(b"Removed".to_vec()).unwrap(),
                by_client: 0,
            }),
        ),
    ] {
        activate_client(&host, &mut host_events, client_id).await;
        let host_control = player_control(0, tick as i32 + 10, tick as i32 + 20, 0);
        host.submit_local_control(legacy_packet_at(0, tick, host_control.clone()))
            .await
            .expect("submit host tick before membership removal");
        assert_no_host_ready(&mut host_events, QUIET_WINDOW).await;

        let aggregate = execute_membership_control(&host, &mut host_events, control).await;
        assert_eq!(
            decode_control_packet(&aggregate)
                .expect("released host-only aggregate decodes")
                .controls,
            vec![host_control]
        );
    }

    client.shutdown().await.expect("shut down client session");
    host.shutdown().await.expect("shut down host session");
}

#[tokio::test(flavor = "multi_thread")]
async fn player_info_update_request_reaches_host_with_transport_origin() {
    // A client sends PID_PlayerInfoUpdReq only to the host; the packet carries
    // C4ClientPlayerInfos but no ByClient, so the transport connection remains
    // a separate identity (src/C4Network2Players.cpp:142-166,392-411;
    // src/C4PlayerInfo.cpp:1800-1803).
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host listener");
    let address = listener.local_addr().expect("host listener address");
    let mut host = clonk_network::start_host(listener, HostConfig::default())
        .await
        .expect("start host session");
    let client = connect_client(
        address,
        ClientConfig::new("admission-client", ParticipantKind::Player),
    )
    .await
    .expect("connect client session");
    let client_id = client.client_id();
    let mut host_events = host.take_event_receiver();
    wait_for_join(&mut host_events, client_id).await;

    let request = PlayerInfoUpdateRequest {
        client_id: 3,
        flags: 1,
        players: vec![ControlPlayerInfoEntry {
            id: 0,
            // C4PlayerInfo compilation always writes this StdStrBuf as a
            // C-string, so decode materializes even an empty value.
            league_progress_data_is_null: false,
            ..Default::default()
        }],
    };
    client
        .submit_player_info_update(request.clone())
        .await
        .expect("submit PlayerInfo update request");

    match timeout(EVENT_WAIT, host_events.recv()).await {
        Ok(Some(HostEvent::PlayerInfoUpdate {
            client_id: actual_origin,
            request: actual_request,
        })) => {
            assert_eq!(actual_origin, client_id);
            let mut expected = request;
            // C4PlayerInfo's binary reader applies VAL_NameNoEmpty even when
            // the sender encoded an empty default Name.
            expected.players[0].name =
                clonk_engine::LegacyCString::from_bytes(b"empty".to_vec()).unwrap();
            assert_eq!(actual_request, expected);
        }
        Ok(Some(event)) => panic!("unexpected host event: {event:?}"),
        Ok(None) => panic!("host event stream ended before PlayerInfo update"),
        Err(_) => panic!("timed out waiting for PlayerInfo update"),
    }

    client.shutdown().await.expect("shut down client session");
    host.shutdown().await.expect("shut down host session");
}

#[tokio::test(flavor = "multi_thread")]
async fn activation_request_reaches_host_with_transport_origin() {
    // PID_ClientActReq carries only the requester's frame tick. C++ derives
    // the target client from the authenticated connection
    // (src/C4Network2.cpp:982-991,1553-1571).
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host listener");
    let address = listener.local_addr().expect("host listener address");
    let mut host = clonk_network::start_host(listener, HostConfig::default())
        .await
        .expect("start host session");
    let mut client = connect_client(
        address,
        ClientConfig::new("activation-client", ParticipantKind::Player),
    )
    .await
    .expect("connect client session");
    let client_id = client.client_id();
    let mut acknowledged = client.take_join_data().expect("client JoinData").status;
    acknowledged.target_tick = 0;
    let mut host_events = host.take_event_receiver();
    wait_for_join(&mut host_events, client_id).await;

    client
        .request_activation(37)
        .await
        .expect("send activation request");

    match timeout(EVENT_WAIT, host_events.recv()).await {
        Ok(Some(HostEvent::ActivationRequest {
            client_id: actual_origin,
            tick,
            waited_for,
            ping_ms,
        })) => {
            assert_eq!(actual_origin, client_id);
            assert_eq!(tick, 37);
            assert!(!waited_for, "a Chasing client is not waited for yet");
            assert_eq!(ping_ms, -1);
        }
        Ok(Some(event)) => panic!("unexpected host event: {event:?}"),
        Ok(None) => panic!("host event stream ended before activation request"),
        Err(_) => panic!("timed out waiting for activation request"),
    }

    client
        .submit_status_ack(acknowledged)
        .await
        .expect("acknowledge lobby status");
    loop {
        match timeout(EVENT_WAIT, host_events.recv()).await {
            Ok(Some(HostEvent::StatusAck {
                client_id: actual, ..
            })) if actual == client_id => break,
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error before status acknowledgement: {error}")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended before status acknowledgement"),
            Err(_) => panic!("timed out waiting for status acknowledgement"),
        }
    }
    client
        .request_activation(38)
        .await
        .expect("send waited-for activation request");
    loop {
        match timeout(EVENT_WAIT, host_events.recv()).await {
            Ok(Some(HostEvent::ActivationRequest {
                client_id: actual_origin,
                tick: 38,
                waited_for: true,
                ping_ms: -1,
            })) if actual_origin == client_id => break,
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error before waited-for activation request: {error}")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended before waited-for activation request"),
            Err(_) => panic!("timed out waiting for waited-for activation request"),
        }
    }

    client.shutdown().await.expect("shut down client session");
    host.shutdown().await.expect("shut down host session");
}

fn player_control(player: i32, command: i32, data: i32, by_client: i32) -> EngineControlPacket {
    EngineControlPacket::PlayerControl(PlayerControlData {
        player,
        command,
        data,
        by_client,
    })
}

fn legacy_packet(client_id: u32, control: EngineControlPacket) -> ControlPacket {
    legacy_packet_at(client_id, 0, control)
}

fn legacy_packet_at(client_id: u32, tick: u32, control: EngineControlPacket) -> ControlPacket {
    encode_control_packet(&LegacyControlFrame {
        client_id,
        tick,
        timestamp_ms: 0,
        controls: vec![control],
    })
    .expect("encode legacy control packet")
}

async fn execute_membership_control(
    host: &clonk_network::HostHandle,
    events: &mut mpsc::Receiver<HostEvent>,
    control: EngineControlPacket,
) -> ControlPacket {
    let encoded = encode_control_entry_payload(&control).expect("encode host membership control");
    host.submit_packet(ControlDelivery::Sync, encoded)
        .await
        .expect("submit host membership control");
    let mut ready = None;
    let mut executed = false;
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::Ready { packet })) => ready = Some(packet),
            Ok(Some(HostEvent::SyncScheduled { controls, .. })) => {
                assert_eq!(controls, vec![control.clone()]);
                executed = true;
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error during membership control: {error}")
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("host event stream ended during membership control"),
            Err(_) => panic!("timed out waiting for membership control effects"),
        }
        if executed {
            if let Some(packet) = ready.take() {
                return packet;
            }
        }
    }
}

async fn activate_client(
    host: &clonk_network::HostHandle,
    events: &mut mpsc::Receiver<HostEvent>,
    client_id: u32,
) {
    let update = ClientUpdateControlData {
        update_type: CLIENT_UPDATE_ACTIVATE,
        client_id: client_id as i32,
        data: 1,
        by_client: 0,
    };
    let encoded = encode_control_entry_payload(&EngineControlPacket::ClientUpdate(update.clone()))
        .expect("encode host activation control");
    host.submit_packet(ControlDelivery::Sync, encoded)
        .await
        .expect("submit host activation control");
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::SyncScheduled { controls, .. })) => {
                assert_eq!(controls, vec![EngineControlPacket::ClientUpdate(update)]);
                return;
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error while activating client: {error}")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended before activation executed"),
            Err(_) => panic!("timed out waiting for activation execution"),
        }
    }
}

async fn wait_for_join(events: &mut mpsc::Receiver<HostEvent>, expected_client: u32) {
    let mut saw_direct_join = false;
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::Direct {
                delivery: ControlDelivery::Direct,
                data,
                ..
            })) => {
                let control = decode_control_entry_payload(&data)
                    .expect("direct admission control must decode");
                if let EngineControlPacket::ClientJoin(join) = control {
                    assert_eq!(join.core.client_id, expected_client as i32);
                    saw_direct_join = true;
                }
            }
            Ok(Some(HostEvent::ClientJoined { client_id, .. })) => {
                assert_eq!(client_id, expected_client);
                assert!(
                    saw_direct_join,
                    "host reported ClientJoined before direct ClientJoin"
                );
                return;
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error while waiting for join: {error}")
            }
            Ok(Some(event)) => panic!("unexpected host event before join: {event:?}"),
            Ok(None) => panic!("host event stream ended before join"),
            Err(_) => panic!("timed out waiting for host join event"),
        }
    }
}

async fn wait_for_host_ready(events: &mut mpsc::Receiver<HostEvent>) -> ControlPacket {
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::Ready { packet })) => return packet,
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error before host Ready: {error}")
            }
            Ok(Some(HostEvent::ClientLeft { client_id })) => {
                panic!("client {client_id} left before host Ready")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended before Ready"),
            Err(_) => panic!("timed out waiting for host Ready"),
        }
    }
}

async fn wait_for_client_ready(events: &mut mpsc::Receiver<ClientEvent>) -> ControlPacket {
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(ClientEvent::Ready { packet })) => return packet,
            Ok(Some(ClientEvent::Disconnected { reason })) => {
                panic!("client disconnected before Ready: {reason:?}")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("client event stream ended before Ready"),
            Err(_) => panic!("timed out waiting for client Ready"),
        }
    }
}

async fn assert_no_host_ready(events: &mut mpsc::Receiver<HostEvent>, duration: Duration) {
    let deadline = Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        match timeout(remaining, events.recv()).await {
            Err(_) => return,
            Ok(Some(HostEvent::Ready { packet })) => {
                panic!(
                    "unexpected second/early host Ready for tick {}",
                    packet.tick()
                )
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error during host quiet window: {error}")
            }
            Ok(Some(HostEvent::ClientLeft { client_id })) => {
                panic!("client {client_id} left during host quiet window")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended during quiet window"),
        }
    }
}

async fn assert_no_client_ready(events: &mut mpsc::Receiver<ClientEvent>, duration: Duration) {
    let deadline = Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        match timeout(remaining, events.recv()).await {
            Err(_) => return,
            Ok(Some(ClientEvent::Ready { packet })) => {
                panic!(
                    "unexpected second/early client Ready for tick {}",
                    packet.tick()
                )
            }
            Ok(Some(ClientEvent::Disconnected { reason })) => {
                panic!("client disconnected during quiet window: {reason:?}")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("client event stream ended during quiet window"),
        }
    }
}
