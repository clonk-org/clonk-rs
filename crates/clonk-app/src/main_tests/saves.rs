// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! saves_fixture {
    (resource: $id:expr, $filename:expr $(,)?) => {
        clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: $id,
            loadable: true,
            filename: $filename,
            ..Default::default()
        }
    };
    (player_list: $last_player_id:expr, $clients:expr $(,)?) => {
        clonk_network::PlayerInfoListSnapshot {
            last_player_id: $last_player_id,
            clients: $clients,
        }
    };
    (client_players: $client_id:expr, $players:expr $(,)?) => {
        clonk_network::ClientPlayerInfosSnapshot {
            client_id: $client_id,
            flags: 0,
            players: $players,
        }
    };
    (player_info_id_name_filename: $id:expr, $name:expr, $filename:expr $(,)?) => {
        clonk_engine::ControlPlayerInfoEntry {
            id: $id,
            name: $name,
            filename: $filename,
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            ..Default::default()
        }
    };
    (player_info_id_flags_name_filename: $id:expr, $flags:expr, $name:expr, $filename:expr $(,)?) => {
        clonk_engine::ControlPlayerInfoEntry {
            id: $id,
            flags: $flags,
            name: $name,
            filename: $filename,
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            ..Default::default()
        }
    };
    (saved_game: $version:expr, $scenario:expr, $definition_load:expr, $focus_id:expr, $user_label:expr, $runtime_music_enabled:expr, $engine_state:expr $(,)?) => {
        SavedGameFile {
            version: $version,
            saved_at_seconds: 0,
            scenario: $scenario,
            definition_load: $definition_load,
            focus_id: $focus_id,
            user_label: $user_label,
            runtime_music_enabled: $runtime_music_enabled,
            source_save_player_infos: None,
            source_string_table: None,
            source_title_png: None,
            engine_state: $engine_state,
        }
    };
    (scenario: $identifier:expr, $title:expr, $kind:expr $(,)?) => {
        clonk_frontend::ScenarioSummary {
            identifier: $identifier,
            title: $title,
            kind: $kind,
        }
    };
    (sourced_gamepad: $gamepad:expr, $cluster:expr, $event:expr $(,)?) => {
        SourcedGamepadEvent {
            gamepad: $gamepad,
            cluster: $cluster,
            event: $event,
        }
    };
    (synchronize: $save_player_files:expr, $sync_clearance:expr $(,)?) => {
        clonk_engine::SynchronizeControlData {
            save_player_files: $save_player_files,
            sync_clearance: $sync_clearance,
            by_client: 0,
        }
    };
    (object_update $(,)?) => {
        ObjectUpdate {
            menu: Some(None),
            ..ObjectUpdate::default()
        }
    };
    (viewport_close: $closed_any:expr, $remaining_count:expr $(,)?) => {
        PhysicalViewportCloseEffect {
            closed_any: $closed_any,
            remaining_count: $remaining_count,
        }
    };
    (player_info_id_name_game_number: $id:expr, $name:expr, $game_number:expr $(,)?) => {
        clonk_engine::ControlPlayerInfoEntry {
            id: $id,
            name: $name,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            game_number: $game_number,
            ..Default::default()
        }
    };
}

#[test]
fn initial_record_uses_the_preinitialize_game_snapshot() {
    let directory = tempdir();
    let scenario_path = directory.path().join("Snapshot.c4s");
    fs::create_dir(&scenario_path).test_value();
    install_record_test_definitions(directory.path());
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Snapshot\n\n[Definitions]\nDefinition1=Objects.c4d\n",
    )
    .test_value();
    fs::write(
        scenario_path.join("PlayerInfos.txt"),
        b"stale initial roster",
    )
    .test_value();
    fs::write(scenario_path.join("CtrlRec.txt"), b"copied text stream").test_value();
    fs::write(
        scenario_path.join("RecPlayerInfos.txt"),
        b"copied final roster",
    )
    .test_value();
    let scenario_data =
        Scenario::load_from_path_with(&scenario_path, &InstallDefinitionResolver::new(None))
            .test_value();
    let scenario = FrontendScenario::from_command_line(&scenario_path);
    let mut app = new_state_only_menu_app(320, 200);
    app.recordings_dir = Some(directory.path().join("Records.c4f"));

    let initial_game_data = clonk_engine::InitialNetworkGameData {
        frame: 41,
        control_tick: 17,
        script_go: true,
        script_counter: 9,
        music_enabled: true,
        ..clonk_engine::InitialNetworkGameData::default()
    };

    app.prepare_recording_for(
        &scenario,
        &scenario_data,
        Some(InitialRecordingSource::Fresh(&initial_game_data)),
        None,
        None,
    )
    .test_value();
    let packed = app.recording_template.test_ref().group.pack().test_value();
    let record = Group::from_memory(PathBuf::from("Snapshot.c4s"), packed).test_value();
    let game = String::from_utf8(record.read_file("Game.txt").test_value()).test_value();

    main_assert!(!record.exists("PlayerInfos.txt"));
    main_assert_eq!(record.read_file("CtrlRec.txt").unwrap() => b"copied text stream");
    main_assert_eq!(record.read_file("RecPlayerInfos.txt").unwrap() => b"copied final roster");
    main_assert!(game.contains("Frame=41\r\n"));
    main_assert!(game.contains("ControlTick=17\r\n"));
    main_assert!(game.contains("Go=true\r\n"));
    main_assert!(game.contains("Counter=9\r\n"));
    main_assert!(game.contains("MusicEnabled=true\r\n"));
    main_assert_eq!(app.engine.frame() => 0, "live engine remains post-menu default");
}

#[test]
fn loaded_initial_record_reconstructs_exact_source_before_finitial_game_splice() {
    let directory = tempdir();
    let scenario_path = directory.path().join("Loaded.c4s");
    let section_path = scenario_path.join("SectArchive.c4g");
    fs::create_dir_all(&section_path).test_value();
    install_record_test_definitions(directory.path());
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Loaded record\n\n[Definitions]\nDefinition1=Objects.c4d\n",
    )
    .test_value();
    fs::write(
        scenario_path.join("Game.txt"),
        "[Game]\nTime=999\n\n[Player999]\nWealth=777\n",
    )
    .test_value();
    fs::write(scenario_path.join("Title.png"), b"original scenario title").test_value();
    fs::write(section_path.join("Archive.bin"), b"section payload").test_value();
    let scenario_data =
        Scenario::load_from_path_with(&scenario_path, &InstallDefinitionResolver::new(None))
            .test_value();
    let scenario = FrontendScenario::from_command_line(&scenario_path);
    let mut app = new_state_only_menu_app(320, 200);
    app.recordings_dir = Some(directory.path().join("Records.c4f"));
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData::new(
            0,
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 117,
                savegame_player: 17,
                name: LegacyCString::from_bytes(b"Current takeover".to_vec()).unwrap(),
                flags: 0,
                ..Default::default()
            }],
            -1,
        ));

    app.engine.register_test_definition(
        clonk_engine::Definition::from_script(
            "TST1",
            "Recorded object",
            "static PostMaterialization;",
        )
        .test_value(),
    );
    app.engine
        .spawn_test_object(clonk_engine::SpawnConfig::new("TST1").with_id(ObjectId::new(41)));
    let mut landscape = clonk_engine::Landscape::flat(2, 1);
    main_assert!(landscape.set_mode(clonk_engine::LANDSCAPE_MODE_EXACT));
    landscape.set_pixel_grid(clonk_engine::landscape::PixelGrid::new(
        2,
        1,
        vec![0, 1],
        vec![0; 256],
        vec![None; 256],
        vec![None; 256],
    ));
    app.engine.set_landscape(landscape);

    let mut restored = app.engine.capture_state();
    restored.frame = 73;
    restored.script_globals.named.insert(
        "PostMaterialization".to_owned(),
        Value::String("materialized string".to_owned().into()),
    );
    restored.players = vec![PlayerState {
        id: 2,
        player_info_id: 17,
        name: "Loaded player".to_owned(),
        status: PlayerStatus::Active,
        wealth: 99,
        ..PlayerState::default()
    }];
    restored.last_player_info_id = 17;
    app.engine.restore_state(&restored).test_value();

    app.prepare_recording_for(
        &scenario,
        &scenario_data,
        Some(InitialRecordingSource::Loaded {
            music_enabled: false,
            source_save_player_infos: Some(b"original saved roster"),
            source_title_png: Some(b"saved game screenshot"),
        }),
        None,
        None,
    )
    .test_value();
    // InitPlayers performs this only after InitControl has snapshotted
    // the initial record's current roster.
    app.control_player_infos
        .resume_joined_savegame_player(17, 0, false);
    let packed = app.recording_template.test_ref().group.pack().test_value();
    let record = Group::from_memory(PathBuf::from("Loaded.c4s"), packed).test_value();
    main_assert_eq!(record.read_file("SavePlayerInfos.txt").expect("original restore roster") => b"original saved roster");
    main_assert_eq!(record.read_file("Title.png").expect("loaded save title") => b"saved game screenshot");
    let initial_players = clonk_network::decode_player_info_list_ini(
        &record.read_file("PlayerInfos.txt").test_value(),
    )
    .test_value();
    main_assert_eq!(initial_players.clients[0].players[0].id => 117);
    main_assert_eq!(initial_players.clients[0].players[0].flags => 0);
    main_assert_eq!(app.control_player_infos.recreation_info_ids() => vec![17], "live takeover rows resume only after fInitial");

    let objects = String::from_utf8(record.read_file("Objects.txt").test_value()).test_value();
    main_assert!(objects.contains("id=TST1\r\n"));
    main_assert!(objects.contains("Number=41\r\n"));
    let landscape = clonk_resources::bitmap::IndexedBitmap::decode(
        &record.read_file("Landscape.bmp").test_value(),
    )
    .test_value();
    main_assert_eq!((landscape.width, landscape.height) => (2, 1));
    main_assert_eq!(landscape.indices => [0, 1]);
    main_assert!(record.exists("Landscape.png"));
    let strings = record.read_file("Strings.txt").test_value();
    main_assert!(strings.windows(b"materialized string".len()).any(|window| window == b"materialized string"));
    let section = record.open_child("SectArchive.c4g").test_value();
    main_assert_eq!(section.read_file("Archive.bin").expect("section payload") => b"section payload");

    let scenario_core =
        String::from_utf8(record.read_file("Scenario.txt").test_value()).test_value();
    for expected in ["SaveGame=1", "NoInitialize=1", "Replay=1"] {
        main_assert!(scenario_core.contains(expected), "missing {expected}");
    }

    let game = String::from_utf8(record.read_file("Game.txt").test_value()).test_value();
    let player_start = game.find("[Player17]").test_value();
    main_assert!(!game.contains("[Player999]"));
    main_assert!(game[player_start..].contains("Wealth=99\r\n"));
    main_assert_eq!(game.matches("[Player").count() => 1);

    let post_materialization = app
        .engine
        .capture_initial_record_game_data(false)
        .test_value();
    let expected_game = clonk_engine::serialize_initial_network_game(&post_materialization, None)
        .expect("serialize post-materialization initial projection")
        .test_value();
    let expected_game = String::from_utf8(expected_game).test_value();
    main_assert_eq!(game[..player_start].trim_end() => expected_game.trim_end(), "the non-player prefix must be the post-materialization fInitial projection",);
    main_assert!(game[..player_start].contains("Frame=73\r\n"));
    main_assert!(game[..player_start].contains("GlobalNamed="));
}

#[test]
fn classic_recdump_writes_cpp_text_and_binary_outputs() {
    let directory = tempdir();
    let text_path = directory.path().join("dump.TXT");
    let leading_dot_text_path = directory.path().join(".tXt");
    let binary_path = directory.path().join("dump.c4b");
    // One RCT_Ctrl with a noncanonical two-byte packed zero, followed by
    // End and an ignored suffix. C4Playback fully loads the record,
    // canonicalizes typed payloads, and stops at the first End.
    let input = [
        5,
        clonk_engine::RCT_CTRL,
        0x86,
        1,
        1,
        0x80,
        0,
        0xff,
        42,
        clonk_engine::RCT_END,
        9,
        clonk_engine::RCT_FRAME,
    ];

    let chunks = clonk_network::decode_control_record(&input).test_value();
    write_classic_record_dump(&chunks, &text_path).test_value();
    write_classic_record_dump(&chunks, &leading_dot_text_path).test_value();
    write_classic_record_dump(&chunks, &binary_path).test_value();

    let expected_text = concat!(
        "[Rec]\r\n",
        "Frame=5\r\n",
        "Type=0\r\n",
        "\r\n",
        "  [IDPacket]\r\n",
        "  ID=134\r\n",
        "\r\n",
        "    [Synchronize]\r\n",
        "    SavePlrs=true\r\n",
        "    SyncClear=true\r\n",
        "    ByClient=0\r\n",
        "\n\n",
        "[Rec]\r\n",
        "Frame=47\r\n",
        "Type=16\r\n",
        "\n\n",
    );
    main_assert_eq!(fs::read(&text_path).unwrap() => expected_text.as_bytes());
    main_assert_eq!(fs::read(&leading_dot_text_path).unwrap() => expected_text.as_bytes());
    main_assert_eq!(fs::read(&binary_path).unwrap() => [5, clonk_engine::RCT_CTRL, 0x86, 1, 1, 0, 0xff, 42, clonk_engine::RCT_END,]);

    let missing_parent = directory.path().join("missing/dump.c4b");
    let error = write_classic_record_dump(&chunks, &missing_parent)
        .expect_err("an unwritable destination must fail replay loading");
    main_assert!(error.to_string().contains("failed to write classic record dump"), "unexpected write diagnostic: {error:#}");
}

#[test]
fn classic_record_stream_forms_share_one_last_assignment() {
    let classic = parse_classic_command_line(&[
        OsString::from("First.c4r"),
        OsString::from("/stream:Second.c4r"),
        OsString::from("Third.C4R"),
    ]);
    main_assert_eq!(classic.record_stream => Some(PathBuf::from("Third.C4R")));

    let prefixed = parse_classic_command_line(&[OsString::from("/stream:Nested/League.c4r")]);
    main_assert_eq!(prefixed.record_stream => Some(PathBuf::from("Nested/League.c4r")), "the .c4r suffix must not retain the /stream: prefix");
    main_assert_eq!(parse_classic_command_line(&[OsString::from("/stream:")]).record_stream => Some(PathBuf::new()));
}

#[test]
fn classic_record_stream_is_converted_and_activated() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let working_directory = env::current_dir().test_value();
    let fixture = tempfile::Builder::new()
        .prefix("lc-record-stream-")
        .tempdir_in(&working_directory)
        .test_value();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);

    let origin_path = fixture.path().join("Origin.c4s");
    let definition_root = fixture.path().join("Defs.c4d");
    let definition_path = definition_root.join("Good.c4d");
    let origin_reference = origin_path
        .strip_prefix(&working_directory)
        .test_value()
        .to_string_lossy()
        .replace('\\', "/");
    let definition_reference = definition_root
        .strip_prefix(paths.install_root())
        .test_value()
        .to_string_lossy()
        .replace('\\', "/");
    fs::create_dir_all(&origin_path).test_value();
    fs::create_dir_all(&definition_path).test_value();
    fs::write(
        definition_path.join("DefCore.txt"),
        "[DefCore]\nid=GOOD\nName=Record fixture\nCategory=1\n",
    )
    .test_value();
    fs::write(definition_path.join("Script.c"), "// record fixture\n").test_value();
    write_test_definition_graphics(&definition_path);
    fs::write(origin_path.join("Scenario.txt"), format!(
        "[Head]\nTitle=Origin\nIcon=2\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1={}\n",
        definition_reference
    )).test_value();
    fs::write(origin_path.join("OriginOnly.txt"), b"copied from origin").test_value();
    fs::write(origin_path.join("Layer.txt"), b"origin").test_value();
    let mut origin_child = MutableGroup::new("OriginChild.c4g");
    origin_child
        .add_file("OriginChild.txt", b"origin child".to_vec())
        .test_value();
    fs::write(
        origin_path.join("OriginChild.c4g"),
        origin_child.pack().test_value(),
    )
    .test_value();

    let mut initial = MutableGroup::new("Initial.c4s");
    initial
            .add_file(
                "Scenario.txt",
                format!(
                    "[Head]\nTitle=Converted record\nIcon=2\nMaxPlayer=1\nSaveGame=1\nNoInitialize=1\nReplay=1\nOrigin={}\n\n[Definitions]\nDefinition1={}\n",
                    origin_reference,
                    definition_reference
                )
                .into_bytes(),
            ).test_value();
    initial
        .add_file("Layer.txt", b"initial".to_vec())
        .test_value();
    initial
        .add_file("InitialOnly.txt", b"initial component".to_vec())
        .test_value();
    let mut initial_child = MutableGroup::new("InitialChild.c4g");
    initial_child
        .add_file("InitialChild.txt", b"initial child".to_vec())
        .test_value();
    initial
        .add_child("InitialChild.c4g", initial_child)
        .test_value();
    let initial = initial.pack().test_value();
    let initial_child_raw =
        Group::from_top_level_memory(PathBuf::from("Initial.c4s"), initial.clone())
            .expect("reopen streamed initial save")
            .open_child("InitialChild.c4g")
            .expect("open original initial child")
            .raw_image()
            .test_value();

    let ignored_name = LegacyCString::from_bytes(b"ignored-initial-name.tmp".to_vec()).test_value();
    let mut raw =
        clonk_network::encode_league_stream_file_chunk(&ignored_name, &initial).test_value();
    let mut append_file = |delta: u8, name: &[u8], data: &[u8]| {
        let name = LegacyCString::from_bytes(name.to_vec()).test_value();
        let mut chunk = clonk_network::encode_league_stream_file_chunk(&name, data).test_value();
        chunk[0] = delta;
        raw.extend_from_slice(&chunk);
    };
    append_file(2, b"Layer.txt", b"later");
    let mut later_child = MutableGroup::new("WrongSortName.tmp");
    later_child
        .add_file("LaterChild.txt", b"later child".to_vec())
        .test_value();
    append_file(3, b"LaterChild.c4g", &later_child.pack().test_value());
    append_file(4, b"CtrlRec.c4b", b"must be replaced");
    raw.extend_from_slice(&[5, clonk_engine::RCT_FRAME]);

    let stream_path = fixture.path().join("League.c4r");
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&raw).test_value();
    fs::write(&stream_path, encoder.finish().test_value()).test_value();

    let classic = parse_classic_command_line(&[
        OsString::from(format!("/stream:{}", stream_path.display())),
        OsString::from("/network"),
    ]);
    main_assert_eq!(classic.record_stream.as_deref() => Some(stream_path.as_path()));
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.apply_classic_command_line(&classic).test_value();
    app.launch_classic_command_line_scenario().test_value();
    main_assert!(app.auto_start_classic_command_line_scenario);
    wait_for_running_with_attempts(&mut app, 2_400);

    let output_path = fixture.path().join("League.c4s");
    main_assert_eq!(app.classic_command_line.scenario.as_deref() => Some(output_path.as_path()));
    main_assert_eq!(app.active_scenario.as_ref().and_then(|scenario| scenario.path.as_deref()) => Some(output_path.as_path()));
    main_assert!(app.control_playback.is_some());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(!app.classic_record_stream_activation_pending);
    main_assert!(output_path.is_dir());
    for child in ["OriginChild.c4g", "InitialChild.c4g", "LaterChild.c4g"] {
        main_assert!(output_path.join(child).is_file(), "folder-backed conversion must retain {child} as a packed file");
    }
    let output = Group::open(&output_path).test_value();
    main_assert_eq!(output.read_file("OriginOnly.txt").expect("origin component") => b"copied from origin");
    main_assert_eq!(output.read_file("InitialOnly.txt").expect("initial component") => b"initial component");
    main_assert_eq!(output.read_file("Layer.txt").expect("later overlay") => b"later");
    main_assert_eq!(
        output.read_file("CtrlRec.c4b").expect("converted CtrlRec") =>
        [14, clonk_engine::RCT_FRAME],
        "file chunks are removed and their deltas folded into the retained stream"
    );
    main_assert_eq!(
        output
            .open_child("LaterChild.c4g")
            .expect("open streamed packed child")
            .read_file("LaterChild.txt")
            .expect("streamed child component") =>
        b"later child"
    );
    main_assert_eq!(
        output
            .open_child("InitialChild.c4g")
            .expect("open initial packed child")
            .raw_image()
            .expect("read initial child image") =>
        initial_child_raw,
        "unpacking the streamed initial save preserves child images"
    );
    reset_cached_app_paths();
}

