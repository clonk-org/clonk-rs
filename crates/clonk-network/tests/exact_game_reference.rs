use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpStream};

use clonk_engine::{
    ClientCoreControlData, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
    PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_JOINED,
};
use clonk_network::{
    encode_host_game_reference_response, encode_league_end_request, encode_league_start_request,
    encode_league_update_request, parse_reference_response, ClientPlayerInfosSnapshot,
    HostGameReference, HostGameReferenceError, HostGameReferenceMetadata,
    JoinClientRegistrySnapshot, JoinDataC4Id, JoinDataIdListEntry, JoinGameParametersEnvelope,
    JoinTeamListSnapshot, JoinTeamSnapshot, LeagueEndRecord, LeagueHeartbeat, LeagueHostSession,
    LeagueReferenceRequestEncodeError, NetpuncherGameIds, NetworkAddress, NetworkGameAdvertiser,
    NetworkGameAdvertiserConfig, NetworkGameReference, NetworkProtocol, PlayerInfoListSnapshot,
};

#[test]
fn cpp_reference_serializes_the_complete_game_parameters_snapshot_in_compile_order() {
    // C4Network2Reference::InitLocal copies the complete C4GameParameters and
    // CompileFunc then serializes it with pScenario == nullptr. This freezes
    // the resulting scalar/resource/player/team/client section shape from the
    // pristine oracle sources (9ffa0a5d src/C4Network2Reference.cpp:49-108;
    // src/C4GameParameters.cpp:553-585; src/C4Network2Res.cpp:109-143;
    // src/C4PlayerInfo.cpp:177-268,601-633,1733-1765;
    // src/C4Teams.cpp:138-150,556-603; src/C4Client.cpp:75-83,353-371).
    let summary = fixture_summary();
    let payload =
        HostGameReference::new(summary.clone(), fixture_metadata(), complete_parameters()).unwrap();

    let encoded = encode_host_game_reference_response(&payload).unwrap();

    assert_eq!(
        encoded,
        concat!(
            "[Reference]\r\n",
            "Icon=7\r\n",
            "State=Lobby\r\n",
            "CtrlMode=2\r\n",
            "Time=23\r\n",
            "Frame=24\r\n",
            "StartTime=100\r\n",
            "LeaguePerformance=25\r\n",
            "Comment=\"Host comment\"\r\n",
            "JoinAllowed=false\r\n",
            "PasswordNeeded=true\r\n",
            "Address=UDP:\"127.0.0.1:11113\",TCP:\"127.0.0.1:11112\"\r\n",
            "Game=\"LegacyClonk\"\r\n",
            "Version=4,9,11\r\n",
            "Build=362\r\n",
            "OfficialServer=true\r\n",
            "RandomSeed=7\r\n",
            "StartupPlayerCount=1\r\n",
            "MaxPlayers=9\r\n",
            "UseFairCrew=true\r\n",
            "FairCrewForced=true\r\n",
            "FairCrewStrength=123\r\n",
            "AllowDebug=false\r\n",
            "IsNetworkGame=true\r\n",
            "ControlRate=2\r\n",
            "AutoFrameSkip=true\r\n",
            "Rules=RULE=1\r\n",
            "Goals=GOAL=2\r\n",
            "League=\"Cup\"\r\n",
            "LeagueAddress=\"https://league.invalid/\"\r\n",
            "Title=\"Fixture\"\r\n",
            "\r\n",
            "  [Scenario]\r\n",
            "  Type=Scenario\r\n",
            "  ID=0\r\n",
            "  FileSize=11\r\n",
            "  FileCRC=12\r\n",
            "  ContentsCRC=13\r\n",
            "  Filename=\"Folder\\\\Fixture.c4s\"\r\n",
            "  Author=\"Maker\"\r\n",
            "\r\n",
            "  [Resource]\r\n",
            "  Type=System\r\n",
            "  ID=1\r\n",
            "  Loadable=false\r\n",
            "  ContentsCRC=14\r\n",
            "  Filename=\"System.c4g\"\r\n",
            "  Author=\"Maker\"\r\n",
            "\r\n",
            "  [PlayerInfos]\r\n",
            "  LastPlayerID=1\r\n",
            "\r\n",
            "    [Client]\r\n",
            "    ID=0\r\n",
            "    Flags=Initial\r\n",
            "\r\n",
            "      [Player]\r\n",
            "      Name=\"Alice\"\r\n",
            "      Filename=\"Alice.c4p\"\r\n",
            "      Flags=Joined\r\n",
            "      ID=1\r\n",
            "      Color=1122867\r\n",
            "      GameNumber=0\r\n",
            "      GameJoinFrame=0\r\n",
            "\r\n",
            "  [Teams]\r\n",
            "  Active=false\r\n",
            "  Custom=false\r\n",
            "  AllowHostilityChange=true\r\n",
            "  AllowTeamSwitch=true\r\n",
            "  AutoGenerateTeams=true\r\n",
            "  LastTeamID=1\r\n",
            "  TeamDistribution=Host\r\n",
            "  TeamColors=true\r\n",
            "  MaxScriptPlayers=2\r\n",
            "  ScriptPlayerNames=\"Bot\"\r\n",
            "  RandomTeamCount=1\r\n",
            "\r\n",
            "    [Team]\r\n",
            "    id=1\r\n",
            "    Name=New Republic\r\n",
            "    PlayerCount=1\r\n",
            "    Players=1\r\n",
            "    Color=16711680\r\n",
            "    IconSpec=\"Flag\"\r\n",
            "    MaxPlayer=2\r\n",
            "\r\n",
            "  [Client]\r\n",
            "  ID=0\r\n",
            "  Activated=true\r\n",
            "  Name=\"Host Name\"\r\n",
            "  Nick=\"Host Nick\"\r\n",
            "\r\n",
            "  [NetpuncherID]\r\n",
            "  IPv4=305419896\r\n",
            "  IPv6=2596069104\r\n",
            "NetpuncherAddr=\"puncher.invalid:11115\"\r\n",
        )
        .as_bytes()
    );
    assert_eq!(parse_reference_response(&encoded).unwrap(), vec![summary]);
}

