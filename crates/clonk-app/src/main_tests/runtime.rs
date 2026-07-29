    // Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
    // sequence, not a child module, so test ids stay `tests::<fn>`.

    #[test]
    fn classic_command_line_keeps_rust_option_values_out_of_legacy_scanning() {
        let cli = Cli::try_parse_from([
            "clonk-app",
            "--test-load",
            "Fixture.c4s",
            "/tmp/Direct.C4S",
            "/future:value",
            "unknown.extension",
            "--player-name",
            "/network",
            "--test-frames",
            "7",
        ])
        .expect("classic positionals coexist with modern switches");
        let classic = parse_classic_command_line(&cli.classic_arguments);

        assert_eq!(cli.test_load, Some(PathBuf::from("Fixture.c4s")));
        assert_eq!(cli.player_name, "/network");
        assert_eq!(cli.test_frames, 7);
        assert_eq!(classic.scenario, Some(PathBuf::from("/tmp/Direct.C4S")));
        assert_eq!(classic.network_active, None);
        assert!(Cli::try_parse_from(["clonk-app", "--future"]).is_err());
    }

    #[test]
    fn classic_command_line_preserves_file_and_definition_order() {
        let classic = parse_classic_command_line(&[
            OsString::from("First.c4s"),
            OsString::from("Players/Alice.C4P"),
            OsString::from("Defs/ExtraOne.c4d"),
            OsString::from("Players/Bob.c4p"),
            OsString::from("Defs/ExtraTwo.C4D"),
            OsString::from("Missions/Last/Scenario.TXT"),
            OsString::from("Patch.c4u"),
            OsString::from("Round.c4r"),
        ]);

        assert_eq!(classic.scenario, Some(PathBuf::from("Missions/Last")));
        assert_eq!(
            classic.player_files,
            vec![
                PathBuf::from("Players/Alice.C4P"),
                PathBuf::from("Players/Bob.c4p")
            ]
        );
        assert_eq!(
            classic.definition_files,
            vec![
                PathBuf::from("Defs/ExtraOne.c4d"),
                PathBuf::from("Defs/ExtraTwo.C4D")
            ]
        );
        assert_eq!(classic.incoming_update, Some(PathBuf::from("Patch.c4u")));
        assert_eq!(classic.record_stream, Some(PathBuf::from("Round.c4r")));
        assert_eq!(
            classic_command_line_definition_modules(
                b"[General]\nDefinitions=Base.c4d;Second.c4d\n",
                &classic.definition_files,
            ),
            vec![
                "Base.c4d",
                "Second.c4d",
                "Defs/ExtraOne.c4d",
                "Defs/ExtraTwo.C4D",
            ]
        );
        assert_eq!(
            classic_command_line_definition_modules(
                b"[General]\nDefinitions=;Base.c4d;;Second.c4d;\n",
                &[],
            ),
            vec!["", "Base.c4d", "", "Second.c4d"],
            "std::getline preserves leading/interior empty modules but not a trailing delimiter"
        );
    }

    #[test]
    fn classic_command_line_maps_process_local_overrides_in_argument_order() {
        let classic = parse_classic_command_line(
            &[
                "/network",
                "/nonetwork",
                "/signup",
                "/nosignup",
                "/league",
                "/noleague",
                "/lobby:-4",
                "/observe",
                "/runtimejoin",
                "/noruntimejoin",
                "/tcpport:2222",
                "/udpport:3333",
                "/pass:secret",
                "/comment:launch comment",
                "/recdump:discarded.bin",
                "/RECDUMP:dump.TXT",
                "/stream:record.example:11114",
                "/faircrew",
                "/trainedcrew",
                "/config:portable.cfg",
                "/verbose",
                "/Language:DE,US",
                "/Language:FR",
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>(),
        );

        assert_eq!(classic.network_active, Some(true));
        assert_eq!(classic.master_server_signup, Some(true));
        assert_eq!(classic.league_server_signup, Some(false));
        assert_eq!(classic.lobby_timeout, Some(Some(0)));
        assert!(classic.observe);
        assert_eq!(classic.runtime_join, Some(false));
        assert_eq!(classic.tcp_port, Some(2222));
        assert_eq!(classic.udp_port, Some(3333));
        assert_eq!(classic.password.as_deref(), Some("secret"));
        assert_eq!(classic.comment.as_deref(), Some("launch comment"));
        assert_eq!(classic.record_dump.as_deref(), Some("dump.TXT"));
        assert_eq!(
            classic.record_stream,
            Some(PathBuf::from("record.example:11114"))
        );
        assert_eq!(classic.fair_crew, Some(false));
        assert_eq!(classic.config_file, Some(PathBuf::from("portable.cfg")));
        assert!(classic.verbose);
        assert_eq!(classic.language.as_deref(), Some("DE,US"));

        let mut app = new_state_only_menu_app(320, 200);
        app.apply_classic_command_line(&classic)
            .expect("apply process-local network options");
        assert!(app.scenario_game_options.values().master_server_signup);
        assert!(!app.scenario_game_options.values().league_server_signup);
        assert_eq!(app.scenario_game_options.values().password, "secret");
        assert_eq!(app.scenario_game_options.values().comment, "launch comment");
        assert!(!app.scenario_game_options.values().fair_crew);
        assert_eq!(app.runtime_network_join_allowed, Some(false));
    }

    #[test]
    fn l028_console_open_real_scenario_reaches_running() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();
        let user_data = tempdir().expect("command-line user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        configure_test_startup_participant(&paths, user_data.path());
        persist_config_value(&paths, "General", "Participants", "MissingConfigured.c4p")
            .expect("make command-line player override observable");
        let player_path = user_data.path().join("Exact.c4p");
        let scenario_path = paths.scenario_dir().join("Direct.c4s");
        let definition_path = scenario_path.join("Defs.c4d");
        fs::create_dir_all(&definition_path).expect("create direct scenario definition");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Direct command line\nMaxPlayer=1\n",
        )
        .expect("write direct scenario core");
        fs::write(
            definition_path.join("DefCore.txt"),
            "[DefCore]\nid=TEST\nName=Test\nCategory=1\n",
        )
        .expect("write direct scenario definition core");
        write_test_definition_graphics(&definition_path);

        let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths))
        .expect("initialise direct command-line app");
        let boot_result = app
            .boot_loading
            .take()
            .expect("real boot worker")
            .receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("finish real boot resources");
        let (boot_sender, boot_receiver) = mpsc::channel();
        app.boot_loading = Some(BootLoadingState::new(boot_receiver));
        app.console_mode = true;
        let command = format!(
            "/open \"{}\" \"{}\" \"{}\"",
            scenario_path.display(),
            player_path.display(),
            definition_path.display(),
        );
        app.process_console_command(&command)
            .expect("open real scenario from console startup");
        assert!(app.loading_state.is_none());
        assert!(app.auto_start_classic_command_line_scenario);
        boot_sender
            .send(boot_result)
            .expect("release held boot resources");
        app.poll_boot_loading();
        assert!(!app.auto_start_classic_command_line_scenario);
        assert!(app.loading_state.is_some());

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if matches!(app.mode, AppMode::Running) {
                break;
            }
            assert_ne!(
                app.mode,
                AppMode::Menu,
                "startup menu must stay suppressed; status={:?}; loader_error={:?}",
                app.status_text,
                app.loader_error,
            );
            assert!(
                Instant::now() < deadline,
                "direct scenario did not finish loading; status={:?}",
                app.status_text,
            );
            app.update().expect("advance direct command-line load");
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            app.active_scenario
                .as_ref()
                .and_then(|scenario| scenario.path.as_deref()),
            Some(scenario_path.as_path())
        );
        assert!(app.startup_dialog_fade.is_none());
        reset_cached_app_paths();
    }

    #[test]
    fn m10_l046_command_and_hash_routes_bypass_plain_script_control() {
        let mut app = new_state_only_running_sandbox_app();
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);

        assert!(app
            .process_developer_console_input("/help", false)
            .expect("slash input reaches ProcessCommand"));

        assert!(app
            .process_developer_console_input("#/sound Bell", false)
            .expect("hash input reaches ProcessInput"));
        let (controls, messages) = commands.take_submitted_decided_controls_and_messages();
        assert!(controls.is_empty());
        assert_eq!(
            messages,
            vec![MessageControlData {
                message_type: MESSAGE_TYPE_SOUND,
                player: app.local_owner,
                to_player: -1,
                message: LegacyCString::from_bytes(b"Bell".to_vec())
                    .expect("fixture message has no NUL"),
                by_client: 0,
            }]
        );
    }

    #[test]
    fn m10_l046_plain_script_checks_editing_and_emits_decide_console_scope() {
        let _lock = env_lock().lock();
        let fixture = tempdir().expect("developer console strictness configuration");
        let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
        let mut config = Config::new();
        config.set_in(Some("Developer"), "ConsoleScriptStrictness", "Strict2");
        config
            .save(paths.config_file())
            .expect("save developer console configuration");

        let mut app = new_state_only_running_sandbox_app();
        app.app_paths = Some(paths);
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);

        assert!(!app
            .process_developer_console_input("SetGravity(41)", false)
            .expect("replay editing gate refuses plain script"));
        assert_eq!(app.status_text, "No editing while replaying.");
        assert!(commands.take_submitted_decided_controls().is_empty());

        assert!(app
            .process_developer_console_input("SetGravity(42)", true)
            .expect("editable console accepts plain script"));
        let decided = commands.take_submitted_decided_controls();
        let [(_, clonk_engine::ControlPacket::Script(script), false)] = decided.as_slice() else {
            panic!("expected one queued console script, got {decided:?}");
        };
        assert_eq!(script.target_object, clonk_engine::SCRIPT_SCOPE_CONSOLE);
        assert_eq!(script.strictness, clonk_engine::ScriptStrictness::Strict2);
        assert_eq!(script.script.as_bytes(), b"SetGravity(42)");
        assert_eq!(script.by_client, 0);
    }

    #[test]
    fn m10_l046_property_script_wraps_live_selection_as_emmo_script() {
        let mut app = new_state_only_running_sandbox_app();
        let (_events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);

        app.submit_editor_selection_script("Mark()", &[41, 7, 41])
            .expect("property script snapshots the edit-cursor selection");

        let decided = commands.take_submitted_decided_controls();
        let [(_, clonk_engine::ControlPacket::EmMoveObject(control), false)] = decided.as_slice()
        else {
            panic!("expected one queued editor script, got {decided:?}");
        };
        assert_eq!(control.action, clonk_engine::EMMO_SCRIPT);
        assert_eq!(control.objects, vec![41, 7, 41]);
        assert_eq!((control.tx, control.ty, control.target_object), (0, 0, -1));
        assert_eq!(control.strictness, clonk_engine::ScriptStrictness::Strict3);
        assert_eq!(control.script.as_bytes(), b"Mark()");
        assert_eq!(control.by_client, 7);
    }

    #[test]
    fn classic_command_line_config_and_language_override_are_process_local() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("command-line config root");
        let custom_config = user_data.path().join("portable/custom.cfg");
        fs::create_dir_all(custom_config.parent().unwrap()).expect("custom config parent");
        let original = b"[General]\nLanguage=US\nLanguageEx=US\nParticipants=Configured.c4p\n\n[Network]\nPortRefServer=23456\n";
        fs::write(&custom_config, original).expect("write custom config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_CONFIG_FILE", None),
            ("LC_LANGUAGE_OVERRIDE", None),
        ]);
        let classic = ClassicCommandLine {
            config_file: Some(custom_config.clone()),
            language: Some("DE,US".to_string()),
            ..ClassicCommandLine::default()
        };

        install_classic_language_override(&classic);
        let paths = AppPaths::discover_with_config_file(classic.config_file.as_deref())
            .expect("discover overridden paths");

        assert_eq!(paths.config_file(), custom_config);
        assert_eq!(paths.language_override(), Some("DE,US"));
        assert_eq!(scenario_title_language(Some(&paths)), "US");
        assert_eq!(
            classic_direct_reference_endpoint("127.0.0.1", Some(&paths))
                .expect("custom reference port"),
            clonk_network::ReferenceEndpoint::Address(SocketAddr::from(([127, 0, 0, 1], 23_456,)))
        );
        assert_eq!(
            classic_loader_language_sequence(&paths).expect("command-line language sequence"),
            vec!["DE", "US"]
        );
        assert_eq!(
            fs::read(paths.config_file()).expect("read unchanged custom config"),
            original
        );
    }

    #[test]
    fn l001_offline_seed_resolution_matches_cpp_time_pin_and_parameters() {
        let first_second = 1_700_000_000_u64;
        let next_second = first_second + 1;

        assert_ne!(
            resolve_offline_round_random_seed(None, first_second, None),
            resolve_offline_round_random_seed(None, next_second, None),
            "different C++ time(nullptr) seconds produce different fresh rounds",
        );
        assert_eq!(
            resolve_offline_round_random_seed(None, first_second, Some("")),
            first_second,
            "an empty LC_PIN_SEED is ignored",
        );
        assert_eq!(
            resolve_offline_round_random_seed(None, first_second, Some("0")),
            0,
        );
        assert_eq!(
            resolve_offline_round_random_seed(None, first_second, Some(" \t-7tail")),
            u64::from((-7_i32) as u32),
            "atoi accepts whitespace, sign, and a decimal prefix",
        );
        assert_eq!(
            resolve_offline_round_random_seed(None, first_second, Some("not-a-number")),
            0,
            "a nonempty malformed atoi input pins zero instead of falling back to time",
        );
        assert_eq!(
            resolve_offline_round_random_seed(None, first_second, Some("73")),
            resolve_offline_round_random_seed(None, next_second, Some("73")),
            "a pin reproduces the same round across different start times",
        );
        assert_eq!(
            resolve_offline_round_random_seed(Some(44), first_second, Some("73")),
            44,
            "compiled Parameters.txt wins and bypasses LC_PIN_SEED",
        );
    }

    #[test]
    fn l001_pinned_offline_seed_reaches_dynamic_map_and_engine() {
        let _pin_guard = EnvGuard::set(&[
            ("LC_PIN_SEED", Some(Path::new("7"))),
            ("LC_RUST_ENGINE_RANDOM_SEED", None),
            ("LC_RUST_ENGINE_MAP_SEED", None),
        ]);
        let user_data = tempdir().expect("isolated offline seed user data");
        let (_paths_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        configure_test_startup_participant(&paths, user_data.path());
        let audio_options = AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        };
        let mut app = GameApp::new(
            320,
            200,
            audio_options,
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Seed parity".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialize pinned offline app");
        wait_for_menu(&mut app);
        let scenario =
            resolve_next_mission_scenario(&app.scenario_catalog, "Tutorial.c4f/Tutorial07.c4s")
                .expect("Tutorial07 is present in the real scenario catalog");

        app.start_scenario(scenario)
            .expect("start pinned offline Tutorial07");
        assert_eq!(
            app.loading_state
                .as_ref()
                .and_then(|loading| loading.offline_random_seed),
            Some(7),
            "the main thread freezes LC_PIN_SEED before spawning the loader",
        );
        // This loads the shipped definition tree and dynamic landscape. Give
        // the loader thread room to run alongside the parallel full suite.
        wait_for_running_with_attempts(&mut app, 2_400);

        assert_eq!(app.engine.random_seed(), 7);
        assert_eq!(
            app.engine
                .landscape()
                .expect("Tutorial07 dynamic landscape")
                .map_seed(),
            42_711,
            "the dynamic map consumes seed 7 before activation (seed 0 would yield 59,893)",
        );
    }

    #[test]
    fn l001_fresh_offline_skyparcour_retries_and_activates_the_accepted_seed() {
        let _pin_guard = EnvGuard::set(&[
            ("LC_PIN_SEED", Some(Path::new("1784903470"))),
            ("LC_RUST_ENGINE_RANDOM_SEED", None),
            ("LC_RUST_ENGINE_MAP_SEED", None),
        ]);
        let user_data = tempdir().expect("isolated SkyParcour seed user data");
        let (_paths_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        configure_test_startup_participant(&paths, user_data.path());
        let audio_options = AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        };
        let mut app = GameApp::new(
            320,
            200,
            audio_options,
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "SkyParcour seed retry".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialize SkyParcour seed retry app");
        wait_for_menu(&mut app);
        let scenario = resolve_next_mission_scenario(
            &app.scenario_catalog,
            "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s",
        )
        .expect("HarpoonRace is present in the real scenario catalog");

        app.start_scenario(scenario)
            .expect("start pinned offline HarpoonRace");
        assert_eq!(
            app.loading_state
                .as_ref()
                .and_then(|loading| loading.offline_random_seed),
            Some(1_784_903_470),
            "the candidate seed is frozen before asynchronous validation",
        );
        wait_for_running_with_attempts(&mut app, 4_800);

        assert_eq!(
            app.engine.random_seed(),
            1_784_903_471,
            "activation, saves, and recordings must use the accepted seed"
        );
    }

    fn app_default_rank_promotion_name(app: &GameApp) -> String {
        let script = r#"#strict 2
    func Award()
    {
        DoCrewExp(1000);
        return GetObjectInfoCoreVal("RankName", "ObjectInfo");
    }
    "#;
        let mut engine = Engine::with_seed(0);
        app.apply_material_library_to(&mut engine);
        let mut crew =
            Definition::from_script("CREW", "Crew", script).expect("default-rank app probe compiles");
        crew.set_crew_member(true);
        engine
            .register_definition(crew)
            .expect("probe crew registers");
        engine.set_player_starts(vec![clonk_engine::scenario::PlayerStart {
            ready_crew: vec![("CREW".to_string(), 1)],
            ..Default::default()
        }]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Rank owner".to_string(),
                player_info_id: 1,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("rank owner joins");
        let crew_id = engine.player(0).expect("rank owner exists").crew()[0];
        let crew_index = engine
            .find_object_index(crew_id)
            .expect("probe crew exists");
        match engine
            .call_object_function(crew_index, "Award", Vec::new())
            .expect("default-rank promotion succeeds")
        {
            clonk_script::Value::String(name) => name.into_string(),
            other => panic!("promotion returned {other:?}"),
        }
    }

    #[test]
    fn second_timer_captures_and_resets_cpp_game_fps() {
        // C4Game::Ticks increments cFPS once per executed simulation frame;
        // Sec1Timer copies it to FPS and resets the accumulator. The host uses
        // that exact FPS for late-client activation lag admission
        // (pristine 9ffa0a5d src/C4Game.cpp:1731-1735,1884-1889;
        // src/C4Network2.cpp:1553-1571).
        let mut app = new_running_sandbox_app();
        for _ in 0..3 {
            app.update().expect("execute simulation frame");
        }

        app.sec1_timer().expect("capture the first second FPS");
        assert_eq!(app.frames_per_second, 3);
        assert_eq!(app.frames_since_second, 0);

        app.update().expect("execute next-second frame");
        app.sec1_timer().expect("capture the next second FPS");
        assert_eq!(app.frames_per_second, 1);
        assert_eq!(app.frames_since_second, 0);
    }

    #[test]
    fn l013_new_game_and_teardown_reset_transient_speed_state() {
        let mut app = new_running_sandbox_app();
        app.full_speed = true;
        app.frame_skip = 500;
        app.configure_running_state("Next game".to_string(), DEFAULT_GROUND_HEIGHT);
        assert!(!app.full_speed);
        assert_eq!(app.frame_skip, 1);

        app.full_speed = true;
        app.frame_skip = 7;
        app.return_to_menu_for_relaunch();
        assert!(!app.full_speed);
        assert_eq!(app.frame_skip, 1);
    }

    #[test]
    fn presentation_benchmark_parser_requires_positive_integer_seconds() {
        assert_eq!(
            parse_presentation_benchmark_window("5"),
            Some(Duration::from_secs(5))
        );
        for rejected in ["", "0", "-1", "1.5", "five"] {
            assert_eq!(parse_presentation_benchmark_window(rejected), None);
        }
    }

    #[test]
    fn graphics_pass_percentiles_use_nearest_rank() {
        let samples = (1..=20)
            .rev()
            .map(Duration::from_millis)
            .collect::<Vec<_>>();

        assert_eq!(
            graphics_pass_percentiles(&samples),
            (
                Duration::from_millis(10),
                Duration::from_millis(19),
                Duration::from_millis(20),
            )
        );
        assert_eq!(
            graphics_pass_percentiles(&[]),
            (Duration::ZERO, Duration::ZERO, Duration::ZERO)
        );
    }

    #[test]
    fn presentation_benchmark_context_reports_actual_network_players() {
        assert_eq!(
                presentation_benchmark_context_line(24, 24, 24, 24, 24),
                "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=24 synchronized_player_infos=24 activated_nonhost_clients=24 runtime_crew_objects=24 runtime_players_with_exactly_one_live_sf5b_crew=24"
            );
    }

    #[test]
    fn presentation_benchmark_network_evidence_uses_unique_preferred_message_routes() {
        let connections = vec![
            clonk_network::RuntimeNetworkConnection {
                connection_id: 1,
                client_id: 0,
                usage: "Data/Msg".to_string(),
                protocol: clonk_network::NetworkProtocol::Tcp,
                peer_address: None,
                packet_loss: 0,
                ping_ms: 7,
                lag_ms: 9,
            },
            clonk_network::RuntimeNetworkConnection {
                connection_id: 2,
                client_id: 2,
                usage: "Msg".to_string(),
                protocol: clonk_network::NetworkProtocol::Udp,
                peer_address: None,
                packet_loss: 3,
                ping_ms: -1,
                lag_ms: 12,
            },
            clonk_network::RuntimeNetworkConnection {
                connection_id: 3,
                client_id: 2,
                usage: "Data".to_string(),
                protocol: clonk_network::NetworkProtocol::Tcp,
                peer_address: None,
                packet_loss: 99,
                ping_ms: 100,
                lag_ms: 101,
            },
        ];

        let evidence = summarize_presentation_benchmark_network(1, &connections, 4, 26_813);

        assert_eq!(evidence.local_client_id, 1);
        assert_eq!(evidence.preferred_message_route_peer_ids, vec![0, 2]);
        assert_eq!(evidence.tcp_preferred_message_routes, 1);
        assert_eq!(evidence.udp_preferred_message_routes, 1);
        assert_eq!(evidence.unknown_preferred_message_routes, 0);
        assert_eq!(evidence.nonnegative_ping_peer_count, 1);
        assert_eq!(evidence.nonnegative_lag_peer_count, 2);
        assert_eq!(evidence.max_nonnegative_ping_ms, Some(7));
        assert_eq!(evidence.max_nonnegative_lag_ms, Some(12));
        assert_eq!(evidence.max_packet_loss, 3);
        assert_eq!(evidence.control_presend, 4);
        assert_eq!(evidence.avg_control_send_time_us, 26_813);
        assert_eq!(
                evidence.machine_line(),
                "LC_APP_PRESENTATION_BENCHMARK_NETWORK inspection_status=ok local_client_id=1 preferred_message_route_peer_count=2 preferred_message_route_peer_ids=[0,2] tcp_preferred_message_routes=1 udp_preferred_message_routes=1 unknown_preferred_message_routes=0 nonnegative_ping_peer_count=1 nonnegative_lag_peer_count=2 max_nonnegative_ping_ms=7 max_nonnegative_lag_ms=12 max_packet_loss=3 control_presend=4 avg_control_send_time_us=26813"
            );
    }

    #[test]
    fn presentation_benchmark_counts_live_player_crew_objects() {
        let mut app = new_lightweight_running_sandbox_app();
        let crew = app.snapshot.players[0].crew[0];
        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == crew)
            .expect("sandbox crew object")
            .alive = true;
        assert_eq!(runtime_crew_object_count(&app.snapshot), 1);

        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == crew)
            .expect("sandbox crew object")
            .alive = false;
        assert_eq!(runtime_crew_object_count(&app.snapshot), 0);
    }

    #[test]
    fn presentation_benchmark_requires_one_live_sf5b_in_each_players_crew() {
        // HarpoonRace creates SF5B and passes it to MakeCrewMember for each
        // player; C++ then retains that exact object in the owning player's
        // Crew list (HarpoonRace.c4s/Script.c:66-73;
        // src/C4Player.cpp:1173-1202).
        let mut app = new_lightweight_running_sandbox_app();
        let first_crew = app.snapshot.players[0].crew[0];
        let first_crew_object = app
            .snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == first_crew)
            .expect("sandbox crew object");
        first_crew_object.definition_id = "SF5B".to_string();
        first_crew_object.alive = true;

        let mut second_crew = app
            .snapshot
            .object(first_crew)
            .expect("sandbox crew object")
            .clone();
        second_crew.id = ObjectId::new(first_crew.as_u64() + 1);
        app.snapshot.objects.push(second_crew);
        app.snapshot.players[0]
            .crew
            .push(ObjectId::new(first_crew.as_u64() + 1));

        let mut second_player = app.snapshot.players[0].clone();
        second_player.id += 1;
        second_player.crew.clear();
        app.snapshot.players.push(second_player);

        assert_eq!(runtime_crew_object_count(&app.snapshot), 2);
        assert_eq!(
            runtime_players_with_exactly_one_live_sf5b_crew(&app.snapshot),
            0
        );

        let second_crew = app.snapshot.players[0]
            .crew
            .pop()
            .expect("second sandbox crew");
        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == second_crew)
            .expect("second sandbox crew object")
            .owner = app.snapshot.players[1].id;
        app.snapshot.players[1].crew.push(second_crew);
        assert_eq!(
            runtime_players_with_exactly_one_live_sf5b_crew(&app.snapshot),
            2
        );

        app.snapshot
            .objects
            .iter_mut()
            .find(|object| object.id == second_crew)
            .expect("second sandbox crew object")
            .alive = false;
        assert_eq!(
            runtime_players_with_exactly_one_live_sf5b_crew(&app.snapshot),
            1
        );
    }

    #[test]
    fn presentation_benchmark_keep_running_requires_explicit_one() {
        assert!(parse_presentation_benchmark_keep_running(Some("1")));
        for value in [None, Some(""), Some("0"), Some("true")] {
            assert!(!parse_presentation_benchmark_keep_running(value));
        }
    }

    #[test]
    fn presentation_benchmark_warms_up_counts_successes_and_reports_one_window() {
        let base = Instant::now();
        let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));

        assert_eq!(benchmark.poll(false, base, 10), None);
        benchmark.record_successful_presentation(base, Duration::from_millis(100), true);
        benchmark.record_automatic_graphics_skip();
        assert_eq!(benchmark.poll(true, base, 10), None);
        assert_eq!(
            benchmark.poll(
                true,
                base + PRESENTATION_BENCHMARK_WARMUP - Duration::from_millis(1),
                69,
            ),
            None
        );
        assert_eq!(
            benchmark.poll(true, base + PRESENTATION_BENCHMARK_WARMUP, 70),
            None
        );
        benchmark.record_automatic_graphics_skip();
        benchmark.record_successful_presentation(
            base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_millis(10),
            Duration::from_millis(10),
            true,
        );
        benchmark.record_successful_presentation(
            base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_millis(20),
            Duration::from_millis(20),
            false,
        );
        assert_eq!(
            benchmark.poll(
                true,
                base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_millis(2_999),
                174,
            ),
            None
        );

        let report = benchmark
            .poll(
                true,
                base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_secs(3),
                175,
            )
            .expect("measurement window completes");
        assert_eq!(report.elapsed, Duration::from_secs(3));
        assert_eq!(report.submissions, 2);
        assert_eq!(report.refreshed_frames, 1);
        assert_eq!(report.simulation_frames, 105);
        assert_eq!(report.automatic_graphics_skips, 1);
        assert_eq!(report.graphics_average, Duration::from_millis(15));
        assert_eq!(report.graphics_max, Duration::from_millis(20));
        assert_eq!(report.graphics_p50, Duration::from_millis(10));
        assert_eq!(report.graphics_p95, Duration::from_millis(20));
        assert_eq!(report.graphics_p99, Duration::from_millis(20));
        assert_eq!(
            report.graphics_samples,
            vec![Duration::from_millis(10), Duration::from_millis(20)]
        );
        assert_eq!(
                report.machine_line(),
                "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=3.000000 successful_present_submissions=2 presentation_submission_fps=0.666667 refreshed_frames=1 simulation_frames=105 simulation_fps=35.000000 automatic_graphics_skips=1 average_graphics_pass_ms=15.000000 max_graphics_pass_ms=20.000000 graphics_pass_sample_count=2 graphics_pass_p50_ms=10.000000 graphics_pass_p95_ms=20.000000 graphics_pass_p99_ms=20.000000 graphics_pass_samples_ns=[10000000,20000000]"
            );
        assert_eq!(
            benchmark.poll(true, base + Duration::from_secs(10), 999),
            None
        );
    }

    #[test]
    fn high_dpi_cursor_defaults_off_and_reads_the_native_boolean() {
        // Deliberate divergence, so it must stay opt-in: with the key absent
        // the cursor keeps C4GraphicsResource's sheet choice exactly
        // (src/C4GraphicsResource.cpp:468-491).
        assert!(!configured_high_dpi_cursor(b""));
        assert!(!configured_high_dpi_cursor(b"[Graphics]\nHighDpiCursor=0\n"));
        assert!(configured_high_dpi_cursor(b"[Graphics]\nHighDpiCursor=1\n"));
    }

    #[test]
    fn the_remaster_switch_supplies_a_default_that_each_key_can_override() {
        // One switch turns the presentation-only divergences on together, but
        // a key the player wrote by hand still wins in both directions.
        assert!(!configured_high_dpi_cursor(b""));
        assert!(!configured_sky_dither(b""));
        assert!(!configured_mipmaps(b""));
        assert!(!configured_smooth_landscape(b""));

        let remastered = b"[Graphics]\nRemaster=1\n";
        assert!(configured_high_dpi_cursor(remastered));
        assert!(configured_sky_dither(remastered));
        assert!(configured_mipmaps(remastered));
        assert!(configured_smooth_landscape(remastered));

        assert!(!configured_sky_dither(
            b"[Graphics]\nRemaster=1\nSkyDither=0\n"
        ));
        assert!(configured_mipmaps(b"[Graphics]\nRemaster=0\nMipmaps=1\n"));
    }

    #[test]
    fn sky_dither_defaults_off_and_reads_the_native_boolean() {
        // C++ draws the sky fade as a plain interpolated quad, so the
        // byte-exact gradient has to stay the default.
        assert!(!configured_sky_dither(b""));
        assert!(!configured_sky_dither(b"[Graphics]\nSkyDither=0\n"));
        assert!(configured_sky_dither(b"[Graphics]\nSkyDither=1\n"));
    }

    #[test]
    fn max_refresh_delay_defaults_to_cpp_30_ms_and_honors_positive_config() {
        // Config.Graphics.MaxRefreshDelay defaults to 30, so the native
        // 28 ms game timer remains one 28 ms graphics opportunity instead of
        // being divided into two 14 ms redraws (src/C4Config.cpp:481-485;
        // src/C4Application.cpp:510-520).
        assert_eq!(configured_max_refresh_delay_ms(b""), 30);
        assert_eq!(
            configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=0\n"),
            30
        );
        assert_eq!(
            configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=-5\n"),
            30
        );
        assert_eq!(
            configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=30\n"),
            30
        );
    }

    /// `Config.Graphics.MaxRefreshDelay` is 30 (C4Config.cpp:485) and
    /// `C4Application` uses it as the divisor ceiling when choosing the
    /// graphics timer interval (C4Application.cpp:510-531). Every path that
    /// materializes the value - the startup resolver and the advanced-config
    /// editor row - has to agree on that default, and a valid positive value
    /// must still reach the divisor.
    #[test]
    fn max_refresh_delay_missing_or_invalid_matches_cpp_thirty_ms() {
        // Absent section, absent key and unparsable values all resolve to 30.
        for config in [
            &b""[..],
            b"[Graphics]\n",
            b"[Graphics]\nMaxRefreshDelay=\n",
            b"[Graphics]\nMaxRefreshDelay=fast\n",
            b"[Graphics]\nMaxRefreshDelay=0\n",
            b"[Graphics]\nMaxRefreshDelay=-1\n",
        ] {
            assert_eq!(
                configured_max_refresh_delay_ms(config),
                30,
                "{:?} must resolve to the native default",
                String::from_utf8_lossy(config)
            );
        }
        // A trailing suffix is not invalid: StdCompiler reads the numeric
        // prefix and ignores the rest, so `16ms` is the positive value 16.
        assert_eq!(
            configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=16ms\n"),
            16
        );

        // A valid positive value is kept verbatim.
        assert_eq!(
            configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=50\n"),
            50
        );
        assert_eq!(
            configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=16\n"),
            16
        );

        // The advanced-config editor materializes the same default rather than
        // inventing a faster one.
        let row = crate::advanced_config::sections(&Config::new())
            .into_iter()
            .flat_map(|section| section.rows)
            .find(|row| row.name == "MaxRefreshDelay")
            .expect("MaxRefreshDelay row");
        assert_eq!(row.value.serialized(), "30");

        // The retained value still feeds the divisor: 30 leaves the 28 ms game
        // timer as one graphics opportunity, a smaller ceiling splits it.
        assert_eq!(
            frame_schedule_for_mode(AppMode::Running, 28, 1, 30).refresh_interval,
            Duration::from_millis(28)
        );
        assert_eq!(
            frame_schedule_for_mode(AppMode::Running, 28, 1, 16).refresh_interval,
            Duration::from_millis(14)
        );
    }

    #[test]
    fn max_refresh_delay_uses_cpp_divisor_without_speeding_simulation() {
        let default = frame_schedule_for_mode(AppMode::Running, 28, 1, 16);
        assert_eq!(default.simulation_interval, Duration::from_millis(28));
        assert_eq!(default.refresh_interval, Duration::from_millis(14));

        let explicit_native_default = frame_schedule_for_mode(AppMode::Running, 28, 1, 30);
        assert_eq!(
            explicit_native_default.simulation_interval,
            Duration::from_millis(28)
        );
        assert_eq!(
            explicit_native_default.refresh_interval,
            Duration::from_millis(28)
        );

        let slow = frame_schedule_for_mode(AppMode::Running, 1_000, 1, 16);
        assert_eq!(slow.simulation_interval, Duration::from_millis(1_000));
        assert_eq!(slow.refresh_interval, Duration::from_millis(15));
    }

    #[test]
    fn offline_startup_queues_all_admitted_players_and_rejects_duplicate_file_use() {
        // C4Game freezes Config.General.Participants before OpenScenario;
        // InitLocal loads every valid module in order, assigns dense info IDs,
        // and queues every admitted local join before the first game tick
        // (pristine 9ffa0a5d src/C4Game.cpp:361-364,2699-2736,2828-2834;
        // src/C4PlayerInfo.cpp:357-395,781-807,1273-1322).
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();
        let install = tempdir().expect("install root");
        let user_data = tempdir().expect("user data");
        install_global_gui_and_loader_test_root(install.path());
        let scenario_path = install.path().join("Scenarios/TwoPlayers.c4s");
        let definition_path = scenario_path.join("Defs.c4d");
        fs::create_dir_all(&definition_path).expect("create scenario definition");
        fs::write(
                scenario_path.join("Scenario.txt"),
                "[Head]\nTitle=Two players\nMaxPlayer=3\n\n[Definitions]\nDefinition1=Scenarios/TwoPlayers.c4s/Defs.c4d\n",
            )
            .expect("write scenario core");
        fs::write(
            definition_path.join("DefCore.txt"),
            "[DefCore]\nid=TEST\nName=Test\nCategory=1\n",
        )
        .expect("write definition core");
        write_test_definition_graphics(&definition_path);

        let write_player = |filename: &str, name: &str, control: i32, auto_stop: bool| {
            let path = install.path().join(filename);
            let mut group = clonk_resources::MutableGroup::new(filename);
            group
                    .add_file_with_metadata(
                        "Player.txt",
                        format!(
                            "[Player]\nName={name}\n\n[Preferences]\nControl={control}\nMouse=0\nAutoStopControl={}\n",
                            i32::from(auto_stop),
                        )
                        .into_bytes(),
                        1,
                        false,
                    )
                    .expect("add player core");
            fs::write(&path, group.pack().expect("pack player")).expect("write player group");
            path
        };
        write_player("Alice.c4p", "Alice", 0, false);
        write_player("Bob.c4p", "Bob", 1, true);
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        paths.ensure_user_dirs().expect("create user directories");
        fs::write(
            paths.config_file(),
            "[General]\nLanguageEx=US\nParticipants=\"Alice.c4p;Bob.c4p\"\n",
        )
        .expect("write configured participants");

        let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths))
        .expect("initialize app");
        wait_for_menu(&mut app);
        fs::write(
            paths.config_file(),
            "[General]\nLanguageEx=US\nParticipants=\"Alice.c4p;Bob.c4p;Alice.c4p\"\n",
        )
        .expect("restore raw duplicate immediately before C4Game::Init");
        let scenario = app
            .scenario_catalog
            .get("TwoPlayers.c4s")
            .cloned()
            .expect("scenario discovered");
        app.start_scenario(scenario).expect("start scenario");
        wait_for_running(&mut app);

        assert_eq!(
            app.snapshot
                .players
                .iter()
                .map(|player| (player.id, player.player_info_id, player.name.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, 1, "Alice"), (1, 2, "Bob")],
        );
        assert_eq!(app.snapshot.frame, 0, "joins precede the first game tick");
        assert_eq!(app.snapshot.hud.local_players, vec![0, 1]);
        assert_eq!(app.control_player_infos.player_count(), 3);
        for (info_id, filename) in [
            (1, b"Alice.c4p".as_slice()),
            (2, b"Bob.c4p".as_slice()),
            (3, b"Alice.c4p".as_slice()),
        ] {
            let info = app
                .control_player_infos
                .get(info_id)
                .expect("admitted player info remains registered");
            assert_eq!(info.filename.as_bytes(), filename);
            assert_eq!(
                info.flags & clonk_engine::PLAYER_INFO_FLAG_JOINED != 0,
                info_id != 3,
            );
        }

        let bob_down = app
            .bindings
            .key_for_set(1, ControlBindingId::Down)
            .expect("keyboard set two has a down key");
        app.handle_key(bob_down, ElementState::Pressed)
            .expect("press Bob's down key");
        let control = |app: &GameApp, owner| {
            app.engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == owner)
                .expect("joined local player")
                .control
        };
        assert_eq!(
            control(&app, 0).pressed_coms & (1 << clonk_engine::COM_DOWN),
            0
        );
        assert_ne!(
            control(&app, 1).pressed_coms & (1 << clonk_engine::COM_DOWN),
            0
        );
        app.handle_key(bob_down, ElementState::Released)
            .expect("release Bob's down key");
        assert_eq!(
            control(&app, 1).pressed_coms & (1 << clonk_engine::COM_DOWN),
            0
        );

        let alice_left = app
            .bindings
            .key_for_set(0, ControlBindingId::Left)
            .expect("keyboard set one has a left key");
        app.handle_key(alice_left, ElementState::Pressed)
            .expect("hold Alice's left key");
        app.handle_key(bob_down, ElementState::Pressed)
            .expect("hold Bob's down key");
        assert_ne!(control(&app, 0).pressed_coms, 0);
        assert_ne!(control(&app, 1).pressed_coms, 0);
        app.handle_focus_lost()
            .expect("focus loss runs its nonfatal UI cleanup");
        // No native backend clears player controls on focus loss
        // (C4FullScreen.cpp:139-145,310-315,432-447).
        assert_ne!(control(&app, 0).pressed_coms, 0);
        assert_ne!(control(&app, 1).pressed_coms, 0);

        app.return_to_menu();
        fs::write(
            paths.config_file(),
            "[General]\nLanguageEx=US\nParticipants=\"\"\n",
        )
        .expect("clear configured participants");
        let scenario = app
            .scenario_catalog
            .get("TwoPlayers.c4s")
            .cloned()
            .expect("scenario remains discovered");
        // This deliberately bypasses C4StartupScenSelDlg::DoOK/CanOpen and
        // exercises C4Game's independent late fullscreen guard. The actual
        // ScenarioBrowser route is covered by
        // local_scenario_start_with_no_participants_shows_cpp_error_before_loading.
        app.start_scenario(scenario)
            .expect("begin zero-player scenario load");
        for _ in 0..480 {
            app.update().expect("poll zero-player startup");
            if !matches!(app.mode, AppMode::Loading) {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(app.mode, AppMode::Menu));
        assert_eq!(app.startup_view, StartupView::MainMenu);
        assert!(app.loading_state.is_none());
        // The return through PreInit re-initializes the loader screen for the
        // next game (src/C4Application.cpp:242-247,373-389).
        assert!(app.loader_screen.is_some());
        assert_startup_error_log(
            &app,
            "Failed to start Two players: Fullscreen mode requires at least one participating player.",
        );
        assert!(app.engine.snapshot().players.is_empty());
        assert_eq!(app.control_player_infos.player_count(), 0);
        reset_cached_app_paths();
    }

    #[test]
    fn selected_player_classic_control_synchronizes_horizontal_key_release() {
        // Classic movement still keeps its direction until the next press,
        // but the key-up itself is synchronized (a clonk-rs divergence from
        // C4Game.cpp:3592-3605) so scripts get Control*Released in both
        // control styles. Renamed from
        // `selected_player_classic_control_ignores_horizontal_key_release`.
        assert_selected_player_horizontal_release(false);
    }

    #[test]
    fn fresh_player_default_up_key_jumps_and_releases_like_cpp() {
        // A new C++ player selects keyboard set 1 with AutoStopControl
        // (C4StartupPlrSelDlg.cpp:1103-1113), whose Up key is S, not an arrow
        // alias (C4Config.cpp:624-635). WALK Up queues Jump
        // (C4ObjectCom.cpp:335-350), and key-up clears the registered Up bit
        // (C4Game.cpp:3579-3592; C4Player.cpp:1490-1551).
        let mut app = GameApp::new(
            320,
            200,
            AudioOptions::default(),
            None,
            RuntimeConfig {
                player_owner: 1,
                player_name: "Fresh player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app");
        install_classic_test_assets(&mut app);

        let mut definition =
            Definition::from_script("JMPR", "Jumper", walker_script()).expect("crew definition");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("flight"),
                ),
            ]),
        );
        definition.set_movement_profile(MovementProfile::default());
        definition.set_physical(clonk_engine::PhysicalInfo {
            walk: 70_000,
            jump: 40_000,
            ..Default::default()
        });
        definition.set_crew_member(true);
        app.engine
            .register_definition(definition)
            .expect("register crew definition");
        app.engine
            .set_player_starts(vec![clonk_engine::scenario::PlayerStart {
                ready_crew: vec![("JMPR".to_string(), 1)],
                ..Default::default()
            }]);
        app.join_local_player().expect("join fresh player");
        app.mode = AppMode::Running;

        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("fresh player's cursor");
        assert!(
            app.engine
                .player(app.local_owner)
                .expect("fresh player")
                .control_style(),
            "new players default to AutoStopControl like C++"
        );
        assert_eq!(
            app.bindings.key_for(ControlBindingId::Up),
            Some(VirtualKeyCode::S)
        );

        app.handle_key(VirtualKeyCode::Up, ElementState::Pressed)
            .expect("unbound arrow press");
        assert!(
            app.engine
                .object_snapshot(cursor)
                .expect("cursor after arrow press")
                .command_stack
                .command_names()
                .is_empty(),
            "the default Up arrow must not alias keyboard-set-1 Up"
        );

        app.handle_key(VirtualKeyCode::S, ElementState::Pressed)
            .expect("press keyboard-set-1 Up");
        assert_eq!(
            app.engine
                .object_snapshot(cursor)
                .expect("cursor after S press")
                .command_stack
                .command_names(),
            vec!["Jump".to_string()],
            "S must traverse GameApp input and queue C4CMD_Jump"
        );
        assert_ne!(
            app.engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == app.local_owner)
                .expect("player after S press")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_UP),
            0,
            "the Up press must be registered before release"
        );

        app.engine.tick().expect("execute queued jump");
        let jumping = app
            .engine
            .object_snapshot(cursor)
            .expect("cursor after jump tick");
        assert_eq!(jumping.action.name, "Jump");
        assert!(
            jumping.velocity.y < 0,
            "ObjectComJump launches upward (C4ObjectCom.cpp:280-307)"
        );

        app.handle_key(VirtualKeyCode::S, ElementState::Released)
            .expect("release keyboard-set-1 Up");
        assert_eq!(
            app.engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == app.local_owner)
                .expect("player after S release")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_UP),
            0,
            "AutoStop key-up clears the registered Up press"
        );
    }

    #[test]
    fn invalid_script_output_in_controls_is_non_fatal_like_cpp() {
        // Whatever a control script RETURNS is script-caused: C++ coerces
        // every result (static_cast<bool>, C4Object.cpp:3300) and never
        // aborts the input path over it. The Rust-only InvalidScriptOutput
        // class must not kill the window event loop either — it downgrades
        // to a status message exactly like Script errors.
        let output_error = EngineError::InvalidScriptOutput {
            definition: "COWB".into(),
            function: "Control".to_string(),
            detail: "control function `ControlDig` returned garbage".into(),
        };
        let status = control_script_error_to_status(output_error)
            .expect("script-output errors downgrade to a status message");
        assert!(
            status.contains("COWB"),
            "status names the definition: {status}"
        );
    }

    #[test]
    fn engine_errors_remain_recoverable_during_scenario_activation() {
        let recoverable = scenario_activation_engine_error(
            "Broken scenario",
            EngineError::UnknownDefinition("MISS".into()),
        );
        assert!(matches!(
            recoverable,
            ScenarioActivationError::Recoverable(ref message)
                if message.contains("Broken scenario")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn l032_writable_config_repairs_when_parent_forbids_staging() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("config root");
        let path = dir.path().join("clonk-rust.config");
        fs::write(&path, "[General]\nConfigResetSafety=7\n").expect("write corrupt config");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("make config writable");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500))
            .expect("forbid sibling staging files");

        let repair = validate_or_repair_startup_config(&path, false);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))
            .expect("restore config directory permissions");

        assert!(repair.expect("repair writable config in place"));
        assert_eq!(
            Config::load(&path)
                .expect("reload in-place repaired config")
                .get_in(Some("General"), "ConfigResetSafety"),
            Some("42")
        );
    }

    #[test]
    fn l032_custom_corrupt_config_aborts_without_default_replacement() {
        let install = tempdir().expect("install root");
        let user_data = tempdir().expect("user data");
        let dir = tempdir().expect("config root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group stub");
        let path = dir.path().join("portable.config");
        let original = b"[General]\nConfigResetSafety=7\nName=Portable\n";
        fs::write(&path, original).expect("write corrupt custom config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_CONFIG_FILE", None),
        ]);

        let error =
            discover_validated_startup_paths(Some(&path)).expect_err("custom corruption must abort");

        assert_eq!(error.to_string(), CUSTOM_CONFIG_CORRUPTED_ERROR);
        assert_eq!(
            fs::read(&path).expect("read untouched custom config"),
            original
        );
    }

    #[test]
    fn l032_environment_config_repairs_instead_of_custom_abort() {
        let install = tempdir().expect("install root");
        let user_data = tempdir().expect("user data");
        let custom = tempdir().expect("environment config root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group stub");
        let path = custom.path().join("environment.config");
        fs::write(&path, "[General]\nConfigResetSafety=7\n").expect("write corrupt environment config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_CONFIG_FILE", Some(path.as_path())),
        ]);

        let paths = discover_validated_startup_paths(None)
            .expect("repair environment-selected config")
            .expect("rediscover environment-selected paths");

        assert_eq!(paths.config_file(), path);
        let repaired = Config::load(paths.config_file()).expect("reload environment config");
        assert_eq!(
            repaired.get_in(Some("General"), "ConfigResetSafety"),
            Some("42")
        );
    }

    #[test]
    fn l032_missing_integrity_fields_use_typed_defaults() {
        let dir = tempdir().expect("config root");
        let path = dir.path().join("clonk-rust.config");
        let original = b"[General]\nName=Keep\n\n[Graphics]\nResolutionY=0\n";
        fs::write(&path, original).expect("write config without integrity fields");

        assert!(!validate_or_repair_startup_config(&path, false)
            .expect("missing integrity fields are defaults"));
        assert_eq!(fs::read(&path).expect("read unchanged config"), original);
    }

    #[test]
    fn l032_default_repair_discards_cached_corrupt_user_path() {
        let install = tempdir().expect("install root");
        let home = tempdir().expect("home root");
        let poison = tempdir().expect("poison user root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group stub");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("HOME", Some(home.path())),
            ("LC_USER_DATA_DIR", None),
            ("LC_CONFIG_FILE", None),
            ("XDG_DATA_HOME", None),
            ("LOCALAPPDATA", None),
            ("APPDATA", None),
        ]);
        let initial = cached_app_paths_with_config_file(None).expect("discover default paths");
        let config_path = initial.config_file();
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("create config parent");
        fs::write(
            &config_path,
            format!(
                "[General]\nConfigResetSafety=7\nUserPath={}\n",
                poison.path().display()
            ),
        )
        .expect("write corrupt user path");

        reset_cached_app_paths();
        let poisoned = cached_app_paths_with_config_file(None).expect("discover poisoned paths");
        assert_eq!(poisoned.user_data_dir(), poison.path());
        let repaired = discover_validated_startup_paths(None)
            .expect("repair poisoned config")
            .expect("rediscover repaired paths");
        let expected_language = if input::is_german_system() {
            "DE"
        } else {
            "US"
        };

        assert_eq!(repaired.config_file(), config_path);
        assert_ne!(repaired.user_data_dir(), poison.path());
        assert_eq!(
            classic_loader_language_sequence(&repaired).expect("post-repair language default"),
            vec![expected_language.to_string()]
        );
        assert_eq!(
            cached_app_paths_with_config_file(None)
                .expect("cache repaired paths")
                .user_data_dir(),
            repaired.user_data_dir()
        );
    }

    #[test]
    fn l005_cli_config_flag_selects_the_explicit_file() {
        let install = tempdir().expect("install root");
        let user_data = tempdir().expect("user data");
        let custom = tempdir().expect("custom config root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group stub");
        let config_file = custom.path().join("command-line.config");
        let cli = Cli::try_parse_from([
            OsString::from("clonk-app"),
            OsString::from("--config"),
            config_file.as_os_str().to_os_string(),
        ])
        .expect("parse --config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_CONFIG_FILE", None),
        ]);

        let paths = cached_app_paths_with_config_file(cli.config_file.as_deref())
            .expect("discover explicit config paths");

        assert_eq!(paths.config_file(), config_file);
    }

    #[test]
    fn l005_environment_config_file_routes_app_reads_and_writes() {
        let install = tempdir().expect("install root");
        let user_data = tempdir().expect("user data");
        let custom = tempdir().expect("custom config root");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub").expect("system group stub");
        let environment_file = custom.path().join("environment.config");
        let command_line_file = custom.path().join("command-line.config");
        let command_line_sentinel = b"[Graphics]\nResolutionX=321\nResolutionY=234\n";
        fs::write(
            &environment_file,
            "[Graphics]\nResolutionX=777\nResolutionY=555\nScale=100\n",
        )
        .expect("environment config");
        fs::write(&command_line_file, command_line_sentinel).expect("command-line sentinel");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_CONFIG_FILE", Some(environment_file.as_path())),
        ]);

        let paths = cached_app_paths_with_config_file(Some(&command_line_file))
            .expect("discover environment config paths");
        paths.ensure_user_dirs().expect("prepare config parent");
        let mut display = DisplayOptions::load(Some(&paths));
        assert_eq!(display.actual_size(), (777, 555));
        display.record_actual_size(888, 666);
        display.persist_if_dirty(&paths);

        let persisted = Config::load(&environment_file).expect("reload environment config");
        assert_eq!(
            persisted.get_in(Some("Graphics"), "ResolutionX"),
            Some("888")
        );
        assert_eq!(
            persisted.get_in(Some("Graphics"), "ResolutionY"),
            Some("666")
        );
        assert_eq!(
            fs::read(&command_line_file).expect("read untouched command-line config"),
            command_line_sentinel
        );
    }

    #[test]
    fn collect_player_overlay_marks_focus_and_energy() {
        let focus = ObjectId::new(1);
        let teammate = ObjectId::new(2);

        let objects = vec![
            ObjectSnapshot {
                id: focus,
                definition_id: "Clonk".into(),
                custom_name: None,
                position: Vector2::new(0, 0),
                velocity: Vector2::ZERO,
                rotation: 0,
                energy: 80,
                need_energy: false,
                construction: clonk_engine::FULL_CON,
                damage: 0,
                magic_energy: 25_000,
                magic_capacity: 0,
                action: ActionState::default(),
                direction: Direction::default(),
                command_direction: CommandDirection::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                current_shape: None,
                current_fire_top: None,
                contact_density: 50,
                own_vertices: None,
                vertex_contacts: Vec::new(),
                solid_mask_override: None,
                container: None,
                layer: None,
                visibility: 0,
                blit_mode: 0,
                color: 0,
                color_modulation: 0,
                picture_rect: Default::default(),
                contents: Vec::new(),
                components: HashMap::new(),
                component_order: Vec::new(),
                status: ObjectStatus::Normal,
                owner: 1,
                controller: 1,
                category: DEFAULT_CATEGORY,
                crew_member: true,
                plr_view_range: 0,
                selected: false,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
                in_liquid: false,
                mobile: false,
                ocf: 0,
                timer: 0,
                own_mass: 0,
                on_fire: false,
                fire_phase: 0,
                fire_caused_by: -1,
                info_physical: Some(clonk_engine::PhysicalInfo {
                    energy: 100,
                    breath: 100,
                    magic: 50_000,
                    ..clonk_engine::PhysicalInfo::default()
                }),
                temporary_physical: None,
                physical_changes: Vec::new(),
                breath: 50,
                last_energy_loss_cause: -1,
                base: -1,
                fixed_position: None,
                fixed_velocity: None,
                rotation_velocity: None,
                fixed_rotation: None,
            },
            ObjectSnapshot {
                id: teammate,
                definition_id: "Balloon".into(),
                custom_name: None,
                position: Vector2::new(10, 0),
                velocity: Vector2::ZERO,
                rotation: 0,
                energy: 40,
                need_energy: false,
                construction: clonk_engine::FULL_CON,
                damage: 0,
                magic_energy: 0,
                magic_capacity: 0,
                action: ActionState::default(),
                direction: Direction::default(),
                command_direction: CommandDirection::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                current_shape: None,
                current_fire_top: None,
                contact_density: 50,
                own_vertices: None,
                vertex_contacts: Vec::new(),
                solid_mask_override: None,
                container: None,
                layer: None,
                visibility: 0,
                blit_mode: 0,
                color: 0,
                color_modulation: 0,
                picture_rect: Default::default(),
                contents: Vec::new(),
                components: HashMap::new(),
                component_order: Vec::new(),
                status: ObjectStatus::Normal,
                owner: 1,
                controller: 1,
                category: DEFAULT_CATEGORY,
                crew_member: true,
                plr_view_range: 0,
                selected: false,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
                in_liquid: false,
                mobile: false,
                ocf: 0,
                timer: 0,
                own_mass: 0,
                on_fire: false,
                fire_phase: 0,
                fire_caused_by: -1,
                // This fixture has no matching live Engine object. Supply the
                // physical backing explicitly: native DrawEnergy always uses
                // GetPhysical()->Energy and never invents a 100-point range.
                info_physical: Some(clonk_engine::PhysicalInfo {
                    energy: 100,
                    ..clonk_engine::PhysicalInfo::default()
                }),
                temporary_physical: None,
                physical_changes: Vec::new(),
                breath: 0,
                last_energy_loss_cause: -1,
                base: -1,
                fixed_position: None,
                fixed_velocity: None,
                rotation_velocity: None,
                fixed_rotation: None,
            },
        ];

        let mut snapshot = SimulationSnapshot {
            frame: 0,
            game_time: 0,
            game_over: false,
            round_results: Default::default(),
            league_name: Vec::new(),
            player_info_league_progress_data: Default::default(),
            player_info_league_scores: Default::default(),
            physics: None,
            objects,
            render_order: Vec::new(),
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            script_globals: Default::default(),
            particles: Vec::new(),
            players: Vec::new(),
            fow_players: Default::default(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners: vec![1],
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: clonk_engine::LcgRng::seed_from_u64(1),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: vec![HudPlayerSnapshot {
                    owner: 1,
                    crew: vec![focus, teammate],
                    focus: Some(focus),
                    eliminated: false,
                    wealth: 120,
                    score: 0,
                }],
                messages: Vec::new(),
                scoreboard: Default::default(),
                scoreboard_presentations: Vec::new(),
                local_players: vec![1],
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            definition_closed_containers: Default::default(),
            definition_lines: HashMap::new(),
            transfer_zones: Vec::new(),
            pathfinder_debug: Default::default(),
            menu_requests: Vec::new(),
            audio: Vec::new(),
        };

        snapshot.players.push(PlayerState {
            id: 1,
            name: "Alice".into(),
            status: PlayerStatus::Active,
            wealth: 120,
            cursor: Some(focus),
            captain: Some(focus),
            crew: vec![focus, teammate],
            select_count: 1,
            show_startup: true,
            control_set: 6,
            mouse_control: -2,
            show_control: 1 | 1 << 10,
            show_control_position: 3,
            control: clonk_engine::PlayerControlState {
                last_com: 5,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        let mut bindings = KeyboardBindings::load(None);
        assert!(bindings.rebind_for_set(2, ControlBindingId::PlayerMenu, VirtualKeyCode::F8,));
        let mut gamepad_bindings = GamepadBindings::default();
        for (binding, button) in [
            (ControlBindingId::Throw, 0),
            (ControlBindingId::PlayerMenu, 1),
        ] {
            gamepad_bindings.rebind_raw(
                2,
                binding,
                input::legacy_gamepad_button_key(2, button).expect("valid Gamepad3 button"),
            );
        }
        let mut engine = Engine::new();
        let mut clonk_definition =
            Definition::from_script("Clonk", "Clonk", "").expect("Clonk definition");
        clonk_definition.set_hide_hud_elements(0x3f);
        clonk_definition
            .set_hide_hud_bars(clonk_engine::HIDE_HUD_BAR_ENERGY | clonk_engine::HIDE_HUD_BAR_BREATH);
        engine
            .register_definition(clonk_definition)
            .expect("register Clonk definition");
        engine
            .register_script_definition("Balloon", "Balloon", "")
            .expect("register Balloon definition");
        let overlay = collect_player_overlays(
            &mut engine,
            &snapshot,
            Some(focus),
            &bindings,
            &gamepad_bindings,
        );
        assert_eq!(overlay.len(), 1);
        let player = &overlay[0];
        assert_eq!(player.owner, 1);
        assert_eq!(player.name, "Alice");
        assert_eq!(player.wealth, 120);
        assert_eq!(player.cursor, Some(focus));
        assert_eq!(player.captain, Some(focus));
        assert!(!player.eliminated);
        assert_eq!(player.crew_count, 2);
        assert_eq!(player.crew.len(), 2);
        assert_eq!(player.owner_color, default_owner_color(1));
        // HUD projection consumes C4Player's cached SelectCount.
        assert_eq!(player.select_count, 1);
        assert!(player.show_startup, "startup hint owner matches");
        assert_eq!(player.control_set, 6, "runtime GamePad3 set is projected");
        assert!(player.mouse_control, "any nonzero MouseControl is true");
        assert_eq!(player.show_control, 1 | 1 << 10);
        assert_eq!(player.show_control_position, 3);
        assert_eq!(player.last_com, 5);
        assert_eq!(player.control_key_labels.len(), 10);
        assert_eq!(
            player.control_key_labels[3],
            gamepad_bindings.key_label_for_set(2, ControlBindingId::Throw)
        );
        assert_eq!(
            player.control_key_labels[9],
            gamepad_bindings.key_label_for_set(2, ControlBindingId::PlayerMenu),
            "the viewport menu hint follows the player's live Gamepad3 set"
        );

        snapshot.players[0].control_set = 2;
        let keyboard3 = collect_player_overlays(
            &mut engine,
            &snapshot,
            Some(focus),
            &bindings,
            &gamepad_bindings,
        );
        assert_eq!(
            keyboard3[0].control_key_labels[9],
            format_key_label(VirtualKeyCode::F8),
            "the viewport menu hint follows the player's live Keyboard3 set"
        );
        snapshot.players[0].control_set = 4;
        let unassigned_gamepad = collect_player_overlays(
            &mut engine,
            &snapshot,
            Some(focus),
            &bindings,
            &GamepadBindings::default(),
        );
        assert!(
            unassigned_gamepad[0].control_key_labels[9].is_empty(),
            "an undefined gamepad menu button draws no key text"
        );
        snapshot.players[0].control_set = 6;

        snapshot.hud.local_players.clear();
        let remote_overlay = collect_player_overlays(
            &mut engine,
            &snapshot,
            Some(focus),
            &bindings,
            &gamepad_bindings,
        );
        assert!(
            !remote_overlay[0].show_startup,
            "C++ suppresses startup hints for non-local players"
        );
        snapshot.hud.local_players.push(1);

        let mut focused = player
            .crew
            .iter()
            .filter(|crew| crew.is_focus)
            .collect::<Vec<_>>();
        assert_eq!(focused.len(), 1, "only cursor object highlighted");
        let focus_entry = focused.pop().expect("focus highlight present");
        assert!(focus_entry.label.contains("Clonk"));
        assert_eq!((focus_entry.energy, focus_entry.energy_capacity), (80, 100));
        assert_eq!(focus_entry.magic_energy, 25_000);
        assert_eq!(focus_entry.magic_capacity, 50_000);
        assert_eq!(focus_entry.breath, 50);
        assert_eq!(focus_entry.breath_capacity, 100);
        assert_eq!(focus_entry.object_id, focus);
        assert_eq!(focus_entry.hide_hud_elements, 0x3f);
        assert_eq!(
            focus_entry.hide_hud_bars,
            clonk_engine::HIDE_HUD_BAR_ENERGY | clonk_engine::HIDE_HUD_BAR_BREATH
        );
        assert!(focus_entry.portrait.is_none());

        let other_entry = player
            .crew
            .iter()
            .find(|crew| crew.label.contains("Balloon"))
            .expect("non-focus crew present");
        assert!(!other_entry.is_focus);
        assert_eq!((other_entry.energy, other_entry.energy_capacity), (40, 100));
        assert_eq!(other_entry.object_id, teammate);
        assert_eq!(other_entry.hide_hud_elements, 0);
        assert_eq!(other_entry.hide_hud_bars, 0);
        assert!(other_entry.portrait.is_none());

        let raw_name = clonk_script::c4_string_from_bytes(&[0xe9]);
        snapshot.players[0].name = raw_name;
        let overlay = collect_player_overlays(
            &mut engine,
            &snapshot,
            Some(focus),
            &bindings,
            &gamepad_bindings,
        );
        assert_eq!(overlay[0].name, "\u{e9}");
        assert_eq!(
            clonk_script::c4_string_bytes(&snapshot.players[0].name),
            [0xe9],
            "presentation decoding does not rewrite synchronized player state"
        );

        snapshot.hud.players[0].crew = vec![focus];
        snapshot.players[0].view_cursor = Some(teammate);
        let overlay = collect_player_overlays(
            &mut engine,
            &snapshot,
            Some(teammate),
            &bindings,
            &gamepad_bindings,
        );
        assert_eq!(overlay[0].cursor, Some(teammate));
        assert_eq!(overlay[0].crew_count, 1, "ViewCursor is not roster crew");
        assert_eq!(
            overlay[0].crew.len(),
            2,
            "non-roster ViewCursor is projected"
        );
        assert_eq!(
            overlay[0]
                .crew
                .iter()
                .find(|crew| crew.object_id == teammate)
                .map(|crew| (crew.hide_hud_elements, crew.hide_hud_bars)),
            Some((0, 0))
        );
    }

    #[test]
    fn participant_module_count_matches_cpp_smodulecount() {
        assert_eq!(c4_module_count(""), 0);
        assert_eq!(c4_module_count("   ;  ;; "), 0);
        assert_eq!(c4_module_count("Alice.c4p;; Bob.c4p"), 2);
        assert_eq!(c4_module_count(" Alice.c4p ; Bob.c4p ;"), 2);
        assert_eq!(
            c4_module_count("\t"),
            1,
            "C++ SModuleCount ignores ASCII spaces only"
        );
    }

    #[test]
    fn configured_mission_access_reaches_fresh_engines_and_survives_replacement() {
        fn install_probe(engine: &mut Engine) -> usize {
            engine
                .register_definition(
                    Definition::from_script(
                        "MACC",
                        "Mission access probe",
                        r#"#strict 2
    public func Has(password) { return GetMissionAccess(password); }
    public func Grant(password) { return GainMissionAccess(password); }
    "#,
                    )
                    .expect("mission-access probe compiles"),
                )
                .expect("mission-access probe registers");
            let object = engine
                .spawn_object(SpawnConfig::new("MACC"))
                .expect("mission-access probe spawns");
            engine
                .find_object_index(object)
                .expect("probe remains live")
        }

        let _lock = env_lock().lock();
        reset_cached_app_paths();
        let user_data = tempdir().expect("isolated mission-access user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "General", "MissionAccess", "Alpha; Beta")
            .expect("configure mission access");

        let mut app = new_menu_app_with_paths(640, 480, &paths);
        let probe = install_probe(&mut app.engine);
        for password in ["alpha", "BETA"] {
            assert_eq!(
                app.engine
                    .call_object_function(
                        probe,
                        "Has",
                        vec![Value::String(password.to_string().into())],
                    )
                    .expect("configured access query executes"),
                Value::Bool(true)
            );
        }
        assert_eq!(
            app.engine
                .call_object_function(probe, "Has", vec![Value::Nil])
                .expect("nil access query executes"),
            Value::Bool(false)
        );
        assert_eq!(
            app.engine
                .call_object_function(
                    probe,
                    "Grant",
                    vec![Value::String("Runtime".to_string().into())],
                )
                .expect("runtime access grant executes"),
            Value::Bool(true)
        );

        app.return_to_menu();
        let probe = install_probe(&mut app.engine);
        assert_eq!(
            app.engine
                .call_object_function(
                    probe,
                    "Has",
                    vec![Value::String("runtime".to_string().into())],
                )
                .expect("replacement engine sees process-local access"),
            Value::Bool(true)
        );
    }

    #[test]
    fn l134_team_options_submit_exact_sets_and_refresh_from_echoes() {
        let mut app = new_menu_app(640, 480);
        let (_events, mut commands) = install_classic_host_network_stub(&mut app);
        let metadata = set_control_test_metadata(
            false,
            vec![
                set_control_test_team(1, Vec::new(), 0),
                set_control_test_team(2, Vec::new(), 0),
            ],
        );
        app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
        assert!(app.select_classic_lobby_sheet(LobbySheet::Options));

        app.submit_classic_lobby_team_setting(LobbyOptionKind::TeamDistribution, 4);
        app.submit_classic_lobby_team_setting(LobbyOptionKind::TeamColors, 1);
        let sets = commands.take_submitted_control_sets();
        assert_eq!(
            sets,
            [
                clonk_network::LegacyControlSet {
                    value_type: 3,
                    data: 4,
                    by_client: 0,
                },
                clonk_network::LegacyControlSet {
                    value_type: 4,
                    data: 1,
                    by_client: 0,
                },
            ]
        );
        let teams = app.network_team_assignment.as_ref().unwrap().teams();
        assert_eq!(
            teams.team_distribution,
            clonk_engine::InitialNetworkTeamDistribution::Free
        );
        assert!(!teams.team_colors, "menu selections wait for host echoes");

        app.execute_control_set(sets[0]);
        app.execute_control_set(sets[1]);
        let options = app
            .classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .option_rows();
        assert!(options.iter().any(|row| {
            row.kind == LobbyOptionKind::TeamDistribution && row.value == "surprise random!"
        }));
        assert!(options
            .iter()
            .any(|row| { row.kind == LobbyOptionKind::TeamColors && row.value == "enabled" }));
        assert!(options
            .iter()
            .any(|row| row.kind == LobbyOptionKind::RandomTeamCount));

        app.submit_classic_lobby_team_setting(LobbyOptionKind::TeamDistribution, 2);
        assert!(
            commands.take_submitted_control_sets().is_empty(),
            "None is not offered for predefined teams"
        );
    }

    #[test]
    fn teams_sheet_groups_in_team_member_order_and_filters_inactive_or_invisible_players() {
        let mut clients = ControlClientRegistry::default();
        clients.replace_snapshot([
            clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 7,
                activated: false,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 8,
                activated: true,
                ..Default::default()
            },
        ]);
        let player = |id, team, flags, player_type| clonk_engine::ControlPlayerInfoEntry {
            id,
            team,
            flags,
            player_type,
            name: LegacyCString::from_bytes(format!("Player {id}").into_bytes()).unwrap(),
            ..Default::default()
        };
        let mut infos = ControlPlayerInfoRegistry::default();
        infos.replace_snapshot(
            1,
            [
                clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    players: vec![
                        player(10, 1, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                        player(
                            11,
                            2,
                            clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                            clonk_engine::PLAYER_INFO_TYPE_USER,
                        ),
                        player(99, 0, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                    ],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 7,
                    players: vec![player(20, 2, 0, clonk_engine::PLAYER_INFO_TYPE_USER)],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 8,
                    players: vec![
                        player(30, 2, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                        player(31, 1, 0, clonk_engine::PLAYER_INFO_TYPE_SCRIPT),
                    ],
                    ..Default::default()
                },
            ],
        );
        let team = |id, name: &[u8], player_ids| clonk_engine::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let metadata = clonk_engine::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 3,
            team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(2, b"Second", vec![20, 30, 11]),
                team(1, b"First", vec![31, 10]),
                team(3, b"Configured empty", Vec::new()),
            ],
        };

        let rows =
            classic_lobby_roster_projection(&clients, &infos, Some(&metadata), 0, LobbySheet::Teams).0;
        assert_eq!(
            rows.iter().map(LobbyRosterRow::id).collect::<Vec<_>>(),
            vec![
                LobbyRosterId::Header(LobbyRosterHeader::Team(2)),
                LobbyRosterId::Player(30),
                LobbyRosterId::Header(LobbyRosterHeader::Team(1)),
                LobbyRosterId::Player(31),
                LobbyRosterId::Player(10),
                LobbyRosterId::Header(LobbyRosterHeader::Team(3)),
            ]
        );
        assert!(rows
            .iter()
            .all(|row| !matches!(row, LobbyRosterRow::Client(_))));

        let mut generated = metadata.clone();
        generated.auto_generate_teams = true;
        let generated =
            classic_lobby_roster_projection(&clients, &infos, Some(&generated), 0, LobbySheet::Teams).0;
        assert!(!generated.iter().any(|row| matches!(
            row,
            LobbyRosterRow::Header(LobbyHeaderRow {
                kind: LobbyRosterHeader::Team(3),
                ..
            })
        )));
    }

    #[test]
    fn l100_exhausted_script_player_names_pick_from_configured_list() {
        let (mut app, mut commands) = l100_script_player_add_fixture(
            b"Alpha|Beta",
            &[(b"alpha".as_slice(), false), (b"BETA".as_slice(), true)],
            2,
        );
        let draws = [1, 10, 20, 30];
        let mut draw = 0;
        let mut ranges = Vec::new();

        app.add_classic_lobby_script_player_with_random(|range| {
            ranges.push(range);
            let value = draws[draw];
            draw += 1;
            value
        });

        let requests = commands.take_player_info_updates();
        let [request] = requests.as_slice() else {
            panic!("expected one script-player request, got {requests:?}");
        };
        assert_eq!(request.players[0].name.as_bytes(), b"Beta");
        assert_ne!(request.players[0].name.as_bytes(), b"Computer");
        assert_eq!(request.players[0].color, 0x001e_140a);
        assert_eq!(ranges, vec![2, 302, 302, 302]);
    }

    #[test]
    fn l100_empty_script_player_names_keep_computer_fallback() {
        let (mut app, mut commands) = l100_script_player_add_fixture(b"", &[], 1);
        let draws = [4, 5, 6];
        let mut draw = 0;
        let mut ranges = Vec::new();

        app.add_classic_lobby_script_player_with_random(|range| {
            ranges.push(range);
            let value = draws[draw];
            draw += 1;
            value
        });

        let requests = commands.take_player_info_updates();
        let [request] = requests.as_slice() else {
            panic!("expected one script-player request, got {requests:?}");
        };
        assert_eq!(request.players[0].name.as_bytes(), b"Computer");
        assert_eq!(request.players[0].color, 0x0006_0504);
        assert_eq!(ranges, vec![302, 302, 302]);
    }

    #[test]
    fn l034_player_shift_tab_wraps_and_continues_backwards() {
        use clonk_frontend::startup_plrsel::{PlrSelControl, PlrSelController};

        let mut app = new_classic_menu_app(640, 480);
        let mut dialog = PlrSelController::new(1);
        dialog.resize(640, 480);
        app.startup_player_dialog = Some(dialog);
        app.replace_startup_view(StartupView::PlayerSelection);
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift");

        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Shift+Tab wraps player focus");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release wrapped Shift+Tab");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::Crew
        );
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("second Shift+Tab continues backwards");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release second Shift+Tab");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::Properties
        );

        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Shift");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("plain Tab keeps the established forward order");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release plain Tab");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::Crew
        );
    }

    #[test]
    fn l060_player_shift_tab_covers_back_list_and_crew_edges() {
        use clonk_frontend::startup_plrsel::{PlrSelControl, PlrSelController};

        let mut app = new_classic_menu_app(640, 480);
        let mut dialog = PlrSelController::new(1);
        dialog.resize(640, 480);
        app.startup_player_dialog = Some(dialog);
        app.replace_startup_view(StartupView::PlayerSelection);

        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::PlayerList
        );
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("plain Tab focuses Back");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release plain Tab");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::Back
        );

        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift");
        for (expected, description) in [
            (PlrSelControl::PlayerList, "Back to PlayerList"),
            (PlrSelControl::Crew, "PlayerList to Crew"),
        ] {
            app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .expect(description);
            app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("release Shift+Tab");
            assert_eq!(
                app.startup_player_dialog
                    .as_ref()
                    .expect("player dialog")
                    .focused_control(),
                expected
            );
        }

        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("release Shift");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("plain Tab still moves Crew to PlayerList");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release final plain Tab");
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player dialog")
                .focused_control(),
            PlrSelControl::PlayerList
        );
    }

    #[test]
    fn l047_player_typeahead_and_apps_route_through_selected_row() {
        let player = |name: &str| clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: name.to_string(),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: 0xff,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: String::new(),
        };
        let mut app = new_classic_menu_app(640, 480);
        app.startup_player_models = ["Thomas", "Ada", "tina", "Tori"]
            .map(player)
            .into_iter()
            .collect();
        app.open_player_selection_dialog();

        for (character, expected) in [('T', 2), ('T', 3), ('t', 0)] {
            app.handle_text_input(character)
                .expect("route list character");
            assert_eq!(
                app.startup_player_dialog
                    .as_ref()
                    .expect("player dialog")
                    .selected_index(),
                Some(expected)
            );
        }
        app.handle_text_input('T').expect("cycle to next T row");
        let (selected, anchor) = app
            .startup_player_dialog
            .as_ref()
            .expect("player dialog")
            .keyboard_context_target()
            .expect("focused selected row");
        assert_eq!(selected, 2);
        app.startup_player_dialog
            .as_mut()
            .expect("player dialog")
            .set_pointer_position(Some(GuiPoint::new(639.0, 479.0)));

        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open selected-row context menu");
        let popup = app.context_menu.as_ref().expect("keyboard context menu");
        let panel = &popup.layout().panels[0];
        assert_eq!(panel.rows.len(), 2);
        assert_eq!(panel.selected, None);
        assert_eq!(
            (panel.bounds.x, panel.bounds.y),
            (anchor.x as i32, anchor.y as i32)
        );
        app.close_context_menu_silently();

        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("move focus away from list");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release Tab");
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("Apps outside list focus is inert");
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn new_player_properties_defaults_match_classic_choices() {
        let mut app = new_classic_menu_app(640, 480);
        let portrait = ImageData::new(
            150,
            150,
            [0_u8, 0, 255, 255]
                .into_iter()
                .cycle()
                .take(150 * 150 * 4)
                .collect(),
        );
        Arc::get_mut(&mut app.assets)
            .expect("frontend assets are app-owned")
            .startup_dialog_images
            .insert("Portrait5.png".to_string(), portrait);

        let controller = app.new_startup_player_properties_controller(7, 4);
        let player = controller.player();
        // No language table is installed here, so both seeds come from their
        // English fallbacks rather than C++'s hardcoded German "Neuling".
        assert_eq!(player.name, "Novice");
        assert_eq!(controller.comment(), "I'm new.");
        assert_eq!(player.pref_color, 7);
        assert_eq!(player.pref_color_dw, 0xf08050);
        assert_eq!(player.pref_control, 0);
        assert!(player.pref_mouse);
        assert!(player.pref_control_style);
        assert!(player.pref_auto_context_menu);
        assert_eq!(
            controller
                .portrait_preview()
                .map(|image| (image.width(), image.height())),
            Some((150, 150))
        );
        assert!(controller
            .big_icon_preview()
            .is_some_and(|image| { image.width() <= 64 && image.height() <= 64 }));
    }

    #[test]
    fn new_player_dialog_seeds_the_name_from_the_localized_first_player_rank() {
        // C4PlayerInfoCore::Default hardcodes the German "Neuling"
        // (C4InfoCore.cpp:69) and C++ seeds the new-player dialog from it even
        // for English players. "Neuling" is rank 0 of IDS_RANKS_PLAYER, whose
        // shipped English ladder starts "Novice" (LanguageUS.txt:1280), so seed
        // the edit box from the localized ladder rather than showing German on
        // the first screen a new player sees. The Player.txt write default and
        // the missing-`Name=` read fallback stay "Neuling" for file parity.
        let mut app = new_menu_app(320, 240);
        app.startup_tooltip_resources.insert(
            "IDS_RANKS_PLAYER".to_string(),
            "Novice|Beginner|Adept".to_string(),
        );

        assert_eq!(
            app.new_startup_player_properties_controller(0, 0)
                .player()
                .name,
            "Novice"
        );
    }

    #[test]
    fn new_player_dialog_still_seeds_neuling_from_the_german_rank_ladder() {
        // The English seed must not be hardcoded: a DE language pack ships
        // "Neuling" as rank 0 (LanguageDE.txt:1279), so a German-configured
        // player keeps exactly the C++ wording.
        let mut app = new_menu_app(320, 240);
        app.startup_tooltip_resources.insert(
            "IDS_RANKS_PLAYER".to_string(),
            "Neuling|Anfänger|Tunichtgut".to_string(),
        );

        assert_eq!(
            app.new_startup_player_properties_controller(0, 0)
                .player()
                .name,
            "Neuling"
        );
    }

    #[test]
    fn l035_portrait_selector_uses_and_persists_last_folder_index() {
        let _lock = env_lock().lock();
        reset_cached_app_paths();
        let program_data = tempdir().expect("portrait program data");
        let user_data = tempdir().expect("portrait user data");
        fs::create_dir_all(program_data.path().join("planet/System.c4g"))
            .expect("create program path marker");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(program_data.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover portrait paths");
        paths.ensure_user_dirs().expect("create portrait user path");
        write_preview_image(
            &paths.user_data_dir().join("Custom.PNG"),
            [12, 34, 56, 255],
            image::ImageFormat::Png,
        );
        write_preview_image(
            &program_data.path().join("Program.BMP"),
            [65, 43, 21, 255],
            image::ImageFormat::Bmp,
        );
        persist_native_config_values(
            &paths,
            "Startup",
            &[(
                "LastPortraitFolderIdx",
                clonk_app_netplay::NativeConfigValue::RawAscii("1"),
            )],
        )
        .expect("seed remembered portrait location");

        let mut app = new_classic_menu_app(640, 480);
        app.app_paths = Some(paths.clone());
        app.open_new_startup_player_properties();
        app.process_startup_player_properties_actions(vec![
            clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
        ]);
        let selector = app
            .startup_player_properties_dialog
            .as_ref()
            .and_then(|pending| pending.controller.portrait_selector())
            .expect("Picture opens the remembered portrait location");
        assert_eq!(selector.current_location_index(), 1);
        assert!(selector
            .items()
            .iter()
            .any(|item| item.filename() == Some("Program.BMP")));

        for _ in 0..6 {
            let actions = app
                .startup_player_properties_dialog
                .as_mut()
                .expect("properties remain open")
                .controller
                .handle_key_down(KeyCode::Tab);
            assert!(actions.is_empty());
        }
        // C4GuiDialogs.cpp:386-421 and C4FileSelDlg.cpp:162-169,564-572
        // put Location after six forward focus steps. C4GuiComboBox.cpp:66-86
        // and C4GuiMenu.cpp:240-299 then open, highlight, and choose row zero.
        for key in [KeyCode::Down, KeyCode::Down] {
            let actions = app
                .startup_player_properties_dialog
                .as_mut()
                .expect("properties remain open")
                .controller
                .handle_key_down(key);
            assert!(actions.is_empty());
        }
        let actions = app
            .startup_player_properties_dialog
            .as_mut()
            .expect("properties remain open")
            .controller
            .handle_key_down(KeyCode::Enter);
        app.process_startup_player_properties_actions(actions);
        let selector = app
            .startup_player_properties_dialog
            .as_ref()
            .and_then(|pending| pending.controller.portrait_selector())
            .expect("selector remains open after changing location");
        assert_eq!(selector.current_location_index(), 0);
        assert!(selector
            .items()
            .iter()
            .any(|item| item.filename() == Some("Custom.PNG")));
        // C4FileSelDlg.cpp:189-194 changes the active path immediately, while
        // C4FileSelDlg.cpp:575-580 remembers its row only from OnClosed.
        assert_eq!(
            clonk_app_netplay::configured_native_value(
                &fs::read(paths.config_file()).expect("read portrait config"),
                "Startup",
                "LastPortraitFolderIdx",
            )
            .expect("portrait location is not persisted before close")
            .as_bytes(),
            b"1"
        );

        let actions = app
            .startup_player_properties_dialog
            .as_mut()
            .expect("properties remain open")
            .controller
            .handle_key_down(KeyCode::Escape);
        assert_eq!(
            actions,
            vec![
                clonk_frontend::startup_plrproperties::PlayerPropertiesAction::
                    PortraitSelectorClosed { location_index: 0 }
            ],
            "C4FileSelDlg.cpp:209-228 and 575-580 remember the current row on Cancel"
        );
        app.process_startup_player_properties_actions(actions);
        assert_eq!(
            clonk_app_netplay::configured_native_value(
                &fs::read(paths.config_file()).expect("read portrait config after close"),
                "Startup",
                "LastPortraitFolderIdx",
            )
            .expect("persisted portrait location after close")
            .as_bytes(),
            b"0"
        );
        persist_native_config_values(
            &paths,
            "Startup",
            &[(
                "LastPortraitFolderIdx",
                clonk_app_netplay::NativeConfigValue::RawAscii("1"),
            )],
        )
        .expect("simulate stale disk state after a failed close-time save");
        app.process_startup_player_properties_actions(vec![
            clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
        ]);
        assert_eq!(
            app.startup_player_properties_dialog
                .as_ref()
                .and_then(|pending| pending.controller.portrait_selector())
                .expect("selector reopens at the persisted location")
                .current_location_index(),
            0,
            "C++ keeps the close-time config row in memory even when disk persistence fails \
             (`C4FileSelDlg.cpp:575-580`)"
        );
        reset_cached_app_paths();
    }

    #[test]
    fn player_new_properties_enter_f2_and_insert_open_the_modal() {
        let mut app = new_real_classic_menu_app(640, 480);
        let model = clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: "Entry Player".to_string(),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: 0xff,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: "entry".to_string(),
        };
        app.startup_player_files.push(StartupPlayerFile {
            path: PathBuf::from("Entry Player.c4p"),
            file_name: "Entry Player.c4p".to_string(),
            player_file: PlayerFile {
                name: "Entry Player".to_string(),
                ..PlayerFile::default()
            },
            render_model: model.clone(),
        });
        app.startup_player_models.push(model);
        app.open_player_selection_dialog();

        app.process_player_dialog_actions(vec![
            clonk_frontend::startup_plrsel::PlrSelAction::NewPlayer,
        ])
        .expect("New opens editor");
        assert!(matches!(
            app.startup_player_properties_dialog
                .as_ref()
                .map(|pending| pending.controller.mode()),
            Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::New)
        ));
        let mut frame = vec![0; 640 * 480 * 4];
        app.render(&mut frame)
            .expect("the player-properties modal renders over selection");
        app.startup_player_properties_dialog = None;

        for key in [VirtualKeyCode::Return, VirtualKeyCode::F2] {
            app.handle_key(key, ElementState::Pressed)
                .expect("existing-player shortcut opens editor");
            assert!(matches!(
                app.startup_player_properties_dialog
                    .as_ref()
                    .map(|pending| pending.controller.mode()),
                Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 0 })
            ));
            app.startup_player_properties_dialog = None;
            app.handle_key(key, ElementState::Released)
                .expect("release shortcut");
        }

        app.handle_key(VirtualKeyCode::Insert, ElementState::Pressed)
            .expect("Insert opens new-player editor");
        assert!(matches!(
            app.startup_player_properties_dialog
                .as_ref()
                .map(|pending| pending.controller.mode()),
            Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::New)
        ));
    }

    #[test]
    fn options_program_focus_traverses_every_control_without_a_boundary() {
        use clonk_frontend::startup_options_dlg::OptionsProgramFocusTarget;

        let mut app = new_classic_menu_app(640, 480);
        app.open_options_menu();
        let expected = [
            OptionsProgramFocusTarget::LanguageCombo,
            OptionsProgramFocusTarget::FontFaceCombo,
            OptionsProgramFocusTarget::FontSizeCombo,
            OptionsProgramFocusTarget::WhiteChatIngame,
            OptionsProgramFocusTarget::WhiteChatLobby,
            OptionsProgramFocusTarget::ShowLogTimestamps,
            OptionsProgramFocusTarget::Preloading,
            OptionsProgramFocusTarget::ResetButton,
            OptionsProgramFocusTarget::AdvancedButton,
        ];
        for target in expected {
            app.handle_gamepad_direction(
                GamepadSlot::new(0),
                ControlButton::Right,
                ElementState::Pressed,
            )
            .unwrap_or_else(|error| panic!("focus {target:?}: {error}"));
            assert_eq!(
                app.startup_options_dialog
                    .as_ref()
                    .expect("options state")
                    .focused_program_control(),
                Some(target)
            );
        }

        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )
        .expect("wrap forward to Back");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .focused_program_control(),
            None
        );
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Left,
            ElementState::Pressed,
        )
        .expect("wrap backward to Advanced");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .focused_program_control(),
            Some(OptionsProgramFocusTarget::AdvancedButton)
        );
    }

    #[test]
    fn show_folder_maps_config_defaults_on_and_reads_an_explicit_false() {
        assert!(load_show_folder_maps(None));
        let user_data = tempdir().expect("ShowFolderMaps config");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "Graphics", "ShowFolderMaps", "0").expect("disable folder maps");
        assert!(!load_show_folder_maps(Some(&paths)));
    }

    #[test]
    fn packed_logical_folder_map_uses_case_insensitive_group_traversal() {
        let root = tempdir().expect("packed FolderMap fixture");
        let png_path = root.path().join("map.png");
        write_map_png(&png_path, 2, 2, [9, 8, 7, 255]);
        let png = fs::read(&png_path).expect("packed map PNG");
        let inner = packed_test_group(&[
            ("fOlDeRmAp.TxT", false, b"[FolderMap]\n"),
            ("FolderMap.png", false, png.as_slice()),
        ]);
        let outer = packed_test_file_group(&[("INNER.C4F", true, inner.as_slice())]);
        let outer_path = root.path().join("Outer.c4f");
        fs::write(&outer_path, outer).expect("packed outer folder");
        let logical_inner = outer_path.join("inner.c4f");

        let mut inner_entry = FrontendScenario::fallback();
        inner_entry.identifier = "Outer.c4f/inner.c4f".to_string();
        inner_entry.kind = ScenarioKind::Folder;
        inner_entry.is_playable = false;
        inner_entry.path = Some(logical_inner.clone());
        let mut outer_entry = FrontendScenario::fallback();
        outer_entry.identifier = "Outer.c4f".to_string();
        outer_entry.kind = ScenarioKind::Folder;
        outer_entry.is_playable = false;
        outer_entry.path = Some(outer_path);
        outer_entry.children = vec![inner_entry.clone()];
        let entries = vec![outer_entry];
        let menu = StartupMenu::new(build_menu_entries(&entries, false), test_font(), None)
            .expect("packed ancestry menu");
        let mut state = MenuState::new(menu, entries);
        state.enter_folder("Outer.c4f");
        state.enter_folder(&inner_entry.identifier);
        assert!(state.configure_current_folder_map(
            true,
            640,
            480,
            &MissionAccessStore::default(),
            &["US".to_string()],
        ));
        assert_eq!(
            state.current_map().expect("packed map").source_path,
            logical_inner
        );
    }

    #[test]
    fn editor_kind_and_edit_action_return_typed_boundaries() {
        let mut editor = FrontendScenario::fallback();
        editor.identifier = "Editor.c4s".to_string();
        editor.title = "Editor".to_string();
        editor.kind = ScenarioKind::Editor;
        let scenarios = vec![editor.clone()];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("editor menu");
        let mut app = new_menu_app(640, 480);
        app.menu_state = MenuState::new(menu, scenarios);
        let summary = clonk_frontend::ScenarioSummary {
            identifier: editor.identifier.clone(),
            title: editor.title.clone(),
            kind: ScenarioKind::Editor,
        };

        for action in [
            StartupMenuAction::OpenEntry(summary.clone()),
            StartupMenuAction::StartScenario(summary.clone()),
        ] {
            assert!(matches!(
                app.process_menu_actions(vec![action]),
                Err(ClassicParityBoundary::EditorScenario { ref identifier })
                    if identifier == &editor.identifier
            ));
        }
        assert!(matches!(
            app.process_menu_actions(vec![StartupMenuAction::EditEntry(summary)]),
            Err(ClassicParityBoundary::EditScenario { ref identifier })
                if identifier == &editor.identifier
        ));
    }

    #[test]
    fn empty_discovery_and_catalog_never_inject_player_facing_sandbox() {
        assert!(build_scenario_catalog(&[]).is_empty());

        let invalid_install = tempdir().expect("install without System.c4g");
        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(invalid_install.path()))]);
        let discovered = load_frontend_scenarios();
        assert!(discovered.is_empty());
        assert!(!build_scenario_catalog(&discovered).contains_key("rust_sandbox"));
    }

    #[test]
    fn game_init_preserves_raw_cpp_participant_config() {
        // C4Config and C4Game retain legacy bytes; startup participant
        // validation changes only the in-memory Participants list and must not
        // reject or UTF-8-reencode an unrelated raw General.Name byte
        // (pristine 9ffa0a5d src/C4StartupMainDlg.cpp:174-199;
        // src/C4Game.cpp:361-364).
        let _lock = env_lock().lock();
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        paths.ensure_user_dirs().expect("create config directory");
        let player = user_data.path().join("Players/Alice.c4p");
        fs::create_dir_all(&player).expect("create player group");
        let mut raw = b"[General]\nName=\"M\x80ker\"\nParticipants=\"".to_vec();
        raw.extend_from_slice(player.as_os_str().as_encoded_bytes());
        raw.extend_from_slice(b"\"\n");
        fs::write(paths.config_file(), &raw).expect("write legacy-byte config");

        let mut app = GameApp::new(
            320,
            200,
            AudioOptions {
                sound_enabled: false,
                music_enabled: false,
                menu_music_enabled: false,
                menu_sound_enabled: false,
                ..AudioOptions::default()
            },
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialize from legacy-byte config");
        wait_for_menu(&mut app);

        assert_eq!(fs::read(paths.config_file()).expect("read config"), raw);
    }

    #[test]
    fn options_reset_confirmation_replaces_config_and_requests_clean_exit() {
        use clonk_frontend::message_dialog::{
            MessageDialogButton, MessageDialogButtons, MessageDialogIcon, MessageDialogResult,
        };
        use clonk_frontend::startup_options_dlg::OptionsDlgAction;

        let _lock = env_lock().lock();
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        paths.ensure_user_dirs().expect("create user directories");
        fs::write(
                paths.config_file(),
                "[General]\nFontName=Endeavour\nFontSize=28\nVendorResetKey=remove\n[Graphics]\nScale=250\n",
            )
            .expect("seed reset config");
        let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths))
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_options_menu();
        app.startup_options_dialog
            .as_mut()
            .unwrap()
            .program_mut()
            .preloading = true;
        let before_cancel = fs::read(paths.config_file()).expect("read config before reset prompt");

        app.process_options_dialog_actions(vec![OptionsDlgAction::ResetConfiguration])
            .expect("open reset confirmation");
        let modal = app.message_dialogs.last().expect("reset modal");
        assert_eq!(modal.state.caption(), "Reset configuration");
        assert_eq!(
                modal.state.message(),
                "Are you sure you want to reset all configuration values?|For changes to take effect the program has to be restarted."
            );
        assert_eq!(modal.state.buttons(), MessageDialogButtons::YES_NO);
        assert_eq!(modal.state.icon(), MessageDialogIcon::NOTIFY);
        assert_eq!(modal.state.focused_button(), Some(MessageDialogButton::Yes));
        app.finish_message_dialog(MessageDialogResult::No)
            .expect("cancel reset");
        assert_eq!(fs::read(paths.config_file()).unwrap(), before_cancel);
        assert!(!app.configuration_reset_requested);
        assert!(!app.take_exit_request());

        app.process_options_dialog_actions(vec![OptionsDlgAction::ResetConfiguration])
            .expect("reopen reset confirmation");
        app.finish_message_dialog(MessageDialogResult::Yes)
            .expect("confirm reset");
        let reset = Config::load(paths.config_file()).expect("load reset config");
        assert_eq!(reset.get_in(Some("General"), "VendorResetKey"), None);
        assert_eq!(reset.get_in(Some("General"), "FontSize"), None);
        assert_eq!(reset.get_in(Some("Graphics"), "Scale"), None);
        assert!(app.configuration_reset_requested);
        assert!(app.take_exit_request());
        assert_eq!(app.startup_view, StartupView::Options);
    }

    #[test]
    fn options_ctrl_tab_traverses_all_six_live_sheets_without_a_boundary() {
        use clonk_frontend::startup_options_dlg::OptionsSheet;

        let mut app = new_classic_menu_app(640, 480);
        app.open_options_menu();
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("hold Ctrl");
        for expected in [
            OptionsSheet::Graphics,
            OptionsSheet::Sound,
            OptionsSheet::Keyboard,
            OptionsSheet::Gamepad,
            OptionsSheet::Network,
            OptionsSheet::Program,
        ] {
            app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .unwrap_or_else(|error| panic!("open {expected:?}: {error}"));
            app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("release Ctrl+Tab");
            assert_eq!(
                app.startup_options_dialog
                    .as_ref()
                    .expect("options dialog")
                    .active_sheet(),
                expected
            );
        }
    }

    #[test]
    fn l079_options_control_set_digit_hotkeys_require_alt_and_respect_visible_sets() {
        use clonk_frontend::startup_options_controls::ControlDevice;
        use clonk_frontend::startup_options_dlg::{OptionsDlgAction, OptionsSheet};

        let mut app = new_classic_menu_app(640, 480);
        app.open_options_menu();
        let controls = load_options_control_state(
            &app.bindings,
            &app.gamepad_bindings,
            3,
            app.gamepad_gui_control,
        );
        let dialog = app.startup_options_dialog.as_mut().expect("options dialog");
        *dialog.controls_mut() = controls;
        dialog.restore_sheet(OptionsSheet::Keyboard);

        app.handle_key(VirtualKeyCode::Key2, ElementState::Pressed)
            .expect("bare digit is inert");
        app.handle_key(VirtualKeyCode::Key2, ElementState::Released)
            .expect("release bare digit");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Keyboard),
            0
        );

        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt");
        app.handle_key(VirtualKeyCode::Key2, ElementState::Pressed)
            .expect("select Keyboard 2");
        app.handle_key(VirtualKeyCode::Key2, ElementState::Released)
            .expect("release Alt+2");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Keyboard),
            1
        );
        app.handle_key(VirtualKeyCode::Numpad1, ElementState::Pressed)
            .expect("SDL Keypad mnemonic is not a digit mnemonic");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Keyboard),
            1
        );

        app.handle_modifiers_changed(ModifiersState::ALT | ModifiersState::SHIFT)
            .expect("hold Alt+Shift");
        app.handle_key(VirtualKeyCode::Key4, ElementState::Pressed)
            .expect("select Keyboard 4 with shifted mnemonic mask");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Keyboard),
            3
        );
        app.handle_modifiers_changed(ModifiersState::ALT | ModifiersState::CTRL)
            .expect("hold unsupported Ctrl+Alt mask");
        app.handle_key(VirtualKeyCode::Key1, ElementState::Pressed)
            .expect("Ctrl+Alt digit is inert");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Keyboard),
            3
        );

        app.startup_options_dialog
            .as_mut()
            .unwrap()
            .restore_sheet(OptionsSheet::Gamepad);
        app.process_options_dialog_actions(vec![OptionsDlgAction::SheetChanged(OptionsSheet::Gamepad)])
            .expect("enter Gamepad sheet");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt on Gamepad sheet");
        app.handle_key(VirtualKeyCode::Key3, ElementState::Pressed)
            .expect("select visible Gamepad 3");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Gamepad),
            2
        );
        assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(2)));

        for key in [VirtualKeyCode::Key4, VirtualKeyCode::Key0] {
            app.handle_key(key, ElementState::Pressed)
                .expect("disconnected Gamepad mnemonic is inert");
        }
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Gamepad),
            2
        );
        assert_eq!(app.gamepads.options_open_slot(), Some(GamepadSlot::new(2)));
    }

    #[test]
    fn l079_options_control_set_hotkeys_do_not_leak_through_modals() {
        use clonk_frontend::message_dialog::MessageDialogResult;
        use clonk_frontend::startup_options_controls::ControlDevice;
        use clonk_frontend::startup_options_dlg::{OptionsDlgAction, OptionsSheet};

        let mut app = new_classic_menu_app(640, 480);
        app.open_options_menu();
        let dialog = app.startup_options_dialog.as_mut().expect("options dialog");
        dialog.restore_sheet(OptionsSheet::Keyboard);
        assert!(dialog.controls_mut().select_set(ControlDevice::Keyboard, 3));
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt");

        app.process_options_dialog_actions(vec![OptionsDlgAction::ResetConfiguration])
            .expect("open reset confirmation above Options");
        app.handle_key(VirtualKeyCode::Key2, ElementState::Pressed)
            .expect("message modal owns the unmatched mnemonic");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Keyboard),
            3
        );
        app.finish_message_dialog(MessageDialogResult::No)
            .expect("dismiss reset confirmation");

        app.process_options_dialog_actions(vec![OptionsDlgAction::OpenGraphicsScaleText])
            .expect("open input modal above Options");
        app.handle_key(VirtualKeyCode::Key2, ElementState::Pressed)
            .expect("input modal owns the unmatched mnemonic");
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .unwrap()
                .controls()
                .selected_set(ControlDevice::Keyboard),
            3
        );
    }

    #[test]
    fn options_close_reports_disk_write_failure() {
        use clonk_frontend::message_dialog::MessageDialogIcon;

        let mut app = new_classic_menu_app(640, 480);
        app.open_options_menu();
        app.close_options_menu_with_persist_result(Some(Err(io::Error::other(
            "simulated config write failure",
        ))))
        .expect("show config save failure and close Options");

        assert_eq!(app.startup_view, StartupView::MainMenu);
        let error = app
            .message_dialogs
            .last()
            .expect("config save failure dialog remains above the main menu");
        assert_eq!(error.state.caption(), "Configuration error");
        assert_eq!(
            error.state.message(),
            "Could not save configuration: simulated config write failure"
        );
        assert_eq!(error.state.icon(), MessageDialogIcon::ERROR);
        assert!(matches!(
            error.continuation,
            MessageDialogContinuation::None
        ));
    }

    #[test]
    fn options_language_loads_real_de_and_selection_reloads_and_persists() {
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        paths.ensure_user_dirs().expect("create user directories");
        fs::write(
            paths.config_file(),
            "[General]\nLanguage=DE - Deutsch\nLanguageEx=DE\n",
        )
        .expect("seed DE config");

        let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths))
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_options_menu();

        let program = app
            .startup_options_dialog
            .as_ref()
            .expect("options dialog")
            .program();
        assert_eq!(program.language_text, "DE - Deutsch");
        assert_eq!(
            program.language_info,
            "Original-Sprachpaket von RedWolf Design."
        );
        assert_eq!(
            program.no_language_info,
            "Sprachpaket nicht verf\u{00fc}gbar."
        );
        let mut codes = program
            .language_infos
            .iter()
            .map(|info| info.code.as_str())
            .collect::<Vec<_>>();
        codes.sort_unstable();
        assert_eq!(codes, vec!["DE", "US"]);
        assert_eq!(app.needed_material_need, "%s|braucht noch");
        assert_eq!(app.object_no_dig, "%s kann|nicht graben.");
        assert_eq!(
            app.default_rank_names
                .as_deref()
                .and_then(|names| names.get(1))
                .map(String::as_str),
            Some("Fähnrich")
        );
        assert_eq!(app.loaded_default_rank_names, app.default_rank_names);
        assert_eq!(app_default_rank_promotion_name(&app), "Fähnrich");

        app.process_options_dialog_actions(vec![
            clonk_frontend::startup_options_dlg::OptionsDlgAction::OpenLanguageCombo,
        ])
        .expect("open language combo");
        assert!(app.context_menu.is_some());
        app.process_context_menu_outcome(ContextMenuOutcome {
            captured: true,
            pass_through: false,
            focus_suppressed: true,
            events: vec![
                ContextMenuEvent::Closed,
                ContextMenuEvent::Activated(AppContextMenuCommand::OptionsLanguage("US".to_string())),
            ],
        })
        .expect("select US language");

        let program = app
            .startup_options_dialog
            .as_ref()
            .expect("recreated options dialog")
            .program();
        assert_eq!(program.language, "US");
        assert_eq!(program.language_text, "US - English");
        assert_eq!(program.language_ex, "US,DE");
        assert_eq!(app.needed_material_need, "%s|needs");
        assert_eq!(app.object_no_dig, "%s cannot dig.");
        assert_eq!(
            app.default_rank_names
                .as_deref()
                .and_then(|names| names.get(1))
                .map(String::as_str),
            Some("Fähnrich")
        );
        assert_eq!(
            app.loaded_default_rank_names
                .as_deref()
                .and_then(|names| names.get(1))
                .map(String::as_str),
            Some("Ensign")
        );
        assert_eq!(app_default_rank_promotion_name(&app), "Fähnrich");

        app.return_to_menu();
        assert_eq!(app.default_rank_names, app.loaded_default_rank_names);
        assert_eq!(app_default_rank_promotion_name(&app), "Ensign");

        let config = Config::load(paths.config_file()).expect("reload selected config");
        assert_eq!(config.get_in(Some("General"), "Language"), Some("US"));
        assert_eq!(config.get_in(Some("General"), "LanguageEx"), Some("US,DE"));
        assert_eq!(config.get_in(Some("General"), "LanguageCharset"), Some(""));
    }

    #[test]
    fn options_non_tab_gui_bindings_require_the_exact_bare_modifier_mask() {
        use clonk_frontend::startup_options_dlg::{OptionsSheet, SoundCheckboxId};

        let modifier_masks = [
            ModifiersState::ALT,
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL,
            ModifiersState::ALT | ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
        ];

        let mut checkbox = new_running_sandbox_app();
        checkbox.return_to_menu();
        enter_unported_startup_subscreen(
            &mut checkbox,
            ClassicStartupSubscreen::Options(OptionsSheet::Sound),
        );
        for modifiers in modifier_masks {
            checkbox
                .handle_modifiers_changed(modifiers)
                .expect("set exact C++ modifier mask");
            for key in [
                VirtualKeyCode::Up,
                VirtualKeyCode::Down,
                VirtualKeyCode::Left,
                VirtualKeyCode::Back,
                VirtualKeyCode::Escape,
                VirtualKeyCode::Right,
            ] {
                checkbox
                    .handle_key(key, ElementState::Pressed)
                    .unwrap_or_else(|error| panic!("modified {key:?} down: {error}"));
                checkbox
                    .handle_key(key, ElementState::Released)
                    .unwrap_or_else(|error| panic!("modified {key:?} up: {error}"));
            }
            assert_eq!(checkbox.startup_view, StartupView::Options);
            assert_eq!(
                checkbox
                    .startup_options_dialog
                    .as_ref()
                    .expect("Options model")
                    .active_sheet(),
                OptionsSheet::Sound,
                "modified Up/Down must not switch sheets"
            );
        }

        checkbox
            .handle_modifiers_changed(ModifiersState::empty())
            .expect("clear modifiers");
        checkbox
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("bare Tab focuses FE Music");
        checkbox
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release bare Tab");
        checkbox
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("bare Tab focuses FE Sound Effects");
        checkbox
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release second bare Tab");
        assert_eq!(
            checkbox
                .startup_options_dialog
                .as_ref()
                .expect("Options model")
                .focused_sound_checkbox(),
            Some(SoundCheckboxId::FrontendSoundEffects)
        );
        let before_checkbox = checkbox
            .startup_options_dialog
            .as_ref()
            .expect("Options model")
            .sound()
            .clone();
        for modifiers in modifier_masks {
            checkbox
                .handle_modifiers_changed(modifiers)
                .expect("set exact C++ modifier mask");
            for key in [
                VirtualKeyCode::Space,
                VirtualKeyCode::Left,
                VirtualKeyCode::Back,
                VirtualKeyCode::Escape,
            ] {
                checkbox
                    .handle_key(key, ElementState::Pressed)
                    .unwrap_or_else(|error| panic!("modified {key:?} down: {error}"));
                checkbox
                    .handle_key(key, ElementState::Released)
                    .unwrap_or_else(|error| panic!("modified {key:?} up: {error}"));
            }
            assert_eq!(checkbox.startup_view, StartupView::Options);
            assert_eq!(
                checkbox
                    .startup_options_dialog
                    .as_ref()
                    .expect("Options model")
                    .sound(),
                &before_checkbox,
                "modified Space must not toggle the focused checkbox"
            );
        }

        checkbox
            .handle_modifiers_changed(ModifiersState::LOGO)
            .expect("Logo is outside the C++ modifier mask");
        checkbox
            .handle_key(VirtualKeyCode::Space, ElementState::Pressed)
            .expect("Logo+Space remains the bare checkbox binding");
        assert_ne!(
            checkbox
                .startup_options_dialog
                .as_ref()
                .expect("Options model")
                .sound(),
            &before_checkbox
        );

        let mut back = new_running_sandbox_app();
        back.return_to_menu();
        enter_unported_startup_subscreen(
            &mut back,
            ClassicStartupSubscreen::Options(OptionsSheet::Sound),
        );
        back.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift for reverse traversal");
        back.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Shift+Tab focuses Back");
        back.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release Shift+Tab");
        for modifiers in modifier_masks {
            back.handle_modifiers_changed(modifiers)
                .expect("set exact C++ modifier mask");
            for key in [
                VirtualKeyCode::Return,
                VirtualKeyCode::NumpadEnter,
                VirtualKeyCode::Space,
            ] {
                back.handle_key(key, ElementState::Pressed)
                    .unwrap_or_else(|error| panic!("modified Back {key:?} down: {error}"));
                back.handle_key(key, ElementState::Released)
                    .unwrap_or_else(|error| panic!("modified Back {key:?} up: {error}"));
            }
            assert_eq!(
                back.startup_view,
                StartupView::Options,
                "modified Enter/Space must not activate Back"
            );
        }
    }

    #[test]
    fn scenario_preset_replaces_seed_while_fixed_selection_wins_publication() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("definition selection user data");
        let content = tempdir().expect("definition selection content");
        let content_root = content.path();
        fs::create_dir_all(content_root.join("Material.c4g")).expect("global material group");
        let custom_root = content_root.join("Custom");
        for (module, id) in [
            ("Seed.c4d", "SEED"),
            ("Preset.c4d", "PSET"),
            ("FixedA.c4d", "FIXA"),
            ("FixedB.c4d", "FIXB"),
        ] {
            install_network_definition_pack(content_root, module, id);
            install_network_definition_pack(&custom_root, module, id);
        }
        let outer = content_root.join("Outer.c4f");
        install_network_definition_pack(&outer, "LocalOnly.c4d", "LOCL");
        let scenario_path = outer.join("Choice.c4s");
        fs::create_dir_all(&scenario_path).expect("selection scenario group");
        fs::write(
                scenario_path.join("Scenario.txt"),
                "[Head]\nTitle=Definition Choice\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=./Preset.c4d\n",
            )
            .expect("selection scenario core");

        let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content_root));
        persist_config_value(&paths, "General", "DefinitionPath", "Custom/")
            .expect("configure DefinitionPath");
        let app = new_menu_app_with_paths(640, 480, &paths);
        let custom_prefix = startup_definition_paths(&paths)
            .expect("read configured DefinitionPath")
            .active_custom_root
            .expect("activate configured DefinitionPath prefix");
        let frontend = FrontendScenario {
            identifier: "Outer.c4f/Choice.c4s".to_string(),
            title: "Definition Choice".to_string(),
            path: Some(scenario_path.clone()),
            ..FrontendScenario::fallback()
        };
        let seed = app
            .prepare_network_host_scenario(
                frontend.clone(),
                ScenarioDefinitionLoad::Seed {
                    modules: vec!["Seed.c4d".to_string()],
                    definition_root: Some(custom_prefix.clone()),
                },
            )
            .expect("stage scenario-preset host");
        let mismatch = build_network_host_preparation(
            &app,
            &seed.frontend,
            &seed.definition_load,
            &seed.effective_definition_modules,
            &[],
            Some((&seed.definition_executable_path, &seed.definition_path)),
            Some((&seed.lobby.local_name, &seed.lobby.nick)),
        )
        .expect("build mismatched staged definition probe")
        .prepare()
        .expect_err("host preparation rejects a changed staged definition vector");
        assert!(matches!(
            mismatch,
            prepared_host_bootstrap::PrepareHostBootstrapError::StagedDefinitionResourcesChanged {
                staged,
                prepared,
            } if staged.is_empty()
                && prepared
                    == vec![
                        custom_root.join("Preset.c4d"),
                        content_root.join("Preset.c4d"),
                        outer.clone(),
                    ]
        ));
        let seed_prepared = prepare_staged_network_host(&app, &seed);
        assert_eq!(
            published_definition_wire_names(&seed_prepared),
            vec![
                b"Custom/./Preset.c4d".to_vec(),
                b"./Preset.c4d".to_vec(),
                b"Outer.c4f".to_vec(),
            ],
            "a nonempty scenario preset replaces the seed before rooted/local expansion"
        );
        fs::write(
                scenario_path.join("Scenario.txt"),
                "[Head]\nTitle=Definition Choice\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Preset.c4d\n",
            )
            .expect("change only the scenario preset spelling after staging");
        let changed_spelling = build_network_host_preparation(
            &app,
            &seed.frontend,
            &seed.definition_load,
            &seed.effective_definition_modules,
            &seed.definition_resources,
            Some((&seed.definition_executable_path, &seed.definition_path)),
            Some((&seed.lobby.local_name, &seed.lobby.nick)),
        )
        .expect("build changed-spelling probe")
        .prepare()
        .expect_err("host preparation rejects changed staged publication spellings");
        assert!(matches!(
            changed_spelling,
            prepared_host_bootstrap::PrepareHostBootstrapError::StagedDefinitionPublicationChanged {
                staged,
                prepared,
            } if staged != prepared
        ));
        fs::write(
                scenario_path.join("Scenario.txt"),
                "[Head]\nTitle=Definition Choice\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Seed.c4d\n",
            )
            .expect("change scenario preset after staging");
        let changed_selection = build_network_host_preparation(
            &app,
            &seed.frontend,
            &seed.definition_load,
            &seed.effective_definition_modules,
            &seed.definition_resources,
            Some((&seed.definition_executable_path, &seed.definition_path)),
            Some((&seed.lobby.local_name, &seed.lobby.nick)),
        )
        .expect("build changed-selection probe")
        .prepare()
        .expect_err("host preparation rejects changed staged selection semantics");
        assert!(matches!(
            changed_selection,
            prepared_host_bootstrap::PrepareHostBootstrapError::StagedDefinitionSelectionChanged {
                staged,
                prepared,
            } if staged == vec!["Preset.c4d".to_owned()]
                && prepared == vec!["Seed.c4d".to_owned()]
        ));

        let fixed = app
            .prepare_network_host_scenario(
                frontend.clone(),
                ScenarioDefinitionLoad::Fixed {
                    modules: vec!["FixedB.c4d".to_string(), "./FixedA.c4d".to_string()],
                    definition_root: Some(custom_prefix.clone()),
                },
            )
            .expect("stage fixed-definition host");
        let fixed_prepared = prepare_staged_network_host(&app, &fixed);
        assert_eq!(
            published_definition_wire_names(&fixed_prepared),
            vec![
                b"Custom/FixedB.c4d".to_vec(),
                b"Custom/./FixedA.c4d".to_vec(),
                b"FixedB.c4d".to_vec(),
                b"./FixedA.c4d".to_vec(),
                b"Outer.c4f".to_vec(),
            ],
            "fixed selection stays authoritative and folder locals append exactly once"
        );
        assert_eq!(
            fixed_prepared.definition_modules(),
            [
                "Custom/FixedB.c4d",
                "Custom/./FixedA.c4d",
                "FixedB.c4d",
                "./FixedA.c4d",
                "Outer.c4f",
            ],
            "the pre-SetModules vector remains available after publication"
        );
        assert_eq!(
            activated_definition_load(
                Some(fixed_prepared.definition_modules().to_vec()),
                ScenarioDefinitionLoad::Fixed {
                    modules: vec!["final/retyped/paths.c4d".to_owned()],
                    definition_root: None,
                },
            ),
            ScenarioDefinitionLoad::Fixed {
                modules: vec![
                    "Custom/FixedB.c4d".to_owned(),
                    "Custom/./FixedA.c4d".to_owned(),
                    "FixedB.c4d".to_owned(),
                    "./FixedA.c4d".to_owned(),
                    "Outer.c4f".to_owned(),
                ],
                definition_root: None,
            },
            "activation retains Game.DefinitionFilenames instead of retyped resource paths"
        );
        let dynamic = fixed_prepared
            .host_config()
            .resource_files
            .iter()
            .find(|resource| {
                resource.core.resource_type == clonk_network::HostResourceType::Dynamic as u8
            })
            .expect("fixed dynamic resource");
        let scenario = Group::open(&dynamic.path)
            .expect("open fixed dynamic")
            .read_file("Scenario.txt")
            .expect("fixed dynamic Scenario.txt");
        let expected = b"Definitions=\"FixedB.c4d\",\"./FixedA.c4d\",\"FixedB.c4d\",\"./FixedA.c4d\",\"Outer.c4f\"";
        assert!(scenario
            .windows(expected.len())
            .any(|window| window == expected));

        let fixed_empty = app
            .prepare_network_host_scenario(
                frontend,
                ScenarioDefinitionLoad::Fixed {
                    modules: Vec::new(),
                    definition_root: Some(custom_prefix),
                },
            )
            .expect("stage fixed-empty definition host");
        let fixed_empty_prepared = prepare_staged_network_host(&app, &fixed_empty);
        assert_eq!(
            published_definition_wire_names(&fixed_empty_prepared),
            vec![b"Outer.c4f".to_vec()],
            "fixed-empty suppresses the nonempty preset while folder locals still append"
        );
    }

    #[test]
    fn player_delete_confirmation_removes_refreshes_and_reports_failure() {
        let _lock = env_lock().lock();
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let player_root = user_data.path().join("Players");
        let ada = player_root.join("Ada.c4p");
        fs::create_dir_all(&ada).expect("create directory player group");
        fs::write(
            ada.join("Player.txt"),
            "[Player]\nName=Ada\nTotalPlayingTime=36001\n\n[Preferences]\nColorDw=255\n",
        )
        .expect("write player core");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
        config.set_in(Some("General"), "Participants", ada.to_string_lossy());
        fs::create_dir_all(paths.config_file().parent().expect("config parent"))
            .expect("create config directory");
        config
            .save(paths.config_file())
            .expect("save player config");

        let mut app = GameApp::new(
            1280,
            720,
            AudioOptions {
                sound_enabled: false,
                music_enabled: false,
                menu_music_enabled: false,
                menu_sound_enabled: false,
                ..AudioOptions::default()
            },
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_player_selection_dialog();
        app.process_player_dialog_actions(vec![
            clonk_frontend::startup_plrsel::PlrSelAction::DeletePlayer(0),
        ])
        .expect("open delete confirmation");

        let confirm = &app.message_dialogs[0].state;
        assert_eq!(confirm.caption(), "Delete");
        assert_eq!(
                confirm.message(),
                "Do you really want to delete player Ada? - this player has a total playing time of 10:00:01!"
            );
        assert_eq!(
            confirm.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
        );
        assert_eq!(
            confirm.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM
        );
        assert_eq!(
            confirm.focused_button(),
            Some(clonk_frontend::message_dialog::MessageDialogButton::Yes)
        );

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
            .expect("decline deletion");
        assert!(ada.exists());
        assert_eq!(app.startup_player_files.len(), 1);

        app.process_player_dialog_actions(vec![
            clonk_frontend::startup_plrsel::PlrSelAction::DeletePlayer(0),
        ])
        .expect("reopen delete confirmation");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
            .expect("confirm deletion");
        assert!(!ada.exists());
        assert!(app.message_dialogs.is_empty());
        assert!(app.startup_player_files.is_empty());
        assert!(app.startup_player_models.is_empty());
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player controller")
                .selected_index(),
            None
        );
        assert_eq!(
            Config::load(paths.config_file())
                .expect("reload player config")
                .get_in(Some("General"), "Participants"),
            Some("")
        );

        let broken = player_root.join("Broken.c4p");
        fs::create_dir_all(&broken).expect("create failure player group");
        fs::write(
            broken.join("Player.txt"),
            "[Player]\nName=Broken\n\n[Preferences]\nColorDw=255\n",
        )
        .expect("write failure player core");
        app.refresh_startup_player_list();
        app.process_player_dialog_actions(vec![
            clonk_frontend::startup_plrsel::PlrSelAction::DeletePlayer(0),
        ])
        .expect("open failure confirmation");
        fs::remove_dir_all(&broken).expect("remove player before confirmation");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
            .expect("handle failed deletion");
        assert_eq!(app.message_dialogs.len(), 1);
        let failure = &app.message_dialogs[0].state;
        assert_eq!(failure.caption(), "Clear");
        assert_eq!(failure.message(), "Delete failure.");
        assert_eq!(
            failure.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::OK
        );
        assert_eq!(
            failure.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::ERROR
        );
        assert!(app.startup_player_files.is_empty());
        reset_cached_app_paths();
    }

    #[test]
    fn l026_unconfigured_stick_and_hat_emit_no_gameplay_controls() {
        let mut app = new_running_sandbox_app();
        app.gamepad_bindings = GamepadBindings::from_config(&Config::new());
        app.local_controls = LocalControlRegistry::default();
        app.local_controls.initialize(LocalControlInit {
            owner: app.local_owner,
            preferred_set: 4,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        let slot = GamepadSlot::new(0);

        app.process_gamepad_event_batch([GamepadEvent::Direction {
            slot,
            button: ControlButton::Left,
            state: ElementState::Pressed,
        }])
        .expect("route standalone semantic direction");
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("control-set four player")
                .control
                .pressed_coms,
            0,
            "semantic direction alone must not restore the hardwired gameplay path"
        );

        app.process_gamepad_event_batch([
            GamepadEvent::Axis {
                slot,
                axis: LegacyGamepadAxis::new(0, false),
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot,
                button: ControlButton::Left,
                state: ElementState::Pressed,
            },
            GamepadEvent::Axis {
                slot,
                axis: LegacyGamepadAxis::new(6, false),
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot,
                button: ControlButton::Left,
                state: ElementState::Pressed,
            },
        ])
        .expect("route unconfigured stick and hat axes");

        let pressed = app
            .engine
            .player(app.local_owner)
            .expect("control-set four player")
            .control
            .pressed_coms;
        assert_eq!(pressed, 0);
    }

    #[test]
    fn l026_axis_up_fires_dig_and_hat_zero_fires_configured_left() {
        let mut config = Config::new();
        config.set_in(
            Some("Gamepad0"),
            "Button6",
            input::legacy_gamepad_axis_key(0, 1, false)
                .expect("axis-up key")
                .to_string(),
        );
        config.set_in(
            Some("Gamepad0"),
            "Button7",
            input::legacy_gamepad_axis_key(0, 6, false)
                .expect("hat-zero-left key")
                .to_string(),
        );
        let mut app = new_running_sandbox_app();
        app.gamepad_bindings = GamepadBindings::from_config(&config);
        app.local_controls = LocalControlRegistry::default();
        app.local_controls.initialize(LocalControlInit {
            owner: app.local_owner,
            preferred_set: 4,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        let slot = GamepadSlot::new(0);

        app.process_gamepad_event_batch([
            GamepadEvent::Axis {
                slot,
                axis: LegacyGamepadAxis::new(1, false),
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot,
                button: ControlButton::Up,
                state: ElementState::Pressed,
            },
            GamepadEvent::Axis {
                slot,
                axis: LegacyGamepadAxis::new(6, false),
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot,
                button: ControlButton::Left,
                state: ElementState::Pressed,
            },
        ])
        .expect("route configured axis and hat controls");

        let pressed = app
            .engine
            .player(app.local_owner)
            .expect("control-set four player")
            .control
            .pressed_coms;
        assert_ne!(pressed & (1 << clonk_engine::COM_DIG), 0);
        assert_ne!(pressed & (1 << clonk_engine::COM_LEFT), 0);
    }

    #[test]
    fn runtime_status_report_failure_remains_stopped_and_unreached() {
        let mut app = new_state_only_running_sandbox_app();
        let (events, commands) = install_running_network_stub(&mut app, 7, 0, 1);
        drop(commands);
        let pause = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 0,
        };
        events
            .send(NetworkEvent::StatusRequested(pause))
            .expect("request immediately reachable Pause");

        app.process_network_events()
            .expect("failed acknowledgement is handled without resuming");

        assert!(!app.network_control_running);
        assert_eq!(
            app.runtime_network_status_barrier,
            Some(RuntimeNetworkStatusBarrier {
                status: pause,
                local_reached: false,
                actual_control_tick: None,
            })
        );
    }

    #[test]
    fn l143_chart_toggle_key_is_default_unbound_configurable_and_escape_owned() {
        assert!(RuntimeKeyConfig::default().chart_toggle.is_empty());
        let parsed = parse_runtime_key_config(b"[Keys]\nChartToggle=F8\n[Keys]\nChartToggle=F7\n")
            .expect("parse the represented default-unbound chart action");
        assert_eq!(
            parsed.chart_toggle,
            vec![RuntimeKeyChord::keyboard(
                VirtualKeyCode::F8,
                ModifiersState::empty(),
            )],
            "StdCompilerINIRead keeps the first action value"
        );

        let mut app = new_running_sandbox_app();
        app.runtime_key_config_cache = OnceLock::new();
        app.runtime_key_config_cache
            .set(Ok(parsed))
            .expect("install chart key registry");
        app.handle_key(VirtualKeyCode::F8, ElementState::Pressed)
            .expect("configured ChartToggle opens");
        assert!(app.network_chart_dialog.is_some());
        app.handle_key(VirtualKeyCode::F8, ElementState::Released)
            .expect("ChartToggle release is consumed");

        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("chart owns stronger bare Escape");
        assert!(app.network_chart_dialog.is_none());
        assert!(
            app.message_dialogs.is_empty(),
            "chart Escape must not also open the abort dialog"
        );
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("matching Escape release remains chart-owned");

        app.handle_key(VirtualKeyCode::F8, ElementState::Pressed)
            .expect("configured ChartToggle reopens");
        app.handle_key(VirtualKeyCode::F8, ElementState::Released)
            .expect("configured release remains consumed");

        assert!(
            !app.handle_network_chart_key(VirtualKeyCode::Up, ElementState::Pressed),
            "the non-exclusive chart must not invent GUI-scope arrow navigation"
        );
        assert_eq!(
            app.network_chart_dialog
                .as_ref()
                .expect("chart remains open")
                .active_tab_index(),
            0
        );

        app.start_running_chat(RunningChatMode::All);
        assert!(app.running_chat_active());
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("foreground chat owns the first Escape");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("foreground chat owns the matching release");
        assert!(app.running_chat_controller().is_none());
        assert!(
            app.network_chart_dialog.is_some(),
            "closing foreground chat must retain the background chart"
        );
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("new top chart owns the next Escape");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("chart owns the matching release");
        assert!(app.network_chart_dialog.is_none());
        assert!(app.message_dialogs.is_empty());

        app.handle_key(VirtualKeyCode::F8, ElementState::Pressed)
            .expect("configured ChartToggle still reopens");
        app.handle_key(VirtualKeyCode::F8, ElementState::Released)
            .expect("configured release remains consumed");
        app.handle_key(VirtualKeyCode::F8, ElementState::Pressed)
            .expect("configured ChartToggle closes");
        assert!(app.network_chart_dialog.is_none());

        let mut priority = new_running_sandbox_app();
        priority.runtime_key_config_cache = OnceLock::new();
        priority
            .runtime_key_config_cache
            .set(Ok(
                parse_runtime_key_config(b"[Keys]\nChartToggle=F2\n").unwrap()
            ))
            .expect("install duplicate base-priority chord");
        priority
            .handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("earlier ChatOpen binding wins duplicate ChartToggle chord");
        assert!(priority.running_chat_active());
        assert!(priority.network_chart_dialog.is_none());

        let mut remapped_priority = new_running_sandbox_app();
        remapped_priority.runtime_key_config_cache = OnceLock::new();
        remapped_priority
            .runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nChatOpen=F8\nChartToggle=F8\n",
            )
            .unwrap()))
            .expect("install duplicate remapped base-priority chord");
        remapped_priority
            .handle_key(VirtualKeyCode::F8, ElementState::Pressed)
            .expect("earlier remapped ChatOpen binding wins duplicate ChartToggle chord");
        assert!(remapped_priority.running_chat_active());
        assert!(remapped_priority.network_chart_dialog.is_none());
    }

    #[test]
    fn l119_running_script_uses_symbolic_console_strictness_and_frozen_sync() {
        let _lock = env_lock().lock();
        let fixture = tempdir().expect("running script configuration");
        let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
        let mut config = Config::new();
        config.set_in(Some("Developer"), "ConsoleScriptStrictness", "Strict1");
        config
            .save(paths.config_file())
            .expect("save script config");

        let mut app = new_state_only_running_sandbox_app();
        app.app_paths = Some(paths);
        app.engine.set_debug_mode(true);
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
        app.control_clients
            .replace_snapshot([message_client(0, b"Host")]);
        app.host_reference_paused = true;

        app.process_running_chat_text("/script return 1");

        let decided = commands.take_submitted_decided_controls();
        let [(_, clonk_engine::ControlPacket::Script(script), true)] = decided.as_slice() else {
            panic!("expected one synchronized script command, got {decided:?}");
        };
        assert_eq!(script.strictness, clonk_engine::ScriptStrictness::Strict1);
    }

    #[test]
    fn l006_runtime_key_config_compiles_lists_modifiers_raw_joy_and_disable_codes() {
        let parsed = parse_runtime_key_config(
                b"[Keys]\nNetObsNextPlayer=F5\nChatOpen=Ctrl+Shift+F2,Return\nScoreboardToggle=None\nGameAbort=Joy2A\nKbd1Key1=\\x0042010a\nUnknownAction=F9\n[Keys]\nNetObsNextPlayer=F6\n",
            )
            .expect("compile the first classic Keys node");
        assert_eq!(
            parsed.net_observer_next_player,
            vec![RuntimeKeyChord::keyboard(
                VirtualKeyCode::F5,
                ModifiersState::empty(),
            )]
        );
        assert_eq!(
            parsed.override_for("ChatOpen"),
            Some(
                [
                    RuntimeKeyChord::keyboard(
                        VirtualKeyCode::F2,
                        ModifiersState::CTRL | ModifiersState::SHIFT,
                    ),
                    RuntimeKeyChord::keyboard(VirtualKeyCode::Return, ModifiersState::empty(),),
                ]
                .as_slice()
            )
        );
        assert_eq!(
            parsed.override_for("ScoreboardToggle").unwrap()[0].physical,
            RuntimePhysicalKey::Disabled
        );
        assert_eq!(
            parsed.override_for("GameAbort").unwrap()[0].physical,
            RuntimePhysicalKey::Gamepad { slot: 1, button: 1 },
            "the first sscanf Joy branch owns every canonical JoyN suffix"
        );
        assert_eq!(
            parsed.override_for("Kbd1Key1").unwrap()[0].physical,
            RuntimePhysicalKey::Gamepad {
                slot: 1,
                button: 10
            }
        );
        assert!(parsed.override_for("UnknownAction").is_none());

        let unknown = parse_runtime_key_config(b"[Keys]\nNetObsNextPlayer=F01\n")
            .expect("SDL unknown names compile as KEY_Default");
        assert_eq!(
            unknown.net_observer_next_player[0].physical,
            RuntimePhysicalKey::Disabled
        );

        let partial = parse_runtime_key_config(
            b"[Keys] ; comment\nChatOpen=CapsLock,F2 ; trailing,Bogus+Q\nGameAbort=Keypad Enter\n",
        )
        .expect("compiler warnings retain the already-compiled prefix");
        assert_eq!(
            partial.override_for("ChatOpen"),
            Some(
                [
                    RuntimeKeyChord::keyboard(VirtualKeyCode::Capital, ModifiersState::empty(),),
                    RuntimeKeyChord::keyboard(VirtualKeyCode::F2, ModifiersState::empty(),),
                ]
                .as_slice()
            )
        );
        assert!(
            partial.override_for("GameAbort").is_none(),
            "the corrupt lexicographically earlier registration aborts later compilation"
        );
        let keypad = parse_runtime_key_config(b"[Keys]\nGameAbort=Keypad Enter\n")
            .expect("compile a canonical SDL keypad name");
        assert_eq!(
            keypad.override_for("GameAbort").unwrap()[0].physical,
            RuntimePhysicalKey::Keyboard(VirtualKeyCode::NumpadEnter)
        );
        let lowercase_keypad = parse_runtime_key_config(b"[Keys]\nGameAbort=keypad 1\n")
            .expect("SDL scancode names are case-insensitive");
        assert_eq!(
            lowercase_keypad.override_for("GameAbort").unwrap()[0].physical,
            RuntimePhysicalKey::Keyboard(VirtualKeyCode::Numpad1)
        );

        let caps_raw = input::encode_virtual_key_code(VirtualKeyCode::Capital)
            .expect("the active platform represents CapsLock");
        let raw_caps = format!("[Keys]\nToggleChat=\\x{caps_raw:x}\n");
        let raw_caps = parse_runtime_key_config(raw_caps.as_bytes())
            .expect("compile an active-platform raw key code");
        assert_eq!(
            raw_caps.override_for("ToggleChat").unwrap()[0].physical,
            RuntimePhysicalKey::Keyboard(VirtualKeyCode::Capital)
        );

        let noncanonical = parse_runtime_key_config(b"[Keys]\nKbd01Key01=F2\n")
            .expect("noncanonical registration names only warn");
        assert!(noncanonical.override_for("Kbd01Key01").is_none());
    }

    #[test]
    fn l077_ownerless_arrow_scroll_carries_momentum_without_player_mutation() {
        let mut app = new_running_sandbox_app();
        let owner = app.local_owner;
        let focus = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.engine
            .replace_player_viewports(
                owner,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(800, 180)).with_focus(Some(focus))],
            )
            .expect("place camera away from every scroll bound");
        app.engine.set_local_players([]);
        app.local_controls = LocalControlRegistry::default();
        app.mouse_control = false;
        app.snapshot = app.engine.snapshot();
        app.film_view_player = Some(owner);
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("render the player-assigned physical observer viewport");

        let initial = app.graphics.active_viewport_projections()[0];
        assert_eq!(initial.owner, owner);
        assert!(initial.is_no_owner_viewport);
        assert!(app.primary_physical_viewport_is_no_owner());
        let players_before = app.engine.snapshot().players;

        app.handle_key(VirtualKeyCode::Left, ElementState::Pressed)
            .expect("production FreeView Left dispatch");
        let production_left = app.graphics.active_viewport_projections()[0];
        assert_eq!(production_left.target_x, initial.target_x - 5);
        assert_eq!(production_left.target_y, initial.target_y);
        app.handle_key(VirtualKeyCode::Left, ElementState::Released)
            .expect("FreeView key-up has no callback");
        assert_eq!(
            app.graphics.active_viewport_projections()[0].target_x,
            production_left.target_x
        );

        app.free_view_scroll_momentum = FreeViewScrollMomentum::default();
        let start = app.graphics.active_viewport_projections()[0];
        let now = Instant::now();
        assert!(app.handle_viewport_player_cycle_key_at(
            VirtualKeyCode::Left,
            ElementState::Pressed,
            now,
        ));
        let first_left = app.graphics.active_viewport_projections()[0];
        assert_eq!(first_left.target_x, start.target_x - 5);
        assert_eq!(first_left.target_y, start.target_y);

        assert!(!app.handle_viewport_player_cycle_key_at(
            VirtualKeyCode::Left,
            ElementState::Released,
            now + Duration::from_millis(25),
        ));
        assert_eq!(app.graphics.active_viewport_projections()[0], first_left);

        assert!(app.handle_viewport_player_cycle_key_at(
            VirtualKeyCode::Left,
            ElementState::Pressed,
            now + Duration::from_millis(50),
        ));
        let second_left = app.graphics.active_viewport_projections()[0];
        assert_eq!(second_left.target_x, start.target_x - 15);
        assert_eq!(second_left.target_y, start.target_y);

        assert!(app.handle_viewport_player_cycle_key_at(
            VirtualKeyCode::Up,
            ElementState::Pressed,
            now + Duration::from_millis(75),
        ));
        let cross_axis = app.graphics.active_viewport_projections()[0];
        assert_eq!(cross_axis.target_x, start.target_x - 25);
        assert_eq!(cross_axis.target_y, start.target_y - 5);

        assert!(app.handle_viewport_player_cycle_key_at(
            VirtualKeyCode::Right,
            ElementState::Pressed,
            now + Duration::from_millis(175),
        ));
        let reset_right = app.graphics.active_viewport_projections()[0];
        assert_eq!(reset_right.target_x, start.target_x - 20);
        assert_eq!(reset_right.target_y, start.target_y - 5);

        assert!(app.handle_viewport_player_cycle_key_at(
            VirtualKeyCode::Down,
            ElementState::Pressed,
            now + Duration::from_millis(275),
        ));
        let reset_down = app.graphics.active_viewport_projections()[0];
        assert_eq!(reset_down.target_x, start.target_x - 20);
        assert_eq!(reset_down.target_y, start.target_y);
        assert_eq!(app.engine.snapshot().players, players_before);
        assert_eq!(app.film_view_player, Some(owner));

        let mut owned = new_running_sandbox_app();
        let mut owned_frame = vec![0_u8; 320 * 200 * 4];
        owned
            .render(&mut owned_frame)
            .expect("render the ordinary local-player viewport");
        assert!(!owned.primary_physical_viewport_is_no_owner());
        let owned_camera = owned.graphics.active_viewport_projections()[0];
        owned
            .engine
            .player_mut(owned.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        for (binding, key, command) in [
            (
                ControlBindingId::Left,
                VirtualKeyCode::Left,
                clonk_engine::COM_LEFT,
            ),
            (
                ControlBindingId::Right,
                VirtualKeyCode::Right,
                clonk_engine::COM_RIGHT,
            ),
            (
                ControlBindingId::Up,
                VirtualKeyCode::Up,
                clonk_engine::COM_UP,
            ),
            (
                ControlBindingId::Down,
                VirtualKeyCode::Down,
                clonk_engine::COM_DOWN,
            ),
        ] {
            owned.bindings.rebind(binding, key);
            owned
                .handle_key(key, ElementState::Pressed)
                .expect("owned arrow reaches its configured player control");
            assert_ne!(
                owned
                    .engine
                    .player(owned.local_owner)
                    .expect("local player")
                    .control
                    .pressed_coms
                    & (1 << command),
                0,
            );
            owned
                .handle_key(key, ElementState::Released)
                .expect("owned arrow release reaches player control");
            assert_eq!(
                owned
                    .engine
                    .player(owned.local_owner)
                    .expect("local player")
                    .control
                    .pressed_coms
                    & (1 << command),
                0,
            );
        }
        let owned_after = owned.graphics.active_viewport_projections()[0];
        assert_eq!(
            (owned_after.target_x, owned_after.target_y),
            (owned_camera.target_x, owned_camera.target_y)
        );
        assert!(owned.free_view_scroll_momentum.most_recent.is_none());
    }

    #[test]
    fn offline_negative_set_max_player_preserves_cap_and_rejects_queued_script_player() {
        // Hazard's CreateScriptPlayer runs after SetMaxPlayer in the same
        // Script1 callback. Exercise the app admission boundary as well as
        // the VM result: a rejected negative change must leave the one-slot
        // Game.Parameters cap in force when the deferred PlayerInfo arrives
        // (src/C4Script.cpp:3693-3705; src/C4PlayerInfo.cpp:781-807).
        let mut app = new_state_only_running_sandbox_app();
        app.network_max_players = 1;
        app.engine.set_max_players(1);
        app.control_player_infos.replace_snapshot(
            1,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        app.engine
            .install_scenario_script_with_convention(
                "SetMaxPlayer negative admission fixture",
                r#"
                    static set_result;

                    global func RejectLimitAndSpawn()
                    {
                        set_result = SetMaxPlayer(-1);
                        CreateScriptPlayer("Rejected Bot", 0x112233, 2, 15, __AI);
                    }
                    "#,
                true,
            )
            .expect("fixture script installs");

        app.engine
            .call_scenario_script_function("RejectLimitAndSpawn", Vec::new())
            .expect("negative SetMaxPlayer does not abort its caller");

        let globals = app.engine.snapshot().script_globals.named;
        assert_eq!(
            globals.get("set_result"),
            Some(&Value::Int(0)),
            "FnSetMaxPlayer has the C4ValueInt false result"
        );
        assert_eq!(app.engine.max_players(), Some(1));

        app.handle_script_player_info_updates()
            .expect("offline admission handles the queued script player");

        assert_eq!(app.network_max_players, 1);
        assert!(
            app.control_player_infos
                .client_info_ids(0)
                .into_iter()
                .filter_map(|id| app.control_player_infos.get(id))
                .all(|info| info.name.as_bytes() != b"Rejected Bot"),
            "the unchanged full cap rejects the PlayerInfo"
        );
        assert!(
            app.engine
                .snapshot()
                .players
                .iter()
                .all(|player| player.name != "Rejected Bot"),
            "a rejected PlayerInfo cannot reach JoinPlayer"
        );
    }

    #[test]
    fn l052_retargeted_primary_survives_its_original_local_player() {
        let mut app = new_lightweight_running_sandbox_app();
        let original = app.local_owner;
        let target = original + 1;
        app.engine
            .register_player(PlayerConfig::new(target, "Film target"))
            .expect("register second local player");
        let target_control = app.local_controls.initialize(LocalControlInit {
            owner: target,
            preferred_set: 1,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        app.engine
            .set_player_runtime_control(target, target_control.runtime_control())
            .expect("install target control");
        app.engine.set_local_players([original, target]);
        app.engine
            .replace_player_viewports(
                original,
                vec![clonk_engine::PlayerViewport::new(Vector2::new(300, 180)).with_zoom(1.75)],
            )
            .expect("install source physical zoom");
        app.snapshot = app.engine.snapshot();
        let _ = app.create_physical_viewport(target, false, true, true);
        app.engine.clear_scenario_script();
        app.engine
                .install_scenario_script_with_convention(
                    "PhysicalViewport.c",
                    &format!(
                        "#strict 3\nfunc Probe() {{ SetViewOffset({original}, 17, 19); SetFilmView({target}); SetViewOffset({original}, 91, 92); }}"
                    ),
                    true,
                )
                .expect("install film retarget probe");
        app.engine.set_replay_control(true);
        app.engine
            .call_scenario_script_function("Probe", Vec::new())
            .expect("retarget physical viewport");
        let _ = app.apply_pending_viewport_presentation_requests();
        assert_eq!(
            app.physical_viewports
                .iter()
                .map(|viewport| viewport.displayed_player)
                .collect::<Vec<_>>(),
            vec![target, target]
        );
        assert_eq!(app.physical_viewports[0].preserved_zoom, 1.75);
        assert_eq!(
            app.physical_viewports[0].preserved_offset,
            Vector2::new(17, 19)
        );

        app.ui_sound_log.clear();
        app.remove_runtime_player_with_viewport_feedback(original)
            .expect("remove original local player");
        assert_eq!(app.physical_viewports.len(), 2);
        assert!(app
            .physical_viewports
            .iter()
            .all(|viewport| viewport.displayed_player == target));
        assert!(app.ui_sound_log.is_empty(), "CloseViewport(A) matches none");

        app.snapshot = app.engine.snapshot();
        let rendered =
            collect_viewport_inputs_from_physical_state(&app.snapshot, &app.physical_viewports)
                .expect("both surviving physical viewports render");
        assert_eq!(rendered.len(), 2);
        assert!(rendered.iter().all(|viewport| viewport.owner == target));
        assert_eq!(rendered[0].zoom, 1.75);
        assert_eq!(rendered[0].offset, Vector2::new(17, 19));

        app.remove_runtime_player_with_viewport_feedback(target)
            .expect("remove displayed target");
        assert_eq!(
            app.ui_sound_log
                .iter()
                .filter(|sound| sound.as_str() == "CloseViewport")
                .count(),
            1,
            "closing both matching physical viewports requests one sound"
        );
        assert_eq!(app.physical_viewports.len(), 1);
        assert!(app.physical_viewports[0].is_no_owner_viewport);
    }

    #[test]
    fn queued_derive_completion_keeps_the_registered_mutable_player_source() {
        // FinishDerive returns on the main thread before its forwarded
        // ResourceComplete event is drained. That later event describes the
        // serving standalone, while C4Network2Res::getFile still resolves the
        // mutable player file used by the next Save
        // (src/C4Player.cpp:452-461; src/C4Network2Res.cpp:718-823).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        let mutable_path = PathBuf::from("Network/Bob.c4p");
        let serving_path = PathBuf::from("Network/Bob_2.c4p");
        let core = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 62,
            derived_id: 61,
            loadable: true,
            ..Default::default()
        };
        app.admission_resources.register_finished_derivation(
            &core,
            mutable_path.clone(),
            clonk_network::ResourceFileOwnership::Temporary,
        );
        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id: core.id,
                core: core.clone(),
                path: serving_path,
                local: true,
            })
            .expect("queue forwarded derive completion");

        app.process_network_events()
            .expect("apply forwarded derive completion");

        assert_eq!(
            app.admission_resources.status(core.id),
            Some(&AdmissionResourceState::Complete {
                path: mutable_path,
                removed: false,
                local: false,
            })
        );
    }

    #[test]
    fn player_command_submission_queues_the_open_tick_without_local_execution() {
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();
        let crew = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        let before = app
            .engine
            .object_snapshot(crew)
            .expect("cursor exists")
            .command_stack
            .command_names();
        let command = PlayerCommandControlData {
            player: app.local_owner,
            command: CommandId::MoveTo as i32,
            x: 120,
            y: 80,
            target: 0,
            target2: 0,
            data: 0,
            add_mode: 1,
            by_client: -1,
        };

        app.submit_or_execute_player_command(command)
            .expect("queue player command");

        assert_eq!(
            commands.take_submitted_player_commands(),
            vec![(
                tick,
                PlayerCommandControlData {
                    by_client: 7,
                    ..command
                }
            )]
        );
        assert_eq!(
            app.engine
                .object_snapshot(crew)
                .expect("cursor survives")
                .command_stack
                .command_names(),
            before,
            "the command executes only when the synchronized tick returns"
        );
    }

    #[test]
    fn player_select_submission_queues_the_open_tick_without_local_execution() {
        let mut app = new_state_only_running_sandbox_app();
        let owner = app.local_owner;
        let first = app.engine.crew_cursor(owner).expect("sandbox cursor");
        let definition = app
            .engine
            .object_snapshot(first)
            .expect("cursor exists")
            .definition_id;
        let second = app
            .engine
            .spawn_object(
                SpawnConfig::new(definition)
                    .with_owner(owner)
                    .with_crew_member(true),
            )
            .expect("second crew spawns");
        app.engine
            .select_crew(owner, [first, second])
            .expect("select both crew members");
        app.engine
            .set_crew_cursor(owner, Some(first))
            .expect("retain first cursor");
        let before = app.engine.selected_crew(owner);
        let (manager, _event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();
        let selection = PlayerSelectControlData {
            player: owner,
            objects: vec![second.as_u64() as i32],
            by_client: -1,
        };

        app.submit_or_execute_player_select(selection.clone())
            .expect("queue player selection");

        assert_eq!(
            commands.take_submitted_player_selects(),
            vec![(
                tick,
                PlayerSelectControlData {
                    by_client: 7,
                    ..selection
                }
            )]
        );
        assert_eq!(
            app.engine.selected_crew(owner),
            before,
            "selection executes only when the synchronized tick returns"
        );
    }

    #[test]
    fn runtime_flash_storage_uses_classic_bytes_and_snapshots_placement() {
        let mut app = new_state_only_running_sandbox_app();

        let cp1252 = app
            .prepare_runtime_flash_message("\u{fc}", RuntimeHelpCharset::Windows1252)
            .expect("encode CP1252 flash")
            .expect("nonempty CP1252 flash");
        assert_eq!(cp1252.text, "\u{fc}");
        assert_eq!(cp1252.remaining_draws, 2, "CP1252 ü is one stored byte");

        let utf8 = app
            .prepare_runtime_flash_message("\u{fc}", RuntimeHelpCharset::Utf8)
            .expect("encode UTF-8 flash")
            .expect("nonempty UTF-8 flash");
        assert_eq!(utf8.remaining_draws, 4, "UTF-8 ü is two stored bytes");

        let unicode = app
            .prepare_runtime_flash_message("\u{100}", RuntimeHelpCharset::Utf8)
            .expect("FontRegular accepts non-CP1252 UTF-8")
            .expect("nonempty Unicode flash");
        assert_eq!(unicode.text, "\u{100}");
        assert_eq!(unicode.remaining_draws, 4);
        assert!(
            app.prepare_runtime_flash_message("\u{100}", RuntimeHelpCharset::Windows1252,)
                .is_err(),
            "the classic CP1252 encoder still rejects an unrepresentable scalar"
        );

        let ascii = "A".repeat(513);
        let truncated = app
            .prepare_runtime_flash_message(&ascii, RuntimeHelpCharset::Windows1252)
            .expect("truncate classic title buffer")
            .expect("nonempty truncated flash");
        assert_eq!(truncated.text.len(), 512);
        assert_eq!(truncated.remaining_draws, 1024);

        let nul = app
            .prepare_runtime_flash_message("A\0ignored", RuntimeHelpCharset::Windows1252)
            .expect("SCopy stops at NUL")
            .expect("prefix remains visible");
        assert_eq!(nul.text, "A");
        assert_eq!(nul.remaining_draws, 2);

        let split_utf8 = format!("{}\u{fc}", "A".repeat(511));
        let error = app
            .prepare_runtime_flash_message(&split_utf8, RuntimeHelpCharset::Utf8)
            .expect_err("unsafe UTF-8 boundary must fail closed");
        assert!(error.to_string().contains("splits a UTF-8 scalar"));

        for (mode, expected_y) in [
            (UpperBoardMode::Hide, 10),
            (UpperBoardMode::Full, 60),
            (UpperBoardMode::Small, 35),
            (UpperBoardMode::Mini, 10),
        ] {
            app.display_flags.upper_board = mode;
            let message = app
                .prepare_runtime_flash_message("A", RuntimeHelpCharset::Windows1252)
                .expect("prepare placement")
                .expect("visible placement");
            assert_eq!(message.y, expected_y, "mode {mode:?}");
        }

        let player = app
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.id == app.local_owner)
            .expect("local player");
        player
            .viewports
            .push(player.viewports.first().expect("primary viewport").clone());
        app.display_flags.upper_board = UpperBoardMode::Full;
        app.set_runtime_flash_message("AB", RuntimeHelpCharset::Windows1252)
            .expect("install split-screen flash");
        assert_eq!(app.runtime_flash_message.as_ref().expect("flash").y, 124);
        app.display_flags.upper_board = UpperBoardMode::Hide;
        app.snapshot
            .players
            .iter_mut()
            .find(|player| player.id == app.local_owner)
            .expect("local player")
            .viewports
            .truncate(1);
        assert_eq!(
            app.runtime_flash_message.as_ref().expect("frozen flash").y,
            124,
            "later board/viewport changes do not reposition an active flash"
        );

        app.set_runtime_flash_message("", RuntimeHelpCharset::Windows1252)
            .expect("empty FlashMessage clears");
        assert!(app.runtime_flash_message.is_none());
    }

    #[test]
    fn runtime_f3_raw_latch_survives_priority_changes_and_focus_loss_resets_modifiers() {
        let left_mask = 1 << clonk_engine::COM_LEFT;

        let mut modified_first = new_running_sandbox_app();
        modified_first
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        modified_first
            .engine
            .player_mut(modified_first.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        modified_first
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("set Alt");
        modified_first
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("modified first down has no exact callback");
        assert!(modified_first
            .pressed_engine_keys
            .contains(&VirtualKeyCode::F3));
        modified_first
            .handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt while F3 remains held");
        modified_first
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("held bare edge reaches player priority as a repeat");
        assert_eq!(
            modified_first
                .engine
                .player(modified_first.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & left_mask,
            0,
            "AutoStop must discard a held F3 repeat"
        );
        assert!(modified_first.runtime_flash_message.is_none());

        let mut game_over = new_game_over_keyboard_app();
        game_over
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        game_over
            .engine
            .player_mut(game_over.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        game_over
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("exclusive game-over suppresses player and retains Generic F3");
        let global_flash = game_over.runtime_flash_message.clone();
        game_over.dismiss_game_over_dialog();
        game_over
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("still-held F3 enters newly exposed player scope as repeat");
        assert_eq!(
            game_over
                .engine
                .player(game_over.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & left_mask,
            0
        );
        assert_eq!(game_over.runtime_flash_message, global_flash);

        let mut changed_on_release = new_running_sandbox_app();
        changed_on_release
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        changed_on_release
            .engine
            .player_mut(changed_on_release.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        changed_on_release
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("player owns first bare down");
        changed_on_release
            .handle_modifiers_changed(ModifiersState::CTRL)
            .expect("add Ctrl before physical up");
        changed_on_release
            .handle_key(VirtualKeyCode::F3, ElementState::Released)
            .expect("raw up clears latch independent of modified dispatch");
        assert!(!changed_on_release
            .pressed_engine_keys
            .contains(&VirtualKeyCode::F3));
        changed_on_release
            .engine
            .player_mut(changed_on_release.local_owner)
            .expect("local player")
            .control
            .pressed_coms = 0;
        changed_on_release
            .handle_modifiers_changed(ModifiersState::empty())
            .expect("clear Ctrl");
        changed_on_release
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("next bare down is fresh");
        assert_ne!(
            changed_on_release
                .engine
                .player(changed_on_release.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & left_mask,
            0
        );

        let mut focus = new_running_sandbox_app();
        let sound_before = focus
            .audio
            .as_ref()
            .expect("test audio")
            .options
            .sound_enabled;
        focus
            .handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set Ctrl before focus loss");
        focus.handle_focus_lost().expect("clear raw keyboard state");
        assert!(focus.keyboard_modifiers.is_empty());
        focus
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("post-focus F3 is bare music, not stale Ctrl+F3");
        assert_eq!(
            focus
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .sound_enabled,
            sound_before
        );
        assert!(focus.runtime_flash_message.is_some());
    }

    #[test]
    fn l013_speed_keys_flash_clamp_and_honor_keyconfig_priority() {
        let mut app = new_running_sandbox_app();
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("set default speed-key modifiers");
        app.handle_key(VirtualKeyCode::NumpadAdd, ElementState::Pressed)
            .expect("Shift+Numpad+ speeds up without terminating");
        assert!(app.full_speed);
        assert_eq!(app.frame_skip, 2);
        assert_eq!(
            app.runtime_flash_message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("Speed: 2x")
        );
        app.handle_key(VirtualKeyCode::NumpadAdd, ElementState::Released)
            .expect("speed-up release has no callback");
        app.handle_key(VirtualKeyCode::NumpadAdd, ElementState::Pressed)
            .expect("repeated speed-up press");
        assert_eq!(app.frame_skip, 3);

        for expected in [2, 1] {
            app.handle_key(VirtualKeyCode::NumpadSubtract, ElementState::Pressed)
                .expect("Shift+Numpad- slows down without terminating");
            assert_eq!(app.frame_skip, expected);
        }
        assert!(!app.full_speed);
        assert_eq!(
            app.runtime_flash_message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("Speed: 1x")
        );

        app.frame_skip = 50;
        app.full_speed = false;
        app.handle_key(VirtualKeyCode::NumpadAdd, ElementState::Pressed)
            .expect("upper-bound speed-up still flashes");
        assert_eq!(app.frame_skip, 50);
        assert!(app.full_speed);
        assert_eq!(
            app.runtime_flash_message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("Speed: 50x")
        );

        let mut rebound = new_running_sandbox_app();
        rebound.runtime_key_config_cache = OnceLock::new();
        rebound
            .runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nGameSpeedUp=G\nGameSlowDown=H\n",
            )
            .expect("parse rebound speed keys")))
            .expect("install rebound speed keys");
        rebound
            .handle_key(VirtualKeyCode::G, ElementState::Pressed)
            .expect("rebound speed-up key");
        assert_eq!(rebound.frame_skip, 2);
        rebound
            .handle_key(VirtualKeyCode::H, ElementState::Pressed)
            .expect("rebound speed-down key");
        assert_eq!(rebound.frame_skip, 1);
        assert!(!rebound.full_speed);

        let mut global_collision = new_running_sandbox_app();
        global_collision.app_paths = None;
        global_collision.runtime_key_config_cache = OnceLock::new();
        global_collision
            .runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nSoundToggle=G\nGameSpeedUp=G\n",
            )
            .expect("parse earlier-global collision")))
            .expect("install earlier-global collision");
        let sound_enabled = global_collision
            .audio
            .as_ref()
            .expect("sandbox audio context")
            .options
            .sound_enabled;
        global_collision
            .handle_key(VirtualKeyCode::G, ElementState::Pressed)
            .expect("earlier registered global owns collision");
        assert_eq!(
            global_collision
                .audio
                .as_ref()
                .expect("sandbox audio context")
                .options
                .sound_enabled,
            !sound_enabled
        );
        assert_eq!(global_collision.frame_skip, 1);
        assert!(!global_collision.full_speed);
        assert!(global_collision.runtime_flash_message.is_none());

        let mut collision = new_running_sandbox_app();
        collision.runtime_key_config_cache = OnceLock::new();
        collision
            .runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nKbd1Key1=G\nGameSpeedUp=G\n",
            )
            .expect("parse player/global collision")))
            .expect("install player/global collision");
        collision
            .handle_key(VirtualKeyCode::G, ElementState::Pressed)
            .expect("higher-priority player binding owns collision");
        assert_eq!(collision.frame_skip, 1);
        assert!(!collision.full_speed);
        assert!(collision.runtime_flash_message.is_none());
        collision
            .handle_key(VirtualKeyCode::G, ElementState::Released)
            .expect("player binding owns collision release");
    }

    #[test]
    fn runtime_f3_priority_matrix_covers_every_recursive_running_layer() {
        #[derive(Clone, Copy, Debug)]
        enum Layer {
            Message,
            Context,
            Scoreboard,
            Save,
            Load,
            Object,
            Observer,
            GameOver,
            GameOverNext,
        }

        let make_layer = |layer| {
            let mut app = match layer {
                Layer::Scoreboard => {
                    let mut app = new_scoreboard_test_app(
                        r#"global func Initialize()
                            {
                                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                            }"#,
                    );
                    toggle_scoreboard(&mut app, ModifiersState::empty());
                    app
                }
                Layer::GameOver | Layer::GameOverNext => new_classic_running_sandbox_app(),
                _ => new_running_sandbox_app(),
            };
            match layer {
                Layer::Message => app
                    .push_message_dialog(
                        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                            "Audio",
                            "Nonexclusive",
                            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                        ),
                        MessageDialogContinuation::None,
                    )
                    .expect("open message"),
                Layer::Context => {
                    app.open_context_menu_at(
                        vec![ContextMenuEntry::<AppContextMenuCommand>::new("Root")
                            .with_submenu(vec![ContextMenuEntry::new("Child")])],
                        GuiPoint::new(24.0, 24.0),
                    )
                    .expect("open context");
                }
                Layer::Save => {
                    app.save_browser = Some(SaveBrowserState::new(
                        SaveBrowserMode::Save {
                            suggested_label: "Slot".to_string(),
                        },
                        Vec::new(),
                    ));
                }
                Layer::Load => {
                    app.save_browser = Some(SaveBrowserState::new(SaveBrowserMode::Load, Vec::new()));
                }
                Layer::Object => {
                    assert!(app.open_object_menu().expect("open object state"));
                }
                Layer::Observer => {
                    app.engine
                        .remove_player(app.local_owner)
                        .expect("remove local player");
                    app.engine.set_local_players([]);
                    app.snapshot = app.engine.snapshot();
                }
                Layer::GameOver | Layer::GameOverNext => {
                    if matches!(layer, Layer::GameOverNext) {
                        let mut state = app.engine.capture_state();
                        state.next_mission = clonk_engine::NextMissionState {
                            path: "Next.c4s".to_string(),
                            text: "Next".to_string(),
                            description: "Continue".to_string(),
                        };
                        app.engine
                            .restore_state(&state)
                            .expect("restore next mission");
                        app.snapshot = app.engine.snapshot();
                    }
                    app.handle_game_over().expect("open evaluation");
                }
                Layer::Scoreboard => {}
            }
            app
        };

        for layer in [
            Layer::Message,
            Layer::Context,
            Layer::Scoreboard,
            Layer::Save,
            Layer::Load,
            Layer::Object,
            Layer::Observer,
            Layer::GameOver,
            Layer::GameOverNext,
        ] {
            let player_scope = !matches!(
                layer,
                Layer::Observer | Layer::GameOver | Layer::GameOverNext
            );

            let mut default_app = make_layer(layer);
            default_app
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .unwrap_or_else(|error| panic!("default F3 on {layer:?}: {error}"));
            assert!(default_app.runtime_flash_message.is_some(), "{layer:?}");
            if matches!(layer, Layer::GameOver | Layer::GameOverNext) {
                let before = default_app
                    .runtime_flash_message
                    .as_ref()
                    .expect("game-over flash")
                    .remaining_draws;
                let mut frame = vec![0_u8; 320 * 200 * 4];
                default_app
                    .render(&mut frame)
                    .unwrap_or_else(|error| panic!("render F3 on {layer:?}: {error:#}"));
                assert_eq!(
                    default_app
                        .runtime_flash_message
                        .as_ref()
                        .expect("music text lasts more than one draw")
                        .remaining_draws,
                    before - 1
                );
            }

            let mut rebound = make_layer(layer);
            rebound
                .bindings
                .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
            if let Ok(player) = rebound.engine.player_mut(rebound.local_owner) {
                player.control.control_style = true;
            }
            rebound
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .unwrap_or_else(|error| panic!("rebound F3 on {layer:?}: {error}"));
            assert_eq!(
                rebound.runtime_flash_message.is_none(),
                player_scope,
                "{layer:?}"
            );

            let mut sound = make_layer(layer);
            let before = sound
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .sound_enabled;
            sound
                .handle_modifiers_changed(ModifiersState::CTRL)
                .expect("set Ctrl");
            sound
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .unwrap_or_else(|error| panic!("Ctrl+F3 on {layer:?}: {error}"));
            assert_eq!(
                sound
                    .audio
                    .as_ref()
                    .expect("test audio")
                    .options
                    .sound_enabled,
                !before,
                "{layer:?}"
            );
            assert!(sound.runtime_flash_message.is_none(), "{layer:?}");
        }
    }

    #[test]
    fn generated_team_name_template_preserves_the_runtime_table_charset() {
        let cp1252 = RuntimeLanguageTable {
            charset: RuntimeHelpCharset::Windows1252,
            entries: HashMap::from([("IDS_MSG_TEAM".to_string(), "Équipe %d".to_string())]),
        };
        assert_eq!(
            generated_team_name_template(&cp1252).as_bytes(),
            b"\xc9quipe %d"
        );

        let utf8 = RuntimeLanguageTable {
            charset: RuntimeHelpCharset::Utf8,
            entries: cp1252.entries,
        };
        assert_eq!(
            generated_team_name_template(&utf8).as_bytes(),
            "Équipe %d".as_bytes()
        );
    }

    #[test]
    fn runtime_resource_lookup_uses_the_process_loaded_language_table() {
        // C4Application owns one ResStrTable and replaces it only when the
        // Options dialog reloads the language. Per-frame console/menu lookup
        // must not reopen and parse System.c4g.
        let mut app = new_menu_app(320, 240);
        app.startup_tooltip_resources.insert(
            "IDS_TEST_PROCESS_RESOURCE".to_string(),
            "process cached É".to_string(),
        );
        app.runtime_language_charset = RuntimeHelpCharset::Windows1252;

        assert_eq!(
            app.runtime_resource_text("IDS_TEST_PROCESS_RESOURCE", "fallback"),
            "process cached É"
        );
        assert_eq!(
            app.runtime_resource_bytes("IDS_TEST_PROCESS_RESOURCE"),
            b"process cached \xc9"
        );
    }

    #[test]
    fn process_language_table_survives_disk_edits_until_an_explicit_options_reload() {
        let install = tempdir().expect("process language install fixture");
        let user_data = tempdir().expect("process language user fixture");
        let system = install.path().join("planet/System.c4g");
        fs::create_dir_all(&system).expect("create process language System.c4g");
        let language = system.join("LanguageUS.txt");
        let table = |generation: &str, charset: &str| {
            format!(
                "IDS_LANG_CHARSET={charset}\n\
                     IDS_MSG_SELECT={generation} select %s\n\
                     IDS_NET_CLIENT_READY={generation} ready %s.\n\
                     IDS_NET_CLIENT_UNREADY={generation} not ready %s.\n\
                     IDS_MSG_NOTALLSAVEGAMEPLAYERSHAVE={generation} unassociated players\n\
                     IDS_MSG_FREESAVEGAMEPLRS={generation} player assignment\n\
                     IDS_MSG_DONTSHOW={generation} don't show\n\
                     IDS_MSG_NOSPLITSCREENINLEAGUE={generation} players %s and %s\n\
                     IDS_NET_ERR_LEAGUE={generation} league error\n\
                     IDS_TEXT_COMMANDSAVAILABLEDURINGLO={generation} lobby commands\n\
                     IDS_PLR_NEWCOMMENT={generation} new player\n\
                     IDS_NET_REFONCLIENT={generation} %s on %s\n\
                     IDS_NET_QUERY_MASTERSRV={generation} internet server\n\
                     IDS_NET_CLIENTONNET=%s on %s\n\
                     IDS_NET_INFOQUERY={generation} querying\n\
                     IDS_CTL_NOLANGINFO={generation} no language info\n\
                     IDS_MSG_TEAM=Team %d\n"
            )
        };
        fs::write(&language, table("Loaded", "")).expect("write initially loaded process language");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover process language fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");
        fs::write(
            paths.config_file(),
            "[General]\nLanguage=US\nLanguageEx=US\n",
        )
        .expect("select fixture language");

        let mut app = new_real_menu_app(640, 480);
        app.app_paths = Some(paths.clone());
        app.reload_application_language_resources()
            .expect("load the process language table");
        assert_eq!(
            app.runtime_language_charset,
            RuntimeHelpCharset::Windows1252
        );

        fs::write(&language, table("Mutated", "UTF-8"))
            .expect("mutate language file after process startup");
        let disk = load_runtime_language_table(Some(&paths))
            .expect("the mutated language table is independently loadable");
        assert_eq!(
            disk.entries.get("IDS_MSG_SELECT").map(String::as_str),
            Some("Mutated select %s")
        );
        assert_eq!(disk.charset, RuntimeHelpCharset::Utf8);
        assert_eq!(
            load_options_program_state(Some(&paths), Some(&app.startup_tooltip_resources),)
                .no_language_info,
            "Loaded no language info"
        );

        attach_l040_network_dialog(&mut app);
        app.startup_game_references = vec![clonk_network::NetworkGameReference {
            title: "HarpoonRace".to_string(),
            host_name: "Host".to_string(),
            state: "Lobby".to_string(),
            max_players: 24,
            ..Default::default()
        }];
        let loads_before_browser = runtime_language_table_load_count(paths.system_group_path());
        assert!(
            loads_before_browser >= 2,
            "the probe observes the initial process load and explicit disk inspection"
        );
        for _ in 0..3 {
            app.sync_startup_network_game_rows();
            let row = &app
                .startup_network_dialog
                .as_ref()
                .expect("network browser")
                .games()[0];
            assert_eq!(row.title, "Loaded HarpoonRace on Host");
            let mut frame = vec![0_u8; 640 * 480 * 4];
            app.render(&mut frame)
                .expect("render network browser from retained language table");
        }
        assert_eq!(
            runtime_language_table_load_count(paths.system_group_path()),
            loads_before_browser,
            "network-browser row projection and rendering must not reopen System.c4g"
        );

        assert_eq!(
            app.runtime_resource_text("IDS_MSG_SELECT", "fallback"),
            "Loaded select %s"
        );
        assert_eq!(
            app.runtime_resource_bytes("IDS_MSG_NOSPLITSCREENINLEAGUE"),
            b"Loaded players %s and %s"
        );
        assert_eq!(
            app.new_startup_player_properties_controller(0, 0).comment(),
            "Loaded new player"
        );

        app.control_clients
            .replace_snapshot([message_client(7, b"Remote")]);
        app.append_remote_lobby_ready_log(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        });
        assert_eq!(
            latest_message_board_logical_entry(&app).as_deref(),
            Some("Loaded ready Remote.")
        );

        install_test_classic_host_team_lobby(&mut app);
        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/help".to_string()))
            .expect("render lobby command help from the retained process table");
        assert!(app
            .classic_host_lobby
            .as_ref()
            .expect("classic host lobby")
            .controller
            .logs()
            .iter()
            .any(|line| line.text == "Loaded lobby commands"));
        app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
            player_id: 7,
        }])
        .expect("open a team selector from the retained process table");
        let menu = app
            .context_menu
            .as_mut()
            .expect("team selector context menu");
        let first = menu.layout().panels[0].rows[0].rect;
        menu.handle_pointer_move(GuiPoint::new((first.x + 1) as f32, (first.y + 1) as f32));
        assert!(
            menu.hovered_tooltip_at(Instant::now() + Duration::from_secs(1))
                .is_some_and(|tooltip| tooltip.starts_with("Loaded select ")),
            "team selector tooltip must not observe the on-disk edit"
        );
        app.close_context_menu_silently();

        app.network_is_league = true;
        app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
            countdown_seconds: 5,
            check_league_rules: true,
            confirm_unassociated_savegame_players: false,
        }])
        .expect("evaluate the cached league split-screen resources");
        let league = app.message_dialogs.last().expect("cached league dialog");
        assert_eq!(
            league.state.message(),
            "Loaded players Chooser and Companion"
        );
        assert_eq!(league.state.caption(), "Loaded league error");
        app.message_dialogs.clear();
        app.network_is_league = false;

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
            countdown_seconds: 5,
            check_league_rules: false,
            confirm_unassociated_savegame_players: true,
        }])
        .expect("show the unassociated-player confirmation");
        let confirmation = app
            .message_dialogs
            .last()
            .expect("cached confirmation dialog");
        assert_eq!(confirmation.state.message(), "Loaded unassociated players");
        assert_eq!(confirmation.state.caption(), "Loaded player assignment");

        app.reload_application_language_resources()
            .expect("Options language reload replaces the process table");
        assert_eq!(app.runtime_language_charset, RuntimeHelpCharset::Utf8);
        assert_eq!(
            app.runtime_resource_text("IDS_MSG_SELECT", "fallback"),
            "Mutated select %s"
        );
        assert_eq!(
            app.new_startup_player_properties_controller(0, 0).comment(),
            "Mutated new player"
        );
        assert_eq!(
            load_options_program_state(Some(&paths), Some(&app.startup_tooltip_resources),)
                .no_language_info,
            "Mutated no language info"
        );
    }

    #[test]
    fn runtime_join_flash_keeps_the_process_language_charset_until_reload() {
        let install = tempdir().expect("runtime-join language install fixture");
        let user_data = tempdir().expect("runtime-join language user fixture");
        let system = install.path().join("planet/System.c4g");
        fs::create_dir_all(&system).expect("create runtime-join System.c4g");
        let language = system.join("LanguageUS.txt");
        let mut initial = b"IDS_LANG_CHARSET=\nIDS_NET_RUNTIMEJOINFREE=".to_vec();
        initial.extend(std::iter::repeat_n(0xe9, 300));
        initial.extend_from_slice(b"\nIDS_NET_RUNTIMEJOINBARRED=Cached barred\nIDS_MSG_TEAM=Team %d\n");
        fs::write(&language, initial).expect("write CP1252 runtime-join language");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover runtime-join fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");
        fs::write(
            paths.config_file(),
            "[General]\nLanguage=US\nLanguageEx=US\n",
        )
        .expect("select fixture language");

        let mut app = new_classic_running_sandbox_app();
        app.app_paths = Some(paths.clone());
        app.reload_application_language_resources()
            .expect("load CP1252 process table");
        fs::write(
            &language,
            "IDS_LANG_CHARSET=UTF-8\n\
                 IDS_NET_RUNTIMEJOINFREE=Reloaded free\n\
                 IDS_NET_RUNTIMEJOINBARRED=Reloaded barred\n\
                 IDS_MSG_TEAM=Team %d\n",
        )
        .expect("replace disk table with UTF-8 after startup");

        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
        assert!(matches!(
            app.runtime_network_role(),
            RuntimeNetworkRole::Host
        ));
        app.control_clients
            .replace_snapshot([message_client(0, b"Host")]);
        app.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("open runtime client list");
        let acknowledgement = thread::spawn(move || {
            let (allowed, completion) = commands.receive_join_allowed();
            assert!(allowed);
            completion
                .send(Ok(()))
                .expect("acknowledge runtime-join change");
        });
        app.apply_runtime_client_list_option(LobbyOptionKind::RuntimeJoin, 1)
            .expect("apply runtime-join option with process charset");
        acknowledgement
            .join()
            .expect("runtime-join acknowledgement thread");

        let flash = app
            .runtime_flash_message
            .as_ref()
            .expect("runtime-join state flashes");
        assert_eq!(
            flash.text.chars().count(),
            300,
            "the retained CP1252 table keeps all 300 one-byte characters"
        );
        assert!(flash.text.chars().all(|character| character == '\u{e9}'));
        assert_eq!(
            app.runtime_language_charset,
            RuntimeHelpCharset::Windows1252
        );

        app.reload_application_language_resources()
            .expect("Options reload adopts the replacement language table");
        assert_eq!(app.runtime_language_charset, RuntimeHelpCharset::Utf8);
        assert_eq!(
            app.classic_lobby_option_labels().runtime_join_free,
            "Reloaded free"
        );
    }

    #[test]
    fn runtime_f1_language_lookup_is_case_insensitive_and_skips_empty_candidates() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("runtime help install fixture");
        let user_data = tempdir().expect("runtime help user fixture");
        let system = install.path().join("planet/System.c4g");
        fs::create_dir_all(&system).expect("fixture System.c4g");
        fs::write(system.join("LANGUAGEzz.TXT"), []).expect("empty first language candidate");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../planet/System.c4g/LanguageDE.txt"),
            system.join("lAnGuAgEdE.TxT"),
        )
        .expect("mixed-case German language fixture");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover runtime help fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");
        fs::write(paths.config_file(), "[General]\nLanguageEx=ZZ,DE\n")
            .expect("write fixture language config");

        let table =
            load_runtime_language_table(Some(&paths)).expect("empty ZZ falls through to mixed-case DE");
        let (need, none) = needed_material_resource_strings(&table);
        assert_eq!(need, "%s|braucht noch");
        assert_eq!(none, "%s braucht kein|weiteres Baumaterial.");
        assert_eq!(
            object_no_dig_resource_string(&table),
            "%s kann|nicht graben."
        );
        assert_eq!(default_rank_resource_names(&table)[1], "Fähnrich");
        let columns = build_runtime_help_columns(&table.entries).expect("build German help");
        assert!(columns.left.starts_with("[Spielfunktionen]\n"));
        assert!(columns.left.contains("F1</c> - Hilfe"));
    }

    /// `C4GraphicsSystem::DrawHelp` asks `GetKeyboardInputName` for each
    /// registered key's *current* code, so a `KeyConfig` override changes the
    /// displayed chord as well as the dispatch
    /// (C4GraphicsSystem.cpp:692-724). The two columns keep their native draw
    /// order and read the same process language table as the rest of the UI.
    #[test]
    fn runtime_f1_help_displays_live_remapped_key_names() {
        let mut app = new_running_sandbox_app();
        let default_columns = app
            .runtime_help_columns()
            .expect("shipped help columns")
            .clone();
        assert!(default_columns.left.contains("F1</c> - "));
        assert!(default_columns.left.contains("Tab</c> - "));

        app.runtime_key_config_cache = OnceLock::new();
        app.runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nToggleShowHelp=Shift+H\nScoreboardToggle=Escape,Return\n                  MusicToggle=Joy1A\nDbgModeToggle=Ctrl+Alt+D\n",
            )
            .expect("parse remapped help chords")))
            .expect("install remapped help chords");
        // The columns are rebuilt lazily, so drop the memoized text.
        app.runtime_help_text_cache = OnceLock::new();
        let columns = app
            .runtime_help_columns()
            .expect("remapped help columns")
            .clone();

        // Each remapped action shows its live ordered binding name.
        assert!(
            columns.left.contains("Shift+H</c> - "),
            "{}",
            columns.left
        );
        assert!(!columns.left.contains("F1</c> - "), "{}", columns.left);
        // Only the first chord of an ordered list is shown for a single slot.
        assert!(columns.left.contains("Escape</c> - "), "{}", columns.left);
        assert!(!columns.left.contains("Tab</c> - "), "{}", columns.left);
        // A gamepad override has no keyboard name, exactly like an
        // unresolvable code.
        assert!(columns.left.contains("<c ffff00></c> - "), "{}", columns.left);
        // Modifier order follows C4KeyCodeEx::ToString.
        assert!(
            columns.right.contains("Ctrl+Alt+D</c> - "),
            "{}",
            columns.right
        );

        // Draw order and the localized right-hand column are untouched.
        assert!(columns.left.starts_with('['));
        assert_eq!(
            columns.right.lines().count(),
            default_columns.right.lines().count()
        );
    }

    #[test]
    fn runtime_language_table_loads_from_language_pack() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("runtime language-pack install fixture");
        let user_data = tempdir().expect("runtime language-pack user fixture");
        fs::create_dir_all(install.path().join("planet/System.c4g")).expect("empty local System.c4g");
        fs::create_dir(install.path().join("planet/System.c4g/LanguageFI.txt"))
            .expect("unreadable local language-table candidate");
        let pack_system = install
            .path()
            .join("planet/Language.c4g/Finnish.c4g/System.c4g");
        fs::create_dir_all(&pack_system).expect("language-pack System.c4g");
        fs::write(
            pack_system.join("LanguageFI.txt"),
            "IDS_LANG_CHARSET=UTF-8\nProbe=paketti\n",
        )
        .expect("pack Finnish language table");
        let decoy_system = install.path().join("Language.c4g/Decoy.c4g/System.c4g");
        fs::create_dir_all(&decoy_system).expect("non-global decoy Language.c4g");
        fs::write(
            decoy_system.join("LanguageUS.txt"),
            "IDS_LANG_CHARSET=UTF-8\nProbe=wrong namespace\n",
        )
        .expect("decoy language table");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover language-pack fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");
        fs::write(paths.config_file(), "[General]\nLanguageEx=US, FI\n")
            .expect("configure whitespace-prefixed Finnish language");

        assert_eq!(
            classic_loader_language_sequence(&paths).expect("component language sequence"),
            vec!["US".to_string(), " F".to_string()]
        );
        assert_eq!(
            classic_runtime_language_sequence(&paths).expect("LoadLanguage sequence"),
            vec!["US".to_string(), "FI".to_string()]
        );

        let table =
            load_runtime_language_table(Some(&paths)).expect("LanguageFI.txt loads from Finnish.c4g");
        assert_eq!(table.charset, RuntimeHelpCharset::Utf8);
        assert_eq!(
            table.entries.get("Probe").map(String::as_str),
            Some("paketti")
        );
    }

    #[test]
    fn global_system_scripts_use_pack_only_string_table() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("global System language-pack fixture");
        let user_data = tempdir().expect("global System user fixture");
        let system = install.path().join("planet/System.c4g");
        fs::create_dir_all(&system).expect("local System.c4g");
        fs::write(
            system.join("Probe.c"),
            "global func PackProbe() { return \"$PackProbe$\"; }\n",
        )
        .expect("global script");
        let pack_system = install
            .path()
            .join("planet/Language.c4g/Finnish.c4g/System.c4g");
        fs::create_dir_all(&pack_system).expect("pack System.c4g");
        fs::write(
            pack_system.join("StringTblUS.txt"),
            "PackProbe=Pack-localized global\n",
        )
        .expect("pack-only global string table");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover global script fixture");
        paths.ensure_user_dirs().expect("create config directory");
        fs::write(paths.config_file(), "[General]\nLanguageEx=US\n")
            .expect("configure component language");

        let group = Group::open(&system).expect("open System.c4g");
        let scripts = load_classic_global_system_scripts(&paths, &group)
            .expect("localize global scripts from Language.c4g");
        assert_eq!(scripts.len(), 1);
        assert!(scripts[0].1.contains("Pack-localized global"));
        assert!(!scripts[0].1.contains("$PackProbe$"));
    }

    #[test]
    fn global_system_scripts_do_not_hide_invalid_explicit_language_config() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("global System config fixture");
        let user_data = tempdir().expect("global System user fixture");
        let system = install.path().join("planet/System.c4g");
        fs::create_dir_all(&system).expect("local System.c4g");
        fs::write(
            system.join("Probe.c"),
            "global func Probe() { return true; }\n",
        )
        .expect("global script");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover global script fixture");
        paths.ensure_user_dirs().expect("create config directory");
        fs::write(
            paths.config_file(),
            format!("[General]\nLanguageEx={}\n", "X".repeat(1025)),
        )
        .expect("write over-capacity explicit language config");

        let group = Group::open(&system).expect("open System.c4g");
        let error = load_classic_global_system_scripts(&paths, &group)
            .expect_err("explicit invalid config must not use the platform fallback");
        assert!(error.to_string().contains("LanguageEx"));
        assert!(error.to_string().contains("1024-byte"));
    }

    #[test]
    fn runtime_f1_language_table_is_frozen_at_application_construction() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated runtime-help user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let app = test_game_app(320, 200, AudioOptions::default(), Some(&paths))
        .expect("construct app under the US process language");
        persist_config_value(&paths, "General", "LanguageEx", "DE")
            .expect("change config after process language initialization");

        let columns = app
            .runtime_help_columns()
            .expect("read process-global cached columns");
        assert!(columns.left.starts_with("[Game Functions]\n"));
        assert!(!columns.left.contains("Spielfunktionen"));
        assert_eq!(app.needed_material_need, "%s|needs");
        assert_eq!(app.needed_material_none, "%s needs|no more material.");
        assert_eq!(app.object_no_dig, "%s cannot dig.");
    }

    #[test]
    fn l006_runtime_key_config_loads_known_remaps_from_directory_and_packed_groups() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("runtime help install fixture");
        let user_data = tempdir().expect("runtime help user fixture");
        let system = install.path().join("planet/System.c4g");
        let extra = install.path().join("planet/Extra.c4g");
        fs::create_dir_all(&system).expect("fixture System.c4g");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../planet/System.c4g/LanguageUS.txt"),
            system.join("LanguageUS.txt"),
        )
        .expect("copy LanguageUS.txt fixture");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover runtime help fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");
        fs::write(paths.config_file(), "[General]\nLanguageEx=US\n")
            .expect("write fixture language config");

        fs::create_dir_all(&extra).expect("directory Extra.c4g");
        fs::write(
            extra.join("kEyCoNfIg.TxT"),
            "[Keys]\nNetObsNextPlayer=Right,F5\n",
        )
        .expect("directory observer KeyConfig fixture");
        let loaded = load_runtime_global_key_config(Some(&paths))
            .expect("directory observer binding is represented");
        assert_eq!(
            loaded.net_observer_next_player,
            vec![
                RuntimeKeyChord::keyboard(VirtualKeyCode::Right, ModifiersState::empty()),
                RuntimeKeyChord::keyboard(VirtualKeyCode::F5, ModifiersState::empty()),
            ]
        );
        fs::write(
            extra.join("kEyCoNfIg.TxT"),
            "[Keys]\nToggleShowHelp=Shift+F2\nUnknownAction=F9\n",
        )
        .expect("directory KeyConfig fixture");
        let loaded = load_runtime_global_key_config(Some(&paths))
            .expect("known directory remaps load while unknown names only warn");
        assert_eq!(
            loaded.override_for("ToggleShowHelp"),
            Some(
                [RuntimeKeyChord::keyboard(
                    VirtualKeyCode::F2,
                    ModifiersState::SHIFT,
                )]
                .as_slice()
            )
        );
        assert!(loaded.override_for("UnknownAction").is_none());
        guard_runtime_global_key_config(Some(&paths))
            .expect("a present directory KeyConfig is not a fatal guard");

        fs::remove_dir_all(&extra).expect("replace directory Extra.c4g");
        fs::write(
            &extra,
            packed_test_file_group(&[(
                "KEYCONFIG.TXT",
                false,
                b"[Keys]\nNetObsNextPlayer=Right,F5\n",
            )]),
        )
        .expect("packed observer Extra.c4g fixture");
        let loaded = load_runtime_global_key_config(Some(&paths))
            .expect("packed observer binding is represented");
        assert_eq!(
            loaded.net_observer_next_player,
            vec![
                RuntimeKeyChord::keyboard(VirtualKeyCode::Right, ModifiersState::empty()),
                RuntimeKeyChord::keyboard(VirtualKeyCode::F5, ModifiersState::empty()),
            ]
        );
        fs::write(
            &extra,
            packed_test_file_group(&[(
                "KEYCONFIG.TXT",
                false,
                b"[Keys]\nToggleShowHelp=F2\nUnknownAction=F9\n",
            )]),
        )
        .expect("packed Extra.c4g fixture");
        let loaded = load_runtime_global_key_config(Some(&paths))
            .expect("known packed remaps load while unknown names only warn");
        assert_eq!(
            loaded.override_for("ToggleShowHelp"),
            Some(
                [RuntimeKeyChord::keyboard(
                    VirtualKeyCode::F2,
                    ModifiersState::empty(),
                )]
                .as_slice()
            )
        );
        guard_runtime_global_key_config(Some(&paths))
            .expect("a present packed KeyConfig is not a fatal guard");

        fs::write(&extra, b"not a C4Group archive")
            .expect("replace packed Extra.c4g with an unreadable group");
        let ignored = load_runtime_global_key_config(Some(&paths))
            .expect("native ignores an unreadable optional Extra.c4g");
        assert_eq!(ignored, RuntimeKeyConfig::default());
        guard_runtime_global_key_config(Some(&paths))
            .expect("an unreadable optional Extra.c4g does not abort startup");
    }

    #[test]
    fn runtime_f1_key_config_ownership_is_snapshotted_once_per_game() {
        let _lock = env_lock().lock();
        let mut app = new_classic_running_sandbox_app();
        let install = tempdir().expect("runtime help install fixture");
        let user_data = tempdir().expect("runtime help user fixture");
        fs::create_dir_all(install.path().join("planet/System.c4g"))
            .expect("fixture System.c4g directory");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover runtime help fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");
        app.app_paths = Some(paths);
        app.configure_running_state("First game".to_string(), DEFAULT_GROUND_HEIGHT);

        let extra = install.path().join("planet/Extra.c4g");
        fs::create_dir_all(&extra).expect("late Extra.c4g directory");
        fs::write(extra.join("KeyConfig.txt"), "[Keys]\nToggleShowHelp=F2\n")
            .expect("late KeyConfig fixture");
        app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("current game retains its already-loaded key registry");
        assert!(app.runtime_help_visible);

        app.configure_running_state("Second game".to_string(), DEFAULT_GROUND_HEIGHT);
        app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("the remapped default key becomes inert next game");
        assert!(!app.runtime_help_visible);
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("the next game applies the newly discovered remap");
        assert!(app.runtime_help_visible);
    }

    #[test]
    fn runtime_f1_supports_every_upper_board_mode_and_mode_aware_geometry() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("upper-board config install fixture");
        let user_data = tempdir().expect("upper-board config user fixture");
        fs::create_dir_all(install.path().join("planet/System.c4g"))
            .expect("fixture System.c4g directory");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover upper-board fixture");
        paths.ensure_user_dirs().expect("create fixture user dirs");
        fs::write(paths.config_file(), "[Graphics]\nUpperBoard=Small\n")
            .expect("write upper-board config");
        assert_eq!(
            load_display_flags(Some(&paths)).upper_board,
            UpperBoardMode::Small,
            "the production guard must see the persisted mode"
        );

        for (mode, expected_top) in [
            (UpperBoardMode::Hide, 0),
            (UpperBoardMode::Full, 50),
            (UpperBoardMode::Small, 25),
            (UpperBoardMode::Mini, 0),
        ] {
            let mut app = new_classic_running_sandbox_app();
            app.display_flags.upper_board = mode;
            app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("all native upper-board modes support F1 before a sync frame");
            assert!(app.runtime_help_visible, "mode {mode:?}");
            let mut frame = vec![0_u8; 320 * 200 * 4];
            app.render(&mut frame)
                .expect("mode-aware help and viewport render");
            assert_eq!(
                app.graphics.preferred_dialog_rect(None).y,
                expected_top,
                "mode {mode:?}"
            );
        }

        let mut missing_board = new_running_sandbox_app();
        missing_board.graphics = GraphicsSystem::new(
            320,
            200,
            DEFAULT_GROUND_HEIGHT,
            "Missing upper board",
            missing_board.assets.font_arc(),
            Arc::clone(&missing_board.sprite_cache),
            missing_board.assets.cursor_atlas(),
            Arc::new(HudGraphics::default()),
        );
        missing_board
            .graphics
            .set_clonk_fonts(missing_board.assets.clonk_fonts.clone());
        let error = missing_board
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect_err("missing UpperBoard cannot shift the anchor to y=0");
        assert!(error.to_string().contains("UpperBoard resource"));
        assert!(!missing_board.runtime_help_visible);

        let mut tiny = new_classic_running_sandbox_app();
        tiny.graphics = GraphicsSystem::new(
            320,
            50,
            DEFAULT_GROUND_HEIGHT,
            "Tiny help surface",
            tiny.assets.font_arc(),
            Arc::clone(&tiny.sprite_cache),
            tiny.assets.cursor_atlas(),
            tiny.assets.hud_graphics(),
        );
        tiny.graphics
            .set_clonk_fonts(tiny.assets.clonk_fonts.clone());
        let error = tiny
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect_err("tiny Full-mode fallback cannot move help to y=0");
        assert!(error.to_string().contains("50px viewport origin"));
        assert!(!tiny.runtime_help_visible);

        let mut tiny_hide = new_classic_running_sandbox_app();
        tiny_hide.display_flags.upper_board = UpperBoardMode::Hide;
        tiny_hide.graphics = GraphicsSystem::new(
            320,
            1,
            DEFAULT_GROUND_HEIGHT,
            "Tiny hidden-board help surface",
            tiny_hide.assets.font_arc(),
            Arc::clone(&tiny_hide.sprite_cache),
            tiny_hide.assets.cursor_atlas(),
            tiny_hide.assets.hud_graphics(),
        );
        tiny_hide
            .graphics
            .set_clonk_fonts(tiny_hide.assets.clonk_fonts.clone());
        let error = tiny_hide
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect_err("tiny Hide-mode fallback cannot masquerade as valid geometry");
        assert!(error.to_string().contains("message-board bounds"));
        assert!(!tiny_hide.runtime_help_visible);

        let mut visible = new_classic_running_sandbox_app();
        visible.runtime_help_visible = true;
        visible.display_flags.upper_board = UpperBoardMode::Small;
        let mut frame = vec![0x6d; 320 * 200 * 4];
        let sentinel = frame.clone();
        visible
            .render(&mut frame)
            .expect("visible help follows a mode change without stale geometry");
        assert_ne!(frame, sentinel);
        assert!(visible.runtime_help_visible);
        assert_eq!(visible.graphics.preferred_dialog_rect(None).y, 25);

        let mut recover = new_classic_running_sandbox_app();
        recover
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("show help with supported Full geometry");
        assert!(recover.runtime_help_visible);
        recover.display_flags.upper_board = UpperBoardMode::Small;
        let mut frame = vec![0_u8; 320 * 200 * 4];
        recover
            .render(&mut frame)
            .expect("visible help moves to Small geometry");
        assert!(recover.runtime_help_visible);
        assert_eq!(recover.graphics.preferred_dialog_rect(None).y, 25);
        recover
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("hide help after the geometry change");
        assert!(!recover.runtime_help_visible);
    }

    #[test]
    fn upper_board_display_toggle_reinitializes_geometry_synchronously() {
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("establish the Full-mode active viewport");
        let initial_strip_width = app.graphics.upper_board_text_strip_width();
        assert_eq!(app.graphics.preferred_dialog_rect(None).y, 50);
        assert_eq!(app.graphics.viewport_rect(owner).expect("viewport").y, 50);

        app.snapshot.game_time = 100 * 60 * 60;

        app.apply_ingame_menu_action(MenuAction::Display(DisplayToggle::UpperBoard))
            .expect("cycle Full to Small");

        assert_eq!(app.display_flags.upper_board, UpperBoardMode::Small);
        assert!(
            app.graphics.upper_board_text_strip_width() > initial_strip_width,
            "the synchronous reinitialization latches the current 100-hour game time"
        );
        assert_eq!(
            app.graphics.preferred_dialog_rect(None).y,
            25,
            "Display:UpperBoard reinitializes viewport/dialog geometry before the next render"
        );
        assert_eq!(app.graphics.viewport_rect(owner).expect("viewport").y, 25);
        assert_eq!(app.graphics.preferred_dialog_rect(Some(owner)).y, 25);
        assert_eq!(
            app.active_ingame_mouse_viewport()
                .expect("active mouse viewport")
                .rect
                .y,
            25
        );
    }

    #[test]
    fn runtime_f1_help_toggles_beneath_nonmatching_running_layers() {
        let mut game_over = new_game_over_keyboard_app();
        game_over
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("toggle help under game over");
        assert!(game_over.runtime_help_visible);
        assert!(game_over.game_over_dialog.is_some());

        let mut message = new_classic_running_sandbox_app();
        message
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Help",
                    "Modal remains open",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("push running modal");
        message
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("toggle help under nonexclusive message");
        assert!(message.runtime_help_visible);
        assert_eq!(message.message_dialogs.len(), 1);

        let mut context = new_classic_running_sandbox_app();
        context
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(24.0, 24.0),
            )
            .expect("open running context menu");
        assert!(context.context_menu.is_some());
        context
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("unmatched context hotkey falls through to help");
        assert!(context.runtime_help_visible);
        assert!(context.context_menu.is_some());

        let mut object = new_classic_running_sandbox_app();
        assert!(object.open_object_menu().expect("open object menu"));
        object
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("toggle help over object menu");
        assert!(object.runtime_help_visible);
        assert!(object.object_menu.is_some());

        let mut ingame = new_classic_running_sandbox_app();
        ingame.open_ingame_menu().expect("open in-game menu");
        ingame
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("toggle help over player menu");
        assert!(ingame.runtime_help_visible);
        assert!(ingame.ingame_menu.is_some());
    }

    #[test]
    fn custom_player_f1_binding_outranks_help_when_control_scope_is_active() {
        let mut app = new_running_sandbox_app();
        app.bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("player control owns F1 down");
        assert!(!app.runtime_help_visible);
        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
        );
        app.handle_key(VirtualKeyCode::F1, ElementState::Released)
            .expect("player control owns F1 up");
        assert!(!app.runtime_help_visible);

        let mut menu = new_running_sandbox_app();
        menu.bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        menu.open_ingame_menu().expect("open player menu");
        menu.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect("player control owns F1 while player menu is active");
        assert!(!menu.runtime_help_visible);
        assert!(menu.ingame_menu.is_some());
    }

    #[test]
    fn l006_secondary_auto_stop_key_config_f1_f3_binding_uses_matching_owner() {
        let mut app = new_running_sandbox_app();
        let primary = app.local_owner;
        let secondary = add_secondary_local_player_for_mouse_option_test(&mut app);
        app.engine
            .player_mut(primary)
            .expect("primary local player")
            .control
            .control_style = false;
        app.engine
            .player_mut(secondary)
            .expect("secondary local player")
            .control
            .control_style = true;
        app.snapshot = app.engine.snapshot();
        assert_eq!(app.local_controls.owner_for_set(1), Some(secondary));
        let left_mask = 1 << clonk_engine::COM_LEFT;

        for (key, source) in [
            (VirtualKeyCode::F1, b"[Keys]\nKbd2Key7=F1\n".as_slice()),
            (VirtualKeyCode::F3, b"[Keys]\nKbd2Key7=F3\n".as_slice()),
        ] {
            app.runtime_key_config_cache = OnceLock::new();
            app.runtime_key_config_cache
                .set(Ok(
                    parse_runtime_key_config(source).expect("parse secondary player remap")
                ))
                .expect("install secondary player remap");

            app.handle_key(key, ElementState::Pressed)
                .expect("secondary player control owns global-key down");
            assert_eq!(
                app.engine
                    .player(primary)
                    .expect("primary local player")
                    .control
                    .pressed_coms
                    & left_mask,
                0,
            );
            assert_ne!(
                app.engine
                    .player(secondary)
                    .expect("secondary local player")
                    .control
                    .pressed_coms
                    & left_mask,
                0,
            );
            assert!(app.pressed_engine_keys.contains(&key));
            assert!(!app.runtime_help_visible);
            assert!(app.runtime_flash_message.is_none());

            app.handle_key(key, ElementState::Released)
                .expect("secondary auto-stop style owns global-key up");
            assert_eq!(
                app.engine
                    .player(secondary)
                    .expect("secondary local player")
                    .control
                    .pressed_coms
                    & left_mask,
                0,
                "{key:?} release must use the matching secondary owner's auto-stop style",
            );
            assert_eq!(
                app.engine
                    .player(primary)
                    .expect("primary local player")
                    .control
                    .pressed_coms
                    & left_mask,
                0,
            );
            assert!(!app.pressed_engine_keys.contains(&key));
            assert!(!app.runtime_help_visible);
            assert!(app.runtime_flash_message.is_none());
        }
    }

    #[test]
    fn modified_f1_does_not_match_an_unmodified_player_binding() {
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL,
            ModifiersState::ALT | ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::LOGO | ModifiersState::SHIFT,
        ] {
            let mut app = new_running_sandbox_app();
            app.bindings
                .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
            app.engine
                .player_mut(app.local_owner)
                .expect("local player")
                .control
                .control_style = true;
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            let pressed_coms = app
                .engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms;
            let pressed_engine_keys = app.pressed_engine_keys.clone();
            assert!(app.show_startup_hint);

            for state in [ElementState::Pressed, ElementState::Released] {
                app.handle_key(VirtualKeyCode::F1, state)
                    .expect("modified F1 falls through without player dispatch");
                assert!(!app.runtime_help_visible, "modifiers {modifiers:?}");
                assert_eq!(
                    app.engine
                        .player(app.local_owner)
                        .expect("local player")
                        .control
                        .pressed_coms,
                    pressed_coms,
                    "modifiers {modifiers:?}, state {state:?}",
                );
                let mut expected_raw_keys = pressed_engine_keys.clone();
                match state {
                    ElementState::Pressed => {
                        expected_raw_keys.insert(VirtualKeyCode::F1);
                    }
                    ElementState::Released => {
                        expected_raw_keys.remove(&VirtualKeyCode::F1);
                    }
                }
                assert_eq!(
                        app.pressed_engine_keys, expected_raw_keys,
                        "raw physical state precedes modified priority dispatch: modifiers {modifiers:?}, state {state:?}",
                    );
                assert!(
                    app.show_startup_hint,
                    "modifiers {modifiers:?}, state {state:?}"
                );
            }
        }
    }

    #[test]
    fn modified_f1_refuses_an_unrepresented_key_config_on_both_edges() {
        let mut app = new_running_sandbox_app();
        app.runtime_key_config_cache = OnceLock::new();
        app.runtime_key_config_cache
            .set(Err("Extra.c4g/KeyConfig.txt override".to_string()))
            .expect("empty input key-config cache");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set modified F1 chord");

        for state in [ElementState::Pressed, ElementState::Released] {
            let error = app
                .handle_key(VirtualKeyCode::F1, state)
                .expect_err("custom global-key ownership must precede modifier fallthrough");
            assert!(matches!(
                error,
                EngineError::ClassicMenuParityBoundary { .. }
            ));
            assert!(!app.runtime_help_visible);
        }
    }

    #[test]
    fn unresolved_runtime_help_language_fails_typed_before_pixels() {
        let mut input_app = new_running_sandbox_app();
        input_app.runtime_key_config_cache = OnceLock::new();
        input_app
            .runtime_key_config_cache
            .set(Err("Extra.c4g/KeyConfig.txt override".to_string()))
            .expect("empty input key-config cache");
        let error = input_app
            .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
            .expect_err("unrepresented key config must fail before toggling");
        assert!(matches!(
            error,
            EngineError::ClassicMenuParityBoundary { .. }
        ));
        assert!(!input_app.runtime_help_visible);

        let mut app = new_classic_running_sandbox_app();
        app.runtime_help_visible = true;
        app.runtime_help_text_cache = OnceLock::new();
        app.runtime_help_text_cache
            .set(Err("LanguageZZ.txt cannot be resolved".to_string()))
            .expect("empty help text cache");
        let before_surface = app.graphics.surface().pixels().to_vec();
        let mut frame = vec![0x6d; 320 * 200 * 4];
        let sentinel = frame.clone();

        let error = app
            .render(&mut frame)
            .expect_err("unresolved localized help cannot draw a partial overlay");
        assert!(error.to_string().contains("runtime F1 help resources"));
        assert!(error.to_string().contains("LanguageZZ.txt"));
        assert_eq!(frame, sentinel);
        assert_eq!(app.graphics.surface().pixels(), before_surface.as_slice());
    }

    #[test]
    fn modified_f1_retains_downstream_priority_without_toggling_help() {
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL,
            ModifiersState::ALT | ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::LOGO | ModifiersState::SHIFT,
        ] {
            let mut app = new_running_sandbox_app();
            app.status_text.clear();
            app.snapshot.hud.messages.clear();
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            let before = runtime_global_ui_snapshot(&app);
            let mut before_pixels = vec![0_u8; 320 * 200 * 4];
            app.render(&mut before_pixels)
                .expect("render before modified F1");

            app.handle_key(VirtualKeyCode::F1, ElementState::Pressed)
                .expect("modified F1 reaches the ordinary downstream route");
            app.handle_key(VirtualKeyCode::F1, ElementState::Released)
                .expect("modified F1 release reaches the downstream route");

            let after = runtime_global_ui_snapshot(&app);
            assert_eq!(after.status_text, before.status_text);
            assert_eq!(after.message_dialogs, before.message_dialogs);
            assert_eq!(after.game_over_open, before.game_over_open);
            assert_eq!(after.ingame_page, before.ingame_page);
            assert_eq!(after.object_menu_open, before.object_menu_open);
            assert_eq!(after.context_menu_open, before.context_menu_open);
            assert_eq!(after.runtime_help_visible, before.runtime_help_visible);
            assert_eq!(after.pressed_engine_keys, before.pressed_engine_keys);
            assert_eq!(
                after.message_dialog_consumed_keys,
                before.message_dialog_consumed_keys
            );
            let mut after_pixels = vec![0_u8; 320 * 200 * 4];
            app.render(&mut after_pixels)
                .expect("render after modified F1");
            assert_eq!(after_pixels, before_pixels);
        }
    }

    #[test]
    fn l128_f4_player_tooltip_names_follow_retained_visibility_and_effective_name() {
        let mut app = new_classic_running_sandbox_app();
        let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
        app.control_clients
            .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
        app.control_player_infos.replace_snapshot(
            3,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                players: vec![
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 1,
                        name: clonk_engine::LegacyCString::from_bytes(b"Raw".to_vec())
                            .expect("raw player name"),
                        league_account: clonk_engine::LegacyCString::from_bytes(
                            b"Visible account".to_vec(),
                        )
                        .expect("league account"),
                        ..Default::default()
                    },
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 2,
                        name: clonk_engine::LegacyCString::from_bytes(b"Removed".to_vec())
                            .expect("removed player name"),
                        flags: clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                        ..Default::default()
                    },
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 3,
                        name: clonk_engine::LegacyCString::from_bytes(b"Invisible".to_vec())
                            .expect("invisible player name"),
                        flags: clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
        );

        let (_, rows, _) = app.runtime_client_list_snapshot();
        assert_eq!(
            rows.iter()
                .find(|row| row.client_id == 7)
                .map(|row| row.player_names.clone()),
            Some(vec!["Visible account".to_string()])
        );
    }

    #[test]
    fn l128_f4_control_mode_waits_for_status_commit() {
        let mut app = new_classic_running_sandbox_app();
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
        app.runtime_network_control_mode = Some(0);
        app.runtime_network_committed_control_mode = Some(0);
        app.control_clients
            .replace_snapshot([message_client(0, b"Host")]);
        let labels = app.classic_lobby_option_labels();
        app.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("open runtime client list");

        app.apply_runtime_client_list_option(LobbyOptionKind::ControlMode, 1)
            .expect("request central runtime control mode");
        assert_eq!(app.runtime_network_control_mode, Some(1));
        assert_eq!(app.runtime_network_committed_control_mode, Some(0));
        assert_eq!(
            app.runtime_client_list
                .as_ref()
                .expect("F4 dialog remains open")
                .option_rows()
                .iter()
                .find(|row| row.kind == LobbyOptionKind::ControlMode)
                .map(|row| row.value.as_str()),
            Some(labels.control_mode_decentral.as_str())
        );

        let expected = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 40,
        };
        assert!(commands
            .take_runtime_status_commands()
            .iter()
            .any(|command| command == &network::TestRuntimeStatusCommand::Change(expected)));
        app.handle_status_committed(expected)
            .expect("commit central runtime control mode");
        app.refresh_runtime_client_list();
        assert_eq!(app.runtime_network_committed_control_mode, Some(1));
        assert_eq!(
            app.runtime_client_list
                .as_ref()
                .expect("F4 dialog remains open")
                .option_rows()
                .iter()
                .find(|row| row.kind == LobbyOptionKind::ControlMode)
                .map(|row| row.value.as_str()),
            Some(labels.control_mode_central.as_str())
        );

        app.host_reference_paused = true;
        app.apply_runtime_client_list_option(LobbyOptionKind::ControlMode, 0)
            .expect("request decentral mode while paused");
        let paused = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_PAUSE,
            control_mode: 0,
            target_tick: 40,
        };
        assert!(commands
            .take_runtime_status_commands()
            .iter()
            .any(|command| command == &network::TestRuntimeStatusCommand::Change(paused)));
        app.handle_status_committed(paused)
            .expect("commit paused status without applying its control mode");
        app.refresh_runtime_client_list();
        assert_eq!(app.runtime_network_committed_control_mode, Some(1));
        assert_eq!(
            app.runtime_client_list
                .as_ref()
                .expect("F4 dialog remains open")
                .option_rows()
                .iter()
                .find(|row| row.kind == LobbyOptionKind::ControlMode)
                .map(|row| row.value.as_str()),
            Some(labels.control_mode_central.as_str())
        );

        let resumed = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            ..paused
        };
        app.handle_status_committed(resumed)
            .expect("apply the pending control mode on Go");
        app.refresh_runtime_client_list();
        assert_eq!(app.runtime_network_committed_control_mode, Some(0));
        assert_eq!(
            app.runtime_client_list
                .as_ref()
                .expect("F4 dialog remains open")
                .option_rows()
                .iter()
                .find(|row| row.kind == LobbyOptionKind::ControlMode)
                .map(|row| row.value.as_str()),
            Some(labels.control_mode_decentral.as_str())
        );
    }

    #[test]
    fn runtime_pause_applies_direct_script_halt_and_toggle_requests() {
        let mut app = new_running_sandbox_app();
        app.engine.clear_scenario_script();
        app.engine
            .install_scenario_script_with_convention(
                "PauseGameProbe.c",
                "#strict 3\nfunc Halt() { PauseGame(); }\nfunc Toggle() { PauseGame(true); }",
                true,
            )
            .expect("install PauseGame probe");

        let initial_frame = app.engine.frame();
        app.engine
            .call_scenario_script_function("Halt", Vec::new())
            .expect("queue script halt");
        app.update()
            .expect("apply direct script halt before simulation");
        assert_eq!(app.engine.frame(), initial_frame);
        assert_ne!(app.offline_halt_count, 0);

        app.engine
            .call_scenario_script_function("Toggle", Vec::new())
            .expect("queue script resume while halted");
        app.update()
            .expect("a pre-existing toggle drains before the halt gate");
        assert_eq!(app.engine.frame(), initial_frame + 1);
        assert_eq!(app.offline_halt_count, 0);

        app.engine
            .call_scenario_script_function("Toggle", Vec::new())
            .expect("queue a running script toggle");
        app.update()
            .expect("direct toggle halts before another tick");
        assert_eq!(app.engine.frame(), initial_frame + 1);
        assert_ne!(app.offline_halt_count, 0);

        let mut game_over = new_game_over_keyboard_app();
        game_over.engine.clear_scenario_script();
        game_over
            .engine
            .install_scenario_script_with_convention(
                "PauseGameToggle.c",
                "#strict 3\nfunc Halt() { PauseGame(); }\nfunc Toggle() { PauseGame(true); }",
                true,
            )
            .expect("install game-over pause probes");
        game_over
            .engine
            .call_scenario_script_function("Halt", Vec::new())
            .expect("queue halt during evaluation");
        game_over
            .engine
            .call_scenario_script_function("Toggle", Vec::new())
            .expect("queue toggle during evaluation");
        game_over
            .update()
            .expect("evaluation consumes queued pause requests before returning");
        assert_eq!(
            game_over.offline_halt_count, 1,
            "evaluation keeps the halt acquired by C4GameOverDlg::OnShown"
        );
        game_over
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("set the Continue mnemonic modifier");
        game_over
            .handle_key(VirtualKeyCode::C, ElementState::Pressed)
            .expect("Continue closes evaluation");
        assert!(game_over.game_over_dialog.is_none());
        game_over
            .update()
            .expect("discarded evaluation requests do not replay after Continue");
        assert_eq!(game_over.offline_halt_count, 0);
    }

    #[test]
    fn runtime_pause_sync_control_inside_go_commit_observes_running_status() {
        for (local_client_id, script, expected_vote_data) in [
            (0, b"PauseGame()".as_slice(), Some(1)),
            (0, b"PauseGame(true)".as_slice(), Some(1)),
            (7, b"PauseGame()".as_slice(), None),
            (7, b"PauseGame(true)".as_slice(), Some(0)),
        ] {
            let mut app = new_running_sandbox_app();
            let (_events, mut commands) = install_running_network_stub(&mut app, local_client_id, 0, 1);
            app.network_is_league = true;
            app.network_control_running = false;
            let go = clonk_network::NetworkStatus {
                state: clonk_network::NETWORK_STATE_GO,
                control_mode: 1,
                target_tick: 0,
            };
            app.runtime_network_status_barrier = Some(RuntimeNetworkStatusBarrier {
                status: go,
                local_reached: true,
                actual_control_tick: Some(0),
            });
            app.network_sync.queue(
                0,
                0,
                vec![NetworkControl::Script(clonk_engine::ScriptControlData {
                    target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                    strictness: clonk_engine::ScriptStrictness::Strict3,
                    script: clonk_engine::LegacyCString::from_bytes(script.to_vec())
                        .expect("script is NUL-free"),
                    by_client: 0,
                })],
            );
            app.handle_status_committed(go)
                .expect("execute the sync control after Go becomes acknowledged");

            if local_client_id == 0 {
                let pause = app
                    .runtime_network_status_barrier
                    .expect("PauseGame starts a new host Pause barrier")
                    .status;
                assert_eq!(pause.state, clonk_network::NETWORK_STATE_PAUSE);
                assert!(app.league_votes.paused_for_vote);
            } else {
                assert!(
                    app.runtime_network_status_barrier.is_none(),
                    "client {local_client_id} left an unexpected barrier for {script:?}: {:?}",
                    app.runtime_network_status_barrier
                );
                assert!(!app.league_votes.paused_for_vote);
            }
            let expected_votes = expected_vote_data
                .into_iter()
                .map(|data| clonk_engine::VoteControlData {
                    vote_type: clonk_engine::VOTE_TYPE_PAUSE,
                    approve: true,
                    data,
                    by_client: i32::try_from(local_client_id).unwrap(),
                })
                .collect::<Vec<_>>();
            assert_eq!(
                commands.take_submitted_votes(),
                expected_votes,
                "client {local_client_id} observes the native status ordering for {script:?}"
            );
            assert!(!app.take_exit_request());
        }
    }

    /// `StdCompilerINIRead::Boolean` (StdCompiler.cpp:692-715) accepts a
    /// leading `1`/`0` not followed by another digit, or a case-sensitive
    /// `true`/`false` prefix. Anything else signals not-found, so the field's
    /// adapted default stays in force — it does not collapse to false.
    #[test]
    fn runtime_config_booleans_follow_stdcompiler_grammar_and_preserve_defaults() {
        // The token grammar itself.
        for (raw, expected) in [
            ("1", Some(true)),
            ("0", Some(false)),
            ("true", Some(true)),
            ("false", Some(false)),
            // A prefix is enough: C++ advances pPos and ignores the rest.
            ("1 ; trailing comment", Some(true)),
            ("truely", Some(true)),
            ("falsehood", Some(false)),
            // A following digit rejects the numeric form.
            ("10", None),
            ("01", None),
            // Case-sensitive, and no leading whitespace is skipped.
            ("TRUE", None),
            ("True", None),
            ("FALSE", None),
            (" 1", None),
            ("\t0", None),
            // Non-native aliases C++ never accepted.
            ("yes", None),
            ("on", None),
            ("no", None),
            ("off", None),
            ("", None),
            ("wobble", None),
        ] {
            assert_eq!(
                parse_native_config_bool(raw),
                expected,
                "{raw:?} must follow the native Boolean grammar"
            );
        }

        // Invalid input keeps each key's adapted default, in both directions.
        let flags = |body: &str| {
            let root = tempdir().expect("boolean config root");
            let user_data = tempdir().expect("boolean user data");
            fs::create_dir_all(root.path().join("planet/System.c4g")).expect("System group");
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(root.path())),
                ("LC_USER_DATA_DIR", Some(user_data.path())),
            ]);
            let paths = AppPaths::discover().expect("fixture app paths");
            paths.ensure_user_dirs().expect("fixture user directories");
            fs::write(paths.config_file(), body).expect("write fixture config");
            load_display_flags(Some(&paths))
        };

        // ShowCrewNames adapts true, ShowClock adapts false.
        let defaults = flags("[Graphics]\nName=Tester\n");
        assert!(defaults.player_names);
        assert!(!defaults.clock);

        // Valid tokens flip both.
        let flipped = flags("[Graphics]\nShowCrewNames=0\nShowClock=1\n");
        assert!(!flipped.player_names);
        assert!(flipped.clock);

        // Invalid values leave both adapted defaults untouched. The old
        // permissive parser collapsed these to false, silently disabling a
        // default-true flag.
        // `" 1"` is only reachable at the raw-value level above: the INI
        // reader strips the whitespace after `=` before the Boolean field
        // consumes it.
        for invalid in ["TRUE", "yes", "on", "10", "wobble"] {
            let kept = flags(&format!(
                "[Graphics]\nShowCrewNames={invalid}\nShowClock={invalid}\n"
            ));
            assert!(
                kept.player_names,
                "{invalid:?} must leave the default-true ShowCrewNames alone"
            );
            assert!(
                !kept.clock,
                "{invalid:?} must leave the default-false ShowClock alone"
            );
        }
    
}

    /// `StdCompilerINIRead::ReadNum` (StdCompiler.h:705-724) skips leading
    /// whitespace, selects base 16 only for a leading `0x`/`0X`, consumes the
    /// longest valid numeric prefix and ignores the rest. No digits is
    /// not-found, so the field's adapted default survives.
    #[test]
    fn runtime_config_scalars_follow_stdcompiler_prefix_hex_and_narrowing() {
        let parse = |raw: &str| parse_startup_config_integer(raw.as_bytes());

        // Plain decimal, sign and surrounding whitespace.
        assert_eq!(parse("42"), Some(42));
        assert_eq!(parse("-7"), Some(-7));
        assert_eq!(parse("+7"), Some(7));
        assert_eq!(parse("   19"), Some(19));
        assert_eq!(parse("\t\r\n5"), Some(5));

        // Hex only with an explicit 0x/0X prefix; a bare leading zero is
        // decimal, and `0x` with no digits is still the value zero because
        // strtol consumed the leading `0`.
        assert_eq!(parse("0x1f"), Some(31));
        assert_eq!(parse("0X1F"), Some(31));
        assert_eq!(parse("010"), Some(10));
        assert_eq!(parse("0x"), Some(0));

        // The longest valid prefix wins and trailing bytes are tolerated.
        assert_eq!(parse("30ms"), Some(30));
        assert_eq!(parse("12 ; comment"), Some(12));
        assert_eq!(parse("0x1fg"), Some(31));
        // A decimal parse stops at the first non-digit, so `1e3` is 1.
        assert_eq!(parse("1e3"), Some(1));

        // No digits at all is not-found.
        assert_eq!(parse(""), None);
        assert_eq!(parse("wobble"), None);
        assert_eq!(parse("-"), None);
        assert_eq!(parse("   "), None);

        // The live settings readers share that grammar and keep their adapted
        // defaults when it yields nothing.
        let audio = |body: &str| {
            let root = tempdir().expect("scalar config root");
            let user_data = tempdir().expect("scalar user data");
            fs::create_dir_all(root.path().join("planet/System.c4g")).expect("System group");
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(root.path())),
                ("LC_USER_DATA_DIR", Some(user_data.path())),
            ]);
            let paths = AppPaths::discover().expect("fixture app paths");
            paths.ensure_user_dirs().expect("fixture user directories");
            fs::write(paths.config_file(), format!("[Sound]\n{body}\n"))
                .expect("write fixture config");
            AudioOptions::load(Some(&paths))
        };
        let defaults = AudioOptions::default();
        // A hex volume is honoured, exactly as C++ would read it.
        assert_eq!(audio("MusicVolume=0x40").music_volume, 64.0 / 100.0);
        // A tolerated suffix keeps the numeric prefix.
        assert_eq!(audio("SoundVolume=75%").sound_volume, 75.0 / 100.0);
        // Digit-less input leaves the adapted default alone rather than
        // clamping to the range floor.
        for invalid in ["", "loud", "-", "   "] {
            let kept = audio(&format!("MusicVolume={invalid}\nMaxChannels={invalid}"));
            assert_eq!(kept.music_volume, defaults.music_volume, "{invalid:?}");
            assert_eq!(kept.max_channels, defaults.max_channels, "{invalid:?}");
        }
    
}