#[test]
fn resumed_savegame_replay_recreates_players_from_recorded_profiles() {
    // Replay RestoreSavegameInfos changes the current row to the saved ID,
    // then RecreatePlayerFiles loads Recreate-<ID>.c4p before RecreatePlayers
    // restores Game.txt runtime state (C4PlayerInfo.cpp:1395-1421,1448-1518,
    // 1524-1607).
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let frontend = install_minimal_prepared_host_fixture(content.path());
    let replay_path = frontend.path.clone().test_value();
    let scenario_core = fs::read_to_string(replay_path.join("Scenario.txt"))
        .test_value()
        .replacen("[Head]\n", "[Head]\nReplay=1\nSaveGame=1\n", 1);
    fs::write(replay_path.join("Scenario.txt"), scenario_core).test_value();
    fs::write(
        replay_path.join("Game.txt"),
        concat!(
            "[Game]\nTime=17\n\n",
            "[Player7]\nStatus=1\nIndex=2\nID=7\n",
            "AtClient=99\nAtClientName=stale\nWealth=19\n\n",
            "[Player8]\nStatus=1\nIndex=3\nID=8\nWealth=23\n",
            "[Player6]\nStatus=1\nIndex=4\nID=6\nWealth=29\n",
        ),
    )
    .test_value();
    fs::write(
        replay_path.join("CtrlRec.c4b"),
        [0, clonk_engine::RCT_FRAME],
    )
    .test_value();
    fs::write(
        replay_path.join("Teams.txt"),
        b"[Teams]\nActive=1\nAutoGenerateTeams=1\n",
    )
    .test_value();
    let native = |bytes: &[u8]| LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let recorded_resource = saves_fixture!(resource: 17, native(b"Alice.c4p"));
    let fallback_profile_path = user_data.path().join("Local.c4p");
    fs::create_dir(&fallback_profile_path).test_value();
    fs::write(
        fallback_profile_path.join("Player.txt"),
        "[Player]\nName=Local recorded profile\nScore=37\n[Preferences]\nControl=1\nMouse=0\n",
    )
    .test_value();
    let fallback_profile_filename = legacy_cstring(clonk_resources::path_to_legacy_bytes(
        &fallback_profile_path,
    ));
    let stale_resource = saves_fixture!(resource: 18, native(b"Wrong.c4p"));
    let stale_resource_path = user_data.path().join("Wrong.c4p");
    fs::create_dir(&stale_resource_path).test_value();
    fs::write(
        stale_resource_path.join("Player.txt"),
        "[Player]\nName=Wrong resource profile\nScore=99\n",
    )
    .test_value();
    let missing_profile_filename = legacy_cstring(clonk_resources::path_to_legacy_bytes(
        &user_data.path().join("Missing.c4p"),
    ));
    let current_infos = saves_fixture!(
        player_list:
            100,
            vec![
                        saves_fixture!(
                client_players:
                    4,
                    vec![
                                        clonk_engine::ControlPlayerInfoEntry {
                                            id: 100,
                                            name: native(b"New-team ordering probe"),
                                            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                                            team: -1,
                                            ..Default::default()
                                        },
                                        clonk_engine::ControlPlayerInfoEntry {
                                            id: 91,
                                            name: native(b"Current replay player"),
                                            filename: native(b"Alice.c4p"),
                                            flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                                            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                                            color: 0x0044_5566,
                                            team: 4,
                                            resource: Some(recorded_resource),
                                            ..Default::default()
                                        },
                                        saves_fixture!(
                        player_info_id_name_filename:
                            93,
                            native(b"Missing replay profile"),
                            missing_profile_filename,
                    ),
                                    ],
            ),
                        saves_fixture!(
                client_players:
                    -1,
                    vec![clonk_engine::ControlPlayerInfoEntry {
                                        id: 92,
                                        name: native(b"Unknown-client replay player"),
                                        filename: fallback_profile_filename,
                                        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                                        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                                        resource: Some(stale_resource.clone()),
                                        ..Default::default()
                                    }],
            ),
                    ],
    );
    let restore_infos = saves_fixture!(
        player_list:
            7,
            vec![
                        saves_fixture!(
                client_players:
                    4,
                    vec![
                                        clonk_engine::ControlPlayerInfoEntry {
                                            id: 7,
                                            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                                                | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
                                            name: native(b"Current replay player"),
                                            filename: native(b"Alice.c4p"),
                                            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                                            color: 0x0011_2233,
                                            team: 3,
                                            ..Default::default()
                                        },
                                        saves_fixture!(
                        player_info_id_flags_name_filename:
                            6,
                            clonk_engine::PLAYER_INFO_FLAG_JOINED
                                                        | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
                            native(b"Missing replay profile"),
                            native(b"Missing.c4p"),
                    ),
                                    ],
            ),
                        saves_fixture!(
                client_players:
                    -1,
                    vec![saves_fixture!(
                        player_info_id_flags_name_filename:
                            8,
                            clonk_engine::PLAYER_INFO_FLAG_JOINED
                                                    | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
                            native(b"Unknown-client replay player"),
                            native(b"Local.c4p"),
                    )],
            ),
                    ],
    );
    fs::write(
        replay_path.join("PlayerInfos.txt"),
        clonk_network::encode_player_info_list_ini(&current_infos).test_value(),
    )
    .test_value();
    fs::write(
        replay_path.join("SavePlayerInfos.txt"),
        clonk_network::encode_player_info_list_ini(&restore_infos).test_value(),
    )
    .test_value();
    let profile_path = replay_path.join("Recreate-7.c4p");
    fs::create_dir(&profile_path).test_value();
    fs::write(
        profile_path.join("Player.txt"),
        "[Player]\nName=Recorded profile\nScore=31\n[Preferences]\nControl=0\nMouse=0\n",
    )
    .test_value();
    let packed_replay_path = content.path().join("PackedReplay.c4s");
    let replay_group = Group::open(&replay_path).test_value();
    fs::write(
        &packed_replay_path,
        MutableGroup::from_group(&replay_group)
            .test_value()
            .pack()
            .test_value(),
    )
    .test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.admission_resources
        .mark_complete(stale_resource.id, stale_resource_path);
    app.start_scenario(FrontendScenario::from_command_line(&packed_replay_path))
        .test_value();
    wait_for_running_with_attempts(&mut app, 2_400);

    let player = app.engine.test_player(2);
    main_assert_eq!(
        (
            player.player_info_id(),
            player.name(),
            player.wealth(),
            player.score(),
            player.at_client(),
            player.at_client_name(),
        ) =>
        (
            7,
            "Current replay player",
            19,
            31,
            clonk_engine::PlayerAtClient::new(4),
            "Replay",
        )
    );
    let resumed_info = app.control_player_infos.get(7).test_value();
    main_assert!(resumed_info.filename.is_empty(), "DeleteTempFile clears the extracted Recreate filename after the join");
    main_assert!(resumed_info.resource.is_none());
    main_assert_eq!(
        resumed_info.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE =>
        clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        "replay DiscardResource has no live resource pointer and preserves HasRes"
    );
    main_assert_eq!((resumed_info.game_number, resumed_info.game_join_frame) => (-1, -1), "non-scenario-init recreation does not call SetJoined");
    main_assert!(app.control_player_infos.get(91).is_none());
    main_assert!(
        app.control_player_infos
            .get(6)
            .is_some_and(|info| { info.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0 }),
        "a failed filename-backed replay join marks its PlayerInfo removed"
    );
    let failed_result = app
        .engine
        .snapshot()
        .round_results
        .players
        .into_iter()
        .find(|result| result.player_info_id == 6)
        .test_value();
    main_assert_eq!((failed_result.total_playing_time, failed_result.score_old, failed_result.score_new,) => (0, 0, None));
    // A failed/missing Recreate extraction leaves the original filename in
    // place, so RecreatePlayers can still load the installed profile
    // (C4PlayerInfo.cpp:1468-1504,1566-1603).
    let local_player = app.engine.test_player(3);
    main_assert_eq!(
        (
            local_player.player_info_id(),
            local_player.name(),
            local_player.wealth(),
            local_player.score(),
            local_player.at_client(),
            local_player.at_client_name(),
        ) =>
        (
            8,
            "Unknown-client replay player",
            23,
            37,
            clonk_engine::PlayerAtClient::UNKNOWN,
            "Replay",
        )
    );
    main_assert!(app.local_controls.assignment(3).is_some());
    main_assert_eq!(app.engine.snapshot().hud.local_players => vec![3]);
    let teams = app.engine.teams();
    main_assert!(teams.iter().any(|team| team.id == 4));
    // RecheckAutoGeneratedTeams visits PlayerInfos in packet/row order. A
    // TEAMID_New row before team 4 therefore generates team 1 first and must
    // not append team 5 (C4PlayerInfo.cpp:819-831; C4Teams.cpp:409-420).
    main_assert!(!teams.iter().any(|team| team.id == 5));
    main_assert_eq!(teams.iter().find(|team| team.id == 3).map(|team| team.player_ids.as_slice()) => Some(&[7][..]));
    main_assert!(app.control_clients.snapshot().is_empty(), "replay PlayerInfos packets do not synthesize Parameters.Clients");
    main_assert_eq!(app.engine.game_time() => 17);
    main_assert_eq!(app.control_player_infos.retained_rows_snapshot().0 => 7, "SavePlayerInfos overwrites the replay PlayerInfos ID counter");
    // InitGame snapshots the raw PlayerInfos before InitPlayers merges the
    // restore list (C4Game.cpp:2390-2399,2827-2850).
    main_assert_eq!(
        app.restart_restore_infos
            .players
            .get(b"Current replay player".as_slice()) =>
        Some(&RestartRestorePlayerInfo {
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            team: 4,
            color: 0x0044_5566,
        })
    );
    main_assert!(app.control_playback.is_some());
    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientUpdate(
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                client_id: 4,
                data: 1,
                by_client: 0,
            },
        )],
    )
    .test_value();
    main_assert!(app.engine.player(2).is_some(), "ClientUpdate is a no-op when Parameters.Clients has no matching row");
    // ClientRemove has a replay-only absent-client fallback that removes the
    // client's runtime players without inventing a client row
    // (C4Control.cpp:578-584,637-649).
    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 4,
                reason: LegacyCString::default(),
                by_client: 0,
            },
        )],
    )
    .test_value();
    main_assert!(app.engine.player(2).is_none());
    main_assert!(app.engine.player(3).is_some());
    reset_cached_app_paths();
}

#[test]
fn savegame_replay_empty_current_packet_does_not_adopt_restore_players() {
    // Runtime-record fallback tests C4PlayerInfoList::GetInfoCount, the client
    // packet count, so one retained empty packet suppresses adoption of the
    // restore list (C4PlayerInfo.cpp:1422-1439; C4PlayerInfo.h:373).
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let frontend = install_minimal_prepared_host_fixture(content.path());
    let replay_path = frontend.path.clone().test_value();
    let scenario_core = fs::read_to_string(replay_path.join("Scenario.txt"))
        .test_value()
        .replacen("[Head]\n", "[Head]\nReplay=1\nSaveGame=1\n", 1);
    fs::write(replay_path.join("Scenario.txt"), scenario_core).test_value();
    fs::write(
        replay_path.join("Game.txt"),
        b"[Player7]\nStatus=1\nIndex=2\nID=7\n",
    )
    .test_value();
    fs::write(
        replay_path.join("Objects.txt"),
        b"[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=262144\nOwner=2\nX=10\nY=10\n",
    )
    .test_value();
    fs::write(
        replay_path.join("CtrlRec.c4b"),
        [0, clonk_engine::RCT_FRAME],
    )
    .test_value();
    let current_infos =
        saves_fixture!(player_list: 91, vec![saves_fixture!(client_players: 4, Vec::new())]);
    let restore_infos = saves_fixture!(
        player_list:
            7,
            vec![saves_fixture!(
                client_players:
                    4,
                    vec![clonk_engine::ControlPlayerInfoEntry {
                                    id: 7,
                                    game_number: 2,
                                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                                        | clonk_engine::PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
                                    player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                                    ..Default::default()
                                }],
            )],
    );
    fs::write(
        replay_path.join("PlayerInfos.txt"),
        clonk_network::encode_player_info_list_ini(&current_infos).test_value(),
    )
    .test_value();
    fs::write(
        replay_path.join("SavePlayerInfos.txt"),
        clonk_network::encode_player_info_list_ini(&restore_infos).test_value(),
    )
    .test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.start_scenario(frontend).test_value();
    wait_for_running_with_attempts(&mut app, 2_400);

    main_assert!(app.engine.player(2).is_none());
    main_assert_eq!(app.engine.object_snapshot(clonk_engine::ObjectId::new(10)).expect("unassociated crew tombstone").status => clonk_engine::ObjectStatus::Deleted);
    main_assert_eq!(app.control_player_infos.retained_rows_snapshot().0 => 7);
    main_assert!(app.control_player_infos.contains_client(4));
    reset_cached_app_paths();
}

#[test]
fn savegame_replay_removed_restore_rows_do_not_gain_associations() {
    // RestoreSavegameInfos does nothing when the restore list has no active
    // players, so even an otherwise exact match stays unassociated
    // (C4Game.cpp:2827-2851; C4PlayerInfo.cpp:1371-1393).
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let frontend = install_minimal_prepared_host_fixture(content.path());
    let replay_path = frontend.path.clone().test_value();
    let scenario_core = fs::read_to_string(replay_path.join("Scenario.txt"))
        .test_value()
        .replacen("[Head]\n", "[Head]\nReplay=1\nSaveGame=1\n", 1);
    fs::write(replay_path.join("Scenario.txt"), scenario_core).test_value();
    fs::write(replay_path.join("Game.txt"), b"[Game]\nTime=11\n").test_value();
    fs::write(
        replay_path.join("CtrlRec.c4b"),
        [0, clonk_engine::RCT_FRAME],
    )
    .test_value();
    let native = |bytes: &[u8]| LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let current_infos = saves_fixture!(
        player_list:
            91,
            vec![saves_fixture!(
                client_players:
                    4,
                    vec![saves_fixture!(
                        player_info_id_name_filename:
                            91,
                            native(b"Removed restore match"),
                            native(b"Alice.c4p"),
                    )],
            )],
    );
    let restore_infos = saves_fixture!(
        player_list:
            7,
            vec![saves_fixture!(
                client_players:
                    4,
                    vec![clonk_engine::ControlPlayerInfoEntry {
                                    id: 7,
                                    name: native(b"Removed restore match"),
                                    filename: native(b"Alice.c4p"),
                                    flags: clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                                    player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                                    ..Default::default()
                                }],
            )],
    );
    fs::write(
        replay_path.join("PlayerInfos.txt"),
        clonk_network::encode_player_info_list_ini(&current_infos).test_value(),
    )
    .test_value();
    fs::write(
        replay_path.join("SavePlayerInfos.txt"),
        clonk_network::encode_player_info_list_ini(&restore_infos).test_value(),
    )
    .test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.start_scenario(frontend).test_value();
    wait_for_running_with_attempts(&mut app, 2_400);

    let current = app.control_player_infos.get(91).test_value();
    main_assert_eq!(current.savegame_player => 0);
    main_assert_eq!(app.control_player_infos.retained_rows_snapshot().0 => 7);
    reset_cached_app_paths();
}

#[test]
fn savegame_replay_removed_only_restore_still_overwrites_id_counter() {
    // C4GameParameters::Load transfers the SavePlayerInfos allocator counter
    // even when every restore row is Removed and InitPlayers has no work
    // (C4GameParameters.cpp:379-399; C4Game.cpp:2827-2851).
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let frontend = install_minimal_prepared_host_fixture(content.path());
    let replay_path = frontend.path.clone().test_value();
    let scenario_core = fs::read_to_string(replay_path.join("Scenario.txt"))
        .test_value()
        .replacen("[Head]\n", "[Head]\nReplay=1\nSaveGame=1\n", 1);
    fs::write(replay_path.join("Scenario.txt"), scenario_core).test_value();
    fs::write(replay_path.join("Game.txt"), b"[Game]\nTime=11\n").test_value();
    fs::write(
        replay_path.join("CtrlRec.c4b"),
        [0, clonk_engine::RCT_FRAME],
    )
    .test_value();
    let restore_infos = saves_fixture!(
        player_list:
            7,
            vec![saves_fixture!(
                client_players:
                    4,
                    vec![clonk_engine::ControlPlayerInfoEntry {
                                    id: 7,
                                    flags: clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                                    player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                                    ..Default::default()
                                }],
            )],
    );
    fs::write(
        replay_path.join("SavePlayerInfos.txt"),
        clonk_network::encode_player_info_list_ini(&restore_infos).test_value(),
    )
    .test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.start_scenario(frontend).test_value();
    wait_for_running_with_attempts(&mut app, 2_400);

    main_assert_eq!(app.control_player_infos.retained_rows_snapshot().0 => 7);
    main_assert!(app.control_player_infos.get(7).is_none());
    reset_cached_app_paths();
}

#[test]
fn unusable_classic_record_stream_exits_without_showing_startup() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let stream_path = fixture.path().join("Broken.c4r");
    fs::write(&stream_path, b"not a zlib stream").test_value();
    let classic = parse_classic_command_line(&[stream_path.clone().into_os_string()]);
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.apply_classic_command_line(&classic).test_value();
    app.launch_classic_command_line_scenario().test_value();
    for _ in 0..2_400 {
        if app.exit_requested {
            break;
        }
        app.test_update();
        thread::sleep(Duration::from_millis(2));
    }
    main_assert!(app.exit_requested, "explicit unusable .c4r must terminate");
    main_assert_ne!(app.mode => AppMode::Menu);
    main_assert!(app.status_text.contains("Could not process record stream data"));
    reset_cached_app_paths();
}

#[test]
fn save_description_language_preserves_an_explicit_empty_first_segment() {
    let _lock = env_lock().lock();
    main_assert_eq!(materialized_save_description_language(b"[General]\nLanguage=,DE\n") => Vec::<u8>::new());
    main_assert_eq!(materialized_save_description_language(b"[General]\nLanguage=DE,US\n") => b"DE");
    main_assert_eq!(materialized_save_description_language(b"[General]\nLanguageEx=DE,US\n") => classic_loader_system_language().unwrap_or("US").as_bytes());
}

#[test]
fn default_bad_safety_restores_and_saves_defaults() {
    let dir = tempdir();
    let path = dir.path().join("clonk-rust.config");
    fs::write(&path, "[General]\nConfigResetSafety=7junk\nVendorPoison=keep\n\n[Graphics]\nResolutionX=1234\n\n[Vendor]\nPoison=yes\n").test_value();

    main_assert!(validate_or_repair_startup_config(&path, false).expect("repair default config"));

    let repaired = Config::load(&path).test_value();
    main_assert_eq!(repaired.get_in(Some("General"), "ConfigResetSafety") => Some("42"));
    main_assert_eq!(repaired.get_in(Some("Graphics"), "ResolutionX") => Some("800"));
    main_assert_eq!(repaired.get_in(Some("General"), "Version") => Some("362"));
    main_assert_eq!(repaired.get_in(Some("Graphics"), "Shader") => Some("1"));
    main_assert_eq!(repaired.get_in(Some("General"), "VendorPoison") => None);
    main_assert_eq!(repaired.get_in(Some("Vendor"), "Poison") => None);
}

#[test]
fn default_zero_resolution_restores_and_saves_defaults() {
    let dir = tempdir();
    let path = dir.path().join("clonk-rust.config");
    fs::write(
        &path,
        "[General]\nConfigResetSafety=42\n\n[Graphics]\nResolutionX=0junk\nResolutionY=777\n",
    )
    .test_value();

    main_assert!(validate_or_repair_startup_config(&path, false).expect("repair zero resolution"));
    let repaired = Config::load(&path).test_value();
    main_assert_eq!(repaired.get_in(Some("Graphics"), "ResolutionX") => Some("800"));
    main_assert_eq!(repaired.get_in(Some("Graphics"), "ResolutionY") => Some("600"));
}

#[test]
fn one_shot_sample_stops_at_the_cpp_twenty_instance_limit() {
    // NewInstance refuses a 21st non-looping instance of one resolved
    // sample before it asks SDL_mixer for another channel
    // (C4SoundSystem.cpp:337-338; C4SoundSystem.h:130).
    let dir = tempdir();
    let scenario = dir.path().join("Goldrush.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("HorseWalk1.wav"), silent_pcm_wav(10_000)).test_value();

    let options = AudioOptions {
        max_channels: 20,
        ..AudioOptions::default()
    };
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));

    let horses: Vec<_> = (0..21)
        .map(|index| {
            make_object(
                index + 1,
                "HORS",
                Vector2::new(i32::try_from(index).test_value() * 51, 100),
            )
        })
        .collect();
    let snapshot = make_snapshot(horses.clone(), Vec::new());
    for horse in &horses {
        audio
            .start_sound(
                "HorseWalk*",
                Some(horse.id),
                100,
                false,
                true,
                None,
                &snapshot,
                &[audio_viewport(0, OWNER_NONE, horses[0].position)],
            )
            .test_value();
    }
    main_assert_eq!(audio.active_channels.len() => 20);
}

#[test]
fn save_file_version_deserializes_legacy_integer() {
    let version: SaveFileVersion = serde_json::from_str("1").test_value();
    main_assert_eq!(version => SaveFileVersion::new(1, 0, 0));
}

#[test]
fn save_file_version_deserializes_string() {
    let version: SaveFileVersion = serde_json::from_str("\"2.3.4\"").test_value();
    main_assert_eq!(version => SaveFileVersion::new(2, 3, 4));
}

#[test]
fn migration_allows_previous_minor_version() {
    let engine = Engine::new();
    let engine_state = engine.capture_state();
    let save = saves_fixture!(
        saved_game:
            SaveFileVersion::new(1, 0, 0),
            SavedScenarioInfo {
                        identifier: "test".to_string(),
                        title: "Test Scenario".to_string(),
                        description: None,
                        path: None,
                        root_label: None,
                        is_editable: false,
                        is_playable: true,
                        label: "Test".to_string(),
                        fallback_ground: 0,
                        sandbox: true,
                    },
            None,
            None,
            None,
            None,
            engine_state,
    );

    let migrated = migrate_save_file(save).test_value();
    main_assert_eq!(migrated.version => SAVE_FILE_VERSION);
}

#[test]
fn saved_scenario_round_trips_basic_metadata() {
    let original = FrontendScenario {
        identifier: "test".into(),
        title: "Test Scenario".into(),
        description: Some("desc".into()),
        kind: ScenarioKind::Scenario,
        is_editable: true,
        is_playable: true,
        mission_access: None,
        path: Some(PathBuf::from("/tmp/test.c4s")),
        source_paths: Vec::new(),
        root_label: Some("Scenarios".into()),
        preview: None,
        children: Vec::new(),
        title_picture: None,
        folder_index: None,
        icon_index: None,
        difficulty: None,
        author: None,
        version: None,
        local_only: None,
        allow_user_change: None,
        definition_modules: Vec::new(),
    };
    let info = SavedScenarioInfo::from_frontend(&original, "Label", 123);
    main_assert_eq!(info.identifier => original.identifier);
    main_assert_eq!(info.title => original.title);
    main_assert_eq!(info.path => original.path);
    main_assert_eq!(info.label => "Label");
    main_assert_eq!(info.fallback_ground => 123);
    let restored = info.to_frontend();
    main_assert_eq!(restored.identifier => original.identifier);
    main_assert_eq!(restored.title => original.title);
    main_assert_eq!(restored.path => original.path);
    main_assert!(restored.children.is_empty());
    main_assert_eq!(restored.kind => ScenarioKind::Scenario);
}

#[test]
fn local_scenario_player_gate_matches_cpp_max_savegame_and_replay_rules() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let scenario_group = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let app = new_menu_app_with_paths(640, 480, &paths);
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(scenario_group.path().to_path_buf());

    persist_config_value(&paths, "General", "Participants", "Alice.c4p;Bob.c4p").test_value();
    fs::write(
        scenario_group.path().join("Scenario.txt"),
        "[Head]\nMinPlayer=1\nMaxPlayer=1\n",
    )
    .test_value();
    main_assert_eq!(
        app.local_scenario_player_count_error(&scenario)
            .expect("inspect regular scenario") =>
        Some("This scenario is designed for a maximum of 1 players.".to_string())
    );

    fs::write(
        scenario_group.path().join("Scenario.txt"),
        "[Head]\nMinPlayer=2\nMaxPlayer=0\nSaveGame=1\n",
    )
    .test_value();
    main_assert_eq!(
        app.local_scenario_player_count_error(&scenario)
            .expect("inspect savegame scenario") =>
        None,
        "savegames raise a stale maximum to the minimum player count"
    );

    persist_config_value(&paths, "General", "Participants", "").test_value();
    fs::write(
        scenario_group.path().join("Scenario.txt"),
        "[Head]\nMinPlayer=9\nMaxPlayer=0\nReplay=1\n",
    )
    .test_value();
    main_assert_eq!(app.local_scenario_player_count_error(&scenario).expect("inspect replay scenario") => None, "replays bypass regular-game player-count checks");
    reset_cached_app_paths();
}

#[test]
fn local_scenario_start_with_no_participants_shows_cpp_error_before_loading() {
    // C4StartupScenSelDlg::DoOK calls Scenario::CanOpen before opening
    // C4DefinitionSelDlg or starting C4Game. A local player-count
    // shortfall keeps the browser active and opens the classic error
    // dialog (C4StartupScenSelDlg.cpp:754-781, 1681-1692).
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let scenario_group = tempdir();
    fs::write(
        scenario_group.path().join("Scenario.txt"),
        "[Head]\nTitle=No participants\nMaxPlayer=4\n\n[Definitions]\nAllowUserChange=true\n",
    )
    .test_value();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "Participants", "").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "NoParticipants.c4s".to_string();
    scenario.title = "No participants".to_string();
    scenario.path = Some(scenario_group.path().to_path_buf());
    scenario.allow_user_change = Some(true);
    let scenarios = vec![scenario.clone()];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scenario_catalog = build_scenario_catalog(&scenarios);
    app.open_scenario_browser();
    app.menu_state.definition_checkbox_checked = true;

    app.handle_menu_input(|_| {
        vec![StartupMenuAction::StartScenario(saves_fixture!(
            scenario:
                scenario.identifier.clone(),
                scenario.title.clone(),
                ScenarioKind::Scenario,
        ))]
    })
    .test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    main_assert!(app.loading_state.is_none());
    main_assert!(app.definition_selector.is_none());
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.message_dialogs[0].state.caption() => "Cannot start scenario.");
    main_assert_eq!(
            app.message_dialogs[0].state.message() =>
            "This scenario is designed for a minimum of 1 players. Please go to the Player Selection dialog and activate the participants for this round."
        );
    main_assert_eq!(app.message_dialogs[0].state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::ERROR);
    main_assert_eq!(app.message_dialogs[0].state.focused_button() => Some(clonk_frontend::message_dialog::MessageDialogButton::Ok));
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);
    let ok = app.top_message_dialog_layout().test_value().buttons[0].rect;
    let ok_point = PhysicalPosition::new(f64::from(ok.x + ok.w / 2), f64::from(ok.y + ok.h / 2));
    app.test_cursor(ok_point);
    main_assert!(app.message_dialogs[0].state.has_pointer_hover());
    main_assert_eq!(app.menu_state.pointer_position() => None, "the exclusive startup popup owns pointer movement");
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(ok_point.x + 1.0, ok_point.y));
    main_assert!(app.message_dialogs[0].state.has_pointer_capture());
    main_assert!(app.message_dialogs[0].state.has_pointer_hover());
    app.test_left_button(ElementState::Released);
    main_assert!(app.message_dialogs.is_empty());
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    main_assert!(app.status_text.is_empty());

    app.handle_menu_input(|_| {
        vec![StartupMenuAction::StartScenario(saves_fixture!(
            scenario:
                scenario.identifier.clone(),
                scenario.title.clone(),
                ScenarioKind::Scenario,
        ))]
    })
    .test_value();
    let close = app
        .top_message_dialog_layout()
        .test_value()
        .close_button
        .test_value();
    let close_point = PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    );
    app.test_cursor(close_point);
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(close_point.x + 1.0, close_point.y));
    main_assert!(app.message_dialogs[0].state.has_pointer_capture());
    main_assert!(app.message_dialogs[0].state.has_pointer_hover());
    app.test_left_button(ElementState::Released);
    main_assert!(app.message_dialogs.is_empty());
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    main_assert!(app.loading_state.is_none());
    main_assert!(app.definition_selector.is_none());
    main_assert!(app.status_text.is_empty());
    reset_cached_app_paths();
}