#[test]
fn exact_host_reference_writes_clan_tag_as_raw_rct_all() {
    let mut parameters = complete_parameters();
    parameters.player_infos.clients[0].players[0].clan_tag = legacy(b"Cl\xe4n");
    let reference =
        HostGameReference::new(fixture_summary(), fixture_metadata(), parameters).unwrap();

    let encoded = encode_host_game_reference_response(&reference).unwrap();

    assert!(encoded
        .windows(b"      ClanTag=Cl\xe4n\r\n".len())
        .any(|window| window == b"      ClanTag=Cl\xe4n\r\n"));
}

#[test]
fn replacing_control_mode_refreshes_the_reference_projection() {
    let reference =
        HostGameReference::new(fixture_summary(), fixture_metadata(), complete_parameters())
            .unwrap();

    let central = reference.replacing_control_mode(1).unwrap();

    assert_eq!(central.summary().control_mode, 1);
    assert_eq!(central.parameters(), reference.parameters());
    let encoded = encode_host_game_reference_response(&central).unwrap();
    assert!(encoded
        .windows(b"CtrlMode=1\r\n".len())
        .any(|window| window == b"CtrlMode=1\r\n"));
    assert!(!encoded
        .windows(b"CtrlMode=2\r\n".len())
        .any(|window| window == b"CtrlMode=2\r\n"));
}

