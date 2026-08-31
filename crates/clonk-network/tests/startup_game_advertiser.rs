use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

use clonk_network::{
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
        compat_profile: None,
        ..Default::default()
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
    assert!(text.contains("Address=UDP:\"0.0.0.0:11113\",TCP:\"0.0.0.0:11112\"\r\n"));
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
            reference_port: Some(0),
            language_charset: String::new(),
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
    assert!(headers.contains("Content-Type: text/plain; charset=CP1252\r\n"));
    assert!(headers.contains("Server: ClonkRust/4.9.11.0 [362]\r\n"));
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

#[test]
fn reference_server_uses_configured_legacy_charset() {
    // GetCharsetCodeName compares only the classic names, ASCII-insensitively,
    // and falls back to CP1252 without trimming or accepting CP aliases
    // (pristine 9ffa0a5d src/C4Config.cpp:875-893). Reference text is already
    // in that native byte domain before StdCompilerINIWrite quotes it.
    for (configured, canonical, title, encoded) in [
        ("SHIFTJIS", "CP932", "日", &[0x93, 0xfa][..]),
        ("hangul", "CP949", "한", &[0xc7, 0xd1][..]),
        ("JOHAB", "CP1361", "한", &[0xd0, 0x65][..]),
        ("CHINESEBIG5", "CP950", "漢", &[0xba, 0x7e][..]),
        ("GREEK", "CP1253", "Α", &[0xc1][..]),
        ("TURKISH", "CP1254", "Ğ", &[0xd0][..]),
        ("VIETNAMESE", "CP1258", "Đ", &[0xd0][..]),
        ("HEBREW", "CP1255", "א", &[0xe0][..]),
        ("ARABIC", "CP1256", "ا", &[0xc7][..]),
        ("BALTIC", "CP1257", "Ą", &[0xc0][..]),
        ("RUSSIAN", "CP1251", "А", &[0xc0][..]),
        ("THAI", "CP874", "ก", &[0xa1][..]),
        ("EASTEUROPE", "CP1250", "Ą", &[0xa5][..]),
        ("UTF-8", "UTF-8", "€", &[0xe2, 0x82, 0xac][..]),
        ("", "CP1252", "€", &[0x80][..]),
        ("CP1251", "CP1252", "€", &[0x80][..]),
        (" RUSSIAN ", "CP1252", "€", &[0x80][..]),
    ] {
        let mut reference = advertised_game();
        reference.title = title.to_string();
        let advertiser = NetworkGameAdvertiser::start(
            NetworkGameAdvertiserConfig {
                discovery_port: 0,
                reference_port: Some(0),
                language_charset: configured.to_string(),
            },
            reference,
        )
        .unwrap();

        let response = http_request(
            advertiser.reference_addr().port(),
            b"GET / HTTP/1.0\r\n\r\n",
        );
        assert_reference_charset(&response, canonical, encoded, configured);

        if configured == "RUSSIAN" {
            let mut updated = advertised_game();
            updated.title = "Б".to_string();
            advertiser.update(&updated);
            let response = http_request(
                advertiser.reference_addr().port(),
                b"GET /updated HTTP/1.0\r\n\r\n",
            );
            assert_reference_charset(&response, "CP1251", &[0xc1], "updated RUSSIAN");
        }
    }
}

fn assert_reference_charset(response: &[u8], charset: &str, encoded: &[u8], context: &str) {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    assert!(
        headers.contains(&format!("Content-Type: text/plain; charset={charset}\r\n")),
        "{context}: {headers}"
    );

    let mut expected = b"Title=\"".to_vec();
    let mut last_was_numeric_escape = false;
    for byte in encoded {
        if byte.is_ascii_graphic()
            && !matches!(*byte, b'\\' | b'\"')
            && !(last_was_numeric_escape && byte.is_ascii_digit())
        {
            expected.push(*byte);
            last_was_numeric_escape = false;
        } else {
            expected.extend_from_slice(format!("\\{byte:o}").as_bytes());
            last_was_numeric_escape = true;
        }
    }
    expected.extend_from_slice(b"\"\r\n");
    assert!(
        response[header_end..]
            .windows(expected.len())
            .any(|window| window == expected),
        "{context}: body does not contain {expected:?}"
    );
}

#[test]
fn a_host_that_cannot_join_the_discovery_group_still_serves_its_reference() {
    // C4Network2IO::Init logs a failed discovery init, leaves pNetIODiscover
    // null and builds the reference server afterwards, so hosting survives a
    // refused multicast join and the game stays reachable by typed address
    // (pinned oracle src/C4Network2IO.cpp:86-89, :151-161).
    let discovery_reservation = std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
    let discovery_port = discovery_reservation.local_addr().unwrap().port();
    drop(discovery_reservation);

    let advertiser = NetworkGameAdvertiser::start(
        NetworkGameAdvertiserConfig {
            discovery_port,
            reference_port: Some(0),
            language_charset: String::new(),
        },
        advertised_game(),
    )
    .expect("a host advertises even where the kernel refuses the discovery group");

    let reference_port = advertiser.reference_addr().port();
    assert_ne!(reference_port, 0, "the reference listener must survive");
    let response = http_request(reference_port, b"GET / HTTP/1.0\r\n\r\n");
    let references = parse_reference_response(&response).expect("reference server answers");
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].title, "Gold Rush");
    drop(advertiser);
}