#[test]
fn replay_staged_scenario_keeps_cpp_player_group_order_through_live_sync() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let frontend = install_minimal_prepared_host_fixture(content.path());
    let scenario_path = frontend.path.clone().test_value();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let mut staged = prepare_minimal_host_lobby(&app, frontend);

    let core_path = scenario_path.join("Scenario.txt");
    let core =
        fs::read_to_string(&core_path)
            .test_value()
            .replacen("[Head]\n", "[Head]\nReplay=1\n", 1);
    fs::write(&core_path, core).test_value();
    let resolver = InstallDefinitionResolver::new(Some(Arc::new(paths.clone())));
    staged.scenario = load_scenario_with_definition_load(
        &scenario_path,
        &resolver,
        &startup_language_sequence(Some(&paths)),
        &staged.definition_load,
    )
    .test_value();
    main_assert!(staged.scenario.lobby_metadata().expect("reloaded lobby metadata").head().is_replay());
    app.staged_network_host_scenario = Some(staged);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Exact Host".to_string(),
        prepared: None,
    }));
    app.control_clients.replace_snapshot([
        clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: LegacyCString::from_bytes(b"Exact Host".to_vec()).test_value(),
            ..Default::default()
        },
        clonk_engine::ClientCoreControlData {
            client_id: 7,
            activated: true,
            name: LegacyCString::from_bytes(b"Remote".to_vec()).test_value(),
            ..Default::default()
        },
    ]);
    let player = |id, flags, player_type| clonk_engine::ControlPlayerInfoEntry {
        id,
        flags,
        player_type,
        name: LegacyCString::from_bytes(format!("Player {id}").into_bytes()).test_value(),
        ..Default::default()
    };
    app.control_player_infos.replace_snapshot(
        1,
        [
            clonk_engine::PlayerInfoControlData::new(
                0,
                0,
                vec![
                    player(30, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                    player(
                        20,
                        clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                        clonk_engine::PLAYER_INFO_TYPE_USER,
                    ),
                    player(40, 0, clonk_engine::PLAYER_INFO_TYPE_SCRIPT),
                ],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                7,
                0,
                vec![
                    player(10, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                    player(
                        5,
                        clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                        clonk_engine::PLAYER_INFO_TYPE_USER,
                    ),
                ],
                -1,
            ),
        ],
    );
    install_test_classic_host_lobby(&mut app);

    app.sync_classic_lobby_roster();

    let rows = app.classic_host_lobby.test_ref().controller.rows();
    main_assert_eq!(
        rows.iter().map(LobbyRosterRow::id).collect::<Vec<_>>() =>
        vec![
            LobbyRosterId::Header(LobbyRosterHeader::ReplayPlayers),
            LobbyRosterId::Player(5),
            LobbyRosterId::Player(10),
            LobbyRosterId::Player(30),
            LobbyRosterId::Header(LobbyRosterHeader::ScriptPlayers),
            LobbyRosterId::Player(40),
            LobbyRosterId::Client(0),
            LobbyRosterId::Client(7),
        ]
    );
    let LobbyRosterRow::Header(replay_header) = &rows[0] else {
        panic!("replay roster starts with its header");
    };
    main_assert_eq!(replay_header.label => "Replay players");
    main_assert_eq!(replay_header.icon => LobbyRosterIcon::Standard(21));
    main_assert!(!replay_header.can_add_player);
    main_assert!(rows.iter().filter_map(|row| match row {LobbyRosterRow::Player(player) => Some(player), _ => None,}).all(|player| player.client_id == -1));
    main_assert!(!rows.iter().any(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 20)));
    reset_cached_app_paths();
}

#[test]
fn singleton_default_music_replays_without_a_second_random_draw() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Only.ogg"), b"only").test_value();
    let resolver = MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    let mut draws = 0;

    let first = resolver
        .select_default_with(None, |range| {
            draws += 1;
            main_assert_eq!(range => 1, "SafeRandom(1) is still consumed initially");
            0
        })
        .test_value();
    let recent = Arc::clone(&first.identity);
    let second = resolver
        .select_default_with(Some(&recent), |_| {
            panic!("sole recent fallback must not consume SafeRandom")
        })
        .test_value();

    main_assert_eq!(draws => 1);
    main_assert!(Arc::ptr_eq(&second.identity, &recent));
}

#[test]
fn duplicate_music_records_exclude_only_the_record_that_started() {
    let dir = tempdir();
    fs::write(dir.path().join("Shared.ogg"), b"shared").test_value();
    let source = Arc::new(Group::open(dir.path()).test_value());
    let catalog = MusicCatalog {
        assets: vec![
            MusicAsset::for_test_path(Arc::clone(&source), PathBuf::from("Shared.ogg")),
            MusicAsset::for_test_path(source, PathBuf::from("Shared.ogg")),
        ],
    };

    let first = catalog
        .select_enabled_with(None, None, |range| {
            main_assert_eq!(range => 2);
            0
        })
        .test_value();
    let recent = Arc::clone(&first.identity);
    let second = catalog
        .select_enabled_with(None, Some(&recent), |range| {
            main_assert_eq!(range => 1);
            0
        })
        .test_value();

    main_assert_eq!(first.full_path_bytes => second.full_path_bytes);
    main_assert!(!Arc::ptr_eq(&first.identity, &second.identity));
}

#[test]
fn menu_dump_writes_main_menu_png_at_1280x720() {
    clonk_logging::init();

    let dir = tempdir();
    let repository = test_repository_root();
    let _guard = test_env_guard(repository, dir.path());
    let app_paths = Arc::new(test_app_paths());
    let path = dir.path().join("menu.png");
    run_menu_dump(
        &path,
        "main",
        Some(&app_paths),
        test_runtime_config_with("Player", false),
    )
    .test_value();

    // PNG IHDR: width/height are big-endian u32 at byte offsets 16/20.
    let png = fs::read(&path).test_value();
    main_assert_eq!(&png[..8] => b"\x89PNG\r\n\x1a\n", "not a PNG file");
    let width = u32::from_be_bytes(png[16..20].try_into().test_value());
    let height = u32::from_be_bytes(png[20..24].try_into().test_value());
    main_assert_eq!((width, height) => (1280, 720));
}

#[test]
fn player_properties_save_refreshes_selection_and_renamed_participant() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let player_root = user_data.path().join("Players");
    let mut config = Config::new();
    config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    config.save(paths.config_file()).test_value();

    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    Arc::get_mut(&mut app.assets)
        .test_value()
        .startup_dialog_images
        .insert(
            "Portrait1.png".to_string(),
            ImageData::new(1, 1, vec![0, 0, 255, 255]),
        );
    app.open_player_selection_dialog();
    app.open_new_startup_player_properties();
    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("");
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);
    main_assert_eq!(app.message_dialogs.last().expect("empty-name modal").state.message() => "You must specify a player name!");
    main_assert!(app.startup_player_properties_dialog.as_ref().is_some_and(|pending| pending.controller.validation_error().is_none()));
    main_assert!(app.startup_player_files.is_empty());
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("Created");
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    let created = player_root.join("Created.c4p");
    main_assert!(created.is_file());
    let created_group = Group::open(&created).test_value();
    main_assert!(created_group.read_file("Player.txt").is_ok());
    main_assert!(created_group.read_file("Portrait.png").is_ok());
    main_assert!(created_group.read_file("BigIcon.png").is_ok());
    main_assert!(app.startup_player_properties_dialog.is_none());
    main_assert_eq!(app.startup_player_files.len() => 1);
    main_assert!(app.startup_player_files[0].render_model.activated);
    main_assert_eq!(app.startup_player_dialog.as_ref().and_then(|dialog| dialog.selected_index()) => Some(0));
    main_assert_eq!(app.selected_player_file.as_ref().map(|player| player.name.as_str()) => Some("Created"));
    main_assert_eq!(Config::load(paths.config_file()).expect("reload config").get_in(Some("General"), "Participants") => Some(created.to_string_lossy().as_ref()));

    app.open_existing_startup_player_properties(0);
    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("Renamed");
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);
    let renamed = player_root.join("Renamed.c4p");
    main_assert!(!created.exists());
    main_assert!(renamed.is_file());
    main_assert_eq!(app.startup_player_files[0].player_file.name => "Renamed");
    main_assert!(app.startup_player_files[0].render_model.activated);
    main_assert_eq!(
        Config::load(paths.config_file())
            .expect("reload renamed config")
            .get_in(Some("General"), "Participants") =>
        Some(renamed.to_string_lossy().as_ref())
    );
    reset_cached_app_paths();
}

#[test]
fn startup_player_properties_post_validation_save_failure_opens_classic_error_dialog() {
    use clonk_frontend::message_dialog::{
        MessageDialogButtons, MessageDialogIcon, MessageDialogResult, MessageDialogSize,
    };

    let user_data = tempdir();
    let (_guard, paths, player_root, mut app) =
        startup_player_properties_validation_app(user_data.path());
    app.startup_player_properties_dialog = None;

    let old = player_root.join("Old.c4p");
    fs::create_dir(&old).test_value();
    fs::write(old.join("Player.txt"), b"[Player]\nName=Old\n").test_value();
    persist_config_value(&paths, "General", "Participants", old.to_string_lossy()).test_value();
    app.refresh_startup_player_list();
    main_assert_eq!(app.startup_player_files.len() => 1);
    main_assert!(app.startup_player_files[0].render_model.activated);

    app.open_existing_startup_player_properties(0);
    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("Renamed");

    // Cross the same partial-save boundary as a native group-open or
    // close failure: the filesystem rename succeeds, then opening the
    // destination as a player group fails.
    fs::remove_dir_all(&old).test_value();
    fs::write(&old, b"not a C4Group").test_value();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    let renamed = player_root.join("Renamed.c4p");
    main_assert!(!old.exists());
    main_assert!(renamed.is_file(), "rename completed before group-open failed");
    main_assert!(app.startup_player_properties_dialog.is_none(), "the properties form closes before the screen-owned error dialog");
    main_assert!(app.startup_player_files.is_empty());
    main_assert!(app.startup_player_models.is_empty());
    let selector = app.startup_player_dialog.test_ref();
    main_assert!(selector.player_activations().is_empty());
    main_assert_eq!(selector.selected_index() => None);
    main_assert!(app.selected_player_file.is_none());
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(Config::load(paths.config_file()).expect("reload reconciled config").get_in(Some("General"), "Participants") => Some(""));

    let modal = app.message_dialogs.last().test_value();
    main_assert_eq!(modal.state.caption() => "Error");
    main_assert!(modal.state.message().starts_with("Error opening file \""));
    main_assert!(modal.state.message().contains(&renamed.display().to_string()));
    main_assert_eq!(modal.state.buttons() => MessageDialogButtons::OK);
    main_assert_eq!(modal.state.icon() => MessageDialogIcon::ERROR);
    main_assert_eq!(modal.state.size() => MessageDialogSize::Regular);
    main_assert!(matches!(modal.continuation, MessageDialogContinuation::None));

    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.startup_player_properties_dialog.is_none());
    main_assert!(app.startup_player_files.is_empty());
    main_assert_eq!(app.startup_player_dialog.as_ref().and_then(|dialog| dialog.selected_index()) => None);
}

#[test]
fn startup_player_properties_save_failure_modal_names_step_and_path() {
    use clonk_frontend::message_dialog::MessageDialogResult;

    let user_data = tempdir();
    let (_guard, paths, player_root, mut app) =
        startup_player_properties_validation_app(user_data.path());
    app.startup_player_properties_dialog = None;

    let old = player_root.join("Old.c4p");
    fs::create_dir(&old).test_value();
    fs::write(old.join("Player.txt"), b"[Player]\nName=Old\n").test_value();
    persist_config_value(&paths, "General", "Participants", old.to_string_lossy()).test_value();
    app.refresh_startup_player_list();
    app.open_existing_startup_player_properties(0);
    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("Renamed");

    // The source group vanishes behind the open form, so the rename step
    // fails and must report itself with both filenames like C++'s
    // IDS_FAIL_RENAME/IDS_ERR_RENAMEFILE composition.
    fs::remove_dir_all(&old).test_value();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    let renamed = player_root.join("Renamed.c4p");
    let modal = app.message_dialogs.last().test_value();
    main_assert_eq!(modal.state.caption() => "Error");
    let message = modal.state.message().to_string();
    let expected_body = format!(
        "Rename failure.\nError renaming file \"{}\" to \"{}\".\n",
        old.display(),
        renamed.display()
    );
    main_assert!(message.starts_with(&expected_body), "rename modal must lead with step and both paths: {message}");
    main_assert!(message.len() > expected_body.len(), "rename modal must keep the underlying error detail: {message}");
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    // The remaining storage steps compose the same way; pin each branch's
    // localized step string and path.
    main_assert_eq!(
        app.startup_player_properties_save_failure_text(&PlayerPropertiesSaveError::Open {
            path: PathBuf::from("/players/Ada.c4p"),
            detail: "boom".to_string(),
        }) =>
        "Error opening file \"/players/Ada.c4p\": boom"
    );
    main_assert_eq!(
        app.startup_player_properties_save_failure_text(&PlayerPropertiesSaveError::WriteCore {
            path: PathBuf::from("/players/Ada.c4p"),
            entry: "Player.txt",
            detail: "boom".to_string(),
        }) =>
        "File modification failure.\n\"/players/Ada.c4p/Player.txt\": boom"
    );
    main_assert_eq!(
        app.startup_player_properties_save_failure_text(&PlayerPropertiesSaveError::WriteImage {
            path: PathBuf::from("/players/Ada.c4p"),
            entry: "BigIcon.png",
            detail: "boom".to_string(),
        }) =>
        "Error at graphics file /players/Ada.c4p/BigIcon.png: boom"
    );
    main_assert_eq!(
        app.startup_player_properties_save_failure_text(&PlayerPropertiesSaveError::Close {
            path: PathBuf::from("/players/Ada.c4p"),
            detail: "boom".to_string(),
        }) =>
        "Close: \"/players/Ada.c4p\": boom"
    );
    main_assert_eq!(
        app.startup_player_properties_save_failure_text(&PlayerPropertiesSaveError::Rename {
            from: PathBuf::from("/players/Old.c4p"),
            to: PathBuf::from("/players/Ada.c4p"),
            detail: "boom".to_string(),
        }) =>
        "Rename failure.\nError renaming file \"/players/Old.c4p\" to \"/players/Ada.c4p\".\nboom"
    );
}

#[test]
fn runtime_point_filtering_reloads_after_advanced_config_save() {
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::write(paths.config_file(), b"[Graphics]\nPointFiltering=true\n").test_value();

    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    app.synchronize_advanced_options_runtime();
    main_assert!(app.graphics.point_filtering());
    main_assert!(app.loader_render_config.expect("loader render config remains materialized").point_filtering());

    fs::write(paths.config_file(), b"[Graphics]\nPointFiltering=false\n").test_value();
    app.synchronize_advanced_options_runtime();
    main_assert!(!app.graphics.point_filtering());
    main_assert!(!app.loader_render_config.expect("loader config follows live advanced save").point_filtering());
}

#[test]
fn runtime_pxs_graphics_reload_is_live_and_presentation_only() {
    // C4PXSSystem::Draw rereads Config.Graphics.PXSGfx for both passes
    // (src/C4PXS.cpp:259-260,279-281); reloading it is presentation-only.
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    persist_config_value(&paths, "Graphics", "PXSGfx", "1").test_value();

    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    app.synchronize_advanced_options_runtime();
    main_assert!(app.display_flags.pxs_gfx);
    main_assert!(app.graphics.pxs_graphics_enabled());

    main_assert!(app.engine.pxs_system.create(
        clonk_engine::MaterialId::new(0).test_value(),
        clonk_engine::math::C4Fixed::from_raw(0x12345),
        clonk_engine::math::C4Fixed::from_raw(0x23456),
        clonk_engine::math::C4Fixed::from_raw(-0x3456),
        clonk_engine::math::C4Fixed::from_raw(0x4567),
    ));
    let simulation = app.engine.snapshot();
    let sync = app.engine.sync_check(0);
    let pxs_slots = app
        .engine
        .pxs_system
        .iter_slots()
        .map(|(chunk, slot, pxs)| (chunk, slot, *pxs))
        .collect::<Vec<_>>();
    let assert_simulation_unchanged = |app: &GameApp| {
        main_assert_eq!(app.engine.snapshot() => simulation);
        main_assert_eq!(app.engine.sync_check(0) => sync);
        main_assert_eq!(app.engine.pxs_system.iter_slots().map(|(chunk, slot, pxs)| (chunk, slot, *pxs)).collect::<Vec<_>>() => pxs_slots);
    };

    persist_config_value(&paths, "Graphics", "PXSGfx", "0").test_value();
    app.synchronize_advanced_options_runtime();
    main_assert!(!app.display_flags.pxs_gfx);
    main_assert!(!app.graphics.pxs_graphics_enabled());
    assert_simulation_unchanged(&app);

    persist_config_value(&paths, "Graphics", "PXSGfx", "1").test_value();
    app.synchronize_advanced_options_runtime();
    main_assert!(app.display_flags.pxs_gfx);
    main_assert!(app.graphics.pxs_graphics_enabled());
    assert_simulation_unchanged(&app);
}

#[test]
fn runtime_advanced_voice_opt_in_retries_a_missing_audio_context() {
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::write(
        paths.config_file(),
        b"[Sound]\nSound=false\nMusic=false\nMenuMusic=false\nMenuSound=false\n[Voice]\nEnabled=true\n",
    )
    .test_value();

    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths);
    app.audio = None;
    app.synchronize_advanced_options_runtime();

    main_assert!(app.audio.as_ref().is_some_and(|audio| audio.options.voice_enabled), "saving Advanced Voice.Enabled must not require an application restart",);
}

fn decode_rgb_screenshot(path: &Path) -> (u32, u32, Vec<u8>) {
    let file = File::open(path).test_value();
    let decoder = png::Decoder::new(io::BufReader::new(file));
    let mut reader = decoder.read_info().test_value();
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("screenshot buffer size fits usize")
    ];
    let info = reader.next_frame(&mut buffer).test_value();
    main_assert_eq!(info.color_type => ColorType::Rgb);
    main_assert_eq!(info.bit_depth => BitDepth::Eight);
    buffer.truncate(info.buffer_size());
    (info.width, info.height, buffer)
}

fn fnv1a_png_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[test]
fn rgba_png_encoding_preserves_png_017_bytes() {
    let pixels = [1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8];
    let encoded = encode_rgba_png(2, 2, &pixels).test_value();

    main_assert_eq!(encoded.len() => 86);
    main_assert_eq!(fnv1a_png_bytes(&encoded) => 11_539_277_311_474_003_906);
}

#[test]
fn screenshot_png_encoding_preserves_png_017_bytes() {
    let pixels = [1, 2, 3, 255, 5, 6, 7, 128, 1, 2, 3, 64, 5, 6, 7, 0];
    let encoded = encode_screenshot_png(2, 2, &pixels).test_value();

    main_assert_eq!(encoded.len() => 82);
    main_assert_eq!(fnv1a_png_bytes(&encoded) => 8_588_350_724_413_462_130);
}

#[test]
fn retained_gpu_save_thumbnail_waits_for_the_presented_frame() {
    let directory = tempdir();
    let save_path = directory.path().join("round.c4s");
    let thumbnail_path = save_path.with_extension("png");
    let mut app = new_running_sandbox_app();
    app.retained_gpu_presentation_active = true;

    app.write_save_thumbnail(&save_path).test_value();

    main_assert_eq!(app.pending_gpu_thumbnail_paths.iter().collect::<Vec<_>>() => vec![&thumbnail_path]);
    main_assert!(!thumbnail_path.exists(), "the stale CPU surface must not be written before GPU readback");
}

#[test]
fn retained_gpu_save_thumbnail_matches_cpp_title_extent() {
    let encoded =
        encode_presented_save_thumbnail(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).test_value();
    let decoder = png::Decoder::new(io::Cursor::new(encoded));
    let mut reader = decoder.read_info().test_value();
    let mut buffer = vec![
        0;
        reader
            .output_buffer_size()
            .expect("thumbnail buffer size fits usize")
    ];
    let info = reader.next_frame(&mut buffer).test_value();

    main_assert_eq!((info.width, info.height) => (SAVE_THUMBNAIL_WIDTH, SAVE_THUMBNAIL_HEIGHT));
    main_assert_eq!(info.color_type => ColorType::Rgba);
}

/// `Screenshot` is registered `KEYSCOPE_Fullscreen | KEYSCOPE_Gui`, so bare F9
/// captures the startup screens too; `ScreenshotEx` is Fullscreen-only and stays
/// inert there (C4Game.cpp:3387-3388). The capture path, numbered slot and
/// localized result log are the same as in running mode
/// (C4GraphicsSystem.cpp:503-525).
#[test]
fn startup_f9_saves_the_presented_classic_gui_frame() {
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    fs::write(install.path().join("planet/System.c4g/LanguageUS.txt"), b"IDS_PRC_SCREENSHOT=Saved screenshot %s.\nIDS_PRC_SCREENSHOTERROR=Failure creating screenshot %s.\n").test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();

    let mut app = new_menu_app(320, 200);
    app.app_paths = Some(paths);
    main_assert_eq!(app.mode => AppMode::Menu);
    let presented = vec![
        1, 2, 3, 4, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170,
        180, 190, 200, 210, 220, 230, 240, 250, 249, 248, 247,
    ];

    // Ctrl+F9 is Fullscreen-only, so it queues nothing in GUI scope.
    app.keyboard_modifiers = ModifiersState::CONTROL;
    app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F9, ElementState::Released);
    main_assert!(app.pending_screenshots.is_empty());

    // Bare F9 queues the presented-frame capture on every startup view.
    app.keyboard_modifiers = ModifiersState::empty();
    for view in [
        StartupView::MainMenu,
        StartupView::ScenarioBrowser,
        StartupView::PlayerSelection,
        StartupView::NetworkGame,
        StartupView::Options,
        StartupView::About,
    ] {
        app.startup_view = view;
        app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
        app.test_key(VirtualKeyCode::F9, ElementState::Released);
        main_assert_eq!(
            app.pending_screenshots
                .pop_back()
                .map(|request| request.kind) =>
            Some(ScreenshotKind::PresentedFrame),
            "{view:?} must queue a presented-frame capture"
        );
    }

    // The queued capture writes through the same numbered slot and log text.
    app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
    let outcome = app
        .save_next_screenshot(Some(&presented), 4, 2, 1.0)
        .test_value();
    main_assert!(outcome.result.is_ok(), "startup screenshot failed: {:?}", outcome.result);
    let path = outcome.path.clone();
    main_assert_eq!(app.report_screenshot_result(Some(outcome)).as_deref() => Some("Saved screenshot Screenshots/Screenshot001.png."));
    main_assert_eq!(path => install.path().join("Screenshots/Screenshot001.png"));
    let (width, height, _) = decode_rgb_screenshot(&path);
    main_assert_eq!((width, height) => (4, 2));

    // A second capture reuses the next free slot, like the running path.
    app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
    let second = app
        .save_next_screenshot(Some(&presented), 4, 2, 1.0)
        .test_value();
    main_assert_eq!(second.path => install.path().join("Screenshots/Screenshot002.png"));
}

