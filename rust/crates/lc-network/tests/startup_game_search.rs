use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6, TcpListener};
use std::time::Duration;

use lc_network::{
    fetch_reference_endpoint, fetch_reference_endpoint_with_config, parse_reference_response,
    LanProbeTrigger, NetworkGameReference, NetworkGameSearch, NetworkGameSearchConfig,
    ReferenceEndpoint, ReferenceQueryConfig, ReferenceQuerySource, SearchCommand,
    StartupGameSearch, StartupGameSearchEvent, DEFAULT_MASTER_SERVER_URL,
};

#[test]
fn refresh_matches_cpp_lan_and_masterserver_fanout() {
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig {
        internet_enabled: true,
        master_server_url: DEFAULT_MASTER_SERVER_URL.to_string(),
        discovery_port: 22_114,
    });

    assert_eq!(
        search.refresh(),
        vec![
            SearchCommand::SendLanProbe {
                target: SocketAddrV6::new("ff02::1".parse::<Ipv6Addr>().unwrap(), 22_114, 0, 0,),
                payload: vec![0x03],
                trigger: LanProbeTrigger::ExplicitRefresh,
            },
            SearchCommand::QueryReferences {
                endpoint: ReferenceEndpoint::Url(DEFAULT_MASTER_SERVER_URL.to_string()),
                source: ReferenceQuerySource::Masterserver,
                timeout: Duration::from_secs(12),
            },
        ]
    );
}

#[test]
fn discovery_probe_commands_preserve_cpp_trigger_context() {
    // C4StartupNetDlg gives the same StartDiscovery call three distinct error
    // policies at its initial, explicit-refresh, and timer call sites (pristine
    // 9ffa0a5d src/C4StartupNetDlg.cpp:736-739, 1093-1105, 1122-1128).
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig::default());

    assert!(matches!(
        search.initial_commands().first(),
        Some(SearchCommand::SendLanProbe {
            trigger: LanProbeTrigger::Initial,
            ..
        })
    ));
    assert!(matches!(
        search.refresh().first(),
        Some(SearchCommand::SendLanProbe {
            trigger: LanProbeTrigger::ExplicitRefresh,
            ..
        })
    ));
    assert!(matches!(
        search.periodic_commands().first(),
        Some(SearchCommand::SendLanProbe {
            trigger: LanProbeTrigger::Periodic,
            ..
        })
    ));
}

#[test]
fn malformed_scheme_only_masterserver_remains_invalid_like_cpp() {
    // C4HTTPClient::Uri::ParseOldStyle only prepends http:// for BAD_SCHEME or
    // UNSUPPORTED_SCHEME. A bare `https:` is otherwise malformed and is not
    // replaced with the official server (src/C4HTTPClient.cpp:105-118;
    // src/C4Network2Reference.cpp:532-543).
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig {
        internet_enabled: true,
        master_server_url: "https:".to_string(),
        discovery_port: 22_114,
    });

    assert!(matches!(
        &search.refresh()[1],
        SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Url(url),
            source: ReferenceQuerySource::Masterserver,
            ..
        } if url == "https:"
    ));
}

#[test]
fn legacy_scheme_less_masterserver_gets_cpp_http_fallback() {
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig {
        internet_enabled: true,
        master_server_url: "league.clonkspot.org:80".to_string(),
        discovery_port: 22_114,
    });

    assert!(matches!(
        &search.refresh()[1],
        SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Url(url),
            source: ReferenceQuerySource::Masterserver,
            ..
        } if url == "http://league.clonkspot.org:80/"
    ));
}

#[test]
fn cpp_abi_padded_lan_reply_preserves_ipv6_scope_and_uses_native_port() {
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig::default());
    let source = SocketAddr::V6(SocketAddrV6::new(
        "fe80::1234".parse().unwrap(),
        11_114,
        0x55,
        7,
    ));
    let port = 11_111_u16.to_ne_bytes();

    assert_eq!(
        search.handle_lan_datagram(source, &[0x04, 0xa5, port[0], port[1]]),
        Some(SearchCommand::QueryReferences {
            endpoint: ReferenceEndpoint::Address(SocketAddr::V6(SocketAddrV6::new(
                "fe80::1234".parse().unwrap(),
                11_111,
                0x55,
                7,
            ))),
            source: ReferenceQuerySource::GameDiscovery,
            timeout: Duration::from_secs(12),
        })
    );
}

#[test]
fn lan_reply_requires_the_cpp_struct_size_and_is_capped_at_64() {
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig::default());
    let source = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 11_114, 0, 0));
    let port = 11_111_u16.to_ne_bytes();

    assert_eq!(
        search.handle_lan_datagram(source, &[0x04, port[0], port[1]]),
        None
    );
    for _ in 0..64 {
        assert!(search
            .handle_lan_datagram(source, &[0x04, 0, port[0], port[1]])
            .is_some());
    }
    assert_eq!(
        search.handle_lan_datagram(source, &[0x04, 0, port[0], port[1]]),
        None
    );

    search.refresh();
    assert!(search
        .handle_lan_datagram(source, &[0x04, 0, port[0], port[1]])
        .is_some());
}