#[test]
fn replacing_netpuncher_state_updates_the_exact_reference_atomically() {
    let reference =
        HostGameReference::new(fixture_summary(), fixture_metadata(), complete_parameters())
            .unwrap();
    let addresses = vec![
        NetworkAddress::new(NetworkProtocol::Tcp, "198.51.100.4:11112".parse().unwrap()),
        NetworkAddress::new(NetworkProtocol::Udp, "198.51.100.4:11113".parse().unwrap()),
        NetworkAddress::new(NetworkProtocol::Udp, "198.51.100.4:43123".parse().unwrap()),
    ];

    let assigned = reference
        .replacing_netpuncher_state(
            NetpuncherGameIds {
                ipv4: 0x1122_3344,
                ipv6: 0x5566_7788,
            },
            addresses.clone(),
        )
        .unwrap();

    assert_eq!(assigned.summary().addresses, addresses);
    assert_eq!(assigned.metadata().addresses, addresses);
    assert_eq!(assigned.summary().netpuncher_ipv4, 0x1122_3344);
    assert_eq!(assigned.metadata().netpuncher_ipv6, 0x5566_7788);
    assert_eq!(
        assigned.summary().tcp_addresses,
        vec!["198.51.100.4:11112".parse::<SocketAddr>().unwrap()]
    );
    let encoded = encode_host_game_reference_response(&assigned).unwrap();
    const ADDRESS_LINE: &[u8] = b"Address=TCP:\"198.51.100.4:11112\",UDP:\"198.51.100.4:11113\",UDP:\"198.51.100.4:43123\"\r\n";
    assert!(encoded
        .windows(ADDRESS_LINE.len())
        .any(|window| window == ADDRESS_LINE));
    assert!(encoded
        .windows(b"  IPv4=287454020\r\n".len())
        .any(|window| window == b"  IPv4=287454020\r\n"));
    assert!(encoded
        .windows(b"  IPv6=1432778632\r\n".len())
        .any(|window| window == b"  IPv6=1432778632\r\n"));
    assert!(encoded
        .windows(b"NetpuncherAddr=\"puncher.invalid:11115\"\r\n".len())
        .any(|window| window == b"NetpuncherAddr=\"puncher.invalid:11115\"\r\n"));

    let runtime = assigned
        .replacing_runtime(assigned.parameters().clone(), "Running", 12, 34, false, 0)
        .unwrap();
    let game_over = runtime
        .replacing_game_over(
            runtime.parameters().clone(),
            "Running",
            56,
            78,
            false,
            9,
            std::iter::empty(),
        )
        .unwrap();
    for rebuilt in [&runtime, &game_over] {
        assert_eq!(rebuilt.summary().addresses, addresses);
        assert_eq!(rebuilt.summary().netpuncher_ipv4, 0x1122_3344);
        assert_eq!(rebuilt.metadata().netpuncher_ipv6, 0x5566_7788);
        assert_eq!(
            rebuilt.summary().tcp_addresses,
            vec!["198.51.100.4:11112".parse::<SocketAddr>().unwrap()]
        );
    }

    let cleared = assigned
        .replacing_netpuncher_state(NetpuncherGameIds::default(), addresses)
        .unwrap();
    let encoded = encode_host_game_reference_response(&cleared).unwrap();
    assert!(!encoded
        .windows(b"[NetpuncherID]".len())
        .any(|window| window == b"[NetpuncherID]"));
    assert!(encoded
        .windows(b"NetpuncherAddr=\"puncher.invalid:11115\"\r\n".len())
        .any(|window| window == b"NetpuncherAddr=\"puncher.invalid:11115\"\r\n"));
}

#[test]
fn cpp_league_host_requests_append_the_exact_reference_after_the_request_head() {
    // C4LeagueClient::{Start,Update,End} insert the complete Reference as a
    // sibling after Request and solve the checksum over both sections
    // (pristine 9ffa0a5d src/C4League.cpp:284-383). This fixture composes with
    // the byte-for-byte Reference assertion above so the boundary, head field
    // order, SHA representation and final proof-of-work bytes are all pinned.
    let reference =
        HostGameReference::new(fixture_summary(), fixture_metadata(), complete_parameters())
            .expect("fixture reference validates");
    let reference_bytes =
        encode_host_game_reference_response(&reference).expect("fixture reference serializes");
    let csid = legacy(b"session-7");
    let checksum_start = 0x1234_5678;

    let start = encode_league_start_request(&reference, checksum_start)
        .expect("start request checksum solves");
    let mut expected_start = b"[Request]\r\n\
Action=Start\r\n\
Checksum=sTSoj\r\n\
\r\n"
        .to_vec();
    expected_start.extend_from_slice(&reference_bytes);
    assert_eq!(checksum_value(&start), b"sTSoj");
    assert_eq!(start, expected_start);

    let update = encode_league_update_request(&csid, &reference, checksum_start)
        .expect("update request checksum solves");
    let mut expected_update = b"[Request]\r\n\
Action=Update\r\n\
CSID=session-7\r\n\
Checksum=QuXoj\r\n\
\r\n"
        .to_vec();
    expected_update.extend_from_slice(&reference_bytes);
    assert_eq!(checksum_value(&update), b"QuXoj");
    assert_eq!(update, expected_update);

    let mut session = LeagueHostSession::new();
    assert!(matches!(
        session.encode_update_request(&reference, checksum_start),
        Err(LeagueReferenceRequestEncodeError::MissingCsid)
    ));
    session
        .accept_start_response(b"[Response]\r\nStatus=Success\r\nCSID=session-7\r\n")
        .expect("Start response saves the host CSID");
    assert_eq!(
        session
            .encode_update_request(&reference, checksum_start)
            .expect("registered session builds Update"),
        update
    );

    let end_without_record = encode_league_end_request(&csid, &reference, None, checksum_start)
        .expect("recordless end request checksum solves");
    let mut expected_end_without_record = b"[Request]\r\n\
Action=End\r\n\
CSID=session-7\r\n\
Checksum=vxToj\r\n\
\r\n"
        .to_vec();
    expected_end_without_record.extend_from_slice(&reference_bytes);
    assert_eq!(checksum_value(&end_without_record), b"vxToj");
    assert_eq!(end_without_record, expected_end_without_record);
    assert_eq!(
        session
            .encode_end_request(&reference, None, checksum_start)
            .expect("registered session builds End"),
        end_without_record
    );

    let record = LeagueEndRecord {
        name: legacy(b"Round 7.c4r"),
        sha1: std::array::from_fn(|index| index as u8),
    };
    let end = encode_league_end_request(&csid, &reference, Some(&record), checksum_start)
        .expect("end request checksum solves");
    let mut expected_end = b"[Request]\r\n\
Action=End\r\n\
CSID=session-7\r\n\
Checksum=Cecoj\r\n\
RecordName=Round 7.c4r\r\n\
RecordSHA=000102030405060708090a0b0c0d0e0f10111213\r\n\
\r\n"
        .to_vec();
    expected_end.extend_from_slice(&reference_bytes);
    assert_eq!(checksum_value(&end), b"Cecoj");
    assert_eq!(end, expected_end);
}