/// F9 copies the physical presented frame; only Ctrl+F9 re-renders and scales
/// (`C4GraphicsSystem::SaveScreenshot(false)`; `game_app/saves.rs`
/// `save_next_screenshot`). So a non-integer window scale must not reach the
/// presented-frame branch at all: the saved image stays the physical buffer,
/// byte for byte and at physical extent.
///
/// That is the whole difference between a screenshot and a resample. Anything
/// that scaled here would resample glyph and facet edges, which is exactly the
/// clipped-text and edge-bleeding failure this path is supposed to rule out
/// (clonk-org/clonk-rs#579).
#[test]
fn f9_capture_ignores_window_scale_and_stays_the_physical_frame() {
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    fs::write(install.path().join("planet/System.c4g/LanguageUS.txt"), b"IDS_PRC_SCREENSHOT=Saved screenshot %s.\nIDS_PRC_SCREENSHOTERROR=Failure creating screenshot %s.\n").test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();

    let mut app = new_menu_app(320, 200);
    app.app_paths = Some(paths);

    // Deliberately asymmetric and fully saturated so a resample of any kind
    // would disturb the bytes rather than average into itself.
    let presented = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 1, 2, 3, 255, 250, 240,
        230, 255, 9, 8, 7, 255, 16, 32, 64, 255,
    ];
    let expected: Vec<u8> = presented
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();

    // 1.0 is the baseline the other F9 tests already use; the rest are the
    // fractional and integer window scales a real display can hand over.
    for scale in [1.0_f32, 1.25, 1.5, 2.0, 2.5] {
        app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
        app.test_key(VirtualKeyCode::F9, ElementState::Released);
        let outcome = app
            .save_next_screenshot(Some(&presented), 4, 2, scale)
            .test_value();
        main_assert!(
            outcome.result.is_ok(),
            "F9 at scale {scale} failed: {:?}",
            outcome.result
        );
        let (width, height, pixels) = decode_rgb_screenshot(&outcome.path);
        main_assert_eq!(
            (width, height) => (4, 2),
            "scale {scale} must not change the captured extent"
        );
        main_assert_eq!(
            pixels => expected.clone(),
            "scale {scale} must not resample the captured pixels"
        );
    }
}

#[test]
fn running_f9_saves_presented_rgb_and_ctrl_f9_saves_full_landscape() {
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    fs::write(install.path().join("planet/System.c4g/LanguageUS.txt"), b"IDS_PRC_SCREENSHOT=Saved screenshot %s.\nIDS_PRC_SCREENSHOTERROR=Failure creating screenshot %s.\n").test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();

    let mut app = new_running_sandbox_app();
    app.app_paths = Some(paths);
    app.set_display_mode(DisplayMode::Window);
    app.clear_message_board_log();
    main_assert!(!app.display_flags.is_fullscreen, "C++ isFullScreen means non-console mode, so an OS window remains eligible");
    let presented = vec![
        1, 2, 3, 4, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170,
        180, 190, 200, 210, 220, 230, 240, 250, 249, 248, 247,
    ];

    app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
    main_assert_eq!(app.pending_screenshots.front().map(|request| request.kind) => Some(ScreenshotKind::PresentedFrame));
    let first_outcome = app
        .save_next_screenshot(Some(&presented), 4, 2, 2.0)
        .test_value();
    main_assert!(first_outcome.result.is_ok(), "presented screenshot failed: {:?}", first_outcome.result);
    let first = first_outcome.path.clone();
    main_assert_eq!(app.report_screenshot_result(Some(first_outcome)).as_deref() => Some("Saved screenshot Screenshots/Screenshot001.png."));
    main_assert_eq!(
        app.message_board
            .log_history
            .iter()
            .cloned()
            .collect::<Vec<_>>() =>
        app.graphics
            .prepare_message_board_lines("Saved screenshot Screenshots/Screenshot001.png.")
    );
    main_assert_eq!(first => install.path().join("Screenshots/Screenshot001.png"));
    let (width, height, rgb) = decode_rgb_screenshot(&first);
    main_assert_eq!((width, height) => (4, 2));
    main_assert_eq!(
        rgb =>
        presented
            .chunks_exact(4)
            .flat_map(|pixel| pixel[..3].iter().copied())
            .collect::<Vec<_>>(),
        "the already-presented gamma-encoded bytes are not transformed again"
    );

    let mut logical_frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut logical_frame);
    let landscape = app.snapshot.landscape.test_ref();
    let expected_width = scaled_screenshot_extent(landscape.width(), 1.5).test_value();
    let expected_height = scaled_screenshot_extent(
        u32::try_from(landscape.estimated_height()).expect("positive landscape height"),
        1.5,
    )
    .test_value();
    let installed_gamma = app
        .graphics
        .active_gamma_ramp(&app.snapshot.environment.gamma);
    app.snapshot
        .environment
        .gamma
        .set_ramp(0, [0x102030, 0x405060, 0x708090]);

    app.start_running_chat(RunningChatMode::All);
    app.keyboard_modifiers = ModifiersState::CONTROL;
    app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
    main_assert_eq!(app.pending_screenshots.front().map(|request| request.kind) => Some(ScreenshotKind::FullLandscape));
    main_assert_eq!(app.pending_screenshots.front().map(|request| &request.gamma) => Some(&installed_gamma), "queued capture retains the ramp installed at keydown");
    main_assert!(app.running_chat.is_some(), "the global screenshot binding works above an open chat dialog");
    app.graphics
        .apply_gamma_now(&app.snapshot.environment.gamma);
    app.clear_message_board_log();
    let second_outcome = app
        .save_next_screenshot(Some(&presented), 4, 2, 1.5)
        .test_value();
    main_assert!(second_outcome.result.is_ok(), "full-landscape screenshot failed: {:?}", second_outcome.result);
    let second = second_outcome.path.clone();
    main_assert_eq!(app.report_screenshot_result(Some(second_outcome)).as_deref() => Some("Saved screenshot Screenshots/Screenshot002.png."));
    main_assert_eq!(
        app.message_board
            .log_history
            .iter()
            .cloned()
            .collect::<Vec<_>>() =>
        app.graphics
            .prepare_message_board_lines("Saved screenshot Screenshots/Screenshot002.png.")
    );
    main_assert_eq!(second => install.path().join("Screenshots/Screenshot002.png"));
    let (width, height, _) = decode_rgb_screenshot(&second);
    main_assert_eq!((width, height) => (expected_width, expected_height));
    main_assert_eq!(app.mode => AppMode::Running, "screenshots do not end the game");

    app.close_running_chat().test_value();
    app.keyboard_modifiers = ModifiersState::empty();
    app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
    main_assert_eq!(
        app.pending_screenshots
            .iter()
            .map(|request| request.kind)
            .collect::<Vec<_>>() =>
        vec![
            ScreenshotKind::PresentedFrame,
            ScreenshotKind::PresentedFrame,
        ],
        "repeated keydown events queue distinct native screenshots"
    );
}

#[test]
fn screenshot_failures_keep_localized_path_for_both_capture_kinds() {
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    fs::write(install.path().join("planet/System.c4g/LanguageUS.txt"), b"IDS_PRC_SCREENSHOT=Localized success: %s\nIDS_PRC_SCREENSHOTERROR=Localized failure: %s\n").test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();

    let mut app = new_running_sandbox_app();
    app.app_paths = Some(paths);
    app.reload_application_language_resources().test_value();
    app.set_display_mode(DisplayMode::Window);
    let expected_path = install.path().join("Screenshots/Screenshot001.png");

    for (modifiers, kind) in [
        (ModifiersState::empty(), ScreenshotKind::PresentedFrame),
        (ModifiersState::CONTROL, ScreenshotKind::FullLandscape),
    ] {
        app.keyboard_modifiers = modifiers;
        app.test_key(VirtualKeyCode::F9, ElementState::Pressed);
        let outcome = app.save_next_screenshot(None, 4, 2, 1.0).test_value();
        main_assert_eq!(outcome.kind => kind);
        main_assert_eq!(outcome.path => expected_path);
        main_assert!(outcome.result.as_ref().expect_err("a missing back buffer must fail before capture").to_string().contains("initialized presentation back buffer"));
        app.clear_message_board_log();
        main_assert_eq!(app.report_screenshot_result(Some(outcome)).as_deref() => Some("Localized failure: Screenshots/Screenshot001.png"));
        main_assert_eq!(
            app.message_board
                .log_history
                .iter()
                .cloned()
                .collect::<Vec<_>>() =>
            app.graphics
                .prepare_message_board_lines("Localized failure: Screenshots/Screenshot001.png")
        );
        main_assert!(!expected_path.exists(), "a failed attempt leaves its numbered slot reusable");
    }
}

#[test]
fn definition_root_vector_font_files_are_ignored_by_loader_and_saved_target() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = test_repository_root();
    let definition_root = tempdir();
    let custom_objects = definition_root.path().join("Objects.c4d");
    fs::create_dir_all(&custom_objects).test_value();
    fs::write(custom_objects.join("Endeavour.ttf"), b"override").test_value();

    let mut frontend = FrontendScenario::fallback();
    frontend.identifier = "Tutorial.c4f/Tutorial01.c4s".to_string();
    frontend.title = "A Clonk".to_string();
    frontend.path = Some(repository.join("content/Tutorial.c4f/Tutorial01.c4s"));
    let definition_load = ScenarioDefinitionLoad::Fixed {
        modules: vec!["Objects.c4d".to_string()],
        definition_root: Some(path_with_trailing_native_separator(definition_root.path())),
    };
    let app = new_menu_app_with_paths(320, 200, &paths);

    let setup = build_scenario_loader(&frontend, &definition_load, &paths, app.assets.as_ref())
        .test_value();
    for name in CLASSIC_GLOBAL_GUI_FONTS {
        main_assert!(!setup.refreshed_global_gui_failures.contains_key(name), "definition-root vector file incorrectly overrode loader {name}");
    }

    let resolution = app
        .loaded_game_global_gui_resolution(&frontend, Some(&definition_load))
        .test_value();
    for name in CLASSIC_GLOBAL_GUI_FONTS {
        main_assert!(!resolution.failures.contains_key(name), "definition-root vector file incorrectly overrode saved target {name}");
    }
    app.assets
        .require_classic_global_gui_bootstrap_resources(&resolution.failures)
        .test_value();
}

#[test]
fn advanced_options_click_save_and_cancel_round_trip_typed_config() {
    use clonk_frontend::message_dialog::{
        MessageDialogButton, MessageDialogButtons, MessageDialogResult,
    };
    use clonk_frontend::startup_options_advanced::{AdvancedConfigAction, AdvancedConfigValue};
    use clonk_frontend::startup_options_dlg::{OptionsDlgAction, OptionsSheet};
    use clonk_frontend::startup_options_network::NetworkTextField;

    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "# standalone note\n[General]\nLanguageEx=DE\nName=Old # keep this note\nFPS=yes\nVersion=347\nConfigResetSafety=42\nVendorExtension=keep\n[Graphics]\nSmokeLevel=200\n[Vendor]\nTemplate=\"<i>keep</i>\"\nEscaped=\"\\101\\x42\\33\"\n").test_value();

    let mut app = new_menu_app_with_paths(1280, 720, &paths);
    install_classic_test_assets(&mut app);
    app.open_options_menu();

    let layout = clonk_frontend::startup_options_dlg::options_dlg_layout(
        1280,
        720,
        app.assets.clonk_fonts.as_deref().test_value(),
        app.assets.options_book_fonts.as_deref().test_value(),
    );
    let advanced_point = PhysicalPosition::new(
        f64::from(layout.advanced_button.x + layout.advanced_button.w / 2),
        f64::from(layout.advanced_button.y + layout.advanced_button.h / 2),
    );
    let before_warning = fs::read(paths.config_file()).test_value();
    app.test_cursor(advanced_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);

    let warning = app.message_dialogs.last().test_value();
    main_assert_eq!(warning.state.buttons() => MessageDialogButtons::OK_CANCEL);
    main_assert_eq!(warning.state.focused_button() => Some(MessageDialogButton::Cancel), "the dangerous action defaults to Cancel");
    main_assert!(app.startup_options_advanced_dialog.is_none());
    app.finish_message_dialog(MessageDialogResult::Cancel)
        .test_value();
    main_assert_eq!(fs::read(paths.config_file()).expect("config after warning cancel") => before_warning);

    {
        let options = app.startup_options_dialog.test_mut();
        options.restore_sheet(OptionsSheet::Network);
        options
            .network_mut()
            .set_text(NetworkTextField::LocalName, "Unsaved host".to_string());
    }
    let before_editor_open = fs::read(paths.config_file()).test_value();
    app.process_options_dialog_actions(vec![OptionsDlgAction::OpenAdvancedSettings])
        .test_value();
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    main_assert_eq!(
        fs::read(paths.config_file()).expect("config after opening advanced editor") =>
        before_editor_open,
        "opening Advanced must not persist or rewrite Options configuration"
    );
    let controller = &app.startup_options_advanced_dialog.test_ref().controller;
    main_assert_eq!(controller.labels().caption => "Erweiterte Einstellungen");
    main_assert_eq!(controller.labels().save => "&Speichern");
    main_assert_eq!(controller.labels().cancel => "Abbrechen");
    main_assert_eq!(controller.sections().len() => 17);
    main_assert_eq!(
        controller
            .sections()
            .iter()
            .map(|section| section.name.as_str())
            .collect::<Vec<_>>() =>
        vec![
            "General",
            "Controls",
            "Gamepad0",
            "Gamepad1",
            "Gamepad2",
            "Gamepad3",
            "Graphics",
            "Sound",
            "Voice",
            "Network",
            "Lobby",
            "IRC",
            "Developer",
            "Startup",
            "Cooldowns",
            "Toasts",
            "Logging",
        ]
    );
    main_assert!(matches!(controller.value("General", "Name"), Some(AdvancedConfigValue::Text(value)) if value == "Old # keep this note"));
    main_assert_eq!(controller.value("General", "FPS") => Some(&AdvancedConfigValue::Bool(false)));
    main_assert!(matches!(controller.value("Graphics", "SmokeLevel"), Some(AdvancedConfigValue::Integer { value: 200, .. })));
    main_assert!(matches!(controller.value("General", "Version"), Some(AdvancedConfigValue::ReadOnly(value)) if value == "347"));
    main_assert_eq!(controller.layout().bounds => clonk_frontend::classic_gui::IntRect::new(160, 90, 960, 540));

    let mut rendered = vec![0_u8; 1280 * 720 * 4];
    app.test_render(&mut rendered);
    main_assert!(rendered.iter().any(|byte| *byte != 0));

    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert!(app.startup_options_advanced_dialog.is_some());
    app.test_key(VirtualKeyCode::KeyS, ElementState::Pressed);
    app.test_modifiers(ModifiersState::empty());
    main_assert!(app.startup_options_advanced_dialog.is_none());
    main_assert_eq!(
        Config::load(paths.config_file())
            .expect("config after Alt+S normalization")
            .get_in(Some("General"), "FPS") =>
        Some("0"),
        "Advanced Save canonicalizes existing typed values even when untouched"
    );
    main_assert_eq!(app.startup_options_dialog.as_ref().expect("Options recreated after Alt+S").active_sheet() => OptionsSheet::Network);
    app.process_options_dialog_actions(vec![OptionsDlgAction::OpenAdvancedSettings])
        .test_value();
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    let slot = GamepadSlot::new(0);
    let sourced =
        |gamepad, cluster, event| saves_fixture!(sourced_gamepad: gamepad, cluster, event);
    app.process_sourced_gamepad_event_batch(
        [sourced(
            1,
            0,
            gamepad_gui_button_event(
                GamepadSlot::new(1),
                GuiButtonClass::High,
                ElementState::Pressed,
            ),
        )],
        true,
    )
    .test_value();
    app.process_sourced_gamepad_event_batch(
        [sourced(
            0,
            1,
            gamepad_gui_button_event(slot, GuiButtonClass::High, ElementState::Pressed),
        )],
        false,
    )
    .test_value();
    main_assert!(app.startup_options_advanced_dialog.is_some());

    {
        let controller = &mut app.startup_options_advanced_dialog.test_mut().controller;
        while controller.focus()
            != clonk_frontend::startup_options_advanced::AdvancedConfigFocus::Save
        {
            controller.handle_focus_step(false);
        }
    }
    app.process_sourced_gamepad_event_batch(
        [sourced(
            0,
            2,
            gamepad_gui_button_event(slot, GuiButtonClass::Low, ElementState::Pressed),
        )],
        true,
    )
    .test_value();
    app.process_sourced_gamepad_event_batch([sourced(0, 3, GamepadEvent::Clear { slot })], true)
        .test_value();
    app.process_sourced_gamepad_event_batch(
        [
            sourced(
                0,
                4,
                gamepad_gui_button_event(slot, GuiButtonClass::Low, ElementState::Released),
            ),
            sourced(
                0,
                4,
                gamepad_action_event(slot, GamepadActionType::Select, ElementState::Released),
            ),
        ],
        true,
    )
    .test_value();
    main_assert!(app.startup_options_advanced_dialog.is_some());

    app.process_sourced_gamepad_event_batch(
        [
            sourced(
                0,
                5,
                gamepad_gui_button_event(slot, GuiButtonClass::High, ElementState::Pressed),
            ),
            sourced(
                0,
                5,
                gamepad_action_event(slot, GamepadActionType::Cancel, ElementState::Pressed),
            ),
        ],
        true,
    )
    .test_value();
    main_assert!(app.startup_options_advanced_dialog.is_none());
    main_assert_eq!(app.startup_view => StartupView::Options);
    main_assert!(app.startup_options_dialog.is_some());
    app.process_options_dialog_actions(vec![OptionsDlgAction::OpenAdvancedSettings])
        .test_value();
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    let controller = &mut app.startup_options_advanced_dialog.test_mut().controller;
    main_assert!(controller.set_value("General", "Name", AdvancedConfigValue::Text("New name".to_string()),));
    main_assert!(controller.set_value("General", "FPS", AdvancedConfigValue::Bool(true)));
    main_assert!(controller.set_value("General", "Record", AdvancedConfigValue::Bool(true),));
    main_assert!(controller.set_value("General", "NoCrew", AdvancedConfigValue::Bool(true),));
    main_assert!(controller.set_value("Graphics", "SmokeLevel", AdvancedConfigValue::Integer {value: 321, min: i128::MIN, max: i128::MAX,},));
    main_assert!(controller.set_value("General", "MissionAccess", AdvancedConfigValue::Text("Secret;Beta".to_string()),));
    main_assert!(controller.set_value("Graphics", "ShowFolderMaps", AdvancedConfigValue::Bool(false),));
    main_assert!(controller.set_value("Sound", "MenuMusic", AdvancedConfigValue::Bool(false),));
    let replacement_key = input::advanced_config_default_raw_keyboard_keys()[0][1];
    main_assert!(controller.set_value("Controls", "Kbd1Key1", AdvancedConfigValue::Integer {value: i128::from(replacement_key), min: i128::MIN, max: i128::MAX,},));
    main_assert!(!controller.set_value("General", "Version", AdvancedConfigValue::ReadOnly("999".to_string()),));
    app.process_options_advanced_actions(vec![AdvancedConfigAction::Save])
        .test_value();

    main_assert!(app.startup_options_advanced_dialog.is_none());
    main_assert_eq!(app.startup_view => StartupView::Options);
    main_assert_eq!(app.graphics_smoke_level => 321);
    main_assert_eq!(app.mission_access.snapshot() => "Secret;Beta");
    main_assert!(!app.show_folder_maps);
    main_assert!(app.startup_view_flags.record);
    main_assert_eq!(app.recording_enabled => app.recordings_dir.is_some());
    main_assert!(app.startup_view_flags.fair_crew);
    main_assert_eq!(app.startup_options_dialog.as_ref().expect("recreated Options dialog").active_sheet() => OptionsSheet::Network);
    main_assert!(
        !app.startup_options_dialog
            .as_ref()
            .expect("recreated Options dialog")
            .sound()
            .frontend_music,
        "the recreated Sound sheet must use the advanced value"
    );
    let saved = Config::load(paths.config_file()).test_value();
    main_assert_eq!(saved.get_in(Some("General"), "Name") => Some("New name"));
    main_assert_eq!(saved.get_in(Some("General"), "FPS") => Some("1"));
    main_assert_eq!(saved.get_in(Some("General"), "Record") => Some("1"));
    main_assert_eq!(saved.get_in(Some("General"), "NoCrew") => Some("true"));
    main_assert!(fs::read(paths.config_file())
        .expect("read native advanced NoCrew")
        .split(|byte| matches!(*byte, b'\r' | b'\n'))
        .any(|line| line == b"NoCrew=true"));
    main_assert_eq!(saved.get_in(Some("Graphics"), "SmokeLevel") => Some("321"));
    main_assert_eq!(saved.get_in(Some("General"), "Version") => Some("347"));
    main_assert_eq!(saved.get_in(Some("General"), "ConfigResetSafety") => Some("42"));
    main_assert_eq!(saved.get_in(Some("General"), "VendorExtension") => Some("keep"));
    main_assert_eq!(saved.get_in(Some("Vendor"), "Template") => Some("<i>keep</i>"));
    main_assert_eq!(saved.get_in(Some("Vendor"), "Escaped") => Some("AB\u{1b}"));
    main_assert_eq!(saved.get_in(Some("Sound"), "MenuMusic") => Some("0"));
    main_assert_eq!(saved.get_in(Some("Network"), "LocalName") => Some("Unsaved host"), "Save commits the retained normal Options draft before recreation");
    let replacement_key_text = replacement_key.to_string();
    main_assert_eq!(saved.get_in(Some("Controls"), "Kbd1Key1") => Some(replacement_key_text.as_str()));
    main_assert!({
        let serialized =
            fs::read_to_string(paths.config_file()).expect("serialized advanced config");
        serialized.contains("Name=\"New name\"")
            && !serialized.contains("keep this note")
            && serialized.contains("# standalone note")
    });
    app.persist_open_options_config()
        .expect("open Options config")
        .test_value();
    let resaved = Config::load(paths.config_file()).test_value();
    main_assert!(resaved.get_in(Some("Sound"), "MenuMusic").is_some_and(|value| !parse_config_bool(value)));
    main_assert_eq!(
        resaved.get_in(Some("Controls"), "Kbd1Key1") =>
        Some(replacement_key_text.as_str()),
        "normal Options persistence must not undo the advanced binding"
    );
    let options_before_cancel = {
        let options = app.startup_options_dialog.test_mut();
        options.network_mut().set_text(
            NetworkTextField::LocalName,
            "Keep this unsaved edit".to_string(),
        );
        options as *const _ as usize
    };
    let before_cancel = fs::read(paths.config_file()).test_value();
    app.process_options_dialog_actions(vec![OptionsDlgAction::OpenAdvancedSettings])
        .test_value();
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    main_assert_eq!(fs::read(paths.config_file()).expect("config after cancel-path open") => before_cancel);
    main_assert!(app
        .startup_options_advanced_dialog
        .as_mut()
        .expect("cancel-path editor")
        .controller
        .set_value(
            "General",
            "Name",
            AdvancedConfigValue::Text("Discard me".to_string()),
        ));
    let cancel = app
        .startup_options_advanced_dialog
        .test_ref()
        .controller
        .layout()
        .cancel_button;
    let cancel_point = GuiPoint::new(
        (cancel.x + cancel.w / 2) as f32,
        (cancel.y + cancel.h / 2) as f32,
    );
    app.test_touch(TouchPhase::Started, cancel_point);
    app.test_touch(TouchPhase::Ended, cancel_point);
    main_assert!(app.startup_options_advanced_dialog.is_none());
    let options_after_cancel = app.startup_options_dialog.test_ref();
    main_assert_eq!(options_after_cancel as *const _ as usize => options_before_cancel);
    main_assert_eq!(options_after_cancel.active_sheet() => OptionsSheet::Network);
    main_assert_eq!(options_after_cancel.network().local_name => "Keep this unsaved edit");
    main_assert_eq!(fs::read(paths.config_file()).expect("config after editor cancel") => before_cancel);

    app.process_options_dialog_actions(vec![OptionsDlgAction::OpenAdvancedSettings])
        .test_value();
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    fs::remove_file(paths.config_file()).test_value();
    fs::create_dir(paths.config_file()).test_value();
    app.process_options_advanced_actions(vec![AdvancedConfigAction::Save])
        .test_value();
    main_assert!(app.startup_options_advanced_dialog.is_some(), "a failed Save keeps the draft editor open");
    main_assert_eq!(app.message_dialogs.last().expect("advanced save error dialog").state.caption() => "Fehler");
    main_assert!(app.status_text.is_empty());
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.startup_options_advanced_dialog.is_some());
    fs::remove_dir(paths.config_file()).test_value();
    reset_cached_app_paths();
}

