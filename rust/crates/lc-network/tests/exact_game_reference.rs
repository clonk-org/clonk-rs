use std::io::{Read, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpStream};

use lc_engine::{
    ClientCoreControlData, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
    PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_JOINED,
};
use lc_network::{
    encode_host_game_reference_response, parse_reference_response, ClientPlayerInfosSnapshot,
    HostGameReference, HostGameReferenceError, HostGameReferenceMetadata,
    JoinClientRegistrySnapshot, JoinDataC4Id, JoinDataIdListEntry, JoinGameParametersEnvelope,
    JoinTeamListSnapshot, JoinTeamSnapshot, NetworkAddress, NetworkGameAdvertiser,
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
    let payload =
        HostGameReference::new(fixture_summary(), fixture_metadata(), complete_parameters())
            .unwrap();
    let expected = encode_host_game_reference_response(&payload).unwrap();
    let advertiser = NetworkGameAdvertiser::start_exact(
        NetworkGameAdvertiserConfig {
            discovery_port: 0,
            reference_port: 0,
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
    let body = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|offset| &response[offset + 4..])
        .unwrap();

    assert_eq!(body, expected);
    assert!(body
        .windows(b"  [PlayerInfos]\r\n".len())
        .any(|window| window == b"  [PlayerInfos]\r\n"));
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
        title: "Fixture".into(),
        host_name: "Host Name".into(),
        host_nick: "Host Nick".into(),
        state: "Lobby".into(),
        control_mode: 2,
        start_time: 100,
        join_allowed: false,
        password_needed: true,
        official_server: true,
        league_address: "https://league.invalid/".into(),
        max_players: 9,
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
    let empty_players = PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
    };
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
    LegacyCString::from_bytes(value.to_vec()).unwrap()
}

fn id(value: [u8; 4], count: i32) -> JoinDataIdListEntry {
    JoinDataIdListEntry {
        id: JoinDataC4Id::from_bytes(value).unwrap(),
        count,
    }
}