#[test]
fn cpp_league_heartbeat_is_initially_due_and_uses_strict_wall_clock_deadlines() {
    // iLastLeagueUpdate starts at zero, so the first host Execute dispatches an
    // Update immediately. A successful dispatch records wall-clock time and
    // restores MasterReferencePeriod. InvalidateReference changes only the
    // delay to ten seconds; C++ tests `now > last + delay`, not `>=`
    // (pristine 9ffa0a5d src/C4Network2.cpp:209-214,707-718,2217-2222,
    // 2439-2464).
    let mut heartbeat = LeagueHeartbeat::new(60);

    assert!(heartbeat.is_due(1_000));

    heartbeat.update_dispatched(1_000);
    assert!(!heartbeat.is_due(1_060));
    assert!(heartbeat.is_due(1_061));

    heartbeat.update_dispatched(1_061);
    heartbeat.invalidate_reference();
    assert!(!heartbeat.is_due(1_071));
    assert!(heartbeat.is_due(1_072));

    heartbeat.update_dispatched(1_072);
    assert!(!heartbeat.is_due(1_132));
    assert!(heartbeat.is_due(1_133));

    heartbeat.invalidate_reference();
    assert!(heartbeat.is_due(2_000));
}

#[test]
fn game_over_reference_projects_global_and_per_player_league_performance() {
    let template =
        HostGameReference::new(fixture_summary(), fixture_metadata(), complete_parameters())
            .expect("fixture reference validates");
    let mut live_parameters = complete_parameters();
    let live_player = &mut live_parameters.player_infos.clients[0].players[0];
    live_player.flags |= PLAYER_INFO_FLAG_HAS_RESOURCE;
    live_player.resource = Some(player_resource(9));
    let updated = template
        .replacing_game_over(
            live_parameters,
            "Running",
            321,
            654,
            false,
            -17,
            [(1, 42), (999, 88)],
        )
        .expect("game-over reference validates");

    assert_eq!(updated.summary().state, "Running");
    assert!(!updated.summary().join_allowed);
    assert_eq!(
        (
            updated.metadata().time,
            updated.metadata().frame,
            updated.metadata().league_performance,
        ),
        (321, 654, -17)
    );
    let player = &updated.parameters().player_infos.clients[0].players[0];
    assert_eq!(player.id, 1);
    assert_eq!(player.league_performance, 42);
    assert_eq!(player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
    assert!(player.resource.is_none());

    let encoded =
        encode_host_game_reference_response(&updated).expect("game-over reference serializes");
    let encoded = String::from_utf8(encoded).expect("fixture reference is ASCII");
    assert!(encoded.contains("LeaguePerformance=-17\r\n"));
    assert!(encoded.contains("      LeaguePerformance=42\r\n"));
    assert!(!encoded.contains("        [Resource]\r\n"));
}

#[test]
fn rebuilt_reference_decodes_native_title_only_in_the_summary_projection() {
    let template =
        HostGameReference::new(fixture_summary(), fixture_metadata(), complete_parameters())
            .expect("fixture reference validates");
    let mut parameters = complete_parameters();
    parameters.title = legacy(b"Caf\xe9 Arena");

    let updated = template
        .replacing_parameters(parameters)
        .expect("native title rebuild validates");

    assert_eq!(updated.summary().title, "Caf\u{e9} Arena");
    assert_eq!(updated.parameters().title.as_bytes(), b"Caf\xe9 Arena");
}

#[test]
fn exact_host_reference_rejects_non_boolean_team_bytes_instead_of_canonicalizing_them() {
    // C4TeamList stores bools and StdCompilerINIWrite can only serialize true
    // or false. Raw noncanonical bytes are representable in the binary join
    // decoder but not in a host-created C++ reference (9ffa0a5d
    // src/C4Teams.cpp:556-578).
    let mut parameters = complete_parameters();
    parameters.teams.active = 2;

    let error =
        HostGameReference::new(fixture_summary(), fixture_metadata(), parameters).unwrap_err();

    assert_eq!(
        error,
        HostGameReferenceError::InvalidTeamBoolean {
            field: "Active",
            value: 2,
        }
    );
}

#[test]
fn exact_host_reference_rejects_raw_team_names_with_line_breaks() {
    // mkStringAdaptMA uses RCT_All and writes the team name directly into one
    // INI value (9ffa0a5d src/C4Teams.cpp:138-149;
    // src/StdCompiler.cpp:362-375). A line break cannot be represented there
    // without changing the surrounding reference structure.
    let mut parameters = complete_parameters();
    parameters.teams.teams[0].name = legacy(b"Broken\nTeam");

    let error =
        HostGameReference::new(fixture_summary(), fixture_metadata(), parameters).unwrap_err();

    assert_eq!(error, HostGameReferenceError::TeamNameContainsLineBreak);
}

#[test]
fn exact_advertiser_serves_the_complete_parameter_payload() {
    let mut parameters = complete_parameters();
    parameters.player_infos.clients[0].players[0].clan_tag = legacy(b"Cl\xe4n");
    parameters.teams.teams[0].name = legacy(b"T\xe4m");
    let payload =
        HostGameReference::new(fixture_summary(), fixture_metadata(), parameters).unwrap();
    let expected = encode_host_game_reference_response(&payload).unwrap();
    let advertiser = NetworkGameAdvertiser::start_exact(
        NetworkGameAdvertiserConfig {
            discovery_port: 0,
            reference_port: Some(0),
            language_charset: "RUSSIAN".to_string(),
        },
        payload,
    )
    .unwrap();
    let mut stream = TcpStream::connect(SocketAddr::from((
        Ipv6Addr::LOCALHOST,
        advertiser.reference_addr().port(),
    )))
    .unwrap();
    stream.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let body = &response[header_end..];

    assert!(headers.contains("Content-Type: text/plain; charset=CP1251\r\n"));
    assert_eq!(body, expected);
    assert!(body
        .windows(b"  [PlayerInfos]\r\n".len())
        .any(|window| window == b"  [PlayerInfos]\r\n"));
    assert!(body
        .windows(b"      ClanTag=Cl\xe4n\r\n".len())
        .any(|window| window == b"      ClanTag=Cl\xe4n\r\n"));
    assert!(body
        .windows(b"    Name=T\xe4m\r\n".len())
        .any(|window| window == b"    Name=T\xe4m\r\n"));
}

#[test]
fn init_local_discards_only_live_player_info_resources_before_reference_serialization() {
    // InitLocal clones Parameters, then DiscardResource clears HasResource and
    // ResCore for every PlayerInfos entry. RestorePlayerInfos is not traversed
    // (9ffa0a5d src/C4Network2Reference.cpp:49-65;
    // src/C4PlayerInfo.cpp:295-306).
    let mut parameters = complete_parameters();
    let live_player = &mut parameters.player_infos.clients[0].players[0];
    live_player.flags |= PLAYER_INFO_FLAG_HAS_RESOURCE;
    live_player.resource = Some(player_resource(9));

    let payload =
        HostGameReference::new(fixture_summary(), fixture_metadata(), parameters).unwrap();
    let live_player = &payload.parameters().player_infos.clients[0].players[0];
    assert_eq!(live_player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
    assert!(live_player.resource.is_none());
    let encoded = encode_host_game_reference_response(&payload).unwrap();
    assert!(!encoded
        .windows(b"HasResource".len())
        .any(|window| window == b"HasResource"));
    assert!(!encoded
        .windows(b"[ResCore]".len())
        .any(|window| window == b"[ResCore]"));

    let mut parameters = complete_parameters();
    let mut restored = parameters.player_infos.clients[0].players[0].clone();
    restored.flags |= PLAYER_INFO_FLAG_HAS_RESOURCE;
    restored.resource = Some(player_resource(10));
    parameters.restore_player_infos = PlayerInfoListSnapshot {
        last_player_id: 1,
        clients: vec![ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: vec![restored],
        }],
    };
    let payload =
        HostGameReference::new(fixture_summary(), fixture_metadata(), parameters).unwrap();
    let restored = &payload.parameters().restore_player_infos.clients[0].players[0];
    assert_ne!(restored.flags & PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
    assert_eq!(restored.resource.as_ref().map(|core| core.id), Some(10));
}

#[test]
fn exact_reference_rejects_a_display_tcp_projection_that_differs_from_metadata() {
    let mut summary = fixture_summary();
    summary.tcp_addresses = vec!["127.0.0.2:11112".parse().unwrap()];

    let error =
        HostGameReference::new(summary, fixture_metadata(), complete_parameters()).unwrap_err();

    assert_eq!(error, HostGameReferenceError::TcpAddressProjectionMismatch);
}

#[test]
fn exact_reference_rejects_a_canonical_address_set_that_differs_from_metadata() {
    // C++ stores and serializes one complete Addrs container; Rust's host-only
    // metadata and public summary projections must therefore describe the same
    // ordered protocol+endpoint set (pristine 9ffa0a5d
    // src/C4Network2Reference.h:68-73;
    // src/C4Network2Reference.cpp:81-85, 88-105).
    let mut summary = fixture_summary();
    summary.addresses.remove(0);

    let error =
        HostGameReference::new(summary, fixture_metadata(), complete_parameters()).unwrap_err();

    assert_eq!(error, HostGameReferenceError::AddressSetMismatch);
}

#[test]
fn exact_reference_rejects_netpuncher_metadata_that_differs_from_summary() {
    // C++ owns one NetpuncherGameID pair and one NetpuncherAddr in the
    // reference; Rust's split host representation must not advertise metadata
    // different from what join consumers see (pristine 9ffa0a5d
    // src/C4Network2Reference.h:62-63, 85-86;
    // src/C4Network2Reference.cpp:77-78, 107-108).
    let mut summary = fixture_summary();
    summary.netpuncher_ipv6 ^= 1;

    let error =
        HostGameReference::new(summary, fixture_metadata(), complete_parameters()).unwrap_err();

    assert_eq!(error, HostGameReferenceError::NetpuncherMetadataMismatch);
}

fn fixture_summary() -> NetworkGameReference {
    NetworkGameReference {
        icon: 7,
        title: "Fixture".into(),
        host_name: "Host Name".into(),
        host_nick: "Host Nick".into(),
        state: "Lobby".into(),
        control_mode: 2,
        time: 23,
        start_time: 100,
        comment: "Host comment".into(),
        join_allowed: false,
        password_needed: true,
        official_server: true,
        use_fair_crew: true,
        goals: vec!["GOAL".into()],
        league: "Cup".into(),
        league_address: "https://league.invalid/".into(),
        max_players: 9,
        player_names: vec!["Alice".into()],
        game: "LegacyClonk".into(),
        version: [4, 9, 11, 0],
        build: 362,
        addresses: vec![
            NetworkAddress::new(NetworkProtocol::Udp, "127.0.0.1:11113".parse().unwrap()),
            NetworkAddress::new(NetworkProtocol::Tcp, "127.0.0.1:11112".parse().unwrap()),
        ],
        source_address: "[::]:0".parse().unwrap(),
        netpuncher_ipv4: 0x1234_5678,
        netpuncher_ipv6: 0x9abc_def0,
        netpuncher_address: "puncher.invalid:11115".into(),
        tcp_addresses: vec!["127.0.0.1:11112".parse().unwrap()],
    }
}

fn fixture_metadata() -> HostGameReferenceMetadata {
    HostGameReferenceMetadata {
        icon: 7,
        time: 23,
        frame: 24,
        league_performance: 25,
        comment: legacy(b"Host comment"),
        addresses: vec![
            NetworkAddress::new(NetworkProtocol::Udp, "127.0.0.1:11113".parse().unwrap()),
            NetworkAddress::new(NetworkProtocol::Tcp, "127.0.0.1:11112".parse().unwrap()),
        ],
        netpuncher_ipv4: 0x1234_5678,
        netpuncher_ipv6: 0x9abc_def0,
        netpuncher_address: legacy(b"puncher.invalid:11115"),
    }
}

fn player_resource(id: i32) -> NetworkResourceCore {
    NetworkResourceCore {
        resource_type: 3,
        id,
        loadable: true,
        file_size: 100,
        file_crc: 101,
        contents_crc: 102,
        filename: legacy(b"Alice.c4p"),
        author: legacy(b"Maker"),
        ..NetworkResourceCore::default()
    }
}

fn complete_parameters() -> JoinGameParametersEnvelope {
    let maker = legacy(b"Maker");
    let player = ControlPlayerInfoEntry {
        name: legacy(b"Alice"),
        filename: legacy(b"Alice.c4p"),
        flags: PLAYER_INFO_FLAG_JOINED,
        id: 1,
        color: 0x0011_2233,
        original_color: 0x0011_2233,
        game_number: 0,
        game_join_frame: 0,
        ..ControlPlayerInfoEntry::default()
    };
    let empty_players = PlayerInfoListSnapshot::default();
    JoinGameParametersEnvelope {
        random_seed: 7,
        startup_player_count: 1,
        max_players: 9,
        use_fair_crew: true,
        fair_crew_forced: true,
        fair_crew_strength: 123,
        allow_debug: false,
        is_network_game: true,
        control_rate: 2,
        auto_frame_skip: true,
        rules: vec![id(*b"RULE", 1)],
        goals: vec![id(*b"GOAL", 2)],
        league: legacy(b"Cup"),
        league_address: legacy(b"https://league.invalid/"),
        title: legacy(b"Fixture"),
        scenario: NetworkResourceCore {
            resource_type: 1,
            id: 0,
            loadable: true,
            file_size: 11,
            file_crc: 12,
            contents_crc: 13,
            filename: legacy(b"Folder/Fixture.c4s"),
            author: maker.clone(),
            ..NetworkResourceCore::default()
        },
        game_resources: vec![NetworkResourceCore {
            resource_type: 5,
            id: 1,
            loadable: false,
            contents_crc: 14,
            filename: legacy(b"System.c4g"),
            author: maker,
            ..NetworkResourceCore::default()
        }],
        player_infos: PlayerInfoListSnapshot {
            last_player_id: 1,
            clients: vec![ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 1 << 2,
                players: vec![player],
            }],
        },
        restore_player_infos: empty_players,
        teams: JoinTeamListSnapshot {
            active: 0,
            custom: 0,
            allow_hostility_change: 1,
            allow_team_switch: 1,
            auto_generate_teams: 1,
            last_team_id: 1,
            team_distribution: 1,
            team_colors: 1,
            max_script_players: 2,
            script_player_names: legacy(b"Bot"),
            random_team_count: 1,
            teams: vec![JoinTeamSnapshot {
                id: 1,
                name: legacy(b"New Republic"),
                player_start_index: 0,
                player_ids: vec![1],
                color: 0x00ff_0000,
                icon_spec: legacy(b"Flag"),
                max_players: 2,
            }],
        },
        clients: JoinClientRegistrySnapshot {
            clients: vec![ClientCoreControlData {
                client_id: 0,
                activated: true,
                observer: false,
                name: legacy(b"Host Name"),
                nick: legacy(b"Host Nick"),
                lobby_ready: false,
            }],
            local_client_id: Some(0),
        },
    }
}

fn legacy(value: &[u8]) -> LegacyCString {
    crate::c4(value)
}

fn checksum_value(request: &[u8]) -> &[u8] {
    let prefix = b"Checksum=";
    let start = request
        .windows(prefix.len())
        .position(|window| window == prefix)
        .expect("league request has a checksum field")
        + prefix.len();
    &request[start..start + 5]
}

fn id(value: [u8; 4], count: i32) -> JoinDataIdListEntry {
    JoinDataIdListEntry {
        id: JoinDataC4Id::from_bytes(value).unwrap(),
        count,
    }
}