#[test]
fn options_gamepad_capture_records_the_exact_axis_key() {
    use clonk_frontend::startup_options_controls::{ControlCaptureTarget, ControlDevice};
    use clonk_frontend::startup_options_dlg::OptionsDlgAction;

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    let target = ControlCaptureTarget {
        device: ControlDevice::Gamepad,
        set: 0,
        control: ControlBindingId::Dig as usize,
    };
    app.process_options_dialog_actions(vec![OptionsDlgAction::BeginControlCapture(target)])
        .test_value();
    let slot = GamepadSlot::new(0);
    app.process_sourced_gamepad_event_batch(
        [
            saves_fixture!(
                sourced_gamepad:
                    0,
                    4,
                    gamepad_axis_event(slot, LegacyGamepadAxis::new(1, false), ElementState::Pressed),
            ),
            saves_fixture!(
                sourced_gamepad:
                    0,
                    4,
                    gamepad_direction_event(slot, ControlButton::Up, ElementState::Pressed),
            ),
        ],
        true,
    )
    .test_value();

    main_assert!(app.message_dialogs.is_empty());
    main_assert_eq!(app.gamepad_bindings.raw_key_for_set(0, ControlBindingId::Dig) => input::legacy_gamepad_axis_key(0, 1, false));
}

// C4GamePadControl::FeedEvent converts every raw SDL joystick event into
// the classic key space before capture sees it: arbitrary axis ordinals
// pass straight through, hats become the axis pair `hat * 2 + 6` and balls
// the pair `ball * 2 + 12` (C4GamePadCon.cpp:335-435).
#[test]
fn options_gamepad_capture_accepts_full_classic_raw_event_space() {
    use clonk_frontend::startup_options_controls::{ControlCaptureTarget, ControlDevice};
    use clonk_frontend::startup_options_dlg::OptionsDlgAction;

    use crate::gamepad::{LegacyHatValue, RawJoystickEvent};

    let slot = GamepadSlot::new(0);
    // A raw ninth axis, hat 2's vertical axis and ball 0's horizontal axis
    // are all unreachable through gilrs' semantic axes and Hat 0 alone.
    for (control, raw_event, expected_axis, expected_high) in [
        (
            ControlBindingId::Dig,
            RawJoystickEvent::Axis {
                axis: 9,
                value: i16::MIN,
            },
            9,
            false,
        ),
        (
            ControlBindingId::Throw,
            RawJoystickEvent::Hat {
                hat: 2,
                value: LegacyHatValue::DOWN,
            },
            11,
            true,
        ),
        (
            ControlBindingId::Special,
            RawJoystickEvent::Ball {
                ball: 0,
                xrel: 7,
                yrel: 0,
            },
            12,
            true,
        ),
    ] {
        let mut app = new_classic_menu_app(640, 480);
        app.open_options_menu();
        let target = ControlCaptureTarget {
            device: ControlDevice::Gamepad,
            set: 0,
            control: control as usize,
        };
        app.process_options_dialog_actions(vec![OptionsDlgAction::BeginControlCapture(target)])
            .test_value();

        let mut manager = GamepadManager::disabled();
        let events = manager.feed_raw_event(slot, raw_event);
        main_assert!(
            events.iter().any(|sourced| matches!(
                sourced.event,
                GamepadEvent::Axis { axis, state: ElementState::Pressed, .. }
                    if axis == LegacyGamepadAxis::new(expected_axis, expected_high)
            )),
            "{raw_event:?} must reach the classic axis space"
        );
        app.process_sourced_gamepad_event_batch(events, true)
            .test_value();

        main_assert!(app.message_dialogs.is_empty(), "capture closed its modal");
        main_assert_eq!(
            app.gamepad_bindings.raw_key_for_set(0, control) =>
            input::legacy_gamepad_axis_key(0, expected_axis, expected_high),
            "{raw_event:?} binds the exact KEY_JOY_Axis code"
        );
    }
}

// BoolConfig mutates Config.General.ShowLogTimestamps immediately, while
// DoBack saves the configuration before returning to the main dialog
// (C4StartupOptionsDlg.cpp:564-568, 1150-1184, 1189-1194).
#[test]
fn options_dialog_saves_log_timestamps_when_closed() {
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    persist_config_value(&paths, "General", "ShowLogTimestamps", "1").test_value();
    persist_config_value(&paths, "Sound", "MaxChannels", "37").test_value();
    persist_config_value(&paths, "Sound", "VendorExtension", "keep-me").test_value();
    let mut app = GameApp::new(
        1280,
        720,
        AudioOptions {
            max_channels: 37,
            sound_enabled: false,
            menu_music_enabled: false,
            sound_volume: 0.27,
            music_volume: 0.83,
            ..Default::default()
        },
        Some(&paths),
        test_runtime_config_with("Player", false),
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_options_menu();
    let gui = app.assets.clonk_fonts.as_deref().test_value();
    let book = app.assets.options_book_fonts.as_deref().test_value();
    let options_layout =
        clonk_frontend::startup_options_dlg::options_dlg_layout(1280, 720, gui, book);
    let checkbox = options_layout.timestamps_check;
    // The port-only voice controls, changed further down through the same real
    // pointer path (clonk-org/clonk-rs#422).
    let voice_group = options_layout.sound.voice.as_ref().test_value();
    let activation_check = voice_group.activation_check;
    let voice_volume_slider = voice_group.volume_slider;
    let point = PhysicalPosition::new(
        f64::from(checkbox.x + checkbox.h / 2),
        f64::from(checkbox.y + checkbox.h / 2),
    );

    app.test_cursor(point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(!app.startup_options_dialog.as_ref().expect("options dialog").program().show_log_timestamps, "checkbox toggles before the dialog closes");
    main_assert_eq!(
        Config::load(paths.config_file())
            .expect("config before close")
            .get_in(Some("General"), "ShowLogTimestamps") =>
        Some("1"),
        "C++ defers Config.Save until DoBack"
    );

    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(app.startup_options_dialog.as_ref().expect("options dialog").sound().frontend_sound_effects, "Ctrl+F3 leaves the classic checkbox visually stale");
    main_assert!(!app.audio.as_ref().expect("test audio").options.menu_sound_enabled, "live audio configuration remains authoritative");
    app.test_modifiers(ModifiersState::empty());

    {
        use clonk_frontend::startup_options_graphics::GraphicsDisplayMode;
        use clonk_frontend::startup_options_network::NetworkPortId;
        let dialog = app.startup_options_dialog.as_mut().test_value();
        dialog
            .graphics_mut()
            .set_display_mode(GraphicsDisplayMode::Window);
        dialog.graphics_mut().smoke_level = 73;
        dialog.graphics_mut().fire_particles = false;
        dialog.network_mut().port_mut(NetworkPortId::Tcp).enabled = false;
        dialog.network_mut().use_alternate_server = true;
        dialog.network_mut().local_name = "Same Name".to_string();
        dialog.network_mut().nick = "Same Name".to_string();
        dialog.network_mut().hide_no_official_league_notice = true;
    }
    app.startup_options_dialog
        .as_mut()
        .test_value()
        .restore_sheet(clonk_frontend::startup_options_dlg::OptionsSheet::Sound);
    app.test_cursor(PhysicalPosition::new(
        f64::from(voice_volume_slider.x + voice_volume_slider.w - 24),
        f64::from(voice_volume_slider.y + voice_volume_slider.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(
        app.audio
            .as_ref()
            .expect("test audio")
            .options
            .voice_volume_percent() =>
        200,
        "the Voice slider's right endpoint is the port-only boost ceiling",
    );
    app.test_cursor(PhysicalPosition::new(
        f64::from(activation_check.x + activation_check.h / 2),
        f64::from(activation_check.y + activation_check.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.audio.as_ref().expect("test audio").options.voice_activation_mode => crate::settings::VoiceActivationMode::VoiceActivated,);

    app.bindings
        .rebind_for_set(2, ControlBindingId::Dig, VirtualKeyCode::KeyZ);
    app.gamepad_bindings
        .rebind_button(1, ControlBindingId::Up, 1, 4);
    app.gamepad_gui_control = true;

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    let config = Config::load(paths.config_file()).test_value();
    main_assert_eq!(config.get_in(Some("General"), "ShowLogTimestamps") => Some("0"));
    main_assert_eq!(config.get_in(Some("Sound"), "Sound") => Some("false"));
    main_assert_eq!(config.get_in(Some("Sound"), "Music") => Some("true"));
    main_assert_eq!(config.get_in(Some("Sound"), "MenuMusic") => Some("false"));
    main_assert_eq!(config.get_in(Some("Sound"), "MenuSound") => Some("false"));
    main_assert_eq!(config.get_in(Some("Sound"), "MusicVolume") => Some("83"));
    main_assert_eq!(config.get_in(Some("Sound"), "SoundVolume") => Some("27"));
    main_assert_eq!(config.get_in(Some("Sound"), "MaxChannels") => Some("37"));
    main_assert_eq!(config.get_in(Some("Sound"), "VendorExtension") => Some("keep-me"));
    // The port-only rows land in their own section, and the activation mode is
    // persisted as the canonical token `VoiceActivationMode::parse` accepts --
    // a localized label here would silently read back as push-to-talk.
    main_assert_eq!(config.get_in(Some("Voice"), "Enabled") => Some("false"));
    main_assert_eq!(config.get_in(Some("Voice"), "Volume") => Some("200"));
    main_assert_eq!(config.get_in(Some("Voice"), "ActivationMode") => Some(crate::settings::VoiceActivationMode::VOICE_ACTIVATED));
    main_assert_eq!(config.get_in(Some("Graphics"), "DisplayMode") => Some("Window"));
    main_assert_eq!(config.get_in(Some("Graphics"), "SmokeLevel") => Some("73"));
    main_assert_eq!(config.get_in(Some("Graphics"), "FireParticles") => Some("0"));
    main_assert_eq!(config.get_in(Some("Network"), "PortTCP") => Some("0"));
    main_assert_eq!(config.get_in(Some("Network"), "UseAlternateServer") => Some("1"));
    main_assert_eq!(config.get_in(Some("Network"), "LocalName") => Some("Same Name"));
    main_assert_eq!(config.get_in(Some("Network"), "Nick") => Some(""));
    main_assert_eq!(config.get_in(Some("Startup"), "HideMsgNoOfficialLeague") => Some("1"));
    main_assert!(config.get_in(Some("Controls"), "Kbd3Key6").is_some());
    main_assert_eq!(config.get_in(Some("Controls"), "GamepadGuiControl") => Some("1"));
    main_assert_eq!(config.get_in(Some("Gamepad1"), "Button5") => input::legacy_gamepad_button_key(1, 4).map(|key| key.to_string()).as_deref());
}

#[test]
fn axis_binding_routes_to_configured_set_not_physical_slot() {
    let mut config = Config::new();
    config.set_in(
        Some("Gamepad0"),
        "Button7",
        input::legacy_gamepad_axis_key(1, 0, false)
            .test_value()
            .to_string(),
    );
    let mut app = new_running_sandbox_app();
    app.gamepad_bindings = GamepadBindings::from_config(&config);
    let primary = app.local_owner;
    let secondary = primary + 1;
    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary"))
        .test_value();
    app.engine.set_local_players([primary, secondary]);
    app.local_controls = LocalControlRegistry::default();
    app.local_controls
        .initialize(test_local_control_init(primary, 4, false, false));
    app.local_controls
        .initialize(test_local_control_init(secondary, 5, false, false));

    let slot = GamepadSlot::new(1);
    app.test_gamepad_events([
        gamepad_axis_event(
            slot,
            LegacyGamepadAxis::new(0, false),
            ElementState::Pressed,
        ),
        gamepad_direction_event(slot, ControlButton::Left, ElementState::Pressed),
    ]);

    let pressed = |app: &GameApp, owner| {
        app.engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner)
            .test_value()
            .control
            .pressed_coms
    };
    main_assert_ne!(pressed(&app, primary) & (1 << clonk_engine::COM_LEFT) => 0);
    main_assert_eq!(pressed(&app, secondary) & (1 << clonk_engine::COM_LEFT) => 0);
}

/// `C4GameSave::SaveCore` shortens the scenario path with
/// `Config.ForceRelativePath` (`C4GameSave.cpp:99-100`), which strips the one
/// `ExePath` holding the data groups (`C4Config.cpp:1438-1442`). A pristine
/// C++ capture of that field is pinned in
/// `clonk-engine/src/scenario/tests/part_01.rs`, and it carries no data-root
/// segment — so a scenario inside the root and one outside it shorten alike.
#[test]
fn initial_record_origin_is_relative_to_cpp_executable_root() {
    let paths = test_app_paths();
    let scenario = paths
        .executable_data_root()
        .join("Missions.c4f/Deep/Game.c4s");

    main_assert_eq!(record_scenario_origin(&scenario, Some(&paths), "wrong-ui-identifier") => "Missions.c4f/Deep/Game.c4s");
    main_assert_eq!(record_scenario_origin(Path::new("/outside/Missions.c4f/Deep/Game.c4s"), Some(&paths), "wrong-ui-identifier",) => "Missions.c4f/Deep/Game.c4s");
}

#[test]
fn record_index_and_basename_match_cpp_directory_scan() {
    let directory = tempdir();
    fs::write(directory.path().join("001-First.c4s"), b"record").test_value();
    fs::create_dir(directory.path().join("002-Unpacked.c4s")).test_value();
    fs::write(directory.path().join("mixed.C4S"), b"record").test_value();
    fs::write(directory.path().join("999-not-a-record.txt"), b"other").test_value();

    main_assert_eq!(next_recording_index(directory.path()).unwrap() => 4);
    main_assert_eq!(sanitize_record_name("Scenario007") => "Scenario");
    main_assert_eq!(sanitize_record_name("123") => "1");
    main_assert_eq!(sanitize_record_name("Odd name!?") => "Odd name!?");
}

#[test]
fn forced_recording_rejects_missing_prepared_storage() {
    let mut app = new_state_only_running_sandbox_app();
    main_assert_eq!(app.start_recording(true) => Err("recording storage was not prepared".to_string()));
    main_assert!(app.recording.is_none());
}

#[test]
fn synchronize_record_request_starts_with_the_executing_control_list() {
    let directory = tempdir();
    let output_path = directory.path().join("001-Runtime.c4s");
    let mut app = new_state_only_running_sandbox_app();
    app.recording_enabled = false;
    install_test_recording_template(&mut app, output_path.clone());
    app.runtime_record_requested = true;
    let synchronize = saves_fixture!(synchronize: false, true);

    app.apply_ready_controls(0, vec![NetworkControl::Synchronize(synchronize.clone())])
        .test_value();
    main_assert!(app.recording.is_some(), "the sync request arms recording");
    main_assert!(app.finish_recording().is_none());

    let group = Group::open(output_path).test_value();
    let stream = group.read_file("CtrlRec.c4b").test_value();
    let mut playback = ControlRecordPlayback::from_bytes(&stream).test_value();
    main_assert_eq!(playback.take_controls(0) => vec![clonk_engine::ControlPacket::Synchronize(synchronize)]);
}

#[test]
fn any_synchronize_starts_only_an_explicitly_requested_runtime_record() {
    let directory = tempdir();
    let mut app = new_state_only_running_sandbox_app();
    install_test_recording_template(&mut app, directory.path().join("001-RuntimeJoin.c4s"));
    app.apply_synchronized_controls(
        0,
        vec![NetworkControl::Synchronize(
            saves_fixture!(synchronize: false, false),
        )],
    )
    .test_value();

    main_assert!(app.recording.is_none());
    main_assert!(app.recording_template.is_some());

    app.runtime_record_requested = true;
    app.apply_synchronized_controls(
        1,
        vec![NetworkControl::Synchronize(
            saves_fixture!(synchronize: false, false),
        )],
    )
    .test_value();

    main_assert!(!app.runtime_record_requested);
    main_assert!(app.recording.is_some());
}

#[test]
fn save_player_files_synchronize_persists_local_player_core_and_crew() {
    let directory = tempdir();
    let profile_path = directory.path().join("Local.c4p");
    fs::create_dir(&profile_path).test_value();
    fs::write(
        profile_path.join("Player.txt"),
        b"[Player]\nName=Stale\nScore=1\n",
    )
    .test_value();
    fs::write(profile_path.join("C4Player.c4b"), b"obsolete core").test_value();
    fs::write(profile_path.join("KeepRoot.dat"), b"root sentinel").test_value();
    fs::write(profile_path.join(".local-metadata"), b"ignored sentinel").test_value();
    for (filename, name) in [("Hero.c4i", "Hero"), ("Idle.c4i", "Idle")] {
        let child = profile_path.join(filename);
        fs::create_dir(&child).test_value();
        fs::write(
            child.join("ObjectInfo.txt"),
            format!("[ObjectInfo]\nid=CLNK\nName={name}\n"),
        )
        .test_value();
        fs::write(child.join("KeepCrew.dat"), format!("{name} sentinel")).test_value();
        fs::write(child.join(".crew-metadata"), format!("{name} metadata")).test_value();
    }

    let mut app = new_state_only_synthetic_crew_running_sandbox_app();
    let player_number = app.local_owner;
    let info_id = 601;
    let crew = |name: &str, filename: &str, total_playing_time: i32, in_action: bool| {
        let core = clonk_engine::CrewInfoCoreFields {
            original_filename: filename.to_string(),
            portrait_file: "none".to_string(),
            ..clonk_engine::CrewInfoCoreFields::default()
        };
        clonk_engine::player_file::CrewInfo {
            id: "CLNK".to_string(),
            name: name.to_string(),
            death_message: format!("{name} fell"),
            core,
            rank: 0,
            rank_name: "Clonk".to_string(),
            experience: if in_action { 321 } else { 123 },
            rounds: 4,
            physical: clonk_engine::PhysicalInfo::default(),
            death_count: if in_action { 2 } else { 1 },
            total_playing_time,
            birthday: 1_234,
            age: 3,
            participation: 1,
            in_action,
            was_in_action: in_action,
            in_action_time: 10,
            has_died: false,
            extra_data: vec![(
                "CrewToken".to_string(),
                Value::Int(if in_action { 77 } else { 33 }),
            )],
            portraits: clonk_engine::CrewPortraitState {
                permanent: clonk_engine::CrewPermanentPortrait::ExplicitNone,
                ..clonk_engine::CrewPortraitState::default()
            },
        }
    };
    let mut state = app.engine.capture_state();
    state.game_time = 25;
    state.player_crew_rosters_authoritative = true;
    let player = state
        .players
        .iter_mut()
        .find(|player| player.id == player_number)
        .test_value();
    player.player_info_id = info_id;
    player.at_client = clonk_engine::PlayerAtClient::HOST;
    player.status = clonk_engine::PlayerStatus::Active;
    player.script_player = false;
    player.score = 900;
    player.rounds = 8;
    player.rounds_won = 5;
    player.rounds_lost = 3;
    player.total_playing_time = 40;
    player.extra_data = vec![("PlayerToken".to_string(), Value::Int(99))];
    let mut info_core = player.player_info_core.take().unwrap_or_default();
    info_core.pref_name = "Persistent Player".to_string();
    player.player_info_core = Some(info_core);
    state.crew_info_rosters.insert(
        player_number,
        vec![
            crew("Hero", "Hero.c4i", 7, true),
            crew("Idle", "Idle.c4i", 11, false),
        ],
    );
    state.crew_info_order.insert(player_number, vec![0, 1]);
    app.engine.restore_state(&state).test_value();
    app.engine
        .test_player_mut(player_number)
        .set_game_join_time(10);
    app.engine.set_local_players([player_number]);
    app.control_player_infos.replace_snapshot(
        info_id,
        [clonk_engine::PlayerInfoControlData::new(
            0,
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: info_id,
                name: legacy_cstring(b"Persistent Player"),
                filename: legacy_cstring(b"Local.c4p"),
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                game_number: player_number,
                ..clonk_engine::ControlPlayerInfoEntry::default()
            }],
            0,
        )],
    );
    app.local_player_profile_paths
        .insert(info_id, profile_path.clone());

    let synchronize = || NetworkControl::Synchronize(saves_fixture!(synchronize: true, false));
    app.apply_synchronized_controls(0, vec![synchronize()])
        .test_value();

    let saved = PlayerFile::load_from_path(&profile_path).test_value();
    main_assert_eq!(saved.name => "Persistent Player");
    main_assert_eq!((saved.score, saved.rounds, saved.rounds_won, saved.rounds_lost) => (900, 8, 5, 3));
    main_assert_eq!(saved.total_playing_time => 55);
    main_assert_eq!(saved.info_core.extra_data => vec![("PlayerToken".to_string(), Value::Int(99))]);
    let hero = saved
        .crew
        .iter()
        .find(|crew| crew.name == "Hero")
        .test_value();
    main_assert_eq!((hero.experience, hero.death_count, hero.total_playing_time) => (321, 2, 22));
    main_assert_eq!(hero.extra_data => vec![("CrewToken".to_string(), Value::Int(77))]);
    let idle = saved
        .crew
        .iter()
        .find(|crew| crew.name == "Idle")
        .test_value();
    main_assert_eq!(idle.total_playing_time => 11, "idle crew has no active-time delta");
    let saved_group = Group::open(&profile_path).test_value();
    main_assert_eq!(saved_group.read_file("KeepRoot.dat").unwrap() => b"root sentinel");
    main_assert_eq!(saved_group.open_child("Hero.c4i").unwrap().read_file("KeepCrew.dat").unwrap() => b"Hero sentinel");
    main_assert_eq!(fs::read(profile_path.join("Hero.c4i/.crew-metadata")).unwrap() => b"Hero metadata");
    main_assert!(!saved_group.exists("C4Player.c4b"));
    main_assert_eq!(fs::read(profile_path.join(".local-metadata")).unwrap() => b"ignored sentinel");

    // A removed local source is not an eligibility failure in C++: its
    // unchecked copy leaves a fresh temp group that Save fills. Recreate
    // it at the same game time and also prove the ledgers are not counted
    // twice at consecutive SavePlrs boundaries.
    fs::remove_dir_all(&profile_path).test_value();
    app.apply_synchronized_controls(1, vec![synchronize()])
        .test_value();
    let recreated = PlayerFile::load_from_path(&profile_path).test_value();
    main_assert_eq!(recreated.total_playing_time => 55);
    main_assert_eq!(recreated.crew.iter().find(|crew| crew.name == "Hero").expect("recreated active crew").total_playing_time => 22);
    let recreated_group = Group::open(&profile_path).test_value();
    main_assert!(!recreated_group.exists("KeepRoot.dat"));
    main_assert!(!recreated_group.exists("C4Player.c4b"));
    main_assert!(fs::read_dir(directory.path())
        .expect("enumerate rewrite directory")
        .all(|entry| !entry
            .expect("rewrite directory entry")
            .file_name()
            .to_string_lossy()
            .starts_with(".Local.c4p.lc-rewrite-")));

    // SavePlrs inside control replay is a complete no-op for both the
    // mutable time checkpoint and the app-owned profile write.
    let profile_before_replay = fs::read(&profile_path).test_value();
    let mut replay_state = app.engine.capture_state();
    replay_state.game_time = 30;
    replay_state
        .players
        .iter_mut()
        .find(|player| player.id == player_number)
        .test_value()
        .score = 901;
    app.engine.restore_state(&replay_state).test_value();
    app.engine
        .test_player_mut(player_number)
        .set_game_join_time(25);
    app.control_playback =
        Some(ControlRecordPlayback::from_bytes(&[0, clonk_engine::RCT_END]).test_value());
    app.engine.set_replay_control(true);
    app.apply_synchronized_controls(2, vec![synchronize()])
        .test_value();
    main_assert_eq!(fs::read(&profile_path).unwrap() => profile_before_replay);
    main_assert_eq!(
        (
            app.engine
                .player(player_number)
                .expect("replay player remains")
                .total_playing_time(),
            app.engine
                .player(player_number)
                .expect("replay player remains")
                .game_join_time(),
        ) =>
        (55, 25)
    );
    let replay_state = app.engine.capture_state();
    let replay_roster = &replay_state.crew_info_rosters[&player_number];
    let replay_hero = replay_roster
        .iter()
        .find(|crew| crew.name == "Hero")
        .test_value();
    main_assert_eq!((replay_hero.total_playing_time, replay_hero.in_action_time) => (22, 25));
}

#[test]
fn developer_console_runtime_record_waits_for_its_queued_synchronize() {
    let directory = tempdir();
    let output_path = directory.path().join("001-ConsoleRuntime.c4s");
    let mut app = new_state_only_running_sandbox_app();
    install_test_recording_template(&mut app, output_path);
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
    let tick = app.local_control_submission_tick();

    main_assert!(app.developer_console_runtime_record_possible());
    main_assert!(app.developer_console_request_runtime_record().expect("queue console runtime record"));
    main_assert!(app.runtime_record_requested);
    main_assert!(!app.developer_console_runtime_record_possible());
    main_assert!(app.recording.is_none());

    let decided = commands.take_submitted_decided_controls();
    main_assert_eq!(decided => vec![(tick, clonk_engine::ControlPacket::Synchronize(saves_fixture!(synchronize: false, true),), false,)]);

    app.apply_ready_controls(
        tick,
        vec![NetworkControl::Synchronize(
            saves_fixture!(synchronize: false, true),
        )],
    )
    .test_value();
    main_assert!(!app.runtime_record_requested);
    main_assert!(app.recording.is_some());
}

#[test]
fn developer_console_save_as_preserves_unpacked_group_representation() {
    let directory = tempdir();
    let destination = directory.path().join("Copy.c4s");
    let mut group = MutableGroup::new("Source.c4s");
    group
        .add_file("Scenario.txt", b"[Head]\nTitle=Copy\n".to_vec())
        .test_value();

    persist_console_save_group(&group, &destination, true).test_value();
    main_assert!(destination.is_dir());
    main_assert_eq!(fs::read(destination.join("Scenario.txt")).unwrap() => b"[Head]\nTitle=Copy\n");
}

#[test]
fn folder_live_save_material_respects_existing_directory_representation() {
    let directory = tempdir();
    let destination = directory.path().join("Live.c4s");
    let material = destination.join("Material.c4g");
    fs::create_dir_all(&material).test_value();
    fs::write(material.join("Keep.dat"), b"keep").test_value();
    fs::write(material.join("TexMap.txt"), b"old").test_value();

    let mut patch = MutableGroup::new("Material.c4g");
    patch
        .add_file("TexMap.txt", b"new texture map".to_vec())
        .test_value();
    let mut journal = developer_console_save::FolderSaveJournal::default();
    journal.merge_material_group(patch.pack_raw().test_value());

    replay_folder_save_journal(&journal, &destination, b"Folder maker").test_value();

    main_assert!(material.is_dir());
    main_assert_eq!(fs::read(material.join("Keep.dat")).unwrap() => b"keep");
    main_assert_eq!(fs::read(material.join("TexMap.txt")).unwrap() => b"new texture map");
}

#[test]
fn ctrlrec_control_executes_at_start_of_recorded_frame() {
    let mut app = new_running_sandbox_app();
    let packet = recorded_right_control(app.local_owner);
    let mut writer = ControlRecordWriter::new();
    writer.record_packet(1, &packet).test_value();
    app.control_playback = Some(ControlRecordPlayback::from_bytes(&writer.finish(1)).test_value());
    app.engine.set_replay_control(true);

    app.test_update();
    main_assert_eq!(app.engine.frame() => 1);
    main_assert_eq!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_RIGHT) =>
        0,
        "frame-one control must not execute during frame zero"
    );

    app.test_update();
    main_assert_eq!(app.engine.frame() => 2);
    main_assert_ne!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_RIGHT) =>
        0,
        "recorded control executes before frame one's simulation tick"
    );
}