#[test]
fn internet_toggle_only_suppresses_masterserver_query() {
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig {
        internet_enabled: false,
        ..NetworkGameSearchConfig::default()
    });

    assert!(matches!(
        search.refresh().as_slice(),
        [SearchCommand::SendLanProbe { payload, .. }] if payload == &[0x03]
    ));
}

#[test]
fn cpp_thirty_second_search_keeps_rows_while_reissuing_lan_and_master_queries() {
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig::default());
    search.merge_references([NetworkGameReference {
        title: "Existing".into(),
        ..NetworkGameReference::default()
    }]);

    let commands = search.periodic_commands();
    assert_eq!(search.references().len(), 1);
    assert!(matches!(
        commands.as_slice(),
        [SearchCommand::SendLanProbe { payload, .. }, SearchCommand::QueryReferences {
            source: ReferenceQuerySource::Masterserver,
            ..
        }] if payload == &[0x03]
    ));
}

#[test]
fn parses_cpp_reference_ini_and_keeps_incompatible_games_visible() {
    let mut response = br#"
[LegacyClonk]
Version=4,9,11,0,362

[Reference]
State=Lobby
CtrlMode=2
StartTime=100
JoinAllowed=true
Address=TCP:"203.0.113.10:11112",UDP:"203.0.113.10:11113"
Game="LegacyClonk"
Version=4,9,11
Build=363
MaxPlayers=13
Title="M~nchen Gold Rush"

  [Client]
  ID=0
  Activated=true
  Name="Host One"
  Nick="OracleNick"

[Reference]
State=Running
StartTime=101
JoinAllowed=false
PasswordNeeded=true
Address=TCP:"[2001:db8::7]:12112",UDP:"[2001:db8::7]:12113"
Game="LegacyClonk"
Version=4,9,11,0
Build=362
Title="Second Game"

  [Client]
  ID=0
  Name="Host Two"
"#
    .to_vec();
    *response.iter_mut().find(|byte| **byte == b'~').unwrap() = 0xfc;

    let references = parse_reference_response(&response).unwrap();
    assert_eq!(references.len(), 2);
    assert_eq!(references[0].title, "München Gold Rush");
    assert_eq!(references[0].host_name, "Host One");
    // These values are nested in C4Network2Status, C4GameParameters, and the
    // host C4ClientCore by C4Network2Reference::CompileFunc (pristine
    // 9ffa0a5d src/C4Network2Reference.cpp:71-105;
    // src/C4Network2.cpp:101-122; src/C4GameParameters.cpp:553-585;
    // src/C4Client.cpp:75-83).
    assert_eq!(references[0].host_nick, "OracleNick");
    assert_eq!(references[0].control_mode, 2);
    assert_eq!(references[0].max_players, 13);
    assert_eq!(references[0].version, [4, 9, 11, 0]);
    assert_eq!(references[0].build, 363);
    assert!(!references[0].is_joinable());
    assert_eq!(
        references[0].tcp_addresses,
        vec!["203.0.113.10:11112".parse().unwrap()]
    );
    assert_eq!(references[1].title, "Second Game");
    assert_eq!(references[1].state, "Running");
    assert!(references[1].password_needed);
    assert!(!references[1].is_joinable());
    assert_eq!(
        references[1].tcp_addresses,
        vec!["[2001:db8::7]:12112".parse().unwrap()]
    );
}

#[test]
fn cpp_dedupe_replaces_only_a_non_older_same_host_and_address() {
    let reference = |host: &str, address: &str, start_time| NetworkGameReference {
        title: format!("{host} game"),
        host_name: host.to_string(),
        start_time,
        tcp_addresses: vec![address.parse().unwrap()],
        ..NetworkGameReference::default()
    };
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig::default());

    search.merge_references([reference("Host", "203.0.113.1:11112", 50)]);
    search.merge_references([reference("Host", "203.0.113.1:11112", 49)]);
    assert_eq!(search.references().len(), 2);
    assert_eq!(search.references()[0].start_time, 50);
    assert_eq!(search.references()[1].start_time, 49);

    search.merge_references([reference("Host", "203.0.113.1:11112", 51)]);
    assert_eq!(search.references().len(), 2);
    assert_eq!(search.references()[0].start_time, 51);

    search.merge_references([reference("Other", "203.0.113.1:11112", 52)]);
    search.merge_references([reference("Host", "203.0.113.2:11112", 52)]);
    assert_eq!(search.references().len(), 4);
}

#[test]
fn cpp_reference_sort_puts_a_league_game_above_a_plain_lobby() {
    // C4Network2Reference::getSortOrder gives league games five points, while
    // lobby and password-free status contribute three and one respectively;
    // C4StartupNetListEntry inserts the higher score first (pristine 9ffa0a5d
    // src/C4Network2Reference.cpp:111-126;
    // src/C4StartupNetDlg.cpp:341-351, 534-557).
    let references = parse_reference_response(
        br#"[Reference]
State=Lobby
PasswordNeeded=false
Title="Plain lobby"

[Reference]
State=Running
PasswordNeeded=true
LeagueAddress="https://league.invalid/"
Title="League game"
"#,
    )
    .unwrap();
    let mut search = NetworkGameSearch::new(NetworkGameSearchConfig::default());

    search.merge_references(references);

    assert_eq!(
        search
            .references()
            .iter()
            .map(|reference| reference.title.as_str())
            .collect::<Vec<_>>(),
        ["League game", "Plain lobby"]
    );
}

