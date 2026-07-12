use std::time::Duration;

use lc_engine::{ControlPacket as EngineControlPacket, PlayerControlData};
use lc_network::{
    connect_client, decode_control_packet, encode_control_packet, ClientConfig, ClientEvent,
    ControlPacket, HostConfig, HostEvent, LegacyControlFrame, ParticipantKind, BROADCAST_CLIENT_ID,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{timeout, Instant};

const EVENT_WAIT: Duration = Duration::from_secs(2);
const QUIET_WINDOW: Duration = Duration::from_millis(100);

#[tokio::test(flavor = "multi_thread")]
async fn synchronized_tick_waits_for_host_and_client_then_broadcasts_one_decodable_aggregate() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind host listener");
    let address = listener.local_addr().expect("host listener address");
    let mut host = lc_network::start_host(listener, HostConfig::default())
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
    drain_initial_exec_sync(&mut client_events).await;

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
    // aggregate as one legacy frame must consume it fully: ClientIdMismatch
    // and TrailingData are both regressions in the synchronized-tick wire path.
    let decoded = decode_control_packet(&host_aggregate)
        .expect("aggregate decodes without ClientIdMismatch or trailing data");
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

fn player_control(player: i32, command: i32, data: i32, by_client: i32) -> EngineControlPacket {
    EngineControlPacket::PlayerControl(PlayerControlData {
        player,
        command,
        data,
        by_client,
    })
}

fn legacy_packet(client_id: u32, control: EngineControlPacket) -> ControlPacket {
    encode_control_packet(&LegacyControlFrame {
        client_id,
        tick: 0,
        timestamp_ms: 0,
        controls: vec![control],
    })
    .expect("encode legacy control packet")
}

async fn wait_for_join(events: &mut mpsc::Receiver<HostEvent>, expected_client: u32) {
    match timeout(EVENT_WAIT, events.recv()).await {
        Ok(Some(HostEvent::ClientJoined { client_id, .. })) => {
            assert_eq!(client_id, expected_client);
        }
        Ok(Some(HostEvent::TransportError { error, .. })) => {
            panic!("transport error while waiting for join: {error}")
        }
        Ok(Some(event)) => panic!("unexpected host event before join: {event:?}"),
        Ok(None) => panic!("host event stream ended before join"),
        Err(_) => panic!("timed out waiting for host join event"),
    }
}

async fn drain_initial_exec_sync(events: &mut mpsc::Receiver<ClientEvent>) {
    match timeout(EVENT_WAIT, events.recv()).await {
        Ok(Some(ClientEvent::ExecSync { .. })) => {}
        Ok(Some(ClientEvent::Disconnected { reason })) => {
            panic!("client disconnected before initial sync: {reason:?}")
        }
        Ok(Some(event)) => panic!("unexpected client event before initial sync: {event:?}"),
        Ok(None) => panic!("client event stream ended before initial sync"),
        Err(_) => panic!("timed out waiting for initial client sync"),
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