#[test]
fn replay_prefers_and_executes_cpp_ctrlrec_text_over_binary() {
    let directory = tempdir();
    let replay_path = directory.path().join("TextWins.c4s");
    fs::create_dir(&replay_path).test_value();
    let mut app = new_running_sandbox_app();
    let player = app.local_owner;
    let text = format!(
        concat!(
            "[Rec]\r\n",
            "Frame=0\r\n",
            "Type=0\r\n",
            "\r\n",
            "  [IDPacket]\r\n",
            "  ID=161\r\n",
            "\r\n",
            "    [Player Control]\r\n",
            "    Player={player}\r\n",
            "    Com={}\r\n",
            "    ByClient=0\r\n",
            "\r\n",
            "[Rec]\r\n",
            "Frame=0\r\n",
            "Type=1\r\n",
            "ID=161\r\n",
            "\r\n",
            "  [Player Control]\r\n",
            "  Player={player}\r\n",
            "  Com={}\r\n",
            "  ByClient=0\r\n",
            "\r\n",
            "[Rec]\r\n",
            "Frame=1\r\n",
            "Type=16\r\n",
        ),
        clonk_engine::COM_RIGHT,
        clonk_engine::COM_UP,
        player = player,
    );
    fs::write(replay_path.join("CtrlRec.txt"), text.as_bytes()).test_value();

    let binary_control = clonk_engine::ControlPacket::PlayerControl(
        clonk_engine::PlayerControlData::new(player, i32::from(clonk_engine::COM_LEFT), 0, 0),
    );
    let mut binary = ControlRecordWriter::new();
    binary.record_packet(0, &binary_control).test_value();
    fs::write(replay_path.join("CtrlRec.c4b"), binary.finish(1)).test_value();

    let replay_group = Group::open(&replay_path).test_value();
    let replay_chunks = replay_control_record_chunks(&replay_group).test_value();
    let dump_path = directory.path().join("preferred.txt");
    write_classic_record_dump(&replay_chunks, &dump_path).test_value();
    let dump = fs::read_to_string(&dump_path).test_value();
    main_assert!(dump.contains(&format!("    Com={}\r\n", clonk_engine::COM_RIGHT)));
    main_assert!(dump.contains(&format!("  Com={}\r\n", clonk_engine::COM_UP)));
    main_assert!(!dump.contains(&format!("  Com={}\r\n", clonk_engine::COM_LEFT)));
    app.control_playback = Some(ControlRecordPlayback::from_chunks(replay_chunks));
    app.engine.set_replay_control(true);
    app.engine.set_control_host(false);
    app.test_update();

    let pressed = app.engine.test_player(player).control.pressed_coms;
    main_assert_ne!(pressed & (1 << clonk_engine::COM_RIGHT) => 0);
    main_assert_ne!(pressed & (1 << clonk_engine::COM_UP) => 0);
    main_assert_eq!(pressed & (1 << clonk_engine::COM_LEFT) => 0, "the lower-priority binary control must not execute");

    let binary_only_path = directory.path().join("BinaryFallback.c4s");
    fs::create_dir(&binary_only_path).test_value();
    let mut binary = ControlRecordWriter::new();
    binary.record_packet(0, &binary_control).test_value();
    fs::write(binary_only_path.join("CtrlRec.c4b"), binary.finish(1)).test_value();
    let binary_only = Group::open(&binary_only_path).test_value();
    let mut fallback =
        ControlRecordPlayback::from_chunks(replay_control_record_chunks(&binary_only).test_value());
    main_assert_eq!(fallback.take_controls(0) => vec![binary_control.clone()]);

    let invalid_text_path = directory.path().join("InvalidTextWins.c4s");
    fs::create_dir(&invalid_text_path).test_value();
    fs::write(
        invalid_text_path.join("CtrlRec.txt"),
        b"[Rec]\nFrame=0\nType=1\nID=255\n",
    )
    .test_value();
    let mut binary = ControlRecordWriter::new();
    binary.record_packet(0, &binary_control).test_value();
    fs::write(invalid_text_path.join("CtrlRec.c4b"), binary.finish(1)).test_value();
    let invalid_text = Group::open(&invalid_text_path).test_value();
    let error = replay_control_record_chunks(&invalid_text)
        .expect_err("loaded malformed text must not fall back to binary");
    main_assert!(error.contains("invalid CtrlRec.txt"));
    main_assert!(error.contains("packet ID 0xff"));
}

#[test]
fn ctrlrec_end_finishes_the_replay_and_restores_local_control() {
    let mut app = new_classic_running_sandbox_app();
    app.control_playback =
        Some(ControlRecordPlayback::from_bytes(&[0, clonk_engine::RCT_END]).test_value());
    app.engine.set_replay_control(true);
    app.engine.set_control_host(false);

    app.test_update();

    main_assert!(app.control_playback.is_none());
    main_assert!(app.engine.is_control_host());
    main_assert!(app.snapshot.game_over);
}

#[test]
fn film_assigned_no_owner_viewport_edge_scrolls_observer_not_player() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let focus = app.engine.test_crew_cursor(owner);
    app.engine
        .replace_player_viewports(
            owner,
            vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
        )
        .test_value();
    app.engine.set_local_players([]);
    app.local_controls = LocalControlRegistry::default();
    app.mouse_control = false;
    app.snapshot = app.engine.snapshot();
    app.display_flags.show_commands = false;
    app.film_view_player = Some(owner);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let before = app.graphics.active_viewport_projections()[0];
    main_assert_eq!(before.owner => owner);
    main_assert!(before.is_no_owner_viewport);
    app.engine
        .scroll_player_view(
            owner,
            Vector2::ZERO,
            before.logical_width,
            before.logical_height,
            true,
        )
        .test_value();
    let player_viewports = app.engine.player(owner).test_value().viewports().to_vec();
    let left = GuiPoint::new(
        before.rect.x as f32,
        (before.rect.y + before.rect.height as i32 / 2) as f32,
    );

    app.test_cursor(PhysicalPosition::new(f64::from(left.x), f64::from(left.y)));

    let after_move = app.graphics.active_viewport_projections()[0];
    main_assert_eq!(after_move.target_x => before.target_x - 10);
    main_assert!(app.ingame_edge_scroll.expect("classified observer edge remains live").observer);
    main_assert_eq!(
        app.engine.player(owner).unwrap().viewports() =>
        player_viewports.as_slice(),
        "temporary film assignment must not turn observer scrolling into player scrolling"
    );

    // SetFilmView changes the viewport's displayed Player but preserves
    // fIsNoOwnerViewport. MouseControl remains assigned to NO_OWNER, so
    // its retained VpX/VpY must continue through that owner change.
    main_assert!(app.set_physical_film_view(OWNER_NONE));
    app.test_render(&mut frame);
    let retargeted = app.graphics.active_viewport_projections()[0];
    main_assert_eq!(retargeted.owner => OWNER_NONE);
    main_assert!(retargeted.is_no_owner_viewport);

    app.test_update();

    let after_tick = app.graphics.active_viewport_projections()[0];
    main_assert_eq!(after_tick.target_x => retargeted.target_x - 10);
    let scroll = app.ingame_edge_scroll.test_value();
    main_assert!(scroll.observer);
    main_assert_eq!(scroll.owner => OWNER_NONE);
    main_assert_eq!(app.engine.player(owner).unwrap().viewports() => player_viewports.as_slice());
}

#[test]
fn replay_film_view_retargets_only_the_existing_primary_viewport() {
    let app = new_state_only_running_sandbox_app();
    let mut snapshot = app.snapshot.clone();
    let local_owner = app.local_owner;
    let local = snapshot
        .players
        .iter()
        .find(|player| player.id == local_owner)
        .cloned()
        .test_value();
    let local_focus = local
        .viewports
        .first()
        .and_then(|viewport| viewport.focus)
        .or(local.cursor)
        .or_else(|| local.crew.first().copied())
        .test_value();

    let film_focus = ObjectId::new(
        snapshot
            .objects
            .iter()
            .map(|object| object.id.as_u64())
            .max()
            .unwrap_or(0)
            + 1,
    );
    let mut film_object = snapshot.object(local_focus).test_value().clone();
    film_object.id = film_focus;
    snapshot.objects.push(film_object);

    let mut split_local = local.clone();
    split_local.view_offset = Vector2::new(17, 19);
    split_local.viewports.push(
        clonk_engine::PlayerViewport::new(Vector2::new(300, 400))
            .with_focus(Some(local_focus))
            .with_zoom(1.5),
    );
    let mut film_player = local;
    film_player.id = local_owner + 1;
    film_player.name = "Film target".to_string();
    film_player.view_offset = Vector2::new(11, 13);
    film_player.viewports = vec![clonk_engine::PlayerViewport::new(Vector2::new(700, 800))
        .with_focus(Some(film_focus))
        .with_zoom(2.0)];
    snapshot.players = vec![split_local, film_player.clone()];
    snapshot.hud.local_players = vec![local_owner, film_player.id];

    let ordinary = collect_viewport_inputs(&snapshot).test_value();
    let film = collect_viewport_inputs_with_film_view(&snapshot, Some(film_player.id)).test_value();
    main_assert_eq!(film.len() => ordinary.len());
    main_assert_eq!(film[0].owner => film_player.id);
    main_assert_eq!(film[0].center => Vector2::new(700, 800));
    main_assert_eq!(film[0].offset => Vector2::new(17, 19), "temporary Init preserves the physical viewport offset");
    main_assert_eq!(film[0].focus.expect("film viewport focus").id => film_focus);
    main_assert_eq!(film[0].zoom => ordinary[0].zoom, "temporary Init preserves the physical viewport zoom");
    main_assert_eq!(
        film[1..]
            .iter()
            .map(|viewport| {
                (
                    viewport.owner,
                    viewport.center,
                    viewport.zoom,
                    viewport.focus.expect("local viewport focus").id,
                )
            })
            .collect::<Vec<_>>() =>
        ordinary[1..]
            .iter()
            .map(|viewport| {
                (
                    viewport.owner,
                    viewport.center,
                    viewport.zoom,
                    viewport.focus.expect("local viewport focus").id,
                )
            })
            .collect::<Vec<_>>(),
        "only the first physical viewport is retargeted"
    );

    let ownerless =
        collect_viewport_inputs_with_film_view(&snapshot, Some(OWNER_NONE)).test_value();
    main_assert_eq!(ownerless[0].owner => OWNER_NONE);
    main_assert_eq!(ownerless[0].center => ordinary[0].center);
    main_assert_eq!(ownerless[0].zoom => ordinary[0].zoom);
    main_assert_eq!(ownerless[0].focus => ordinary[0].focus);
    main_assert_eq!(ownerless.len() => ordinary.len());

    let mut no_target_focus = snapshot.clone();
    let target = no_target_focus
        .players
        .iter_mut()
        .find(|player| player.id == film_player.id)
        .test_value();
    target.viewports[0].focus = None;
    target.cursor = None;
    target.view_cursor = None;
    target.crew.clear();
    no_target_focus.hud.local_players = vec![local_owner];
    let unfocused =
        collect_viewport_inputs_with_film_view(&no_target_focus, Some(film_player.id)).test_value();
    main_assert_eq!(unfocused[0].owner => film_player.id);
    main_assert_eq!(unfocused[0].center => Vector2::new(700, 800));
    main_assert_eq!(unfocused[0].focus => ordinary[0].focus);

    let mut no_target_viewport = no_target_focus;
    no_target_viewport
        .players
        .iter_mut()
        .find(|player| player.id == film_player.id)
        .test_value()
        .viewports
        .clear();
    let without_local_slot =
        collect_viewport_inputs_with_film_view(&no_target_viewport, Some(film_player.id))
            .test_value();
    main_assert_eq!(without_local_slot[0].owner => film_player.id);
    main_assert_eq!(without_local_slot[0].center => ordinary[0].center);
}

#[test]
fn viewport_player_cycle_matches_film_and_observer_end_states() {
    let mut app = new_running_sandbox_app();
    let first = app.local_owner;
    let second = first + 1;
    let third = first + 2;
    for player in [second, third] {
        app.engine
            .register_player(PlayerConfig::new(player, format!("Player {player}")))
            .test_value();
    }
    app.clear_physical_viewport_states();
    let observer = app.ownerless_physical_viewport_state();
    app.physical_viewports.push(observer);
    app.physical_viewports_authoritative = true;

    let observer_flash = RuntimeFlashMessage {
        text: "Observer controls".to_string(),
        remaining_draws: 10,
        y: 10,
    };
    app.runtime_flash_message = Some(observer_flash.clone());
    main_assert!(app.cycle_primary_viewport_player(true));
    main_assert_eq!(app.film_view_player => Some(first));
    main_assert!(app.runtime_flash_message.is_none(), "temporary Init to an owned player clears the observer flash");

    main_assert!(app.cycle_primary_viewport_player(true));
    main_assert_eq!(app.film_view_player => Some(second));

    main_assert!(app.set_physical_film_view(third));
    main_assert!(app.cycle_primary_viewport_player(true));
    main_assert_eq!(app.film_view_player => Some(first), "film wraps to First");

    main_assert!(app.set_physical_film_view(third));
    app.runtime_flash_message = Some(observer_flash.clone());
    main_assert!(app.cycle_primary_viewport_player(false));
    main_assert_eq!(app.film_view_player => Some(OWNER_NONE));
    main_assert_eq!(app.runtime_flash_message => Some(observer_flash.clone()), "temporary Init to NO_OWNER retains the observer flash");
    main_assert!(app.cycle_primary_viewport_player(false));
    main_assert_eq!(app.film_view_player => Some(first));

    main_assert!(app.set_physical_film_view(77));
    main_assert!(app.cycle_primary_viewport_player(false));
    main_assert_eq!(app.film_view_player => Some(first), "invalid player selects First");

    app.engine.remove_player(second).test_value();
    app.engine.remove_player(third).test_value();
    app.runtime_flash_message = Some(observer_flash.clone());
    main_assert!(!app.cycle_primary_viewport_player(true));
    main_assert_eq!(app.runtime_flash_message => Some(observer_flash.clone()));

    app.engine.remove_player(first).test_value();
    main_assert!(app.set_physical_film_view(OWNER_NONE));
    main_assert!(!app.cycle_primary_viewport_player(true));
    main_assert_eq!(app.film_view_player => Some(OWNER_NONE));
}

#[test]
fn film_replay_hides_viewport_menus_but_keeps_messages_and_film_view() {
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    let cursor = app.engine.test_crew_cursor(owner);

    for (replay, film, overlays_visible) in [
        (0, 0, true),
        (1, 0, true),
        (0, 1, true),
        (1, 1, false),
        (1, 2, false),
    ] {
        app.engine
            .apply_object_update(cursor, saves_fixture!(object_update))
            .test_value();
        set_test_scenario_head_flags(&mut app, replay, film);
        app.snapshot.hud.messages.clear();

        let message_board = app.message_board.clone();
        let mut without_menu = vec![0_u8; 320 * 200 * 4];
        app.test_render(&mut without_menu);
        install_test_cursor_menu(&mut app, cursor, two_item_script_menu(cursor));
        app.message_board = message_board;
        let mut with_menu = vec![0_u8; 320 * 200 * 4];
        app.test_render(&mut with_menu);
        main_assert_eq!(with_menu != without_menu => overlays_visible, "Replay={replay}, Film={film}");
    }

    app.engine
        .apply_object_update(cursor, saves_fixture!(object_update))
        .test_value();
    set_test_scenario_head_flags(&mut app, 1, 1);
    app.snapshot.hud.messages.clear();
    let message_board = app.message_board.clone();
    let mut without_message = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut without_message);

    app.snapshot.hud.messages = vec![clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::GlobalPlayer,
        lines: vec!["Film message remains".to_string()],
        target: None,
        player: Some(owner),
        offset: Vector2::ZERO,
        color: 0xffff_ffff,
        flags: 0,
        width: None,
        decoration: None,
        frame_decoration: None,
        portrait: None,
    }];
    app.message_board = message_board.clone();
    let mut with_message = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut with_message);
    main_assert_ne!(with_message => without_message, "the clean film viewport must retain Game.Messages");

    let mut invalid_hidden_menu = two_item_script_menu(cursor);
    invalid_hidden_menu.style = 99;
    install_test_cursor_menu(&mut app, cursor, invalid_hidden_menu);
    app.ingame_menu.replace(
        owner,
        IngameMenuState::main_menu(
            &MainMenuConditions {
                has_player: true,
                player_count: 1,
                ..MainMenuConditions::default()
            },
            &IngameMenuLabels::default(),
        ),
    );
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let pointer = GuiPoint::new(
        viewport.x as f32 + viewport.width as f32 / 2.0,
        viewport.y as f32 + viewport.height as f32 / 2.0,
    );
    app.ingame_pointer = app.graphics.viewport_point_at(pointer);
    app.ingame_mouse_help_caption = Some(IngameMouseHelpCaption {
        text: "Hidden mouse caption".to_string(),
        keep_moves: 1,
    });
    app.message_board = message_board;
    let mut with_hidden_menus = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut with_hidden_menus);
    main_assert_eq!(with_hidden_menus => with_message, "script and player menus contribute no film-replay pixels");
    main_assert_eq!(app.script_menu_pointer_target_for_owner(owner, GuiPoint::new(160.0, 100.0)).expect("hidden menu pointer routing is inert") => None);
    main_assert_eq!(app.ingame_menu_pointer_target(GuiPoint::new(160.0, 100.0)) => None);
    let menu_button = clonk_frontend::hud::viewport_button_rect(
        viewport,
        clonk_frontend::hud::ViewportButton::PlayerMenu,
    );
    main_assert_eq!(
        app.ingame_viewport_region(
            owner,
            GuiPoint::new(menu_button.x as f32 + 1.0, menu_button.y as f32 + 1.0),
        ) =>
        None,
        "suppressed HUD regions cannot consume film input"
    );

    app.ingame_menu.clear();
    app.engine
        .apply_object_update(cursor, saves_fixture!(object_update))
        .test_value();
    let next_owner = owner + 1;
    app.engine
        .register_player(PlayerConfig::new(next_owner, "Second film player"))
        .test_value();
    main_assert!(app.set_physical_film_view(owner));
    app.test_key(VirtualKeyCode::ArrowRight, ElementState::Pressed);
    main_assert_eq!(app.film_view_player => Some(next_owner));
}

