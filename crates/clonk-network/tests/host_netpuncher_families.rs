//! Host startup over the netpuncher, across both address families.
//!
//! Covers clonk-org/clonk-rs#41: a host bound to the IPv4 wildcard answered
//! `EAFNOSUPPORT` for every datagram aimed at a netpuncher that had resolved
//! from an AAAA record, and the resulting socket error took its whole
//! reliable-UDP transport down.

use std::net::SocketAddr;
use std::time::Duration;

use clonk_network::{start_host_with_bindings, HostConfig, HostEvent, HostUdpBinding};
use tokio::net::UdpSocket;

const PUNCHER_WAIT: Duration = Duration::from_secs(5);

/// Accepts the first datagram a host sends to this stand-in netpuncher, or the
/// transport error the host reported instead.
async fn first_puncher_datagram(
    puncher: &UdpSocket,
    events: &mut tokio::sync::mpsc::Receiver<HostEvent>,
) -> Result<SocketAddr, String> {
    let mut buffer = [0_u8; 512];
    tokio::select! {
        received = puncher.recv_from(&mut buffer) => received
            .map(|(_, source)| source)
            .map_err(|error| error.to_string()),
        transport_error = async {
            loop {
                match events.recv().await {
                    Some(HostEvent::TransportError { error, .. }) => return error,
                    Some(_) => continue,
                    None => return "host loop ended".to_string(),
                }
            }
        } => Err(transport_error),
        _ = tokio::time::sleep(PUNCHER_WAIT) => Err("no netpuncher datagram".to_string()),
    }
}

async fn host_reaches_netpuncher(puncher: UdpSocket) -> Result<SocketAddr, String> {
    let puncher_address = puncher.local_addr().expect("stand-in netpuncher address");
    let config = HostConfig {
        // The startup UI binds the IPv4 wildcard for "any interface"; the
        // netpuncher endpoint comes from DNS and may be either family.
        udp_bind_address: Some(SocketAddr::from(([0, 0, 0, 0], 0))),
        configured_tcp_port: Some(0),
        netpuncher_addresses: vec![puncher_address],
        ..HostConfig::default()
    };
    let udp_binding = HostUdpBinding::bind(&config);
    assert!(
        udp_binding.local_addr().is_some(),
        "the reliable-UDP listener must bind: {:?}",
        udp_binding.bind_error()
    );
    let mut host = start_host_with_bindings(None, config, udp_binding)
        .await
        .expect("UDP-only host startup");
    let mut events = host.take_event_receiver();
    let result = first_puncher_datagram(&puncher, &mut events).await;
    host.shutdown().await.expect("host shutdown");
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ipv4_wildcard_host_reaches_an_ipv6_netpuncher() {
    let Ok(puncher) = UdpSocket::bind("[::1]:0").await else {
        // A host without IPv6 loopback cannot answer this question either way.
        return;
    };
    host_reaches_netpuncher(puncher)
        .await
        .expect("IPv6 netpuncher must receive the host's reliable-UDP connect");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ipv4_wildcard_host_reaches_an_ipv4_netpuncher() {
    let puncher = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("IPv4 loopback netpuncher");
    host_reaches_netpuncher(puncher)
        .await
        .expect("IPv4 netpuncher must receive the host's reliable-UDP connect");
}