#[tokio::test]
async fn reference_fetch_sends_cpp_identity_and_decodes_default_cp1252() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]);
        assert!(request.starts_with("GET / HTTP/1.1\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("user-agent: legacyclonk/4.9.11.0 [362]"));
        assert!(request
            .to_ascii_lowercase()
            .contains("accept-encoding: gzip"));
        assert!(request
            .to_ascii_lowercase()
            .contains("accept-charset: cp1252\r\n"));
        assert!(request
            .to_ascii_lowercase()
            .contains("accept-language: \r\n"));

        let mut body = br#"[Reference]
Address=TCP:"0.0.0.0:11112"
Version=4,9,11,0
Build=362
Title="Euro ~"
"#
        .to_vec();
        *body.iter_mut().find(|byte| **byte == b'~').unwrap() = 0x80;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain;charset=CP1252\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });

    let references =
        fetch_reference_endpoint(ReferenceEndpoint::Address(address), Duration::from_secs(2))
            .await
            .unwrap();
    server.join().unwrap();
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].title, "Euro €");
    assert_eq!(
        references[0].tcp_addresses,
        vec!["[::1]:11112".parse().unwrap()]
    );
}

#[tokio::test]
async fn reference_fetch_uses_cpp_configured_language_headers_and_charset() {
    // C4Network2HTTPClient::Query canonicalizes LanguageCharset through
    // C4Config::GetCharsetCodeName and sends LanguageEx verbatim, while the
    // response remains in that internal charset (pristine 9ffa0a5d
    // src/C4HTTPClient.cpp:184-200; src/C4Network2Reference.cpp:641-645;
    // src/C4Config.cpp:875-893).
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]).to_ascii_lowercase();
        assert!(request.contains("accept-charset: cp1252\r\n"));
        assert!(request.contains("accept-language: us,de\r\n"));

        let mut body = br#"[Reference]
Address=TCP:"0.0.0.0:11112"
Version=4,9,11,0
Build=362
Title="Euro ~"
"#
        .to_vec();
        *body.iter_mut().find(|byte| **byte == b'~').unwrap() = 0x80;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });

    let references = fetch_reference_endpoint_with_config(
        ReferenceEndpoint::Address(address),
        Duration::from_secs(2),
        &ReferenceQueryConfig {
            language_charset: "ANSI".to_string(),
            language_sequence: "US,DE".to_string(),
        },
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert_eq!(references[0].title, "Euro €");
}

#[test]
fn background_search_threads_reference_query_config_to_worker() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).unwrap();
    let master_address = listener.local_addr().unwrap();
    let discovery_port = std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]).to_ascii_lowercase();
        assert!(request.contains("accept-charset: cp1251\r\n"));
        assert!(request.contains("accept-language: ru,us\r\n"));
        let mut body = br#"[Reference]
Address=TCP:"127.0.0.1:11112"
Version=4,9,11,0
Build=362
Title="Visible ~ Game"

  [Client]
  ID=0
  Name="Visible Host"
"#
        .to_vec();
        *body.iter_mut().find(|byte| **byte == b'~').unwrap() = 0xc0;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });

    let search = StartupGameSearch::start_with_reference_config(
        NetworkGameSearchConfig {
            internet_enabled: true,
            master_server_url: format!("http://{master_address}/"),
            discovery_port,
        },
        ReferenceQueryConfig {
            language_charset: "RUSSIAN".to_string(),
            language_sequence: "RU,US".to_string(),
        },
    )
    .unwrap();
    search.refresh().unwrap();

    let mut visible = None;
    for _ in 0..10 {
        match search
            .events()
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
        {
            StartupGameSearchEvent::ReferencesUpdated(references) => {
                visible = Some(references);
                break;
            }
            StartupGameSearchEvent::Cleared | StartupGameSearchEvent::SearchError { .. } => {}
        }
    }
    server.join().unwrap();
    let visible = visible.expect("master reference update");
    assert_eq!(visible[0].title, "Visible А Game");
    assert_eq!(visible[0].host_name, "Visible Host");
}

#[tokio::test]
#[ignore = "requires the live C++ masterserver"]
async fn live_cpp_masterserver_response_is_accepted() {
    let references = fetch_reference_endpoint(
        ReferenceEndpoint::Url(DEFAULT_MASTER_SERVER_URL.to_string()),
        Duration::from_secs(12),
    )
    .await
    .unwrap();
    eprintln!(
        "live master returned {} game(s): {:?}",
        references.len(),
        references
            .iter()
            .map(|reference| reference.title.as_str())
            .collect::<Vec<_>>()
    );
}