#[test]
fn disabled_reference_server_keeps_discovery_only_advertiser_clean() {
    let discovery_reservation = std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
    let discovery_port = discovery_reservation.local_addr().unwrap().port();
    // Keep the TCP reservation while the advertiser starts. TCP and UDP have
    // separate namespaces, so this does not prevent the discovery socket from
    // binding. An advertiser that opens a TCP listener on the discovery port
    // instead fails to start, preserving the assertion below without a
    // post-start port-reuse race.
    let tcp_reservation = std::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, discovery_port))
        .expect("the TCP reservation must be available");
    drop(discovery_reservation);

    let advertiser = NetworkGameAdvertiser::start(
        NetworkGameAdvertiserConfig {
            discovery_port,
            reference_port: None,
            language_charset: String::new(),
        },
        advertised_game(),
    )
    .expect("discovery-only advertising must not create a TCP listener");

    assert_eq!(advertiser.reference_addr().port(), 0);
    advertiser.update(&advertised_game());
    drop(advertiser);
    drop(tcp_reservation);
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

/// The compatibility profile rides in the reference as an ordinary named key,
/// and only when the host actually runs one (clonk-org/clonk-rs#583).
///
/// Two properties matter more than the round trip itself. A host in the
/// ordinary profile must emit **exactly** the bytes it emitted before, because
/// every existing peer -- stock C++ included -- parses that reference today.
/// And a reference without the key must read back as "said nothing" rather
/// than as some default profile, because a stock C++ host is precisely the
/// peer that will never send it: `StdCompilerINIRead` reads by name, so C++
/// neither writes this key nor looks it up.
#[test]
fn the_compatibility_profile_is_advertised_only_when_the_host_runs_one() {
    let ordinary = advertised_game();
    assert_eq!(
        ordinary.compat_profile, None,
        "a reference names no profile until a host sets one"
    );
    let ordinary_bytes = encode_reference_response(&ordinary);
    assert!(
        !String::from_utf8_lossy(&ordinary_bytes).contains("CompatProfile"),
        "an ordinary host's reference must be byte-for-byte what it was"
    );
    assert_eq!(
        parse_reference_response(&ordinary_bytes).expect("the ordinary reference parses")[0]
            .compat_profile,
        None,
        "and a reference without the key reads back as silence, not a default"
    );

    let mut compatible = advertised_game();
    compatible.compat_profile = Some("legacy-clonk".to_string());
    let encoded = encode_reference_response(&compatible);
    assert!(
        String::from_utf8_lossy(&encoded).contains("CompatProfile=legacy-clonk\r\n"),
        "a host running a profile advertises it as a named key"
    );
    assert_eq!(
        parse_reference_response(&encoded).expect("the profile-bearing reference parses")[0]
            .compat_profile
            .as_deref(),
        Some("legacy-clonk"),
        "and a peer reads it back"
    );

    // An empty value is silence too: a peer that writes the key with nothing
    // in it has not named a profile, and must not be read as having one.
    let mut empty = advertised_game();
    empty.compat_profile = Some(String::new());
    let encoded = encode_reference_response(&empty);
    assert!(!String::from_utf8_lossy(&encoded).contains("CompatProfile"));
    assert_eq!(
        parse_reference_response(&encoded).expect("the reference parses")[0].compat_profile,
        None
    );
}