#[test]
fn bare_film_right_cycles_on_down_through_nonexclusive_overlays() {
    let mut app = new_running_sandbox_app();
    let first = app.local_owner;
    let second = first + 1;
    app.engine
        .register_player(PlayerConfig::new(second, "Second film player"))
        .test_value();
    main_assert!(app.set_physical_film_view(first));

    main_assert!(!app.handle_film_view_key_for_mode(VirtualKeyCode::ArrowRight, ElementState::Pressed, false,));
    main_assert_eq!(app.film_view_player => Some(first));

    main_assert!(app.handle_film_view_key_for_mode(VirtualKeyCode::ArrowRight, ElementState::Pressed, true,));
    main_assert_eq!(app.film_view_player => Some(second), "ViewportCheck assigns Players.First before the first film key");
    main_assert!(!app.handle_film_view_key_for_mode(VirtualKeyCode::ArrowRight, ElementState::Released, true,));
    main_assert_eq!(app.film_view_player => Some(second), "C4KeyCB has no key-up callback");

    app.keyboard_modifiers = ModifiersState::SHIFT;
    main_assert!(!app.handle_film_view_key_for_mode(VirtualKeyCode::ArrowRight, ElementState::Pressed, true,));
    main_assert_eq!(app.film_view_player => Some(second));
    app.keyboard_modifiers = ModifiersState::empty();

    app.scoreboard_dialog = Some(app.scoreboard_request());
    main_assert!(app.handle_film_view_key_for_mode(VirtualKeyCode::ArrowRight, ElementState::Pressed, true,));
    main_assert_eq!(app.film_view_player => Some(first), "the nonexclusive scoreboard does not acquire GUI key focus");

    app.start_running_chat(RunningChatMode::All);
    main_assert!(!app.handle_film_view_key_for_mode(VirtualKeyCode::ArrowRight, ElementState::Pressed, true,));
    main_assert_eq!(app.film_view_player => Some(first), "the exclusive chat owns GUI scope");
}

#[test]
fn set_film_view_builtin_reaches_the_real_replay_viewport() {
    let mut app = new_running_sandbox_app();
    let local_owner = app.local_owner;
    let film_player = local_owner + 1;
    let focus = app
        .snapshot
        .players
        .iter()
        .find(|player| player.id == local_owner)
        .and_then(|player| {
            player
                .viewports
                .first()
                .and_then(|viewport| viewport.focus)
                .or(player.cursor)
                .or_else(|| player.crew.first().copied())
        })
        .test_value();
    app.engine
        .register_player(PlayerConfig::new(film_player, "Film target"))
        .test_value();
    app.engine
        .replace_player_viewports(
            film_player,
            vec![clonk_engine::PlayerViewport::new(Vector2::new(700, 800))
                .with_focus(Some(focus))
                .with_zoom(2.0)],
        )
        .test_value();
    app.engine.clear_scenario_script();
    app.engine.install_scenario_script_with_convention("FilmView.c",
    &format!(
        "#strict 3\nfunc Probe() {{ SetViewOffset({local_owner}, 17, 19); return SetFilmView({film_player}); }}"
    ),
    true,).test_value();

    app.engine.set_replay_control(false);
    app.engine
        .call_scenario_script_function("Probe", Vec::new())
        .test_value();
    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0; app.graphics.surface().pixels().len()];
    app.render_running(&mut frame, false).test_value();
    main_assert_eq!(app.film_view_player => None);
    main_assert_eq!(app.graphics.active_viewport_projections()[0].owner => local_owner);

    app.engine.set_replay_control(true);
    app.engine
        .call_scenario_script_function("Probe", Vec::new())
        .test_value();
    app.snapshot = app.engine.snapshot();
    app.render_running(&mut frame, false).test_value();
    main_assert_eq!(app.film_view_player => Some(film_player));
    main_assert_eq!(app.graphics.active_viewport_projections()[0].owner => film_player);
    let inputs =
        collect_viewport_inputs_from_physical_state(&app.snapshot, &app.physical_viewports)
            .test_value();
    main_assert_eq!(inputs[0].offset => Vector2::new(17, 19));
}

fn assert_running_viewport_boundary(app: &mut GameApp, expected_reason: ClassicViewportBoundary) {
    app.snapshot.hud.messages.clear();
    app.graphics.surface_mut().fill(Color::opaque(91, 47, 13));
    let mut frame = vec![0x5a; app.graphics.surface().pixels().len()];
    let frame_before = frame.clone();
    let surface_before = app.graphics.surface().pixels().to_vec();
    let expected = ClassicParityBoundary::RunningViewport(expected_reason);

    let error = app
        .render_running(&mut frame, false)
        .expect_err("unsupported viewport state must fail closed");

    main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&expected));
    main_assert!(
        error.to_string().contains("solid navy")
            && error.to_string().contains("arbitrary first-object"),
        "boundary must name both rejected viewport substitutes: {error:#}"
    );
    main_assert_eq!(frame => frame_before, "caller frame must remain byte-identical");
    main_assert_eq!(app.graphics.surface().pixels() => surface_before.as_slice(), "graphics surface must remain byte-identical");
}

#[test]
fn every_physical_owned_viewport_requires_a_player_and_slot_before_any_pixels() {
    let mut app = new_running_sandbox_app();
    let local_owner = app.local_owner;
    let missing_owner = local_owner + 99;
    let missing_viewport = app.owned_physical_viewport_state(missing_owner, true);
    app.physical_viewports.push(missing_viewport);
    app.physical_viewports_authoritative = true;
    app.update_film_viewport_availability();

    // A valid first physical viewport must not make a mixed valid/invalid
    // physical list partially renderable.
    assert_running_viewport_boundary(
        &mut app,
        ClassicViewportBoundary::LocalViewportUnavailable {
            owner: missing_owner,
        },
    );

    app.physical_viewports
        .retain(|viewport| viewport.displayed_player != missing_owner);
    app.snapshot
        .players
        .iter_mut()
        .find(|player| player.id == local_owner)
        .test_value()
        .viewports
        .clear();
    assert_running_viewport_boundary(
        &mut app,
        ClassicViewportBoundary::LocalViewportUnavailable { owner: local_owner },
    );
}

#[test]
fn focusless_owned_slot_renders_in_normal_cursor_mode() {
    let mut app = new_running_sandbox_app();
    let local_owner = app.local_owner;
    let invalid_focus = ObjectId::new(u64::MAX);
    let valid_focus = app
        .snapshot
        .players
        .iter()
        .find(|player| player.id == local_owner)
        .and_then(|player| {
            player
                .viewports
                .first()
                .and_then(|viewport| viewport.focus)
                .or(player.cursor)
                .or_else(|| player.crew.first().copied())
        })
        .filter(|focus| app.snapshot.object(*focus).is_some())
        .test_value();
    let player = app
        .snapshot
        .players
        .iter_mut()
        .find(|player| player.id == local_owner)
        .test_value();
    player.viewports[0].focus = Some(valid_focus);
    player.cursor = None;
    player.crew.clear();
    player.viewports.push(
        clonk_engine::PlayerViewport::new(Vector2::new(900, 700))
            .with_focus(Some(invalid_focus))
            .with_zoom(1.25),
    );

    let inputs = collect_viewport_inputs(&app.snapshot).test_value();
    main_assert_eq!(inputs.len() => 2);
    main_assert_eq!(inputs[0].focus.map(|focus| focus.id) => Some(valid_focus));
    main_assert!(inputs[1].focus.is_none());
    main_assert_eq!(inputs[1].owner => local_owner);
    main_assert_eq!(inputs[1].center => Vector2::new(900, 700));
    main_assert_eq!(inputs[1].zoom => 1.25);

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.render_running(&mut frame, false).test_value();
    main_assert_eq!(app.graphics.active_viewport_projections().len() => 2);
}

#[test]
fn close_effect_uses_displayed_film_owners_not_local_assignments() {
    main_assert_eq!(
        physical_viewport_close_effect(&[10, 11], Some(11), 11) =>
        saves_fixture!(viewport_close: true, 0),
        "retargeting the primary viewport can make every viewport match"
    );
    main_assert_eq!(
        physical_viewport_close_effect(&[10], Some(11), 10) =>
        saves_fixture!(viewport_close: false, 1),
        "the local owner no longer owns its retargeted physical viewport"
    );
    main_assert_eq!(
        physical_viewport_close_effect(&[], Some(11), 11) =>
        saves_fixture!(viewport_close: true, 0),
        "film can retarget the sole ownerless physical viewport"
    );
}

#[test]
fn view_offset_and_film_view_share_one_physical_request_order() {
    for (offset_after_film_view, expected_offset) in
        [(true, Vector2::new(41, 43)), (false, Vector2::ZERO)]
    {
        let mut app = new_lightweight_running_sandbox_app();
        let target = app.local_owner + 1;
        let body = if offset_after_film_view {
            format!("SetFilmView({target}); SetViewOffset({target}, 41, 43);")
        } else {
            format!("SetViewOffset({target}, 41, 43); SetFilmView({target});")
        };
        app.engine
            .register_player(PlayerConfig::new(target, "Remote film target"))
            .test_value();
        app.engine.clear_scenario_script();
        app.engine
            .install_scenario_script_with_convention(
                "ViewportRequestOrder.c",
                &format!("#strict 3\nfunc Probe() {{ {body} }}"),
                true,
            )
            .test_value();
        app.engine.set_replay_control(true);
        app.set_runtime_flash_message("Film Init clears me", RuntimeHelpCharset::Windows1252)
            .test_value();

        app.engine
            .call_scenario_script_function("Probe", Vec::new())
            .test_value();
        let _ = app.apply_pending_viewport_presentation_requests();

        main_assert_eq!(app.physical_viewports[0].displayed_player => target);
        main_assert_eq!(app.physical_viewports[0].preserved_offset => expected_offset);
        main_assert!(app.runtime_flash_message.is_none(), "valid temporary C4Viewport::Init clears the flash");
    }
}

#[test]
fn film_assigned_ownerless_offset_is_consumed_after_one_draw() {
    let mut app = new_lightweight_running_sandbox_app();
    let target = app.local_owner;
    app.local_controls = LocalControlRegistry::default();
    app.engine.set_local_players([]);
    app.refresh_non_authoritative_physical_viewports();
    main_assert!(app.physical_viewports[0].is_no_owner_viewport);
    app.engine.clear_scenario_script();
    app.engine
        .install_scenario_script_with_convention(
            "OwnerlessViewportOffset.c",
            &format!(
        "#strict 3\nfunc Probe() {{ SetFilmView({target}); SetViewOffset({target}, 13, 17); }}"
    ),
            true,
        )
        .test_value();
    app.engine.set_replay_control(true);
    app.engine
        .call_scenario_script_function("Probe", Vec::new())
        .test_value();
    let _ = app.apply_pending_viewport_presentation_requests();
    main_assert_eq!(app.physical_viewports[0].preserved_offset => Vector2::new(13, 17));

    app.snapshot = app.engine.snapshot();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    main_assert_eq!(app.physical_viewports[0].preserved_offset => Vector2::ZERO);
    main_assert!(app.physical_viewports[0].is_no_owner_viewport);
}

#[test]
fn recalculation_does_not_reapply_a_stale_scalar_film_target() {
    let mut app = new_lightweight_running_sandbox_app();
    let lower_layout = app.local_owner + 1;
    let high_layout_target = app.local_owner + 2;
    let temporary = app.local_owner + 3;
    for (player, name, control_set) in [
        (lower_layout, "Lower layout", 2),
        (high_layout_target, "High layout film", 1),
        (temporary, "Middle layout", 3),
    ] {
        app.engine
            .register_player(PlayerConfig::new(player, name))
            .test_value();
        app.engine
            .set_player_runtime_control(
                player,
                clonk_engine::PlayerRuntimeControl::new(control_set, 0),
            )
            .test_value();
    }
    main_assert!(app.create_physical_viewport(lower_layout, true, true, false));
    main_assert!(app.set_physical_film_view(high_layout_target));
    main_assert_eq!(app.physical_viewports.iter().map(|viewport| viewport.displayed_player).collect::<Vec<_>>() => vec![high_layout_target, lower_layout]);

    main_assert!(app.create_physical_viewport(temporary, true, true, false));
    main_assert!(app.close_physical_viewports(temporary, true, true));
    main_assert_eq!(
        app.physical_viewports
            .iter()
            .map(|viewport| viewport.displayed_player)
            .collect::<Vec<_>>() =>
        vec![lower_layout, high_layout_target],
        "RecalculateViewports may sort the retargeted physical viewport away from index zero"
    );

    app.sync_film_view_presentation();
    main_assert_eq!(
        app.physical_viewports
            .iter()
            .map(|viewport| viewport.displayed_player)
            .collect::<Vec<_>>() =>
        vec![lower_layout, high_layout_target],
        "the compatibility scalar must not execute SetFilmView a second time"
    );
}

#[test]
fn remote_film_close_does_not_resurrect_the_original_primary() {
    let mut app = new_lightweight_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = primary + 1;
    let remote = primary + 2;
    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary local"))
        .test_value();
    let control = app
        .local_controls
        .initialize(test_local_control_init(secondary, 1, false, false));
    app.engine
        .set_player_runtime_control(secondary, control.runtime_control())
        .test_value();
    app.engine.set_local_players([primary, secondary]);
    app.engine
        .register_player(PlayerConfig::new(remote, "Remote film target"))
        .test_value();
    let _ = app.create_physical_viewport(secondary, false, true, true);
    main_assert!(app.set_physical_film_view(remote));
    app.ui_sound_log.clear();

    app.remove_runtime_player_with_viewport_feedback(remote)
        .test_value();

    main_assert_eq!(
        app.physical_viewports
            .iter()
            .map(|viewport| viewport.displayed_player)
            .collect::<Vec<_>>() =>
        vec![secondary],
        "the erased primary must not be regenerated from local controls"
    );
    main_assert_eq!(app.ui_sound_log.iter().filter(|sound| sound.as_str() == "CloseViewport").count() => 1);
}

#[test]
fn replay_film_startup_and_late_player_follow_viewport_check() {
    let mut app = new_lightweight_running_sandbox_app();
    let first = app.local_owner;
    let mut state = app.engine.capture_state();
    let mut scenario_values = serde_json::to_value(
        state
            .scenario_values
            .take()
            .expect("captured scenario values"),
    )
    .test_value();
    for (name, value) in [("Replay", 1), ("Film", 1)] {
        let target = scenario_values["sections"]
            .as_array_mut()
            .and_then(|sections| {
                sections
                    .iter_mut()
                    .find(|section| section["name"] == "Head")
            })
            .and_then(|head| head["entries"].as_array_mut())
            .and_then(|entries| entries.iter_mut().find(|entry| entry["name"] == name))
            .and_then(|entry| entry["values"].as_array_mut())
            .and_then(|values| values.first_mut())
            .test_value();
        *target = serde_json::json!({ "Int": value });
    }
    state.scenario_values = Some(serde_json::from_value(scenario_values).test_value());
    app.engine.restore_state(&state).test_value();
    app.engine.set_replay_control(true);
    app.local_controls = LocalControlRegistry::default();
    app.engine.set_local_players([]);
    app.snapshot = app.engine.snapshot();
    app.ui_sound_log.clear();

    app.initialize_physical_viewports(false);
    main_assert_eq!(app.physical_viewports.len() => 1);
    main_assert_eq!(app.physical_viewports[0].displayed_player => first);
    main_assert!(!app.physical_viewports[0].is_no_owner_viewport);
    main_assert_eq!(
        app.ui_sound_log
            .iter()
            .filter(|sound| sound.as_str() == "CloseViewport")
            .count() =>
        1,
        "zero-view replay film creates the first-player viewport non-silently"
    );

    app.engine.remove_player(first).test_value();
    app.snapshot = app.engine.snapshot();
    app.ui_sound_log.clear();
    app.initialize_physical_viewports(false);
    main_assert!(app.physical_viewports[0].is_no_owner_viewport);
    main_assert!(app.ui_sound_log.is_empty());

    app.film_view_player = Some(first);
    app.sync_film_view_presentation();
    main_assert_eq!(app.film_view_player => None, "stale scalar target is ignored");
    main_assert_eq!(app.physical_viewports[0].displayed_player => OWNER_NONE);

    app.engine
        .register_player(PlayerConfig::new(first, "Late replay player"))
        .test_value();
    app.snapshot = app.engine.snapshot();
    app.set_runtime_flash_message(
        "Late replay Init clears me",
        RuntimeHelpCharset::Windows1252,
    )
    .test_value();
    app.check_fullscreen_physical_viewports(false);
    main_assert_eq!(app.physical_viewports[0].displayed_player => first);
    main_assert!(app.physical_viewports[0].is_no_owner_viewport, "late film retarget preserves the ownerless classification");
    main_assert!(app.runtime_flash_message.is_none());
    main_assert!(app.ui_sound_log.is_empty(), "ownerless retarget is silent");
}

#[test]
fn removing_a_remote_film_target_closes_its_physical_viewport_once() {
    let mut app = new_lightweight_running_sandbox_app();
    let film_player = app.local_owner + 1;
    app.engine
        .register_player(PlayerConfig::new(film_player, "Film target"))
        .test_value();
    app.film_view_player = Some(film_player);
    app.snapshot = app.engine.snapshot();
    app.ui_sound_log.clear();

    app.remove_runtime_player_with_viewport_feedback(film_player)
        .test_value();

    main_assert_eq!(app.film_view_player => None);
    main_assert_eq!(app.ui_sound_log.iter().filter(|sound| sound.as_str() == "CloseViewport").count() => 1, "the retargeted physical viewport closes once");
}

#[test]
fn film_target_removal_recreates_the_first_player_viewport() {
    let mut app = new_lightweight_running_sandbox_app();
    let local_player = app.local_owner;
    let film_player = local_player + 1;
    app.engine
        .register_player(PlayerConfig::new(film_player, "Film target"))
        .test_value();
    let film_control =
        app.local_controls
            .initialize(test_local_control_init(film_player, 1, false, false));
    app.engine
        .set_player_runtime_control(film_player, film_control.runtime_control())
        .test_value();
    app.engine.set_local_players([local_player, film_player]);

    let mut state = app.engine.capture_state();
    let mut scenario_values = serde_json::to_value(
        state
            .scenario_values
            .take()
            .expect("captured scenario values"),
    )
    .test_value();
    for name in ["Replay", "Film"] {
        let value = scenario_values["sections"]
            .as_array_mut()
            .and_then(|sections| {
                sections
                    .iter_mut()
                    .find(|section| section["name"] == "Head")
            })
            .and_then(|head| head["entries"].as_array_mut())
            .and_then(|entries| entries.iter_mut().find(|entry| entry["name"] == name))
            .and_then(|entry| entry["values"].as_array_mut())
            .and_then(|values| values.first_mut())
            .test_value();
        *value = serde_json::json!({ "Int": 1 });
    }
    state.scenario_values = Some(serde_json::from_value(scenario_values).test_value());
    app.engine.restore_state(&state).test_value();
    app.engine.set_replay_control(true);
    app.film_view_player = Some(film_player);
    app.snapshot = app.engine.snapshot();
    app.ui_sound_log.clear();

    let physical_owners = app.live_local_viewport_owners_with_primary_first();
    main_assert_eq!(physical_owners => [local_player, film_player]);
    main_assert_eq!(
        physical_viewport_close_effect(&physical_owners, app.film_view_player, film_player,) =>
        saves_fixture!(viewport_close: true, 0),
        "SetFilmView retargets the first physical viewport, producing two matching targets"
    );

    app.remove_runtime_player_with_viewport_feedback(film_player)
        .test_value();

    main_assert_eq!(app.film_view_player => Some(local_player));
    main_assert_eq!(
        app.ui_sound_log
            .iter()
            .filter(|sound| sound.as_str() == "CloseViewport")
            .count() =>
        2,
        "film mode closes the old viewport and creates its replacement"
    );
}

#[test]
fn saved_game_rxmusic_reenables_music_but_not_transient_flash() {
    let mut app = new_running_sandbox_app();
    let scenario = app
        .active_scenario
        .clone()
        .unwrap_or_else(FrontendScenario::fallback);
    let save = saves_fixture!(
        saved_game:
            SAVE_FILE_VERSION,
            SavedScenarioInfo::from_frontend(
                        &scenario,
                        &app.scenario_label,
                        app.fallback_ground,
                    ),
            app.active_definition_load.clone(),
            app.focus_id,
            Some("runtime music state".to_string()),
            Some(false),
            app.engine.capture_state(),
    );
    app.audio.test_mut().options.music_enabled = true;
    app.runtime_music_enabled = true;
    app.set_runtime_flash_message("not serialized", RuntimeHelpCharset::Windows1252)
        .test_value();

    app.apply_loaded_game(save).test_value();

    main_assert!(app.audio.as_ref().expect("test audio").options.music_enabled, "RXMusic remains an independent configured option");
    main_assert!(app.runtime_music_enabled, "RXMusic force-enables resume");
    main_assert!(app.audio.as_ref().expect("test audio").music_is_playing());
    main_assert!(app.runtime_flash_message.is_none());
}

#[test]
fn saved_game_control_values_are_overwritten_by_current_local_assignment() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let scenario = app
        .active_scenario
        .clone()
        .unwrap_or_else(FrontendScenario::fallback);
    let mut engine_state = app.engine.capture_state();
    let saved_player = engine_state
        .players
        .iter_mut()
        .find(|player| player.id == owner)
        .test_value();
    saved_player.at_client = clonk_engine::PlayerAtClient::new(77);
    saved_player.at_client_name = Some("stale client".to_string());
    saved_player.control_set = 7;
    saved_player.mouse_control = 0;
    saved_player.control.control_style = true;
    saved_player.control.auto_context_menu = true;
    saved_player.control.last_com = i32::from(clonk_engine::COM_RIGHT);
    saved_player.control.last_com_down_double = 2;
    saved_player.control.pressed_coms = 0x3ff;
    saved_player.message_status = 2;
    saved_player.view_wealth = 5;
    saved_player.no_elimination_check = true;
    let save = saves_fixture!(
        saved_game:
            SAVE_FILE_VERSION,
            SavedScenarioInfo::from_frontend(
                        &scenario,
                        &app.scenario_label,
                        app.fallback_ground,
                    ),
            app.active_definition_load.clone(),
            app.focus_id,
            Some("local control restore".to_string()),
            Some(app.runtime_music_enabled),
            engine_state,
    );

    app.apply_loaded_game(save).test_value();

    let player = app.engine.test_player(owner);
    main_assert_eq!((player.control_set(), player.mouse_control()) => (0, 1));
    main_assert_eq!(player.at_client() => clonk_engine::PlayerAtClient::HOST);
    main_assert_eq!(player.at_client_name() => "Local");
    main_assert!(!player.control.control_style);
    main_assert!(!player.control.auto_context_menu);
    main_assert_eq!(player.control.last_com => 0);
    main_assert_eq!(player.control.last_com_down_double => 1);
    main_assert_eq!(player.control.pressed_coms => 0);
    main_assert_eq!(player.message_status() => 1);
    main_assert_eq!(player.view_wealth() => 4);
    main_assert!(player.no_elimination_check());
    main_assert_eq!(app.local_controls.mouse_owner() => Some(owner));
    main_assert!(app.mouse_control);
}

#[test]
fn saved_game_skips_removed_current_player_without_deleting_objects() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let info_id = app.engine.test_player(owner).player_info_id();
    let object = app.engine.snapshot().objects.first().test_value().id;
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData::new(
            0,
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: info_id,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                    | clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                ..Default::default()
            }],
            -1,
        ));
    let scenario = app
        .active_scenario
        .clone()
        .unwrap_or_else(FrontendScenario::fallback);
    let save = saves_fixture!(
        saved_game:
            SAVE_FILE_VERSION,
            SavedScenarioInfo::from_frontend(
                        &scenario,
                        &app.scenario_label,
                        app.fallback_ground,
                    ),
            app.active_definition_load.clone(),
            app.focus_id,
            Some("removed player skipped".to_string()),
            Some(app.runtime_music_enabled),
            app.engine.capture_state(),
    );

    app.apply_loaded_game(save).test_value();

    main_assert!(app.engine.player(owner).is_none());
    let object = app.engine.test_object_snapshot(object);
    main_assert_eq!(object.owner => clonk_engine::OWNER_NONE);
}

