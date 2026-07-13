use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

use lc_network::{
    discovery_reply_for_packet, encode_reference_response, parse_reference_response,
    NetworkAddress, NetworkGameAdvertiser, NetworkGameAdvertiserConfig, NetworkGameReference,
    NetworkProtocol,
};

fn advertised_game() -> NetworkGameReference {
    NetworkGameReference {
        title: "Gold Rush".into(),
        host_name: "Host One".into(),
        host_nick: "OracleNick".into(),
        state: "Lobby".into(),
        control_mode: 2,
        start_time: 1234,
        join_allowed: true,
        password_needed: false,
        official_server: false,
        league_address: String::new(),
        max_players: 13,
        game: "LegacyClonk".into(),
        version: [4, 9, 11, 0],
        build: 362,
        addresses: vec![
            NetworkAddress::new(NetworkProtocol::Udp, "0.0.0.0:11113".parse().unwrap()),
            NetworkAddress::new(NetworkProtocol::Tcp, "0.0.0.0:11112".parse().unwrap()),
        ],
        source_address: "[::]:0".parse().unwrap(),
        netpuncher_ipv4: 0x1234_5678,
        netpuncher_ipv6: 0x9abc_def0,
        netpuncher_address: "puncher.invalid:11115".into(),
        tcp_addresses: vec!["0.0.0.0:11112".parse().unwrap()],
    }
}

#[test]
fn cpp_discovery_probe_produces_the_abi_padded_native_endian_reply() {
    let port = 11_111_u16.to_ne_bytes();
    assert_eq!(
        discovery_reply_for_packet(&[0x03], 11_111),
        Some([0x04, 0x00, port[0], port[1]])
    );
    assert_eq!(discovery_reply_for_packet(&[0x03, 0x00], 11_111), None);
    assert_eq!(discovery_reply_for_packet(&[0x04], 11_111), None);
}

#[test]
fn advertised_reference_round_trips_through_the_cpp_ini_shape() {
    // C4Network2Reference serializes GameStatus (including CtrlMode), then
    // Parameters (including MaxPlayers and the host C4ClientCore Nick) in this
    // shape (pristine 9ffa0a5d src/C4Network2Reference.cpp:71-105;
    // src/C4Network2.cpp:101-122; src/C4GameParameters.cpp:553-585;
    // src/C4Client.cpp:75-83).
    let encoded = encode_reference_response(&advertised_game());
    let text = String::from_utf8(encoded.clone()).unwrap();
    assert!(text.starts_with("[Reference]\r\n"));
    assert!(text.contains("CtrlMode=2\r\n"));
    assert!(text.contains(
        "Address=UDP:\"0.0.0.0:11113\",TCP:\"0.0.0.0:11112\"\r\n"
    ));
    assert!(text.contains("MaxPlayers=13\r\n"));
    assert!(text.contains("  [Client]\r\n  ID=0\r\n"));
    assert!(text.contains("  Name=\"Host One\"\r\n  Nick=\"OracleNick\"\r\n"));
    assert!(text.contains(
        "\r\n  [NetpuncherID]\r\n  IPv4=305419896\r\n  IPv6=2596069104\r\n\
NetpuncherAddr=\"puncher.invalid:11115\"\r\n"
    ));

    let decoded = parse_reference_response(&encoded).unwrap();
    assert_eq!(decoded, vec![advertised_game()]);
}

#[test]
fn reference_server_answers_cpp_http_get_with_current_reference() {
    let advertiser = NetworkGameAdvertiser::start(
        NetworkGameAdvertiserConfig {
            discovery_port: 0,
            reference_port: 0,
        },
        advertised_game(),
    )
    .unwrap();
    let reference_port = advertiser.reference_addr().port();
    let response = http_request(reference_port, b"GET / HTTP/1.0\r\nHost: [::1]\r\n\r\n");

    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    assert!(headers.starts_with("HTTP/1.0 200 OK\r\n"));
    assert!(headers.contains("Content-Type: text/plain; charset=ISO-8859-1\r\n"));
    assert_eq!(
        parse_reference_response(&response[header_end..]).unwrap(),
        vec![advertised_game()]
    );

    let mut updated = advertised_game();
    updated.state = "Running".into();
    updated.title = "Updated".into();
    advertiser.update(&updated);
    let response = http_request(reference_port, b"GET /anything HTTP/1.0\r\n\r\n");
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(
        parse_reference_response(&response[header_end..]).unwrap(),
        vec![updated]
    );

    assert_eq!(
        http_request(reference_port, b"POST / HTTP/1.0\r\n\r\n"),
        b"HTTP/1.0 405 Method Not Allowed\r\n\r\n"
    );

    let mut incomplete =
        TcpStream::connect(SocketAddr::from((Ipv6Addr::LOCALHOST, reference_port))).unwrap();
    incomplete.write_all(b"GET / HTTP/1.0\r\n").unwrap();
    incomplete.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    incomplete.read_to_end(&mut response).unwrap();
    assert!(response.is_empty());
}

fn http_request(reference_port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream =
        TcpStream::connect(SocketAddr::from((Ipv6Addr::LOCALHOST, reference_port))).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    response
}