#[test]
fn always_debug_survives_rust_saves_and_rearms_restored_rounds() {
    for (config, expected) in [
        (&b""[..], false),
        (&b"[General]\nDebugMode=true\n"[..], true),
        (&b"[General]\nDebugMode=1\n"[..], true),
        (&b"[General]\nDebugMode= true\n"[..], false),
        (&b"[General]\nDebugMode=invalid\n"[..], false),
    ] {
        let mut engine = Engine::new();
        arm_graphical_engine_debug_mode(&mut engine, config);
        main_assert_eq!(engine.debug_mode() => expected);
    }

    let enabled_config = b"[General]\nDebugMode=true\n";
    let mut allowed = Engine::new();
    arm_graphical_engine_debug_mode(&mut allowed, enabled_config);
    let state = allowed.capture_state();
    let mut restored = Engine::new();
    restored.restore_state(&state).test_value();
    main_assert!(!restored.debug_mode(), "DebugMode is not serialized");
    arm_graphical_engine_debug_mode(&mut restored, enabled_config);
    main_assert!(restored.debug_mode(), "round restore reapplies AlwaysDebug");
    let mut denied = Engine::new();
    denied.set_allow_debug(false);
    arm_graphical_engine_debug_mode(&mut denied, enabled_config);
    main_assert!(!denied.debug_mode());
    let mut disabled = Engine::new();
    arm_graphical_engine_debug_mode(&mut disabled, b"");
    main_assert!(!disabled.debug_mode());

    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_native_config_values(
        &paths,
        "General",
        &[(
            "DebugMode",
            clonk_app_netplay::NativeConfigValue::RawAscii("true"),
        )],
    )
    .test_value();

    persist_config_value(&paths, "Network", "Comment", "preserve debug").test_value();
    let saved = fs::read(paths.config_file()).test_value();
    main_assert_eq!(clonk_app_netplay::configured_native_boolean(&saved, "General", "DebugMode") => Some(true));
    let mut next_round = Engine::new();
    arm_configured_graphical_engine_debug_mode(&mut next_round, Some(&paths));
    main_assert!(next_round.debug_mode());
    reset_cached_app_paths();
}

#[test]
fn savegame_slot_path_uses_configured_folder_and_scenname_scheme() {
    let fixture = tempdir();
    let user_data = fixture.path().join("user-data");
    let configured_folder = fixture.path().join("Configured Saves.c4f");
    let (_guard, paths) = exact_loader_test_paths(&user_data, None);
    persist_config_value(
        &paths,
        "General",
        "SaveGameFolder",
        configured_folder.to_string_lossy().into_owned(),
    )
    .test_value();

    let mut app = new_state_only_lightweight_running_sandbox_app();
    app.app_paths = Some(paths.clone());

    let localized_title = "  Höhlenübung: Zurück  ";
    let mut old_style = FrontendScenario::fallback();
    old_style.path = Some(paths.install_root().join("planet/Missions.c4f/01.c4s"));
    old_style.identifier = "StaleAlias.c4f/Wrong999.c4s".to_string();
    old_style.title = localized_title.to_string();
    app.active_scenario = Some(old_style.clone());

    let old_slot = configured_folder.join("Missions.c4f").join("Missions1.c4s");
    main_assert_eq!(app.savegame_slot_path(1) => old_slot);
    main_assert_eq!(app.savegame_slot_path(10) => configured_folder.join("Missions.c4f").join("Missions10.c4s"));

    let mut new_style = old_style;
    new_style.identifier = "AnotherAlias.c4f/AlsoWrong.c4s".to_string();
    new_style.path = Some(
        paths
            .install_root()
            .join("planet/Tutorial.c4f/Tutorial007.c4s"),
    );
    app.active_scenario = Some(new_style);
    main_assert_eq!(app.savegame_slot_path(10) => configured_folder.join("Tutorial.c4f").join("Tutorial10.c4s"));

    let mut loose_numeric = FrontendScenario::fallback();
    loose_numeric.identifier = "Loose/01.c4s".to_string();
    loose_numeric.path = Some(paths.install_root().join("planet/Loose/01.c4s"));
    app.active_scenario = Some(loose_numeric);
    main_assert_eq!(app.savegame_slot_path(1) => configured_folder.join("01.c4f").join("011.c4s"), "a regular directory is not Game.pParentGroup");

    persist_config_value(&paths, "General", "SaveGameFolder", "Relative Saves.c4f").test_value();
    main_assert_eq!(configured_savegame_directory(Some(&paths)) => paths.install_root().join("Relative Saves.c4f"));
    persist_config_value(
        &paths,
        "General",
        "SaveGameFolder",
        configured_folder.to_string_lossy().into_owned(),
    )
    .test_value();

    main_assert!(looks_like_cpp_integer("+999999999999999999999999"));
    main_assert!(looks_like_cpp_integer("-01"));
}

#[test]
fn configured_native_savegames_folder_is_browsable_and_selects_a_resume() {
    // C4StartupScenSelDlg::OnShown loads ExePath and exposes the configured
    // Savegames.c4f as a SubFolder whose activation loads its saved .c4s
    // children (C4StartupScenSelDlg.cpp:948-958, 1431-1439, 1669-1678).
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let fixture = tempdir();
    let user_data = fixture.path().join("user-data");
    let save_root = fixture.path().join("External Savegames.c4f");
    let scenario_saves = save_root.join("Missions.c4f");
    let saved_scenario = scenario_saves.join("Missions1.c4s");
    let (_guard, paths) = exact_loader_test_paths(&user_data, None);
    persist_config_value(
        &paths,
        "General",
        "SaveGameFolder",
        save_root.to_string_lossy().into_owned(),
    )
    .test_value();

    fs::create_dir_all(&scenario_saves).test_value();
    fs::write(save_root.join("Title.txt"), b"US:Savegames").test_value();
    fs::write(scenario_saves.join("Title.txt"), b"US:Cave mission saves").test_value();
    let mut saved = MutableGroup::new("Missions1.c4s");
    saved
            .add_file(
                "Scenario.txt",
                b"[Head]\nTitle=Resume cave mission\nSaveGame=1\nNoInitialize=1\nMinPlayer=0\nMaxPlayer=0\n"
                    .to_vec(),
            ).test_value();
    saved
        .add_file("Game.txt", b"[Game]\nFrame=37\n".to_vec())
        .test_value();
    fs::write(&saved_scenario, saved.pack().test_value()).test_value();

    let entries = load_frontend_scenarios_from_paths(&paths);
    let savegames = entries
        .iter()
        .find(|entry| entry.path.as_deref() == Some(save_root.as_path()))
        .test_value();
    main_assert_eq!(savegames.title => "Savegames");
    main_assert_eq!(savegames.kind => ScenarioKind::Folder);
    main_assert_eq!(scensel_entry_icon(savegames) => 0, "a .c4f save playlist uses the classic yellow folder phase");
    let mission = savegames
        .children
        .iter()
        .find(|entry| entry.path.as_deref() == Some(scenario_saves.as_path()))
        .test_value();
    let resume = mission
        .children
        .iter()
        .find(|entry| entry.path.as_deref() == Some(saved_scenario.as_path()))
        .test_value();
    main_assert_eq!(resume.title => "Resume cave mission");
    main_assert_eq!(resume.kind => ScenarioKind::Scenario);

    let player_root = fixture.path().join("Players");
    fs::create_dir(&player_root).test_value();
    configure_test_startup_participant(&paths, &player_root);
    let menu =
        StartupMenu::new(build_menu_entries(&entries, false), test_font(), None).test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.menu_state = MenuState::new(menu, entries.clone());
    app.scenario_catalog = build_scenario_catalog(&entries);
    app.open_scenario_browser();
    let summary = |entry: &FrontendScenario| saves_fixture!(scenario: entry.identifier.clone(), entry.title.clone(), entry.kind);
    app.process_menu_actions(vec![StartupMenuAction::OpenEntry(summary(savegames))])
        .test_value();
    app.process_menu_actions(vec![StartupMenuAction::OpenEntry(summary(mission))])
        .test_value();
    let (selected, _) = app
        .process_menu_actions(vec![StartupMenuAction::StartScenario(summary(resume))])
        .test_value();
    let selected = selected.test_value();
    main_assert_eq!(
        app.scenario_catalog
            .get(&selected)
            .and_then(|entry| entry.path.as_deref()) =>
        Some(saved_scenario.as_path()),
        "the selected row resumes from the stored native C4Group"
    );

    app.handle_menu_actions(vec![StartupMenuAction::StartScenario(summary(resume))])
        .test_value();
    main_assert_eq!(app.mode => AppMode::Loading);
    main_assert_eq!(app.loading_state.as_ref().and_then(|loading| loading.scenario.path.as_deref()) => Some(saved_scenario.as_path()));
    reset_cached_app_paths();
}

#[test]
fn savegame_slot_probe_uses_c4group_validity() {
    let fixture = tempdir();
    let user_data = fixture.path().join("user-data");
    let save_root = fixture.path().join("Savegames.c4f");
    let (_guard, paths) = exact_loader_test_paths(&user_data, None);
    persist_config_value(
        &paths,
        "General",
        "SaveGameFolder",
        save_root.to_string_lossy().into_owned(),
    )
    .test_value();

    let mut app = new_state_only_lightweight_running_sandbox_app();
    app.app_paths = Some(paths);
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "Probe.c4s".to_string();
    scenario.path = Some(fixture.path().join("Probe.c4s"));
    app.active_scenario = Some(scenario);
    app.network_is_league = true;
    main_assert!(app.can_quick_save(), "offline saves ignore retained league state");

    let slot_root = save_root.join("Probe.c4f");
    fs::create_dir_all(&slot_root).test_value();
    fs::write(slot_root.join("Probe1.c4s"), b"{\"not\":\"a group\"}").test_value();
    let mut packed = MutableGroup::new("Probe2.c4s");
    packed
        .add_file("Scenario.txt", b"[Head]\nTitle=Packed\n".to_vec())
        .test_value();
    fs::write(slot_root.join("Probe2.c4s"), packed.pack().test_value()).test_value();
    fs::create_dir(slot_root.join("Probe3.c4s")).test_value();
    fs::write(slot_root.join("Probe4.c4s"), [0x1f, 0x8b, 0x08]).test_value();

    let slots = app.savegame_slots();
    main_assert!(slots[0].free, "plain files are not occupied C4Groups");
    main_assert!(!slots[1].free, "packed C4Groups occupy slots");
    main_assert!(!slots[2].free, "folder C4Groups occupy slots");
    main_assert!(slots[3].free, "malformed packed files remain free");
    main_assert!(slots[4..].iter().all(|slot| slot.free));
}

#[test]
fn save_demo_folder_controls_recording_directory() {
    let fixture = tempdir();
    let user_data = fixture.path().join("user-data");
    let absolute_records = fixture.path().join("Absolute Records.c4f");
    let (_guard, paths) = exact_loader_test_paths(&user_data, None);

    main_assert_eq!(paths.recordings_dir() => paths.install_root().join("Records.c4f"));

    persist_config_value(&paths, "General", "SaveDemoFolder", "Relative Records.c4f").test_value();
    let relative_records = paths.install_root().join("Relative Records.c4f");
    main_assert_eq!(paths.recordings_dir() => relative_records);

    let app = GameApp::new(
        320,
        200,
        AudioOptions::default(),
        Some(&paths),
        test_runtime_config_with("Player", true),
    )
    .test_value();
    main_assert_eq!(app.recordings_dir.as_deref() => Some(relative_records.as_path()));

    persist_config_value(
        &paths,
        "General",
        "SaveDemoFolder",
        absolute_records.to_string_lossy().into_owned(),
    )
    .test_value();
    main_assert_eq!(paths.recordings_dir() => absolute_records);
}

#[test]
fn game_app_uses_selected_user_root_for_scenario_discovery() {
    let fixture = tempdir();
    let selected_user = fixture.path().join("selected-user");
    let ambient_user = fixture.path().join("ambient-user");
    let config_file = fixture.path().join("selected.config");
    let repository = test_repository_root();
    fs::write(
        &config_file,
        format!(
            "[General]\nUserPath=\"{}\"\nLanguageEx=US\n\n[Network]\nLocalName=Exact Host\n",
            selected_user.display()
        ),
    )
    .test_value();
    let _selected_guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(repository)),
        ("LC_CONTENT_DIR", None),
        ("LC_USER_DATA_DIR", None),
        ("LC_CONFIG_FILE", None),
    ]);
    let paths = AppPaths::discover_with_config_file(Some(&config_file)).test_value();
    paths.ensure_user_dirs().test_value();
    let scenario = paths.scenario_dir().join("L016Configured.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(
        scenario.join("Scenario.json"),
        br#"{"name":"Selected User Scenario"}"#,
    )
    .test_value();

    // Make a fresh ambient discovery disagree with the already-selected
    // paths. The constructor must pass its AppPaths into the worker.
    let _ambient_guard = EnvGuard::set(&[("LC_USER_DATA_DIR", Some(&ambient_user))]);
    let app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();

    main_assert_eq!(paths.user_data_dir() => selected_user);
    main_assert_eq!(paths.config_file() => config_file);
    main_assert_eq!(app.scenario_catalog.get("L016Configured.c4s").map(|scenario| scenario.title.as_str()) => Some("Selected User Scenario"));
}

#[test]
fn quick_save_round_trips_state() {
    clonk_logging::init();

    reset_cached_app_paths();
    {
        let _guard = EnvGuard::set(&[]);
        reset_cached_app_paths();

        let mut app = test_game_app(320, 200, AudioOptions::default(), None).test_value();
        install_classic_test_assets(&mut app);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .test_value();

        for _ in 0..5 {
            app.test_update();
        }
        main_assert_eq!(app.snapshot.game_time => 0, "headless frame updates do not synthesize real-time pulses");
        main_assert!(app.sec1_timer().expect("explicit saved-game clock pulse"), "explicit pulse consumes the tick latch");
        let saved_frame = app.snapshot.frame;
        let saved_game_time = app.snapshot.game_time;
        let saved_player_info_id = app.engine.test_player(app.local_owner).player_info_id();
        let saved_big_icon = ImageData::new(1, 1, vec![12, 34, 56, 255]);
        app.runtime_player_big_icons
            .insert(saved_player_info_id, saved_big_icon.clone());

        app.quick_save().test_value();
        main_assert!(app.last_save_path.as_ref().map(|path| path.ends_with(QUICK_SAVE_FILE)).unwrap_or(false), "quick save should note the save path");
        let thumbnail_path = resolve_save_directory().join("quicksave.png");
        main_assert!(thumbnail_path.exists(), "expected quick save thumbnail to be written");

        for _ in 0..3 {
            app.test_update();
        }
        main_assert!(app.sec1_timer().expect("later saved-game clock pulse"), "later pulse advances Game.Time");
        main_assert!(app.snapshot.frame > saved_frame, "frame should advance after save");
        main_assert!(app.snapshot.game_time > saved_game_time);

        app.quick_load().test_value();
        main_assert_eq!(app.snapshot.frame => saved_frame, "quick load should restore saved frame");
        main_assert_eq!(app.snapshot.game_time => saved_game_time, "quick load should restore Game.Time");
        main_assert_eq!(app.game_time_seconds() => saved_game_time.max(0) as u64);
        main_assert_eq!(
            app.runtime_player_big_icons.get(&saved_player_info_id) =>
            Some(&saved_big_icon),
            "in-round restore keeps C4Player::BigIcon by stable player-info ID"
        );
        main_assert!(matches!(app.mode, AppMode::Running), "quick load should keep the game running");

        cleanup_quicksave_file();
    }
    reset_cached_app_paths();
}

#[test]
fn quick_save_persists_across_sessions() {
    clonk_logging::init();

    reset_cached_app_paths();

    let fixture = tempdir();
    let user_dir = fixture.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
    let scripts_dir = scenario_dir.join("scripts");
    fs::create_dir_all(&scripts_dir).test_value();
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Alpha Mission\n",
    )
    .test_value();
    fs::write(
        scenario_dir.join("Scenario.json"),
        r#"
            {
                "name": "Alpha Mission",
                "ground_height": 72,
                "landscape": { "kind": "flat", "width": 160, "height": 80 },
                "definitions": [
                    { "id": "Mover", "name": "Mover", "script": "scripts/mover.aul" }
                ],
                "initial_objects": [
                    {
                        "definition": "Mover",
                        "position": [40, 48],
                        "owner": 1,
                        "crew_member": true,
                        "alive": true
                    }
                ]
            }
            "#,
    )
    .test_value();
    fs::write(scripts_dir.join("mover.aul"), walker_script()).test_value();

    let quicksave_path = user_dir.join(SAVE_DIR_NAME).join(QUICK_SAVE_FILE);

    {
        let (_guard, paths) = exact_loader_test_paths(&user_dir, None);

        reset_cached_app_paths();

        let saved_frame = {
            let mut app =
                test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();

            let scenario = app.scenario_catalog.get("Alpha.c4s").cloned().test_value();
            app.start_scenario(scenario).test_value();
            wait_for_running(&mut app);

            for _ in 0..5 {
                app.test_update();
            }
            let frame_before_save = app.snapshot.frame;

            app.quick_save().test_value();
            main_assert!(quicksave_path.exists(), "expected quick save file to be written");
            main_assert!(quicksave_path.with_extension("png").exists(), "expected quick save thumbnail to be written");

            frame_before_save
        };

        {
            let mut app =
                test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();

            // Boot loading is asynchronous; let it settle to the menu before
            // asserting the fresh session is at the menu (not mid-game).
            wait_for_menu(&mut app);

            main_assert!(app.last_save_path.as_ref().map(|path| path.ends_with(QUICK_SAVE_FILE)).unwrap_or(false), "expected quick save path to be remembered");
            main_assert!(matches!(app.mode, AppMode::Menu), "new session should start in menu");

            app.quick_load().test_value();

            main_assert!(matches!(app.mode, AppMode::Running), "quick load should enter running mode");
            main_assert_eq!(app.snapshot.frame => saved_frame, "quick load should restore the saved frame");
            main_assert!(
                app.active_scenario
                    .as_ref()
                    .and_then(|scenario| scenario.path.as_ref())
                    .map(|path| path.ends_with("Alpha.c4s"))
                    .unwrap_or(false),
                "loaded scenario should reference disk path"
            );
        }

        reset_cached_app_paths();
    }
    reset_cached_app_paths();
}

/// `RestoreSavegameInfos` logs `IDS_MSG_PLAYERASSIGNMENT` for every wild match
/// and, with a GUI in fullscreen outside a replay, shows one modal per
/// assignment captioned `IDS_MSG_FREESAVEGAMEPLRS`; its checkbox persists
/// `Config.Startup.HideMsgPlrTakeOver` without changing any assignment
/// (C4PlayerInfo.cpp:1383-1391; C4Config.cpp:1514).
#[test]
fn offline_wild_takeover_logs_and_presents_hideable_warning() {
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    fs::write(install.path().join("planet/System.c4g/LanguageUS.txt"), b"IDS_MSG_PLAYERASSIGNMENT=Participant %s will continue for player %s from the savegame.\n          IDS_MSG_FREESAVEGAMEPLRS=Player assignment\n          IDS_MSG_DONTSHOW=&Don't display this message in the future.\n").test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();

    let takeovers = vec![
        crate::offline_savegame::OfflineWildTakeover {
            participant: b"Carol".to_vec(),
            savegame_player: b"Ghost".to_vec(),
        },
        crate::offline_savegame::OfflineWildTakeover {
            participant: b"Dave".to_vec(),
            savegame_player: b"Stranger".to_vec(),
        },
    ];

    // One modal per wild assignment, in order, with the native caption.
    let mut app = new_menu_app(320, 200);
    app.app_paths = Some(paths.clone());
    app.startup_tooltip_resources = load_runtime_language_table(Some(&paths))
        .map(|table| table.entries)
        .unwrap_or_default();
    app.report_offline_wild_takeovers(&takeovers).test_value();
    main_assert_eq!(app.message_dialogs.len() => 2);
    main_assert_eq!(app.message_dialogs[0].state.message() => "Participant Carol will continue for player Ghost from the savegame.");
    main_assert_eq!(app.message_dialogs[0].state.caption() => "Player assignment");
    main_assert_eq!(app.message_dialogs[1].state.message() => "Participant Dave will continue for player Stranger from the savegame.");

    // The checkbox persists Startup.HideMsgPlrTakeOver.
    // The `&Don't display...` mnemonic is how the native checkbox is toggled.
    main_assert_eq!(app.message_dialogs[0].state.checkbox_checked() => Some(false));
    app.message_dialogs[0].state.handle_hotkey('D');
    main_assert_eq!(app.message_dialogs[0].state.checkbox_checked() => Some(true));
    app.persist_message_dialog_checkbox_changes(0);
    // `ShowMessageModal` writes the `HideMsg*` flag through its by-pointer
    // argument and no call site saves (C4ChatDlg.cpp:624; no `Config.Save()` in
    // C4Gui.cpp/C4GuiDialogs.cpp), so the file is written at the next save
    // surface. This test's subject is the stored value, so it flushes here.
    app.flush_deferred_config();
    let config = Config::load(paths.config_file()).test_value();
    main_assert_eq!(config.get_in(Some("Startup"), "HideMsgPlrTakeOver") => Some("1"));

    // With the preference set, later sessions log but show nothing.
    let mut hidden = new_menu_app(320, 200);
    hidden.app_paths = Some(paths.clone());
    hidden
        .report_offline_wild_takeovers(&takeovers)
        .test_value();
    main_assert!(hidden.message_dialogs.is_empty());

    // An empty set never opens a dialog at all.
    let mut none = new_menu_app(320, 200);
    none.app_paths = Some(paths.clone());
    none.report_offline_wild_takeovers(&[]).test_value();
    main_assert!(none.message_dialogs.is_empty());
}

/// `RestoreSavegameInfos` logs an unassociated current participant only for a
/// savegame, logs the remaining restore-row count before
/// `RemoveUnassociatedPlayers`, and logs each joined row removed by
/// `RemoveUnjoined` (C4PlayerInfo.cpp:1420-1435,1620-1629).
#[test]
fn savegame_resume_logs_localized_unassociated_player_removals() {
    let native = |text: &str| LegacyCString::from_bytes(text.as_bytes().to_vec()).test_value();
    let mut current = ControlPlayerInfoRegistry::default();
    current.apply(clonk_engine::PlayerInfoControlData::new(
        0,
        0,
        vec![
            clonk_engine::ControlPlayerInfoEntry {
                name: native("Resumed participant"),
                savegame_player: 7,
                ..Default::default()
            },
            clonk_engine::ControlPlayerInfoEntry {
                name: native("New participant"),
                ..Default::default()
            },
        ],
        -1,
    ));
    let restore = vec![
        saves_fixture!(player_info_id_name_game_number: 7, native("Resumed player"), 7),
        saves_fixture!(player_info_id_name_game_number: 8, native("Unclaimed player"), 8),
    ];
    let resources = HashMap::from([
        (
            "IDS_PRC_RESUMENOPLRASSOCIATION".to_string(),
            "localized participant %s".to_string(),
        ),
        (
            "IDS_PRC_RESUMEREMOVEPLRS".to_string(),
            "localized remaining %d".to_string(),
        ),
        (
            "IDS_PRC_REMOVEPLR".to_string(),
            "localized removed %s".to_string(),
        ),
    ]);

    let (before, after) = savegame_player_removal_log_lines(&current, &restore, true, &resources);

    main_assert_eq!(before => vec!["localized participant New participant".to_string(), "localized remaining 1".to_string(),]);
    main_assert_eq!(after => vec!["localized removed Unclaimed player".to_string()]);

    let capture = clonk_logging::ConsoleLogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_level(false)
        .with_writer(capture.clone())
        .finish();
    let mut engine = Engine::default();
    tracing::subscriber::with_default(subscriber, || {
        remove_unassociated_savegame_player_objects_with_logs(
            &mut engine,
            &current,
            &restore,
            true,
            &resources,
        )
        .test_value();
    });
    let logged = capture.take();
    main_assert_eq!(
        logged.lines().map(str::to_owned).collect::<Vec<_>>() =>
        vec![
            "localized participant New participant".to_string(),
            "localized remaining 1".to_string(),
            "localized removed Unclaimed player".to_string(),
        ]
    );

    let (regular_before, regular_after) =
        savegame_player_removal_log_lines(&current, &restore, false, &resources);
    main_assert_eq!(regular_before => vec!["localized remaining 1".to_string()]);
    main_assert_eq!(regular_after => vec!["localized removed Unclaimed player".to_string()]);
}
