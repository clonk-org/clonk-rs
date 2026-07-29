    // Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
    // sequence, not a child module, so test ids stay `tests::<fn>`.

    #[test]
    fn l028_console_lobby_start_is_host_only_and_restarts_countdown() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated console lobby config");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "Lobby", "CountdownTime", "7")
            .expect("configure console lobby countdown");
        let mut host = new_menu_app(640, 480);
        host.app_paths = Some(paths);
        let (_events, mut commands) = install_classic_host_network_stub(&mut host);
        // C4Application::OnCommand forwards non-/start lobby commands through
        // C4MessageInput::ProcessInput, whose /set maxplayer branch accepts
        // the network control host (oracle src/C4Application.cpp:622-644;
        // src/C4MessageInput.cpp:472-490).
        host.process_console_command("/set maxplayer 24")
            .expect("console lobby maximum-player command");
        assert_eq!(
            commands.take_submitted_control_sets(),
            vec![clonk_network::LegacyControlSet {
                value_type: 2,
                data: 24,
                by_client: 0,
            }]
        );
        host.process_console_command("/start")
            .expect("start configured-default countdown");
        assert_eq!(
            commands.take_submitted_lobby_countdowns(),
            vec![clonk_network::LobbyCountdownPacket::new(7)]
        );
        host.process_console_command("/abort")
            .expect("non-start lobby input routes through MessageInput");
        assert!(commands.take_submitted_lobby_countdowns().is_empty());
        assert_eq!(
            host.host_lobby_countdown,
            Some(HostLobbyCountdown::with_seconds(7))
        );
        assert!(host
            .classic_host_lobby
            .as_ref()
            .expect("classic host lobby")
            .controller
            .logs()
            .last()
            .is_some_and(|line| line.text.contains("Unknown command: \"abort\"")));
        host.process_console_command("/starter 12junk")
            .expect("native prefix command replaces countdown");
        assert_eq!(
            commands.take_submitted_lobby_countdowns(),
            vec![
                clonk_network::LobbyCountdownPacket::new(-1),
                clonk_network::LobbyCountdownPacket::new(12),
            ]
        );
        host.ui_sound_log.clear();
        host.process_console_command("/start ")
            .expect("invalid explicit timeout is consumed");
        assert!(commands.take_submitted_lobby_countdowns().is_empty());
        assert!(
            host.ui_sound_log.is_empty(),
            "native console validation only logs the usage error"
        );
        assert_eq!(
            host.classic_host_lobby
                .as_ref()
                .expect("classic host lobby")
                .controller
                .logs()
                .last()
                .map(|line| line.text.as_str()),
            Some("Usage: /start [timer]")
        );
        host.process_console_command("/start 0")
            .expect("zero remains a one-second console countdown");
        assert_eq!(
            commands.take_submitted_lobby_countdowns(),
            vec![
                clonk_network::LobbyCountdownPacket::new(-1),
                clonk_network::LobbyCountdownPacket::new(0),
            ]
        );
        assert_eq!(
            host.host_lobby_countdown,
            Some(HostLobbyCountdown::with_seconds(0))
        );
        assert_eq!(host.mode, AppMode::Menu);

        install_message_fixture(&mut host);
        host.snapshot = host.engine.snapshot();
        host.process_console_command("/private Sender secret")
            .expect("running-only private syntax is rejected in the lobby");
        assert!(commands.take_submitted_messages().is_empty());
        assert!(host
            .classic_host_lobby
            .as_ref()
            .expect("classic host lobby")
            .controller
            .logs()
            .last()
            .is_some_and(|line| line.text.contains("Unknown command: \"private\"")));
        host.process_console_command("\"hello")
            .expect("leading quote remains an ordinary lobby message");
        assert_eq!(
            commands.take_submitted_messages(),
            vec![MessageControlData {
                message_type: MESSAGE_TYPE_NORMAL,
                player: -1,
                to_player: -1,
                message: LegacyCString::from_bytes(b"\"hello".to_vec())
                    .expect("fixture message is NUL-free"),
                by_client: 0,
            }]
        );
        assert!(host.engine.set_team_distribution(4));
        host.process_console_command("^hidden")
            .expect("hidden teams reject lobby team messages");
        assert!(commands.take_submitted_messages().is_empty());
        assert_eq!(
            host.classic_host_lobby
                .as_ref()
                .expect("classic host lobby")
                .controller
                .logs()
                .last()
                .map(|line| line.text.as_str()),
            Some("Can't send team message: Teams not known.")
        );

        let mut client = new_menu_app(640, 480);
        client.startup_view = StartupView::NetworkLobby;
        client.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
        client
            .process_console_command("/start 3")
            .expect("client start is consumed");
        assert_eq!(
            client
                .network_lobby
                .as_ref()
                .expect("generic client lobby")
                .logs
                .last()
                .map(|line| line.text.as_str()),
            Some("Host only!")
        );
    }

    #[test]
    fn muted_loop_releases_channel_but_survives_and_restarts_on_unmute() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(1_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        snapshot.audio.push(test_sound_command(true));
        audio.process_audio(&snapshot, &mut runtime_music_enabled);
        snapshot.audio.clear();

        let original_channel = audio.active_channels[&key]
            .channel
            .expect("enabled loop has a mixer channel");
        assert!(audio.system.channel_is_playing(original_channel));

        audio.options.sound_enabled = false;
        audio.process_audio(&snapshot, &mut runtime_music_enabled);
        assert!(audio.active_channels.contains_key(&key));
        assert!(audio.active_channels[&key].channel.is_none());
        assert!(!audio.system.channel_is_playing(original_channel));

        audio.options.sound_enabled = true;
        audio.process_audio(&snapshot, &mut runtime_music_enabled);
        let restored_channel = audio.active_channels[&key]
            .channel
            .expect("unmuted loop reacquires a mixer channel");
        assert!(audio.system.channel_is_playing(restored_channel));
    }

    #[test]
    fn next_mission_lookup_normalizes_cpp_backslashes_and_case() {
        // C4Application::QuitGame converts the SetNextMission path's
        // backslashes to the platform separator before starting it
        // (C4Application.cpp:385-399).
        let scenario = FrontendScenario {
            identifier: "Tutorial.c4f/Tutorial02.c4s".to_string(),
            title: "The First Hut".to_string(),
            description: None,
            kind: ScenarioKind::Scenario,
            is_editable: false,
            is_playable: true,
            mission_access: None,
            path: None,
            source_paths: Vec::new(),
            root_label: None,
            preview: None,
            title_picture: None,
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
            author: None,
            version: None,
            local_only: None,
            allow_user_change: None,
            definition_modules: Vec::new(),
        };
        let catalog = HashMap::from([(scenario.identifier.clone(), scenario)]);

        assert_eq!(
            resolve_next_mission_scenario(&catalog, "tutorial.c4f\\TUTORIAL02.c4s")
                .map(|scenario| scenario.title),
            Some("The First Hut".to_string())
        );
    }

    #[test]
    fn placeholder_preview_has_expected_dimensions() {
        let preview = generate_preview_placeholder(ScenarioKind::Scenario, "Alpha");
        assert_eq!(preview.width(), PLACEHOLDER_PREVIEW_WIDTH);
        assert_eq!(preview.height(), PLACEHOLDER_PREVIEW_HEIGHT);
        let pixels = preview.pixels();
        let mut chunks = pixels.chunks_exact(4);
        let mut varied = false;
        if let Some(first) = chunks.next() {
            for chunk in chunks {
                if chunk != first {
                    varied = true;
                    break;
                }
            }
        }
        assert!(varied, "placeholder preview should contain color variation");
    }

    #[test]
    fn local_scenario_load_failure_returns_to_remembered_selector_with_error_log() {
        let _lock = env_lock().lock();
        reset_cached_app_paths();
        let user_data = tempdir().expect("isolated local-start failure user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        let StagedNetworkHostScenario {
            frontend,
            loader_screen,
            loader_refreshed_resources,
            ..
        } = prepare_tutorial_host_lobby(&app, repository);
        app.open_scenario_browser();
        wait_for_scenario_selector_discovery(&mut app);

        let (sender, receiver) = mpsc::channel();
        app.loader_screen = loader_screen;
        app.loading_state = Some(ScenarioLoadingState::new(
            frontend,
            loader_refreshed_resources,
            HashMap::new(),
            Vec::new(),
            receiver,
        ));
        app.mode = AppMode::Loading;
        sender
            .send(ScenarioLoadingEvent::Finished(Err(
                "controlled local load failure".to_string(),
            )))
            .expect("queue controlled local load failure");

        app.poll_loading()
            .expect("failed local load restarts the startup selector");

        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert_eq!(app.scenario_selector_mode, ScenarioSelectorMode::Local);
        assert_eq!(
            app.last_startup_dialog,
            StartupDialog::ScenarioBrowser(ScenarioSelectorMode::Local)
        );
        assert_eq!(app.startup_scenario_back_dialog, None);
        assert!(app.loading_state.is_none());
        // The return through PreInit re-initializes the loader screen for the
        // next game (src/C4Application.cpp:242-247,373-389).
        assert!(app.loader_screen.is_some());
        assert!(app.loader_error.is_none());
        assert!(app.active_scenario.is_none());
        assert!(app.active_definition_load.is_none());
        assert!(app.active_global_gui_failures.is_empty());
        assert!(app.runtime_client_list.is_none());
        assert_startup_error_log(&app, "controlled local load failure");
        assert_eq!(
            app.startup_restart_diagnostics,
            StartupRestartDiagnostics::default()
        );

        let mut frame = vec![0x4c; 800 * 600 * 4];
        app.render(&mut frame)
            .expect("render restored local selector and Error Log");
        assert!(frame.iter().any(|byte| *byte != 0x4c));
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss local startup Error Log");
        assert!(app.message_dialogs.is_empty());
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert_eq!(app.scenario_selector_mode, ScenarioSelectorMode::Local);
        assert_eq!(app.startup_scenario_back_dialog, None);
        reset_cached_app_paths();
    }

    #[test]
    fn l148_restart_diagnostics_bound_order_deduplicate_and_reset() {
        let mut diagnostics = StartupRestartDiagnostics::default();
        diagnostics.mark_quit_with_error();
        for index in 0..=STARTUP_RESTART_LOG_CAPACITY {
            diagnostics.add_log_entry(format!("entry-{index:03}"));
        }
        assert_eq!(
            diagnostics.take_presentation(),
            Some(StartupRestartPresentation::Ringbuffer(
                (1..=STARTUP_RESTART_LOG_CAPACITY)
                    .map(|index| format!("entry-{index:03}"))
                    .collect()
            ))
        );
        assert_eq!(diagnostics, StartupRestartDiagnostics::default());

        diagnostics.add_fatal_error("fatal");
        diagnostics.add_fatal_error("fatal");
        diagnostics.begin_game_init();
        diagnostics.add_log_entry("ordinary");
        assert_eq!(
            diagnostics.take_presentation(),
            Some(StartupRestartPresentation::Fatal("fatal".to_string()))
        );
        assert_eq!(diagnostics, StartupRestartDiagnostics::default());

        diagnostics.mark_quit_with_error();
        assert_eq!(
            diagnostics.take_presentation(),
            Some(StartupRestartPresentation::Empty)
        );
        assert_eq!(diagnostics, StartupRestartDiagnostics::default());
    }

    #[test]
    fn l148_disconnected_startup_worker_reaches_ringbuffer_only_restart_branch() {
        let mut app = new_real_classic_menu_app(800, 600);
        attach_l040_network_dialog(&mut app);
        let (sender, receiver) =
            mpsc::channel::<std::result::Result<(NetworkMode, NetworkManager), NetworkStartError>>();
        drop(sender);
        app.startup_network_connection = Some(StartupNetworkConnection::new(
            receiver,
            None,
            StartupNetworkPurpose::Join,
        ));

        app.poll_startup_network_connection()
            .expect("disconnected worker restarts startup with retained log");

        let info = app.runtime_client_list.as_ref().expect("static Error Log");
        assert!(info.is_static_info_only());
        assert_eq!(
            info.info_lines(),
            ["network worker disconnected before reporting readiness"]
        );
        assert!(app.message_dialogs.is_empty());
        assert!(app.status_text.is_empty());
        let (preferred, line_height) = app
            .runtime_client_list_input_geometry()
            .expect("static InfoDialog geometry");
        let bottom_close = app
            .runtime_client_list
            .as_ref()
            .and_then(|dialog| dialog.info_layout(preferred, line_height))
            .and_then(|layout| layout.bottom_close_button)
            .expect("bottom Close button");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(bottom_close.x + bottom_close.w / 2),
            f64::from(bottom_close.y + bottom_close.h / 2),
        ))
        .expect("point at bottom Close button");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press bottom Close button");
        app.handle_mouse_button(ElementState::Released)
            .expect("release bottom Close button");
        assert!(app.runtime_client_list.is_none());
        assert_eq!(
            app.startup_restart_diagnostics,
            StartupRestartDiagnostics::default()
        );
    }

    #[test]
    fn l148_restart_ringbuffer_uses_static_ten_line_error_log_info_dialog() {
        let mut app = new_real_classic_menu_app(800, 600);
        attach_l040_network_dialog(&mut app);
        app.startup_network_dialog
            .as_mut()
            .expect("remembered NetDlg")
            .set_join_address("remembered.example:11112");
        app.status_text = "stale generic status".to_string();
        let mut entries = (0..16)
            .map(|index| format!("retained-log-{index:02}"))
            .collect::<Vec<_>>();
        entries[15] = format!("retained-log-15 {}TAIL", "wrapped-segment ".repeat(80));

        app.startup_restart_diagnostics.mark_quit_with_error();
        for entry in &entries {
            app.startup_restart_diagnostics.add_log_entry(entry.clone());
        }
        app.finish_startup_network_restart(StartupNetworkPurpose::Join)
            .expect("reconstruct NetDlg and show retained log");

        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::NetworkGame);
        assert!(app.startup_network_dialog.is_some());
        assert!(app.message_dialogs.is_empty());
        assert!(app.status_text.is_empty());
        let info = app.runtime_client_list.as_ref().expect("Error Log info");
        assert!(info.is_info_only());
        assert!(info.info_is_open());
        assert_eq!(info.info_client_id(), None);
        assert_eq!(info.info_caption(), "Error Log");
        assert_eq!(info.info_requested_line_count(), 10);
        assert_eq!(info.info_lines(), entries);

        let (preferred, _) = app
            .runtime_client_list_input_geometry()
            .expect("InfoDialog geometry");
        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        assert_eq!(
            app.runtime_client_list
                .as_ref()
                .expect("Error Log info")
                .visible_info_lines(preferred, &fonts.text)
                .first()
                .map(String::as_str),
            Some("retained-log-00")
        );
        assert!(app
            .runtime_client_list
            .as_ref()
            .expect("Error Log info")
            .info_scroll_metrics(preferred, &fonts.text)
            .is_some_and(|metrics| metrics.max_scroll > 0));
        let mut frame = vec![0x4c; 800 * 600 * 4];
        app.render(&mut frame)
            .expect("render reconstructed NetDlg and Error Log info");
        assert!(frame.iter().any(|byte| *byte != 0x4c));

        app.handle_key(VirtualKeyCode::End, ElementState::Pressed)
            .expect("scroll retained log to end");
        assert!(app
            .runtime_client_list
            .as_ref()
            .expect("scrolled Error Log info")
            .visible_info_lines(preferred, &fonts.text)
            .last()
            .is_some_and(|line| line.ends_with("TAIL")));
        app.handle_key(VirtualKeyCode::End, ElementState::Released)
            .expect("release retained-log scroll key");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("dismiss retained Error Log info");
        assert!(app.runtime_client_list.is_none());
        assert_eq!(app.startup_view, StartupView::NetworkGame);
        assert!(app.startup_network_dialog.is_some());
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("dismissed info owns Return release");
        assert_eq!(app.startup_view, StartupView::NetworkGame);
        assert_eq!(
            app.startup_restart_diagnostics,
            StartupRestartDiagnostics::default()
        );
    }

    #[test]
    fn l148_empty_restart_log_uses_regular_error_modal_over_restored_host_selector() {
        let mut app = new_real_classic_menu_app(800, 600);
        app.open_network_game_dialog();
        app.open_network_host_scenario_browser();
        app.status_text = "stale generic status".to_string();

        app.startup_restart_diagnostics.mark_quit_with_error();
        app.finish_startup_network_restart(StartupNetworkPurpose::StagedHost)
            .expect("reconstruct host selector and show empty-log fallback");

        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert_eq!(
            app.scenario_selector_mode,
            ScenarioSelectorMode::NetworkHost
        );
        assert!(app.runtime_client_list.is_none());
        assert_startup_error_log(&app, "(no error)");
        let mut frame = vec![0x4c; 800 * 600 * 4];
        app.render(&mut frame)
            .expect("render restored host selector and empty-log fallback");
        assert!(frame.iter().any(|byte| *byte != 0x4c));
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss empty-log fallback");
        assert!(app.message_dialogs.is_empty());
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert_eq!(
            app.startup_restart_diagnostics,
            StartupRestartDiagnostics::default()
        );
    }

    #[test]
    fn l074_restart_restore_team_submits_full_player_packet_on_roster_construction() {
        let mut app = new_menu_app(640, 480);
        let (mut chooser, companion) = install_test_classic_host_team_lobby(&mut app);
        chooser.forced_name =
            LegacyCString::from_bytes(b"Restart Alias".to_vec()).expect("valid forced name");
        let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        let mut recorded = chooser.clone();
        recorded.team = 2;
        let mut recorded_companion = companion.clone();
        recorded_companion.team = 5;
        app.control_player_infos.replace_snapshot(
            8,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![recorded, recorded_companion],
                by_client: 0,
            }],
        );
        app.restart_restore_infos
            .capture_player_infos(&app.control_player_infos);
        app.control_player_infos.replace_snapshot(
            8,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![chooser.clone(), companion.clone()],
                by_client: 0,
            }],
        );
        app.engine
            .execute_script_control(
                &clonk_engine::ScriptControlData {
                    target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                    strictness: clonk_engine::ScriptStrictness::Strict3,
                    script: LegacyCString::from_bytes(b"SetRestoreInfos(RESTORE_PlayerTeams)".to_vec())
                        .expect("script has no NUL"),
                    by_client: 0,
                },
                ScriptControlPolicy::live(false),
            )
            .expect("SetRestoreInfos executes");
        assert_eq!(app.engine.restart_restore_info_mask(), 2);
        app.retain_restart_restore_mask_for_restart();

        app.sync_classic_lobby_roster();

        let mut restored = chooser.clone();
        restored.team = 2;
        let mut restored_companion = companion.clone();
        restored_companion.team = 5;
        assert_eq!(
            commands.take_player_info_updates(),
            vec![
                clonk_network::PlayerInfoUpdateRequest {
                    client_id: 0,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![restored.clone(), companion],
                },
                clonk_network::PlayerInfoUpdateRequest {
                    client_id: 0,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![restored, restored_companion],
                },
            ],
            "each synchronous PlayerListItem update carries earlier restored teammates forward"
        );
        assert!(
            app.network_team_assignment
                .as_ref()
                .unwrap()
                .teams()
                .teams
                .iter()
                .any(|team| team.id == 5),
            "GetGenerateTeamByID creates a missing restored team before submission"
        );
        assert_eq!(
            app.control_player_infos
                .client_update_request(0)
                .unwrap()
                .players[0]
                .team,
            1,
            "the roster waits for the authoritative PlayerInfo echo"
        );

        app.sync_classic_lobby_roster();
        assert!(
            commands.take_player_info_updates().is_empty(),
            "an existing PlayerListItem does not rerun its constructor hook"
        );
    }

    #[test]
    fn l074_host_round_restart_returns_to_network_lobby_staging() {
        let mut app = new_running_sandbox_app();
        configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
        app.engine
            .execute_script_control(
                &clonk_engine::ScriptControlData {
                    target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                    strictness: clonk_engine::ScriptStrictness::Strict3,
                    script: LegacyCString::from_bytes(b"SetRestoreInfos(RESTORE_PlayerTeams)".to_vec())
                        .expect("script has no NUL"),
                    by_client: 0,
                },
                ScriptControlPolicy::live(false),
            )
            .expect("SetRestoreInfos executes");

        app.restart_current_scenario()
            .expect("host restart selects network lobby staging");

        assert_eq!(
            app.scenario_selector_mode,
            ScenarioSelectorMode::NetworkHost,
            "a hosted round must rebuild its lobby instead of launching locally"
        );
        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(
            app.restart_restore_infos.what, RESTART_RESTORE_PLAYER_TEAMS,
            "the lobby handoff retains the raw SetRestoreInfos mask"
        );
        assert!(
            app.status_text.starts_with("Cannot host"),
            "the pathless sandbox fixture reaches host staging and fails there"
        );
    }

    #[test]
    fn l074_restart_restore_team_obeys_mask_user_and_equal_team_guards() {
        let submitted = |mask: i32, player_type: u8, live_team: i32, restore_team: i32| {
            let mut app = new_menu_app(640, 480);
            let (mut chooser, companion) = install_test_classic_host_team_lobby(&mut app);
            chooser.player_type = player_type;
            chooser.team = live_team;
            app.control_player_infos.replace_snapshot(
                8,
                [clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![chooser.clone(), companion],
                    by_client: 0,
                }],
            );
            let (network, _events, mut commands) =
                NetworkManager::test_stub_with_commands_for_client_id(0);
            app.network = Some(network);
            app.network_mode = Some(NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                player_name: "Host".to_string(),
                prepared: None,
            }));
            app.restart_restore_infos.what = mask;
            app.restart_restore_infos.players.insert(
                b"Chooser".to_vec(),
                RestartRestorePlayerInfo {
                    player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                    team: restore_team,
                    color: chooser.color,
                },
            );

            app.sync_classic_lobby_roster();
            commands.take_player_info_updates()
        };

        assert!(
            submitted(0, clonk_engine::PLAYER_INFO_TYPE_USER, 1, 2).is_empty(),
            "RESTORE_PlayerTeams must be selected"
        );
        assert!(
            submitted(
                RESTART_RESTORE_PLAYER_TEAMS,
                clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                1,
                2,
            )
            .is_empty(),
            "only current User rows run the restore hook"
        );
        assert!(
            submitted(
                RESTART_RESTORE_PLAYER_TEAMS,
                clonk_engine::PLAYER_INFO_TYPE_USER,
                2,
                2,
            )
            .is_empty(),
            "an already-restored team is a no-op"
        );
    }

    #[test]
    fn frontend_music_uses_catalog_once_per_startup_entry_and_toggle_restarts() {
        let _lock = env_lock().lock();
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create music group");
        // The decoder sniffs the payload; naming valid WAV bytes `.mid` lets
        // this test exercise catalog-extension handling without FluidSynth.
        fs::write(global.join("Frontend.mid"), silent_pcm_wav(1_000))
            .expect("write frontend MIDI fixture");

        let mut app = new_menu_app(320, 200);
        let audio = app.audio.as_mut().expect("test audio");
        audio.stop_music();
        audio.music_resolver =
            MusicResolver::with_global_group(Group::open(&global).expect("open global music group"))
                .expect("build music resolver");
        audio.options.menu_music_enabled = false;
        audio.set_scenario_music_level(Some(25));
        let stale_recent = Arc::clone(
            &audio
                .music_resolver
                .resolve("Frontend.mid")
                .expect("frontend fixture")
                .identity,
        );
        lock_unpoisoned(&audio.music_control).most_recently_played = Some(stale_recent);

        app.begin_frontend_music_entry();
        assert!(app.frontend_music_attempted_for_entry);
        let audio = app.audio.as_ref().expect("test audio");
        assert_eq!(audio.music_resolver.playlist.as_deref(), Some("Frontend.*"));
        assert_eq!(
            audio
                .music_resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"Frontend.mid".as_slice()),
            "all C++ music extensions resolve through the frontend playlist"
        );
        assert_eq!(lock_unpoisoned(&audio.music_control).scenario_level, None);
        assert!(lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .is_none());
        assert!(!audio.music_is_playing());

        app.set_frontend_music_option(true)
            .expect("enable frontend music");
        let audio = app.audio.as_ref().expect("test audio");
        let first_generation = lock_unpoisoned(&audio.music_control).generation;

        let wait_for_mixer_start = |app: &GameApp| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !app
                .audio
                .as_ref()
                .expect("test audio")
                .system
                .music_is_playing()
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            app.audio
                .as_ref()
                .expect("test audio")
                .system
                .music_is_playing()
        };
        assert!(wait_for_mixer_start(&app), "Frontend.mid starts playback");

        let mixer = Arc::clone(app.audio.as_ref().expect("test audio").system.mixer());
        let mut output = vec![0_i16; mixer.sample_rate() as usize * 2 * 2];
        mixer.mix_i16(&mut output);
        assert!(
            !app.audio
                .as_ref()
                .expect("test audio")
                .system
                .music_is_playing(),
            "draining past the asset end proves frontend music is non-looping"
        );

        app.update().expect("idle frontend update");
        app.ensure_menu_music();
        assert_eq!(
            lock_unpoisoned(&app.audio.as_ref().expect("test audio").music_control).generation,
            first_generation,
            "frontend navigation does not pump an ended track"
        );

        app.return_to_menu();
        assert!(app.frontend_music_attempted_for_entry);
        assert!(
            wait_for_mixer_start(&app),
            "a new startup entry restarts frontend music"
        );

        app.set_frontend_music_option(false)
            .expect("disable frontend music");
        assert!(
            !app.audio
                .as_ref()
                .expect("test audio")
                .options
                .menu_music_enabled
        );
        assert!(!app
            .audio
            .as_ref()
            .expect("test audio")
            .system
            .music_is_playing());
        app.set_frontend_music_option(true)
            .expect("re-enable frontend music");
        assert!(
            wait_for_mixer_start(&app),
            "FEMusic re-enable restarts the frontend playlist"
        );

        app.runtime_music_enabled = false;
        app.play_sandbox_audio();
        assert_eq!(
            app.audio
                .as_ref()
                .expect("test audio")
                .music_resolver
                .playlist,
            None,
            "game entry restores the default playlist"
        );
    }

    #[test]
    fn playlist_restart_selection_randomizes_new_matches_and_named_lookup_bypasses_filter() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        for name in ["A.ogg", "B.ogg", "C.ogg"] {
            fs::write(global.join(name), name.as_bytes()).expect("write music fixture");
        }
        let mut resolver =
            MusicResolver::with_global_group(Group::open(&global).expect("open global music group"))
                .expect("build music resolver");
        resolver.set_playlist(Some("B.*;C.*".to_string()));

        assert_eq!(
            resolver
                .resolve("A")
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"A.ogg".as_slice()),
            "an explicit Music(\"Name\") lookup ignores the default playlist"
        );
        let selected = resolver
            .select_default_with(None, |range| {
                assert_eq!(range, 2);
                1
            })
            .expect("filtered default selection");
        assert_eq!(
            selected.file_name_bytes, b"C.ogg",
            "a restarted playlist uses the random choice instead of its first match"
        );
    }

    #[test]
    fn set_music_playlist_command_restarts_only_when_enabled_at_its_event_position() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create music group");
        fs::write(global.join("Frontend.ogg"), silent_pcm_wav(20))
            .expect("write decodable music fixture");

        let group = Group::open(&global).expect("open global music");
        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.music_resolver = MusicResolver::with_global_group(group).expect("build music resolver");
        let fixture = audio
            .system
            .load_music(&silent_pcm_wav(20))
            .expect("predecode controlled music fixture");
        audio.control_music_loads_with(fixture);
        let snapshot = make_snapshot(Vec::new(), Vec::new());
        let event = AudioCommand::SetMusicPlaylist {
            playlist: Some("Frontend.*".to_string()),
            restart: true,
        };

        let initial_generation = lock_unpoisoned(&audio.music_control).generation;
        let mut runtime_music_enabled = false;
        audio.handle_events(
            std::slice::from_ref(&event),
            &snapshot,
            &[],
            &mut runtime_music_enabled,
        );
        assert_eq!(
            lock_unpoisoned(&audio.music_control).generation,
            initial_generation,
            "a restart request must not start music while Game.IsMusicEnabled is false"
        );
        assert_eq!(
            audio
                .music_resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"Frontend.ogg".as_slice()),
            "the filter still applies while playback is disabled"
        );

        runtime_music_enabled = true;
        audio.handle_events(
            std::slice::from_ref(&event),
            &snapshot,
            &[],
            &mut runtime_music_enabled,
        );
        assert_ne!(
            lock_unpoisoned(&audio.music_control).generation,
            initial_generation,
            "an enabled restart replaces the current music generation"
        );
        assert!(audio
            .complete_next_controlled_music_load()
            .expect("complete enabled playlist restart"));

        let before_play_restart_stop = lock_unpoisoned(&audio.music_control).generation;
        runtime_music_enabled = false;
        audio.handle_events(
            &[
                AudioCommand::PlayMusic {
                    name: "__missing__".to_string(),
                    looped: false,
                },
                event.clone(),
                AudioCommand::StopMusic,
            ],
            &snapshot,
            &[],
            &mut runtime_music_enabled,
        );
        assert!(!runtime_music_enabled);
        assert_eq!(
            lock_unpoisoned(&audio.music_control).generation,
            before_play_restart_stop + 2,
            "PlayMusic enables the intervening playlist restart before StopMusic disables it"
        );

        let before_stop_restart_play = lock_unpoisoned(&audio.music_control).generation;
        audio.handle_events(
            &[
                AudioCommand::StopMusic,
                event,
                AudioCommand::PlayMusic {
                    name: "__missing__".to_string(),
                    looped: false,
                },
            ],
            &snapshot,
            &[],
            &mut runtime_music_enabled,
        );
        assert!(runtime_music_enabled);
        assert_eq!(
            lock_unpoisoned(&audio.music_control).generation,
            before_stop_restart_play + 1,
            "StopMusic suppresses the intervening restart before the later PlayMusic"
        );
    }

    #[test]
    fn queued_playlist_restart_uses_its_command_time_filter() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        for name in ["A.ogg", "B.ogg", "C.ogg"] {
            fs::write(global.join(name), name.as_bytes()).expect("write music fixture");
        }

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.music_resolver =
            MusicResolver::with_global_group(Group::open(&global).expect("open global music group"))
                .expect("build music resolver");
        let b_identity = Arc::clone(
            &audio
                .music_resolver
                .resolve("B")
                .expect("resolve B")
                .identity,
        );
        let fixture = audio
            .system
            .load_music(&silent_pcm_wav(20))
            .expect("predecode controlled music fixture");
        audio.control_music_loads_with(fixture);

        let mut runtime_music_enabled = true;
        audio.handle_events(
            &[
                AudioCommand::PlayMusic {
                    name: "A".to_string(),
                    looped: false,
                },
                AudioCommand::SetMusicPlaylist {
                    playlist: Some("B.*".to_string()),
                    restart: true,
                },
                AudioCommand::SetMusicPlaylist {
                    playlist: Some("C.*".to_string()),
                    restart: false,
                },
            ],
            &make_snapshot(Vec::new(), Vec::new()),
            &[],
            &mut runtime_music_enabled,
        );
        assert_eq!(audio.queued_music_starts.len(), 1);
        assert_eq!(audio.music_resolver.playlist.as_deref(), Some("C.*"));

        assert!(audio
            .complete_next_controlled_music_load()
            .expect("complete named predecessor"));
        let controlled = audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading");
        assert_eq!(controlled.requests.len(), 1);
        assert!(controlled
            .requests
            .front()
            .and_then(|request| request.identity.as_ref())
            .is_some_and(|identity| Arc::ptr_eq(identity, &b_identity)));
    }

    #[test]
    fn running_global_gui_guard_precedes_scoreboard_and_root_overlay_pixels() {
        let check = |mut app: GameApp, label: &str| {
            app.scoreboard_initial_reconcile_pending = true;
            let before = runtime_global_ui_snapshot(&app);
            remove_global_gui_sheet(&mut app, "GUIBigArrows.png");
            let mut frame = vec![0x73; 320 * 200 * 4];
            let error = app
                .render(&mut frame)
                .expect_err("running overlay bypassed global GUI preflight");
            assert_global_gui_boundary(
                &error,
                vec![ClassicGuiBootstrapIssue::missing("GUIBigArrows")],
            );
            assert_eq!(runtime_global_ui_snapshot(&app), before, "{label}");
            assert!(frame.iter().all(|byte| *byte == 0x73), "{label}");
        };

        check(new_running_sandbox_app(), "base running view");

        let mut context = new_running_sandbox_app();
        context
            .open_context_menu_at(Vec::new(), GuiPoint::new(20.0, 20.0))
            .expect("open running context");
        check(context, "running context");

        let mut message = new_running_sandbox_app();
        message
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Message",
                    "Caption",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("open running message");
        check(message, "running message");

        let mut menu = new_running_sandbox_app();
        menu.ingame_menu
            .replace(menu.local_owner, Some(IngameMenuState::surrender_menu(&IngameMenuLabels::default())));
        check(menu, "running player menu");

        let mut evaluation = new_classic_running_sandbox_app();
        evaluation.handle_game_over().expect("open evaluation");
        check(evaluation, "running evaluation");
    }

    #[test]
    fn l002_abort_action_opens_confirmation_with_control_host_restart() {
        let mut app = new_menu_app(320, 200);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start explicit test sandbox");
        app.apply_ingame_menu_action(MenuAction::Abort)
            .expect("open C4AbortGameDialog confirmation");
        let dialog = app.message_dialogs.last().expect("abort dialog is visible");
        assert_eq!(
            dialog.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::YES_RESTART_NO
        );
        assert_eq!(
            dialog.state.size(),
            clonk_frontend::message_dialog::MessageDialogSize::Fixed(400)
        );
        assert_eq!(
            dialog.state.focused_button(),
            Some(clonk_frontend::message_dialog::MessageDialogButton::Yes)
        );
        assert!(matches!(app.mode, AppMode::Running));
    }

    #[test]
    fn l038_local_round_abort_and_evaluation_end_restore_fresh_browser() {
        let mut aborted = l038_running_browser_sandbox(ScenarioSelectorMode::Local);
        confirm_abort_dialog(&mut aborted);
        assert_l038_browser_return(&aborted, ScenarioSelectorMode::Local);

        let mut evaluated = l038_running_browser_sandbox(ScenarioSelectorMode::Local);
        evaluated
            .handle_game_over()
            .expect("open local L038 evaluation dialog");
        assert!(evaluated.game_over_dialog.is_some());
        evaluated
            .handle_game_over_action(GameOverAction::End)
            .expect("end evaluated local L038 round");
        assert_l038_browser_return(&evaluated, ScenarioSelectorMode::Local);
    }

    #[test]
    fn l027_reload_button_and_f5_restart_and_repopulate_search() {
        fn exercise(use_f5: bool, title: &str) {
            let listener = std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
                .expect("bind L027 masterserver fixture");
            listener
                .set_nonblocking(true)
                .expect("make L027 fixture bounded");
            let master_address = listener.local_addr().expect("fixture address");

            let discovery_port = std::net::UdpSocket::bind((std::net::Ipv6Addr::LOCALHOST, 0))
                .expect("reserve discovery port")
                .local_addr()
                .expect("reserved discovery address")
                .port();
            let mut app = new_classic_menu_app(800, 600);
            let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(
                app.assets
                    .clonk_fonts
                    .as_deref()
                    .expect("classic startup fonts"),
            );
            let layout = clonk_frontend::startup_netdlg::net_dlg_layout(800, 600, &metrics);
            let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
                clonk_frontend::startup_netdlg::NetDlgConfig {
                    masterserver_signup: true,
                    record: false,
                },
                metrics,
            );
            dialog.resize(800, 600);
            app.startup_view = StartupView::NetworkGame;
            app.startup_network_dialog = Some(dialog);
            app.startup_game_search = Some(
                clonk_network::StartupGameSearch::start(clonk_network::NetworkGameSearchConfig {
                    internet_enabled: true,
                    use_alternate_server: false,
                    master_server_url: format!("http://{master_address}/"),
                    discovery_port,
                })
                .expect("start L027 network search"),
            );
            app.startup_game_references = vec![clonk_network::NetworkGameReference {
                title: "Stale game".to_string(),
                ..Default::default()
            }];
            app.startup_direct_reference_queries = vec![StartupDirectReferenceQuery {
                id: 27,
                address: "stale.invalid".to_string(),
                state: StartupDirectReferenceQueryState::Pending,
                expires_at: None,
            }];
            app.sync_startup_network_game_rows();
            app.netdlg_last_click = Some((0, Instant::now()));
            app.startup_network_last_refresh = Some(Instant::now() - Duration::from_secs(2));
            assert_eq!(
                app.startup_network_dialog.as_ref().unwrap().games().len(),
                2
            );

            // Start the bounded server clock only after the expensive classic
            // app fixture is ready. Under a parallel full-suite run, starting
            // it before app construction can consume the whole accept timeout
            // before Reload/F5 is even able to issue the request.
            let server_title = title.to_string();
            let start_server = move || {
                thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(12);
                    let (mut stream, _) = loop {
                        match listener.accept() {
                            Ok(connection) => break connection,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                if Instant::now() >= deadline {
                                    return false;
                                }
                                thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) => panic!("accept L027 masterserver request: {error}"),
                        }
                    };
                    stream
                        .set_nonblocking(false)
                        .expect("make fixture connection blocking");
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .expect("bound fixture request read");
                    let mut request = [0_u8; 4096];
                    let size = stream
                        .read(&mut request)
                        .expect("read masterserver request");
                    assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET / HTTP/1.1"));
                    let body = format!(
                        "[Reference]\nTitle=\"{server_title}\"\nState=Lobby\nJoinAllowed=1\nAddress=TCP:\"127.0.0.1:31112\"\nGame=LegacyClonk\nVersion=4,9,11,0\nBuild=362\n"
                    );
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .expect("write fixture response headers");
                    stream
                        .write_all(body.as_bytes())
                        .expect("write fixture response body");
                    true
                })
            };

            if use_f5 {
                app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
                    .expect("F5 restarts network search");
                app.handle_key(VirtualKeyCode::F5, ElementState::Released)
                    .expect("release F5");
            } else {
                let reload = layout.buttons[1];
                app.handle_cursor_moved(PhysicalPosition::new(
                    f64::from(reload.x + reload.w / 2),
                    f64::from(reload.y + reload.h / 2),
                ))
                .expect("move over Reload");
                app.handle_mouse_button(ElementState::Pressed)
                    .expect("press Reload");
                app.handle_mouse_button(ElementState::Released)
                    .expect("release Reload");
            }
            // The listener is already bound, so a promptly scheduled client
            // can wait in the kernel backlog. Arm the fixture deadline only
            // after the refresh command has been delivered to the worker.
            let server = start_server();

            assert!(app.startup_game_references.is_empty());
            assert!(app.startup_direct_reference_queries.is_empty());
            assert!(app
                .startup_network_dialog
                .as_ref()
                .unwrap()
                .games()
                .is_empty());
            assert!(app.netdlg_last_click.is_none());
            assert!(
                app.status_text.is_empty(),
                "query presentation belongs to the native masterserver row"
            );
            assert!(!app.take_exit_request());

            let deadline = Instant::now() + Duration::from_secs(14);
            while !app
                .startup_game_references
                .iter()
                .any(|reference| reference.title == title)
                && Instant::now() < deadline
            {
                app.poll_startup_game_search()
                    .expect("apply restarted search event");
                // Hosts without a usable multicast route report the explicit
                // LAN probe failure in a modal. L070 freezes the queued
                // masterserver result until that native prompt is dismissed.
                if app
                    .message_dialogs
                    .last()
                    .is_some_and(|dialog| dialog.state.caption() == "Search Error")
                {
                    app.finish_message_dialog(
                        clonk_frontend::message_dialog::MessageDialogResult::Cancel,
                    )
                    .expect("dismiss incidental LAN discovery failure");
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert!(server.join().expect("L027 masterserver fixture thread"));
            assert_eq!(
                app.startup_game_references
                    .iter()
                    .map(|reference| reference.title.as_str())
                    .collect::<Vec<_>>(),
                [title]
            );
            assert_eq!(
                app.startup_network_dialog.as_ref().unwrap().games().len(),
                1
            );
            assert!(
                app.status_text.is_empty(),
                "result presentation belongs to the native query/game rows"
            );
            assert_eq!(app.startup_view, StartupView::NetworkGame);
            assert!(!app.take_exit_request());
        }

        exercise(false, "Reload button result");
        exercise(true, "F5 result");
    }

    #[test]
    fn l027_subsecond_refresh_only_plays_error_and_preserves_rows() {
        let sound_root = tempdir().expect("L027 sound fixture");
        let scenario = sound_root.path().join("Cooldown.c4s");
        fs::create_dir(&scenario).expect("create L027 sound fixture");
        fs::write(scenario.join("Error.wav"), silent_pcm_wav(1_000))
            .expect("write Error sound fixture");

        let mut app = new_classic_menu_app(800, 600);
        let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(
            app.assets
                .clonk_fonts
                .as_deref()
                .expect("classic startup fonts"),
        );
        let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
            clonk_frontend::startup_netdlg::NetDlgConfig::default(),
            metrics,
        );
        dialog.resize(800, 600);
        app.startup_view = StartupView::NetworkGame;
        app.startup_network_dialog = Some(dialog);
        app.startup_game_search = None;
        app.startup_game_references = vec![clonk_network::NetworkGameReference {
            title: "Retained game".to_string(),
            ..Default::default()
        }];
        app.startup_direct_reference_queries = vec![StartupDirectReferenceQuery {
            id: 28,
            address: "retained.invalid".to_string(),
            state: StartupDirectReferenceQueryState::Pending,
            expires_at: None,
        }];
        app.sync_startup_network_game_rows();
        app.status_text = "Retained status".to_string();
        let now = Instant::now();
        app.startup_network_last_refresh = Some(now);
        app.netdlg_last_click = Some((0, now));
        let expected_references = app.startup_game_references.clone();
        let expected_queries = app.startup_direct_reference_queries.clone();
        let expected_games = app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .games()
            .to_vec();
        let audio = app.audio.as_mut().expect("menu audio context");
        audio.options.menu_sound_enabled = true;
        audio.configure_scenario(Some(&scenario));
        audio.missing_sounds.clear();

        app.request_startup_network_refresh_at(now + Duration::from_millis(999))
            .expect("cooldown rejection is nonfatal");

        assert_eq!(app.startup_network_last_refresh, Some(now));
        assert_eq!(app.startup_game_references, expected_references);
        assert_eq!(app.startup_direct_reference_queries, expected_queries);
        assert_eq!(
            app.startup_network_dialog.as_ref().unwrap().games(),
            expected_games
        );
        assert_eq!(app.status_text, "Retained status");
        assert_eq!(app.netdlg_last_click, Some((0, now)));
        assert!(app.message_dialogs.is_empty());
        let audio = app.audio.as_ref().unwrap();
        assert!(
            audio
                .loaded_sounds
                .keys()
                .any(|key| key.to_ascii_lowercase().contains("error.wav")),
            "the rejected refresh must request only the Error GUI sound"
        );
        assert!(audio.missing_sounds.is_empty());
        assert!(!app.take_exit_request());
    }

    #[test]
    fn running_chat_raw_gamepad_owner_outranks_game_over_source_eligibility() {
        let mut app = new_game_over_keyboard_app();
        let mut config = Config::new();
        config.set_in(
            Some("Gamepad1"),
            "Button7",
            input::legacy_gamepad_axis_key(1, 0, false)
                .expect("gamepad-two left-axis key")
                .to_string(),
        );
        app.gamepad_bindings = GamepadBindings::from_config(&config);
        app.local_controls.remove(app.local_owner);
        app.local_controls.initialize(LocalControlInit {
            owner: app.local_owner,
            preferred_set: GamepadSlot::new(1).control_set(),
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        app.start_running_chat(RunningChatMode::All);
        app.engine
            .player_mut(app.local_owner)
            .expect("local sandbox player")
            .control
            .pressed_coms = 0;

        app.process_sourced_gamepad_event_batch(
            [
                SourcedGamepadEvent {
                    gamepad: 1,
                    cluster: 17,
                    event: GamepadEvent::Axis {
                        slot: GamepadSlot::new(1),
                        axis: LegacyGamepadAxis::new(0, false),
                        state: ElementState::Pressed,
                    },
                },
                SourcedGamepadEvent {
                    gamepad: 1,
                    cluster: 17,
                    event: GamepadEvent::Direction {
                        slot: GamepadSlot::new(1),
                        button: ControlButton::Left,
                        state: ElementState::Pressed,
                    },
                },
            ],
            false,
        )
        .expect("chat forwards raw input from a non-GUI gamepad above evaluation");

        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local sandbox player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0
        );
        assert!(app.game_over_dialog.is_some());
        assert!(app.running_chat.is_some());
        assert!(app.ingame_menu.is_none());
    }

    #[test]
    fn l006_named_remaps_drive_chat_scoreboard_abort_menu_and_player_candidates() {
        let config = parse_runtime_key_config(
                b"[Keys]\nChatOpen=G,Joy2A\nScoreboardToggle=H\nGameAbort=B\nFullscreenMenuDown=J\nKbd1Key1=Shift+T\nKbd1Key2=\\x0042000a\n",
            )
            .expect("parse modeled named remaps");
        let mut app = new_running_sandbox_app();
        app.runtime_key_config_cache = OnceLock::new();
        app.runtime_key_config_cache
            .set(Ok(config))
            .expect("install per-game key registry");

        assert!(app.handle_running_chat_open_key(VirtualKeyCode::G, ElementState::Pressed));
        assert!(app.running_chat_active());
        app.close_running_chat()
            .expect("close custom keyboard chat through the production lifecycle");
        assert!(!app.handle_running_chat_open_key(VirtualKeyCode::Return, ElementState::Pressed,));
        app.handle_gamepad_direction(
            GamepadSlot::new(1),
            ControlButton::Left,
            ElementState::Pressed,
        )
        .expect("custom Joy spelling reaches the named chat callback");
        assert!(app.running_chat_active());
        app.close_running_chat()
            .expect("close custom gamepad chat through the production lifecycle");

        assert!(app
            .handle_scoreboard_key(VirtualKeyCode::H, ElementState::Pressed)
            .expect("custom scoreboard callback"));
        assert!(!app
            .handle_scoreboard_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("replaced scoreboard default"));

        let shifted =
            app.runtime_control_candidates_for_keyboard(VirtualKeyCode::T, ElementState::Pressed);
        assert!(shifted.is_empty(), "the custom chord requires Shift");
        app.keyboard_modifiers = ModifiersState::SHIFT;
        assert_eq!(
            app.runtime_control_candidates_for_keyboard(VirtualKeyCode::T, ElementState::Pressed,),
            vec![KeyboardBindings::control_candidate_for_set(
                0,
                ControlBindingId::CursorLeft,
                ElementState::Pressed,
            )
            .expect("first keyboard callback")]
        );
        app.keyboard_modifiers = ModifiersState::empty();
        assert_eq!(
            app.runtime_control_candidates_for_gamepad_button(0, 0, ElementState::Pressed,),
            vec![KeyboardBindings::control_candidate_for_set(
                0,
                ControlBindingId::CursorToggle,
                ElementState::Pressed,
            )
            .expect("second keyboard callback")]
        );

        app.ingame_menu.replace(
            OWNER_NONE,
            IngameMenuState::main_menu(&MainMenuConditions {
                has_player: false,
                player_count: 2,
                ..MainMenuConditions::default()
            }, &IngameMenuLabels::default()),
        );
        let before = app
            .ingame_menu
            .get(OWNER_NONE)
            .expect("ownerless menu")
            .selection();
        assert!(app
            .handle_runtime_fullscreen_menu_key(VirtualKeyCode::J, ElementState::Pressed,)
            .expect("custom ownerless menu callback"));
        assert_ne!(
            app.ingame_menu
                .get(OWNER_NONE)
                .expect("ownerless menu remains")
                .selection(),
            before
        );
        app.ingame_menu.clear();

        app.handle_key(VirtualKeyCode::B, ElementState::Pressed)
            .expect("custom abort callback");
        assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::AbortGame { .. }
        )));

        let mut context_priority = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        context_priority.runtime_key_config_cache = OnceLock::new();
        context_priority
            .runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nScoreboardToggle=Escape,Return,R\n",
            )
            .expect("parse custom scoreboard chord")))
            .expect("install custom scoreboard chord");
        context_priority
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open higher-priority context menu");
        context_priority
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("context Escape precedes custom ScoreboardToggle");
        assert!(context_priority.context_menu.is_none());
        assert!(context_priority.scoreboard_dialog.is_none());
        context_priority
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("reopen context for Return priority");
        context_priority
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("context Return precedes custom ScoreboardToggle");
        assert!(context_priority.scoreboard_dialog.is_none());
        context_priority.close_context_menu_silently();
        context_priority
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new("Remain open").with_hotkey('R')],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("reopen context for hotkey priority");
        context_priority
            .handle_key(VirtualKeyCode::R, ElementState::Pressed)
            .expect("context hotkey precedes custom ScoreboardToggle");
        assert!(context_priority.scoreboard_dialog.is_none());

        let mut gamepad_priority = new_running_sandbox_app();
        gamepad_priority.runtime_key_config_cache = OnceLock::new();
        gamepad_priority
            .runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(b"[Keys]\nChatOpen=Joy1A\n")
                .expect("parse colliding gamepad chat chord")))
            .expect("install colliding gamepad chat chord");
        let mut gamepad_config = Config::new();
        gamepad_config.set_in(
            Some("Gamepad0"),
            "Button7",
            input::legacy_gamepad_axis_key(0, 0, false)
                .expect("primary left-axis key")
                .to_string(),
        );
        gamepad_priority.gamepad_bindings = GamepadBindings::from_config(&gamepad_config);
        gamepad_priority.local_controls = LocalControlRegistry::default();
        gamepad_priority
            .local_controls
            .initialize(LocalControlInit {
                owner: gamepad_priority.local_owner,
                preferred_set: 4,
                prefers_mouse: false,
                gamepads_enabled: true,
                replay: false,
                disable_mouse: false,
            });
        gamepad_priority
            .process_gamepad_event_batch([
                GamepadEvent::Axis {
                    slot: GamepadSlot::new(0),
                    axis: LegacyGamepadAxis::new(0, false),
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Left,
                    state: ElementState::Pressed,
                },
            ])
            .expect("assigned gamepad player callback precedes custom chat");
        assert!(!gamepad_priority.running_chat_active());
        assert_ne!(
            gamepad_priority
                .engine
                .player(gamepad_priority.local_owner)
                .expect("local gamepad player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
        );

        let mut chat_priority = new_running_sandbox_app();
        chat_priority.runtime_key_config_cache = OnceLock::new();
        chat_priority
            .runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nChatOpen2Allies=Up\n",
            )
            .expect("parse chat-history collision")))
            .expect("install chat-history collision");
        chat_priority.start_running_chat(RunningChatMode::All);
        chat_priority
            .handle_key(VirtualKeyCode::Up, ElementState::Pressed)
            .expect("chat history precedes the remapped global callback");
        assert_eq!(chat_priority.running_chat_text(), Some(""));

        let mut game_over_chat = new_game_over_keyboard_app();
        game_over_chat.runtime_key_config_cache = OnceLock::new();
        game_over_chat
            .runtime_key_config_cache
            .set(Ok(
                parse_runtime_key_config(b"[Keys]\nChatOpen=G\n").expect("parse game-over chat remap")
            ))
            .expect("install game-over chat remap");
        game_over_chat
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("game-over OnEnter opens chat independently of ChatOpen");
        assert!(game_over_chat.running_chat_active());
        assert_eq!(game_over_chat.running_chat_text(), Some(""));
    }

    #[test]
    fn observer_and_game_over_use_the_ownerless_fullscreen_camera() {
        let mut observer = new_running_sandbox_app();
        observer.snapshot.hud.local_players.clear();
        let observer_inputs = collect_viewport_inputs(&observer.snapshot)
            .expect("ownerless fullscreen viewport is available");
        assert_eq!(observer_inputs.len(), 1);
        assert_eq!(observer_inputs[0].owner, OWNER_NONE);

        let mut game_over_observer = new_classic_running_sandbox_app();
        game_over_observer
            .assets
            .require_classic_game_over_resources()
            .expect("repository game-over resources are complete");
        game_over_observer
            .handle_game_over()
            .expect("show observer game-over dialog");
        game_over_observer.status_text.clear();
        game_over_observer.snapshot.hud.local_players.clear();
        assert!(game_over_observer.game_over_dialog.is_some());
        let game_over_inputs = collect_viewport_inputs(&game_over_observer.snapshot)
            .expect("game-over observer keeps the ownerless viewport");
        assert_eq!(game_over_inputs.len(), 1);
        assert_eq!(game_over_inputs[0].owner, OWNER_NONE);
    }

    #[test]
    fn client_league_round_result_packet_applies_persistent_evaluation_fields() {
        let mut app = new_state_only_running_sandbox_app();
        let (manager, event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.control_player_infos.replace_snapshot(
            10,
            [clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 10,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let packet = clonk_network::LeagueRoundResultsPacket {
            success: true,
            result_string: clonk_engine::LegacyCString::from_bytes(b"evaluated".to_vec()).unwrap(),
            players: vec![clonk_network::LeagueRoundResultsPlayer {
                player_info_id: 10,
                total_playing_time: 90,
                settlement_score_old: 11,
                settlement_score_new: 12,
                league_score_new: 80,
                league_score_gain: 5,
                league_rank_new: 3,
                league_rank_symbol_new: 4,
                league_progress_data: clonk_engine::LegacyCString::from_bytes(b"progress".to_vec())
                    .unwrap(),
                status: clonk_network::LeagueRoundPlayerStatus::Won,
            }],
        };
        event_tx
            .send(NetworkEvent::LeagueRoundResults(packet))
            .expect("queue league result packet");

        app.process_network_events()
            .expect("apply league result packet");

        let info = app.control_player_infos.get(10).unwrap();
        assert_eq!(
            (info.league_score, info.league_rank, info.league_rank_symbol),
            (0, 0, 0),
            "EvaluateLeague does not overwrite live PlayerInfo"
        );
        let engine_snapshot = app.engine.snapshot();
        assert_eq!(app.snapshot.round_results, engine_snapshot.round_results);
        assert_eq!(
            engine_snapshot.round_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::LeagueOk)
        );
        assert_eq!(
            engine_snapshot
                .round_results
                .network_result_message
                .as_slice(),
            &b"evaluated"[..]
        );
        let result = engine_snapshot
            .round_results
            .players
            .iter()
            .find(|result| result.player_info_id == 10)
            .unwrap();
        assert_eq!((result.score_old, result.score_new), (-1, None));
        assert_eq!(
            (
                result.league_score_new,
                result.league_score_gain,
                result.league_rank_new,
                result.league_rank_symbol_new,
            ),
            (80, 5, 3, 4)
        );
        assert_eq!(
            result.league_progress_data.as_deref(),
            Some(&b"progress"[..])
        );
    }

    #[test]
    fn synchronized_activation_restarts_playerless_activity_window() {
        // C4Client::SetActivated(true) stamps the current FrameCounter. The
        // strict delay begins at synchronized execution, not at connection or
        // request time (src/C4Client.cpp:104-110; src/C4Control.cpp:589-602).
        let mut app = new_running_sandbox_app();
        for _ in 0..400 {
            app.engine.tick().expect("advance before activation");
        }
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        app.network_control_running = false;
        app.control_clients.register(3, false, false);
        app.apply_ready_controls(
            0,
            vec![NetworkControl::ClientUpdate(
                clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    client_id: 3,
                    data: 1,
                    by_client: 0,
                },
            )],
        )
        .expect("execute synchronized activation at frame 400");

        for _ in 0..500 {
            app.engine
                .tick()
                .expect("advance to activation delay boundary");
        }
        app.update().expect("scan activation age 500");
        assert!(commands.take_submitted_client_updates().is_empty());

        app.engine.tick().expect("advance past activation delay");
        app.update().expect("scan activation age 501");
        assert_eq!(
            commands.take_submitted_client_updates(),
            vec![clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 3,
                data: 0,
                by_client: 0,
            }]
        );
    }

    #[test]
    fn game_over_goal_hover_uses_localized_cpp_tooltips_and_shared_delay() {
        let mut app = new_classic_running_sandbox_app();
        for (id, name, description) in [
            ("GFDN", "Build the %s bridge", "Reach the other side"),
            ("GOPN", "Find the gold", "Recover the treasure"),
        ] {
            let mut definition =
                Definition::from_script(id, name, "#strict 3\n").expect("goal definition compiles");
            definition.set_description(Some(description.to_string()));
            app.engine
                .register_definition(definition)
                .expect("goal definition registers");
        }
        app.snapshot.round_results.goals = vec!["GFDN".into(), "GOPN".into()];
        app.snapshot.round_results.fulfilled_goals = vec!["GFDN".into()];
        app.handle_game_over().expect("show goal evaluation");

        let goal_rects = {
            let surface = app.graphics.surface();
            let dialog = app.game_over_dialog.as_ref().expect("evaluation dialog");
            assert_eq!(
                dialog
                    .evaluation()
                    .goals()
                    .iter()
                    .map(|goal| goal.tooltip.as_str())
                    .collect::<Vec<_>>(),
                vec![
                    "Goal Build the %s bridge fulfilled: Reach the other side",
                    "Goal Find the gold not fulfilled: Recover the treasure",
                ]
            );
            let layout = dialog.classic_evaluation_layout(
                surface.width(),
                surface.height(),
                app.assets.clonk_fonts.as_deref().expect("classic fonts"),
            );
            layout
                .goals
                .into_iter()
                .map(|goal| goal.picture)
                .collect::<Vec<_>>()
        };

        for (rect, expected) in goal_rects.iter().zip([
            "Goal Build the %s bridge fulfilled: Reach the other side",
            "Goal Find the gold not fulfilled: Recover the treasure",
        ]) {
            app.handle_cursor_moved(PhysicalPosition::new(
                f64::from(rect.x + rect.w / 2),
                f64::from(rect.y + rect.h / 2),
            ))
            .expect("hover goal picture");
            assert_eq!(
                app.game_over_dialog
                    .as_ref()
                    .expect("evaluation dialog")
                    .hovered_description(),
                expected
            );
        }

        let first = goal_rects[0];
        let first_center = GuiPoint::new(
            (first.x + first.w / 2) as f32,
            (first.y + first.h / 2) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(first_center.x),
            f64::from(first_center.y),
        ))
        .expect("restore first goal hover");
        let started = Instant::now() - clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY;
        app.startup_tooltip = ClassicTooltipTracker::new_at(started);
        app.startup_tooltip
            .note_pointer_move_at(first_center, started);
        assert_eq!(app.startup_tooltip.eligible_pointer(), Some(first_center));
        assert!(app
            .render_game_over_tooltip(Some(startup_gamma()))
            .expect("draw classic delayed tooltip"));

        // A newer dialog can consume motion and then close before the shared
        // delay expires. Re-resolve at the tracker's current pointer instead
        // of drawing this dialog's stale cached goal hover there.
        let consumed_pointer = GuiPoint::new(0.0, 0.0);
        let started = Instant::now() - clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY;
        app.startup_tooltip = ClassicTooltipTracker::new_at(started);
        app.startup_tooltip
            .note_pointer_move_at(consumed_pointer, started);
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .expect("evaluation dialog")
                .hovered_description(),
            "Goal Build the %s bridge fulfilled: Reach the other side",
            "the lower dialog intentionally retains its last routed hover"
        );
        assert!(!app
            .render_game_over_tooltip(Some(startup_gamma()))
            .expect("ignore stale lower-dialog hover"));
    }

    #[test]
    fn game_over_custom_text_wheel_uses_app_routing_and_stays_below_newer_dialogs() {
        let mut app = new_classic_running_sandbox_app();
        app.snapshot.round_results.custom_evaluation_strings = (0..40)
            .map(|index| format!("Line {index}"))
            .collect::<Vec<_>>()
            .join("|");
        app.handle_game_over().expect("show scrollable evaluation");
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let custom = app
            .game_over_dialog
            .as_ref()
            .expect("evaluation dialog")
            .classic_evaluation_layout(
                width,
                height,
                app.assets.clonk_fonts.as_deref().expect("classic fonts"),
            )
            .custom_evaluation
            .expect("custom text layout");
        assert!(custom.scrollable);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(custom.viewport.x + 1),
            f64::from(custom.viewport.y + 1),
        ))
        .expect("hover custom evaluation viewport");

        let render_version = app.menu_render_version;
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("route wheel into evaluation");
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .expect("evaluation dialog")
                .custom_evaluation_scroll(),
            60
        );
        assert_ne!(app.menu_render_version, render_version);

        configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
        app.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("open client list above evaluation");
        assert!(app.runtime_client_list_owns_game_over());
        app.running_pointer_position = Some(GuiPoint::new(0.0, 0.0));
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("newer client list keeps underlying evaluation inactive");
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .expect("evaluation dialog")
                .custom_evaluation_scroll(),
            60
        );
    }

    fn assert_game_over_resource_boundary(error: &anyhow::Error, expected_missing: Vec<&'static str>) {
        let expected = ClassicParityBoundary::GameOverResources {
            missing: expected_missing.into_iter().map(str::to_string).collect(),
        };
        assert_eq!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(&expected)
        );
        assert!(
            error.to_string().contains("refusing generic Rust fallback"),
            "boundary must explain why the fallback is unreachable: {error:#}"
        );
    }

    fn assert_startup_game_over_boundary(error: &anyhow::Error, view: StartupView) {
        let expected = ClassicParityBoundary::StartupGameOver { view };
        assert_eq!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(&expected)
        );
        assert!(
            error.to_string().contains("running-mode only"),
            "boundary must identify the invalid lifecycle state: {error:#}"
        );
    }

    #[test]
    fn game_over_missing_resources_fail_typed_before_touching_output_frame() {
        let mut app = new_game_over_keyboard_app();
        let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
        assets
            .startup_dialog_images
            .remove("Player.png")
            .expect("fixture player icon");
        let hud = Arc::make_mut(&mut assets.hud_graphics);
        hud.player = None;
        hud.score = None;
        app.scoreboard_initial_reconcile_pending = true;
        let before = runtime_global_ui_snapshot(&app);
        let mut frame = vec![0x5a; 320 * 200 * 4];
        let sentinel = frame.clone();

        let error = app
            .render(&mut frame)
            .expect_err("asset-less game over must not render a fallback");

        assert_game_over_resource_boundary(&error, vec!["Player.png", "Score.png"]);
        assert_eq!(frame, sentinel, "preflight must precede every output write");
        assert_eq!(runtime_global_ui_snapshot(&app), before);
    }

    #[test]
    fn game_over_recursive_inventory_covers_global_sheets_crew_and_frozen_images() {
        let mut app = new_game_over_keyboard_app();
        let (gui_icons2, gui_scroll) = {
            let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
            let gui_icons2 = assets
                .startup_dialog_images
                .remove("GUIIcons2.png")
                .expect("fixture extended GUI icon sheet");
            let gui_scroll = assets
                .startup_dialog_images
                .remove("GUIScroll.png")
                .expect("fixture GUI scroll sheet");
            Arc::make_mut(&mut assets.hud_graphics).crew = None;
            (gui_icons2, gui_scroll)
        };
        let error = app
            .assets
            .require_classic_game_over_resources()
            .expect_err("recursive direct inventory rejects missing child resources");
        assert_eq!(
            error,
            ClassicParityBoundary::GameOverResources {
                missing: vec![
                    "GUIIcons2.png".to_string(),
                    "GUIScroll.png".to_string(),
                    "Crew.png".to_string(),
                ],
            }
        );

        // In the live app, C4GUI::Resource owns the two GUI sheets and its
        // process-global boundary deliberately wins before the recursive
        // game-over check.
        let mut frame = vec![0x7a; 320 * 200 * 4];
        let sentinel = frame.clone();
        let error = app
            .render(&mut frame)
            .expect_err("global GUI gate must retain boundary precedence");
        let boundary = error
            .downcast_ref::<ClassicParityBoundary>()
            .expect("typed classic boundary");
        assert!(matches!(
            boundary,
            ClassicParityBoundary::GlobalGuiBootstrapResources { .. }
        ));
        assert_eq!(frame, sentinel);

        // Once the process-global sheets are restored, the recursive child
        // inventory owns Crew and must still fail before any output pixels.
        {
            let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
            assets
                .startup_dialog_images
                .insert("GUIIcons2.png".to_string(), gui_icons2);
            assets
                .startup_dialog_images
                .insert("GUIScroll.png".to_string(), gui_scroll);
        }
        let mut frame = vec![0x29; 320 * 200 * 4];
        let sentinel = frame.clone();
        let error = app
            .render(&mut frame)
            .expect_err("missing recursive Crew resource must fail before drawing");
        assert_game_over_resource_boundary(&error, vec!["Crew.png"]);
        assert_eq!(frame, sentinel);

        let mut app = new_game_over_keyboard_app();
        let invalid = ImageData::new(1, 1, Vec::new());
        app.game_over_dialog
            .as_mut()
            .expect("evaluation dialog")
            .set_evaluation(EvaluationViewModel::new(
                vec![EvaluationGoal {
                    definition_id: "MISS".into(),
                    fulfilled: false,
                    tooltip: "Goal Missing not fulfilled: Missing image".into(),
                    picture: Some(invalid.clone()),
                }],
                vec![EvaluationPlayer {
                    player_info_id: 41,
                    team_id: None,
                    name: "Player".into(),
                    won: false,
                    color_dw: 0,
                    total_playing_time: 0,
                    score_old: -1,
                    score_new: None,
                    custom_evaluation_strings: String::new(),
                    big_icon: Some(invalid),
                    league_score_old: None,
                    league_score_gain: None,
                    league_score_new: None,
                    joined_color_dw: None,
                }],
            ));
        let mut frame = vec![0x3d; 320 * 200 * 4];
        let sentinel = frame.clone();
        let error = app
            .render(&mut frame)
            .expect_err("malformed frozen images fail before a partial render");
        assert_game_over_resource_boundary(
            &error,
            vec!["goal definition picture `MISS`", "player 41 BigIcon"],
        );
        assert_eq!(frame, sentinel);
    }

    #[test]
    fn game_over_freezes_cached_player_big_icon_when_portraits_are_hidden() {
        let mut app = new_classic_running_sandbox_app();
        app.display_flags.portraits = false;
        let player_info_id = app
            .snapshot
            .players
            .first()
            .expect("sandbox player")
            .player_info_id;
        app.snapshot.round_results.players = vec![clonk_engine::RoundResultsPlayerState {
            player_info_id,
            ..clonk_engine::RoundResultsPlayerState::default()
        }];
        let icon = ImageData::new(1, 1, vec![12, 34, 56, 255]);
        let file_name = "Player.c4p".to_string();
        app.control_player_infos.replace_snapshot(
            player_info_id,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: player_info_id,
                    name: LegacyCString::from_bytes(b"Player".to_vec()).expect("fixture player name"),
                    filename: LegacyCString::from_bytes(file_name.as_bytes().to_vec())
                        .expect("fixture player filename"),
                    ..Default::default()
                }],
                by_client: 0,
            }],
        );
        app.startup_player_files.insert(
            0,
            StartupPlayerFile {
                path: PathBuf::from(&file_name),
                file_name,
                player_file: PlayerFile::default(),
                render_model: clonk_frontend::startup_plrsel::PlrSelPlayer {
                    name: "Player".into(),
                    activated: true,
                    big_icon: Some(icon.clone()),
                    portrait: None,
                    color_dw: 0,
                    score: 0,
                    rounds: 0,
                    rounds_won: 0,
                    rounds_lost: 0,
                    total_playing_time: 0,
                    comment: String::new(),
                },
            },
        );
        app.runtime_player_big_icons.clear();
        app.runtime_player_big_icon_misses.clear();

        app.handle_game_over().expect("show evaluation dialog");
        assert_eq!(
            app.runtime_player_big_icons.get(&player_info_id),
            Some(&icon),
            "evaluation hydration must ignore the viewport portrait switch"
        );
        app.runtime_player_big_icons.remove(&player_info_id);
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .expect("evaluation dialog")
                .evaluation()
                .player_by_info_id(player_info_id)
                .and_then(|player| player.big_icon.as_ref()),
            Some(&icon),
            "the frozen evaluation dialog owns its copied BigIcon"
        );
    }

    // C4RoundResults::EvaluatePlayer copies C4Player::BigIcon when the player
    // is evaluated, which for an eliminated or disconnected player happens
    // inside the simulation, long before the evaluation dialog is built
    // (src/C4RoundResults.cpp:52-73,338-344; src/C4PlayerList.cpp:241).
    // Freezing only at dialog construction loses the icon once the player and
    // its file/resource are gone.
    #[test]
    fn game_over_uses_elimination_time_big_icon_after_player_resource_departure() {
        let mut app = new_classic_running_sandbox_app();
        let player_info_id = app
            .snapshot
            .players
            .first()
            .expect("sandbox player")
            .player_info_id;
        let icon = ImageData::new(1, 1, vec![9, 8, 7, 255]);
        let file_name = "Departed.c4p".to_string();
        app.control_player_infos.replace_snapshot(
            player_info_id,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: player_info_id,
                    name: LegacyCString::from_bytes(b"Departed".to_vec())
                        .expect("fixture player name"),
                    filename: LegacyCString::from_bytes(file_name.as_bytes().to_vec())
                        .expect("fixture player filename"),
                    ..Default::default()
                }],
                by_client: 0,
            }],
        );
        app.startup_player_files.insert(
            0,
            StartupPlayerFile {
                path: PathBuf::from(&file_name),
                file_name,
                player_file: PlayerFile::default(),
                render_model: clonk_frontend::startup_plrsel::PlrSelPlayer {
                    name: "Departed".into(),
                    activated: true,
                    big_icon: Some(icon.clone()),
                    portrait: None,
                    color_dw: 0,
                    score: 0,
                    rounds: 0,
                    rounds_won: 0,
                    rounds_lost: 0,
                    total_playing_time: 0,
                    comment: String::new(),
                },
            },
        );
        app.runtime_player_big_icons.clear();
        app.runtime_player_big_icon_misses.clear();

        // The player is evaluated and retired inside the simulation.
        app.engine
            .round_results
            .players
            .push(clonk_engine::RoundResultsPlayerState {
                player_info_id,
                ..clonk_engine::RoundResultsPlayerState::default()
            });
        app.freeze_evaluated_player_big_icons();
        assert_eq!(
            app.runtime_player_big_icons.get(&player_info_id),
            Some(&icon),
            "evaluation copies the icon while the player still exists"
        );

        // Its player file and resource then depart, so nothing can supply the
        // icon any more.
        app.startup_player_files.clear();
        app.control_player_infos.replace_snapshot(player_info_id + 1, []);
        app.snapshot.round_results.players = vec![clonk_engine::RoundResultsPlayerState {
            player_info_id,
            ..clonk_engine::RoundResultsPlayerState::default()
        }];

        app.handle_game_over().expect("show evaluation dialog");
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .expect("evaluation dialog")
                .evaluation()
                .player_by_info_id(player_info_id)
                .and_then(|player| player.big_icon.as_ref()),
            Some(&icon),
            "the dialog consumes the elimination-time snapshot"
        );
    }

    #[test]
    fn every_game_over_icon_source_obeys_global_then_overlay_preflight() {
        let mut app = new_game_over_keyboard_app();
        app.assets
            .require_classic_game_over_resources()
            .expect("repository game-over fixture");

        let gui_icons = Arc::get_mut(&mut app.assets)
            .expect("frontend assets are app-owned")
            .startup_dialog_images
            .remove("GUIIcons.png")
            .expect("fixture global GUI icon sheet");
        let mut frame = vec![0xa5; 320 * 200 * 4];
        let sentinel = frame.clone();
        let error = app
            .render(&mut frame)
            .expect_err("shared GUIIcons must fail at the process-global preflight");
        assert_global_gui_boundary(&error, vec![ClassicGuiBootstrapIssue::missing("GUIIcons")]);
        assert_eq!(frame, sentinel);
        Arc::get_mut(&mut app.assets)
            .expect("frontend assets are app-owned")
            .startup_dialog_images
            .insert("GUIIcons.png".to_string(), gui_icons);

        {
            let name = "Player.png";
            let (image, hud_player) = {
                let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
                let image = assets
                    .startup_dialog_images
                    .remove(name)
                    .expect("fixture image");
                let hud_player = Arc::make_mut(&mut assets.hud_graphics)
                    .player
                    .take()
                    .expect("fixture HUD player image");
                (image, hud_player)
            };
            let mut frame = vec![0xa5; 320 * 200 * 4];
            let sentinel = frame.clone();

            let error = app
                .render(&mut frame)
                .expect_err("missing game-over icon source must fail typed");
            assert_game_over_resource_boundary(&error, vec![name]);
            assert_eq!(frame, sentinel, "{name} guard must run before pixels");

            let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
            assets.startup_dialog_images.insert(name.to_string(), image);
            Arc::make_mut(&mut assets.hud_graphics).player = Some(hud_player);
        }

        let score = {
            let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
            Arc::make_mut(&mut assets.hud_graphics)
                .score
                .take()
                .expect("fixture score image")
        };
        let mut frame = vec![0x3c; 320 * 200 * 4];
        let sentinel = frame.clone();
        let error = app
            .render(&mut frame)
            .expect_err("missing score source must fail typed");
        assert_game_over_resource_boundary(&error, vec!["Score.png"]);
        assert_eq!(frame, sentinel, "Score.png guard must run before pixels");
        let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
        Arc::make_mut(&mut assets.hud_graphics).score = Some(score);
    }

    #[test]
    fn game_over_with_complete_classic_resources_renders_without_fallback() {
        let mut app = new_game_over_keyboard_app();
        app.assets
            .require_classic_game_over_resources()
            .expect("repository game-over fixture");
        let mut frame = vec![0x5a; 320 * 200 * 4];
        let sentinel = frame.clone();

        app.render(&mut frame)
            .expect("complete classic game-over resources render");

        assert_ne!(
            frame, sentinel,
            "classic renderer must compose an output frame"
        );
    }

    #[test]
    fn stale_menu_game_over_fails_typed_on_all_startup_roots_before_lower_boundaries() {
        let mut app = new_real_classic_menu_app(640, 480);

        // Retain three recursive child states so their own typed boundaries
        // are also known to be lower priority than the invalid lifecycle.
        for child in [
            RetainedStartupChild::Unported(ClassicStartupSubscreen::Options(
                clonk_frontend::startup_options_dlg::OptionsSheet::Graphics,
            )),
            RetainedStartupChild::AboutLicenses,
        ] {
            enter_retained_startup_child(&mut app, child);
            app.show_main_menu();
        }
        app.open_network_game_dialog();
        activate_startup_network_chat(&mut app);
        app.show_main_menu();
        app.open_player_selection_dialog();
        app.show_main_menu();
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .expect("retained Options model")
                .active_sheet(),
            clonk_frontend::startup_options_dlg::OptionsSheet::Graphics
        );
        assert_eq!(
            app.startup_about_dialog
                .as_ref()
                .expect("retained About model")
                .current_page(),
            clonk_frontend::startup_about_dlg::AboutPage::Licenses
        );
        assert_eq!(
            app.startup_network_dialog
                .as_ref()
                .expect("retained Network model")
                .mode(),
            clonk_frontend::startup_netdlg::NetDlgMode::Chat
        );
        assert!(app.startup_player_dialog.is_some());
        app.handle_game_over()
            .expect("forge stale menu evaluation state");
        app.assets
            .require_classic_game_over_resources()
            .expect("fixture has the complete running evaluation bundle");

        for (index, view) in StartupView::ALL.into_iter().enumerate() {
            // Exhaustive arms force future startup roots into this lifecycle
            // invariant instead of silently omitting the evaluation dialog.
            match view {
                StartupView::MainMenu => app.startup_view = StartupView::MainMenu,
                StartupView::ScenarioBrowser => {
                    app.startup_view = StartupView::ScenarioBrowser;
                }
                StartupView::NetworkLobby => {
                    app.startup_view = StartupView::NetworkLobby;
                    app.classic_host_lobby = None;
                }
                StartupView::NetworkGame => app.startup_view = StartupView::NetworkGame,
                StartupView::Options => app.startup_view = StartupView::Options,
                StartupView::About => app.startup_view = StartupView::About,
                StartupView::PlayerSelection => {
                    app.startup_view = StartupView::PlayerSelection;
                }
            }
            app.status_text = format!("lower-priority status for {view:?}");
            let cached = vec![0x20 + index as u8; 640 * 480 * 4];
            app.menu_frame_cache = Some(MenuFrameCache {
                view,
                version: app.menu_render_version,
                width: 640,
                height: 480,
                native_text_deferred: false,
                frame: cached.clone(),
            });

            let mut frame = vec![0xc3; 640 * 480 * 4];
            let error = app
                .render(&mut frame)
                .expect_err("stale evaluation must precede startup cache or pixels");
            assert_startup_game_over_boundary(&error, view);
            assert!(frame.iter().all(|byte| *byte == 0xc3));
            assert_eq!(
                app.menu_frame_cache.as_ref().expect("cache retained").frame,
                cached
            );

            let mut native = vec![0x6d; 1280 * 960 * 4];
            let error = app
                .render_native_main_menu_text(&mut native, 1280, 960)
                .expect_err("native pass must enforce the same lifecycle boundary");
            assert_startup_game_over_boundary(&error, view);
            assert!(native.iter().all(|byte| *byte == 0x6d));
        }
    }

    #[test]
    fn stale_menu_game_over_lifecycle_boundary_precedes_missing_resources() {
        let mut app = new_real_classic_menu_app(320, 200);
        let mut cached = vec![0_u8; 320 * 200 * 4];
        app.render(&mut cached)
            .expect("populate startup frame cache");
        assert!(app.menu_frame_cache.is_some());
        app.handle_game_over().expect("show stale game-over dialog");
        app.status_text.clear();
        app.assets = Arc::new(FrontendAssets::load(None));
        let mut frame = vec![0xc3; 320 * 200 * 4];
        let sentinel = frame.clone();

        let error = app
            .render(&mut frame)
            .expect_err("invalid lifecycle must win without resource lookup");
        assert_startup_game_over_boundary(&error, StartupView::MainMenu);
        assert_eq!(
            frame, sentinel,
            "startup preflight must precede every pixel"
        );
        assert_eq!(
            app.menu_frame_cache.as_ref().expect("cache retained").frame,
            cached
        );

        let mut native = vec![0x47; 640 * 400 * 4];
        let error = app
            .render_native_main_menu_text(&mut native, 640, 400)
            .expect_err("native pass must reject stale evaluation before resources");
        assert_startup_game_over_boundary(&error, StartupView::MainMenu);
        assert!(native.iter().all(|byte| *byte == 0x47));
    }

    fn current_scoreboard_test_layout(
        app: &mut GameApp,
    ) -> clonk_frontend::scoreboard::ScoreboardLayout {
        app.materialize_scoreboard_presentation()
            .expect("scoreboard presentation resources");
        app.scoreboard_runtime
            .presentation
            .as_ref()
            .expect("retained scoreboard presentation")
            .layout()
            .clone()
    }

    fn install_visible_scoreboard_highlight_fixture(app: &mut GameApp) {
        let highlight_pixel = [20_u8, 10, 5, 255];
        let highlight = ImageData::new(
            2,
            2,
            highlight_pixel
                .into_iter()
                .cycle()
                .take(2 * 2 * 4)
                .collect(),
        );
        Arc::get_mut(&mut app.assets)
            .expect("scoreboard fixture owns its frontend asset bundle")
            .game_over_button_highlight = Some(highlight);
    }

    fn frames_differ_in_rect(
        before: &[u8],
        after: &[u8],
        width: u32,
        rect: clonk_frontend::classic_gui::IntRect,
    ) -> bool {
        let height = (before.len() / 4 / width as usize) as u32;
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = ((rect.x + rect.w).max(0) as u32).min(width);
        let y1 = ((rect.y + rect.h).max(0) as u32).min(height);
        (y0..y1).any(|y| {
            (x0..x1).any(|x| {
                let offset = ((y * width + x) * 4) as usize;
                before.get(offset..offset + 4) != after.get(offset..offset + 4)
            })
        })
    }

    #[test]
    fn scoreboard_tab_uses_exact_matrix_and_refcount_eligibility() {
        let mut empty = new_scoreboard_test_app("");
        let before_empty = runtime_global_ui_snapshot(&empty);
        empty
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("empty scoreboard is consumed without opening");
        empty
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release empty scoreboard key");
        assert_eq!(runtime_global_ui_snapshot(&empty), before_empty);

        let mut dimensionless_positive =
            new_scoreboard_test_app("global func Initialize() { DoScoreboardShow(1); }");
        assert_eq!(
            dimensionless_positive.snapshot.hud.scoreboard.show_count(),
            1
        );
        assert_eq!(
            dimensionless_positive.snapshot.hud.scoreboard.row_count(),
            0
        );
        let before_dimensionless = runtime_global_ui_snapshot(&dimensionless_positive);
        dimensionless_positive
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("a positive empty board still cannot open");
        dimensionless_positive
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release dimensionless scoreboard key");
        assert_eq!(
            runtime_global_ui_snapshot(&dimensionless_positive),
            before_dimensionless
        );

        let mut negative = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       DoScoreboardShow(-1);
                   }"#,
        );
        assert_eq!(
            (
                negative.snapshot.hud.scoreboard.row_count(),
                negative.snapshot.hud.scoreboard.column_count()
            ),
            (1, 1)
        );
        assert_eq!(negative.snapshot.hud.scoreboard.show_count(), -1);
        let before_negative = runtime_global_ui_snapshot(&negative);
        negative
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("negative refcount disables user opening");
        negative
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release disabled scoreboard key");
        assert_eq!(runtime_global_ui_snapshot(&negative), before_negative);

        let mut eligible = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "PRIVATE_CELL_TEXT");
                   }"#,
        );
        eligible.graphics.set_scroll_smooth(1);
        let mut hidden = vec![0_u8; 320 * 200 * 4];
        eligible
            .render(&mut hidden)
            .expect("render the same live matrix without its dialog");
        toggle_scoreboard(&mut eligible, ModifiersState::empty());
        assert_eq!(
            eligible.scoreboard_dialog,
            Some(eligible.scoreboard_request())
        );
        let mut frame = vec![0_u8; 320 * 200 * 4];
        eligible
            .render(&mut frame)
            .expect("render user-open scoreboard");
        let layout = current_scoreboard_test_layout(&mut eligible);
        assert!(frames_differ_in_rect(&hidden, &frame, 320, layout.bounds,));

        toggle_scoreboard(&mut eligible, ModifiersState::empty());
        assert!(eligible.scoreboard_dialog.is_none());

        // Logo is not represented by C4KeyCodeEx and therefore remains an
        // exact bare-Tab ScoreboardToggle.
        toggle_scoreboard(&mut eligible, ModifiersState::LOGO);
        assert_eq!(
            eligible.scoreboard_dialog,
            Some(eligible.scoreboard_request())
        );
    }

    #[test]
    fn scoreboard_close_uses_cpp_drag_move_and_release_hit_testing() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        install_visible_scoreboard_highlight_fixture(&mut app);
        toggle_scoreboard(&mut app, ModifiersState::empty());
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("lay out the scoreboard");
        let baseline = app.graphics.surface().pixels().to_vec();
        let close = current_scoreboard_test_layout(&mut app)
            .close_button
            .expect("titled board close button");
        let point = PhysicalPosition::new(
            f64::from(close.x + close.w / 2),
            f64::from(close.y + close.h / 2),
        );
        app.handle_cursor_moved(point).expect("hover close");
        assert!(app.scoreboard_runtime.close_hovered);
        app.render(&mut frame).expect("render close hover pass");
        let hovered = app.graphics.surface().pixels().to_vec();
        assert!(frames_differ_in_rect(&baseline, &hovered, 320, close));
        let sounds_before_press = app.ui_sound_log.len();
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press close");
        assert_eq!(
            &app.ui_sound_log[sounds_before_press..],
            &["ArrowHit".to_string()]
        );
        app.render(&mut frame).expect("render close down pass");
        let down = app.graphics.surface().pixels().to_vec();
        assert!(frames_differ_in_rect(&hovered, &down, 320, close));
        assert!(app.scoreboard_close_pointer_capture);
        assert!(app.scoreboard_dialog.is_some());

        let outside = PhysicalPosition::new(0.0, 199.0);
        let sounds_before_leave = app.ui_sound_log.len();
        app.handle_cursor_moved(outside)
            .expect("captured close drag remains in scoreboard");
        assert_eq!(
            &app.ui_sound_log[sounds_before_leave..],
            &["ArrowHit".to_string()]
        );
        assert!(app.scoreboard_close_pointer_capture);
        assert!(app.ingame_pointer.is_none());
        app.handle_mouse_button(ElementState::Released)
            .expect("outside release clears capture and falls through");
        assert!(app.scoreboard_dialog.is_some());
        assert!(!app.scoreboard_close_pointer_capture);

        app.handle_cursor_moved(point).expect("hover close again");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press close again");
        app.handle_cursor_moved(outside)
            .expect("drag outside before re-entry");
        let sounds_before_reentry = app.ui_sound_log.len();
        app.handle_cursor_moved(point)
            .expect("drag back over close");
        assert_eq!(
            &app.ui_sound_log[sounds_before_reentry..],
            &["ArrowHit".to_string()]
        );
        let sounds_before_click = app.ui_sound_log.len();
        app.handle_mouse_button(ElementState::Released)
            .expect("release after re-entering close");
        assert_eq!(
            &app.ui_sound_log[sounds_before_click..],
            &["Click".to_string()]
        );
        assert!(app.scoreboard_dialog.is_none());
        assert!(!app.scoreboard_close_pointer_capture);
    }

    #[test]
    fn scoreboard_title_drag_and_cached_placement_survive_frames_until_data_update() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "A scoreboard title");
                   }
                   global func InvalidateLayout()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "A scoreboard title");
                   }"#,
        );
        toggle_scoreboard(&mut app, ModifiersState::empty());
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("materialize scoreboard layout");
        let original = current_scoreboard_test_layout(&mut app);
        let caption = original.caption.expect("title-bearing scoreboard");
        let title = PhysicalPosition::new(
            f64::from(caption.x + caption.w / 2),
            f64::from(caption.y + caption.h / 2),
        );
        app.handle_cursor_moved(title)
            .expect("hover scoreboard title");
        assert_eq!(
            app.classic_dialog_title_tooltip_target_at(GuiPoint::new(title.x as f32, title.y as f32,)),
            Some(StartupTooltip::text("A scoreboard title")),
        );
        app.handle_mouse_button(ElementState::Pressed)
            .expect("start scoreboard title drag");
        let moved_pointer = PhysicalPosition::new(title.x + 35.0, title.y + 22.0);
        app.handle_cursor_moved(moved_pointer)
            .expect("move retained scoreboard title drag");
        app.handle_mouse_button(ElementState::Released)
            .expect("finish scoreboard title drag");
        let moved = current_scoreboard_test_layout(&mut app);
        assert_eq!(moved.bounds.x, original.bounds.x + 35);
        assert_eq!(moved.bounds.y, original.bounds.y + 22);

        app.render(&mut frame)
            .expect("render a subsequent frame without new drag input");
        assert_eq!(current_scoreboard_test_layout(&mut app), moved);

        app.resize(640, 480)
            .expect("change the live preferred viewport rectangle");
        let mut resized_frame = vec![0_u8; 640 * 480 * 4];
        app.render(&mut resized_frame)
            .expect("preferred-only change keeps cached placement");
        assert_eq!(current_scoreboard_test_layout(&mut app), moved);

        call_scoreboard_function_and_update(&mut app, "InvalidateLayout");
        app.render(&mut resized_frame)
            .expect("data invalidation performs native Update and placement");
        assert_ne!(
            current_scoreboard_test_layout(&mut app).bounds,
            moved.bounds
        );
    }

    #[test]
    fn asynchronously_shown_message_stays_active_during_scoreboard_title_drag() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        app.resize(1024, 768).expect("resize shared running screen");
        toggle_scoreboard(&mut app, ModifiersState::empty());
        let mut frame = vec![0_u8; 1024 * 768 * 4];
        app.render(&mut frame)
            .expect("materialize scoreboard layout");
        let before = current_scoreboard_test_layout(&mut app);
        let caption = before.caption.expect("scoreboard title");
        let start = PhysicalPosition::new(
            f64::from(caption.x + caption.w / 2),
            f64::from(caption.y + caption.h / 2),
        );
        app.handle_cursor_moved(start)
            .expect("hover scoreboard title");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("capture scoreboard title");
        assert!(app.scoreboard_runtime.title_drag.is_some());

        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Asynchronous notice",
                "Higher dialog",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("show a higher dialog during the retained drag");
        assert!(matches!(
            app.running_active_dialog,
            Some(RunningDialogStackEntry::Message(_))
        ));

        let moved_pointer = PhysicalPosition::new(start.x - 24.0, start.y + 17.0);
        let message = app.top_message_dialog_layout().expect("message layout");
        assert!(!GameApp::point_in_message_dialog_bounds(
            GuiPoint::new(moved_pointer.x as f32, moved_pointer.y as f32),
            &message,
        ));
        app.handle_cursor_moved(moved_pointer)
            .expect("global drag updates below the active message");
        let moved = current_scoreboard_test_layout(&mut app);
        assert_eq!(moved.bounds.x, before.bounds.x - 24);
        assert_eq!(moved.bounds.y, before.bounds.y + 17);
        assert!(matches!(
            app.running_active_dialog,
            Some(RunningDialogStackEntry::Message(_))
        ));
        assert!(app.ingame_pointer.is_none());

        app.remove_message_dialog_at(0)
            .expect("close the asynchronously active message");
        assert!(app.scoreboard_runtime.title_drag.is_none());
        assert!(app.message_dialogs.is_empty());
        let after_close = current_scoreboard_test_layout(&mut app);
        app.handle_cursor_moved(PhysicalPosition::new(
            moved_pointer.x + 31.0,
            moved_pointer.y - 9.0,
        ))
        .expect("movement after active close no longer drags the scoreboard");
        assert_eq!(current_scoreboard_test_layout(&mut app), after_close);
        app.handle_mouse_button(ElementState::Released)
            .expect("release after active close is harmless");
    }

    #[test]
    fn scoreboard_pointer_before_draw_cannot_stamp_new_revision_onto_old_matrix() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }
                   global func GrowBetweenFrames()
                   {
                       SetScoreboardData(1, SBRD_Caption, "A much wider second column");
                   }"#,
        );
        toggle_scoreboard(&mut app, ModifiersState::empty());
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("materialize initial geometry");
        let initial = current_scoreboard_test_layout(&mut app);
        let initial_revision = app.scoreboard_runtime.layout_revision;
        let point = GuiPoint::new(
            (initial.bounds.x + initial.bounds.w / 2) as f32,
            (initial.bounds.y + initial.bounds.h / 2) as f32,
        );

        app.engine
            .call_scenario_script_function("GrowBetweenFrames", Vec::new())
            .expect("mutate the live scoreboard between frames");
        assert!(app
            .scoreboard_pointer_target(point)
            .expect("pointer route")
            .is_some());
        assert_eq!(app.snapshot.hud.scoreboard.row_count(), 2);
        assert_eq!(app.scoreboard_runtime.layout_revision, initial_revision);
        assert_eq!(
            app.scoreboard_runtime
                .presentation
                .as_ref()
                .expect("retained presentation")
                .layout(),
            &initial,
            "pointer input retains pre-draw C++ geometry",
        );

        app.render(&mut frame)
            .expect("the next draw lazily applies the live matrix update");
        assert_eq!(
            app.scoreboard_runtime.layout_revision,
            app.engine.scoreboard_layout_revision(),
        );
        assert_ne!(current_scoreboard_test_layout(&mut app), initial);
    }

    #[test]
    fn scoreboard_show_then_grow_keeps_constructor_hit_bounds_until_first_draw() {
        let mut app = new_scoreboard_test_app(
            r#"global func ShowThenGrow()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "S");
                       DoScoreboardShow(1);
                       SetScoreboardData(1, SBRD_Caption, "A deliberately huge late column");
                   }"#,
        );
        call_scoreboard_function_and_update(&mut app, "ShowThenGrow");
        let request_revision = app
            .scoreboard_dialog
            .as_ref()
            .expect("show request opened the dialog")
            .layout_revision;
        assert!(request_revision < app.engine.scoreboard_layout_revision());

        let constructor = current_scoreboard_test_layout(&mut app);
        let late_column_point = GuiPoint::new(
            (constructor.bounds.x - 2) as f32,
            (constructor.client.y + constructor.client.h / 2) as f32,
        );
        assert!(
            app.scoreboard_pointer_target(late_column_point)
                .expect("pre-draw pointer route")
                .is_none(),
            "the late column is outside the synchronous constructor bounds",
        );
        assert_eq!(app.scoreboard_runtime.layout_revision, request_revision);
        assert_eq!(current_scoreboard_test_layout(&mut app), constructor);

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("first Draw applies the pending SetCell invalidation");
        let updated = current_scoreboard_test_layout(&mut app);
        assert!(updated.bounds.x < late_column_point.x as i32);
        assert!(app
            .scoreboard_pointer_target_cached(late_column_point)
            .is_some());
    }

    #[test]
    fn synchronous_scoreboard_show_joins_pointer_routing_before_update_or_draw() {
        let mut app = new_scoreboard_test_app(
            r#"global func ShowNow()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "S");
                       DoScoreboardShow(1);
                   }"#,
        );
        app.engine
            .call_scenario_script_function("ShowNow", Vec::new())
            .expect("show scoreboard synchronously");
        assert!(app.scoreboard_dialog.is_none());

        let point = GuiPoint::new(299.0, 50.0);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("the first pointer ingress observes the synchronous dialog");

        assert!(app.scoreboard_dialog.is_some());
        assert!(app.scoreboard_pointer_target_cached(point).is_some());
        assert_eq!(
            app.running_active_dialog,
            Some(RunningDialogStackEntry::Scoreboard),
        );
        assert!(app.ingame_pointer.is_none());
    }

    #[test]
    fn scoreboard_resize_releases_title_drag_without_replacing_cached_geometry() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        toggle_scoreboard(&mut app, ModifiersState::empty());
        let before = current_scoreboard_test_layout(&mut app);
        let caption = before.caption.expect("scoreboard title");
        let start = PhysicalPosition::new(
            f64::from(caption.x + caption.w / 2),
            f64::from(caption.y + caption.h / 2),
        );
        app.handle_cursor_moved(start)
            .expect("hover scoreboard title");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("capture scoreboard title");
        assert!(app.scoreboard_runtime.title_drag.is_some());

        app.resize(360, 220).expect("resize running screen");
        assert!(app.scoreboard_runtime.title_drag.is_none());
        assert!(app.scoreboard_runtime.pointer.is_none());
        assert!(!app.scoreboard_runtime.close_hovered);
        assert!(!app.scoreboard_close_pointer_capture);
        let cached = current_scoreboard_test_layout(&mut app);
        assert_eq!(cached, before);

        app.handle_cursor_moved(PhysicalPosition::new(start.x + 30.0, start.y + 20.0))
            .expect("movement after resize does not continue the old drag");
        assert_eq!(current_scoreboard_test_layout(&mut app), cached);
    }

    #[test]
    fn scoreboard_touch_capture_is_released_when_a_new_message_owns_end_or_cancel() {
        let board = r#"global func Initialize()
                          {
                              SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                          }"#;
        let notice = || {
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Notice",
                "Higher dialog",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            )
        };

        let mut close = new_scoreboard_test_app(board);
        toggle_scoreboard(&mut close, ModifiersState::empty());
        let close_button = current_scoreboard_test_layout(&mut close)
            .close_button
            .expect("scoreboard close button");
        let close_point = GuiPoint::new(
            (close_button.x + close_button.w / 2) as f32,
            (close_button.y + close_button.h / 2) as f32,
        );
        close
            .handle_touch(TouchPhase::Started, close_point)
            .expect("capture scoreboard close touch");
        assert!(close.scoreboard_close_pointer_capture);
        close
            .push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("show higher message");
        let message = close.top_message_dialog_layout().expect("message layout");
        let message_point = GuiPoint::new(
            (message.bounds.x + message.bounds.w / 2) as f32,
            (message.bounds.y + message.bounds.h / 2) as f32,
        );
        close
            .handle_touch(TouchPhase::Ended, message_point)
            .expect("message owns release-time hit");
        assert!(close.scoreboard_dialog.is_some());
        assert!(!close.scoreboard_close_pointer_capture);

        let mut drag = new_scoreboard_test_app(board);
        toggle_scoreboard(&mut drag, ModifiersState::empty());
        let layout = current_scoreboard_test_layout(&mut drag);
        let caption = layout.caption.expect("scoreboard title");
        let title_point = GuiPoint::new(
            (caption.x + caption.w / 2) as f32,
            (caption.y + caption.h / 2) as f32,
        );
        drag.handle_touch(TouchPhase::Started, title_point)
            .expect("capture scoreboard title touch");
        assert!(drag.scoreboard_runtime.title_drag.is_some());
        drag.push_message_dialog(notice(), MessageDialogContinuation::None)
            .expect("show higher message during title drag");
        drag.handle_touch(TouchPhase::Cancelled, title_point)
            .expect("cancel the screen-global touch");
        assert!(drag.scoreboard_runtime.title_drag.is_none());
        assert!(!drag.scoreboard_close_pointer_capture);
        let after_cancel = current_scoreboard_test_layout(&mut drag);
        drag.handle_cursor_moved(PhysicalPosition::new(
            f64::from(title_point.x + 30.0),
            f64::from(title_point.y + 20.0),
        ))
        .expect("movement after cancellation cannot resume the drag");
        assert_eq!(current_scoreboard_test_layout(&mut drag), after_cancel);
    }

    #[test]
    fn scoreboard_bounds_consume_secondary_middle_wheel_and_touch_input() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
        toggle_scoreboard(&mut app, ModifiersState::empty());
        let layout = current_scoreboard_test_layout(&mut app);
        let point = GuiPoint::new(
            (layout.client.x + layout.client.w / 2) as f32,
            (layout.client.y + layout.client.h / 2) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("move over scoreboard client");
        app.ingame_mouse_init_centered = false;

        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("scoreboard consumes right down");
        app.handle_right_mouse_button(ElementState::Released)
            .expect("scoreboard consumes right up");
        assert!(!app.ingame_mouse_init_centered);
        assert!(commands.take_submitted_local().is_empty());

        app.handle_other_mouse_button(ElementState::Pressed)
            .expect("scoreboard consumes middle down");
        app.handle_other_mouse_button(ElementState::Released)
            .expect("scoreboard consumes middle up");
        assert!(!app.ingame_mouse_init_centered);
        assert!(commands.take_submitted_local().is_empty());

        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("scoreboard consumes wheel");
        assert!(!app.ingame_mouse_init_centered);
        assert!(commands.take_submitted_local().is_empty());

        let before_touch = current_scoreboard_test_layout(&mut app);
        let caption = before_touch.caption.expect("scoreboard title");
        let touch_title = GuiPoint::new(
            (caption.x + caption.w / 2) as f32,
            (caption.y + caption.h / 2) as f32,
        );
        let touch_moved = GuiPoint::new(touch_title.x + 12.0, touch_title.y + 9.0);
        app.handle_touch(TouchPhase::Started, touch_title)
            .expect("scoreboard consumes touch start");
        assert!(app.scoreboard_runtime.title_drag.is_some());
        app.handle_touch(TouchPhase::Moved, touch_moved)
            .expect("scoreboard consumes touch move");
        let after_touch_move = current_scoreboard_test_layout(&mut app);
        assert_eq!(after_touch_move.bounds.x, before_touch.bounds.x + 12);
        assert_eq!(after_touch_move.bounds.y, before_touch.bounds.y + 9);
        app.handle_touch(TouchPhase::Ended, touch_moved)
            .expect("scoreboard consumes touch end");
        assert!(app.scoreboard_runtime.title_drag.is_none());
        assert_eq!(current_scoreboard_test_layout(&mut app), after_touch_move);
        assert!(commands.take_submitted_local().is_empty());
        assert!(app.scoreboard_dialog.is_some());
    }

    #[test]
    fn running_context_menu_routes_before_shared_scoreboard_dialogs() {
        const BOARD: &str = r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "A deliberately wide scoreboard");
                SetScoreboardData(1, 1, "A deliberately wide value");
            }"#;

        let mut overlap = new_scoreboard_test_app(BOARD);
        overlap
            .resize(1024, 768)
            .expect("resize shared running screen");
        toggle_scoreboard(&mut overlap, ModifiersState::empty());
        let mut frame = vec![0_u8; 1024 * 768 * 4];
        overlap.render(&mut frame).expect("materialize scoreboard");
        let bounds = current_scoreboard_test_layout(&mut overlap).bounds;
        overlap
            .scoreboard_runtime
            .presentation
            .as_mut()
            .expect("scoreboard presentation")
            .layout_mut()
            .translate(40 - bounds.x, 40 - bounds.y);
        overlap
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Message",
                    "Higher shared dialog",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("show message above scoreboard");
        overlap
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new("Popup row")],
                GuiPoint::new(48.0, 48.0),
            )
            .expect("open popup over scoreboard");
        let popup_row = overlap
            .context_menu
            .as_ref()
            .expect("context menu")
            .layout()
            .panels[0]
            .rows[0]
            .rect;
        let popup_point = GuiPoint::new(
            (popup_row.x + popup_row.w / 2) as f32,
            (popup_row.y + popup_row.h / 2) as f32,
        );
        assert!(overlap
            .scoreboard_pointer_target_cached(popup_point)
            .is_some());
        assert!(!GameApp::point_in_message_dialog_bounds(
            popup_point,
            &overlap.top_message_dialog_layout().expect("message layout"),
        ));
        overlap
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(popup_point.x),
                f64::from(popup_point.y),
            ))
            .expect("popup owns overlapping movement");
        overlap
            .handle_mouse_button(ElementState::Pressed)
            .expect("popup owns overlapping press");
        assert!(overlap.context_menu.is_some());
        assert!(!overlap.scoreboard_close_pointer_capture);
        assert!(matches!(
            overlap.running_active_dialog,
            Some(RunningDialogStackEntry::Message(_))
        ));
        overlap
            .handle_mouse_button(ElementState::Released)
            .expect("release popup row");

        let mut outside = new_scoreboard_test_app(BOARD);
        toggle_scoreboard(&mut outside, ModifiersState::empty());
        outside
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new("Popup row")],
                GuiPoint::new(10.0, 10.0),
            )
            .expect("open popup away from scoreboard");
        let scoreboard = current_scoreboard_test_layout(&mut outside);
        let body = GuiPoint::new(
            (scoreboard.client.x + scoreboard.client.w / 2) as f32,
            (scoreboard.client.y + scoreboard.client.h / 2) as f32,
        );
        assert!(!outside
            .context_menu
            .as_ref()
            .expect("context menu")
            .captures_point(body));
        outside
            .handle_cursor_moved(PhysicalPosition::new(f64::from(body.x), f64::from(body.y)))
            .expect("move outside popup over scoreboard");
        outside.ingame_mouse_init_centered = false;
        outside
            .handle_right_mouse_button(ElementState::Pressed)
            .expect("outside right down closes popup then reaches scoreboard");
        assert!(outside.context_menu.is_none());
        assert!(!outside.ingame_mouse_init_centered);
    }

    #[test]
    fn scoreboard_wheel_does_not_scroll_an_overlapped_lower_f4_dialog() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        app.resize(1024, 768).expect("resize shared running screen");
        let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
        app.control_clients
            .replace_snapshot((0..40).map(|client_id| message_client(client_id, b"Remote")));
        toggle_scoreboard(&mut app, ModifiersState::empty());
        app.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("open F4 above the scoreboard");
        app.activate_running_dialog(RunningDialogStackEntry::Scoreboard);

        let (preferred, line_height) = app
            .runtime_client_list_input_geometry()
            .expect("F4 geometry");
        let f4_layout = app
            .runtime_client_list
            .as_ref()
            .expect("F4 dialog")
            .layout(preferred, line_height);
        let scoreboard = current_scoreboard_test_layout(&mut app);
        let dx = f4_layout.list.x + 8 - scoreboard.client.x;
        let dy = f4_layout.list.y + 8 - scoreboard.client.y;
        app.scoreboard_runtime
            .presentation
            .as_mut()
            .expect("scoreboard presentation")
            .layout_mut()
            .translate(dx, dy);
        let scoreboard = current_scoreboard_test_layout(&mut app);
        let point = GuiPoint::new(
            (scoreboard.client.x + 4) as f32,
            (scoreboard.client.y + 4) as f32,
        );
        assert!(point.x < (f4_layout.list.x + f4_layout.list.w) as f32);
        assert!(point.y < (f4_layout.list.y + f4_layout.list.h) as f32);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("hover the overlapping scoreboard and F4 list");
        let before = app
            .runtime_client_list
            .as_ref()
            .expect("F4 dialog")
            .scroll_row(preferred, line_height);
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("top scoreboard consumes wheel");
        let after = app
            .runtime_client_list
            .as_ref()
            .expect("F4 remains open")
            .scroll_row(preferred, line_height);
        assert_eq!(after, before, "wheel cannot fall through to lower F4");
    }

    #[test]
    fn shared_message_dialog_allows_exposed_scoreboard_close_click() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        install_visible_scoreboard_highlight_fixture(&mut app);
        app.resize(1024, 768).expect("resize shared running screen");
        toggle_scoreboard(&mut app, ModifiersState::empty());
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Message remains open",
                "Shared dialog",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("show ordinary shared message dialog");

        let close = current_scoreboard_test_layout(&mut app)
            .close_button
            .expect("titled board close button");
        let point = GuiPoint::new(
            (close.x + close.w / 2) as f32,
            (close.y + close.h / 2) as f32,
        );
        let message_layout = app.top_message_dialog_layout().expect("message layout");
        assert!(!GameApp::point_in_message_dialog_bounds(
            point,
            &message_layout,
        ));

        let mut frame = vec![0_u8; 1024 * 768 * 4];
        app.render(&mut frame)
            .expect("render inactive scoreboard below message");
        let baseline = app.graphics.surface().pixels().to_vec();

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("route outside-dialog hover to the exposed scoreboard");
        assert!(matches!(
            app.running_active_dialog,
            Some(RunningDialogStackEntry::Message(_))
        ));
        assert!(app.scoreboard_runtime.close_hovered);
        app.render(&mut frame)
            .expect("shared inactive scoreboard still draws mouse hover");
        let hovered = app.graphics.surface().pixels().to_vec();
        assert!(frames_differ_in_rect(&baseline, &hovered, 1024, close));
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press exposed scoreboard close button");
        app.handle_mouse_button(ElementState::Released)
            .expect("release exposed scoreboard close button");
        assert_eq!(app.message_dialogs.len(), 1);
        assert!(app.scoreboard_dialog.is_none());
    }

    #[test]
    fn scoreboard_uses_shared_cpp_show_and_left_activation_stack_order() {
        const BOARD: &str = r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
            }"#;

        let mut f4 = new_scoreboard_test_app(BOARD);
        let (_events, _commands) = install_running_network_stub(&mut f4, 0, 40, 4);
        f4.control_clients
            .replace_snapshot([message_client(0, b"Host")]);
        f4.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("show equal-z F4 list first");
        toggle_scoreboard(&mut f4, ModifiersState::empty());
        assert!(f4.scoreboard_is_above_runtime_client_list());
        f4.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("close F4 list");
        f4.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("show F4 list after scoreboard");
        assert!(!f4.scoreboard_is_above_runtime_client_list());

        let mut messages = new_scoreboard_test_app(BOARD);
        messages.resize(1024, 768).expect("resize shared screen");
        toggle_scoreboard(&mut messages, ModifiersState::empty());
        messages
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "First message",
                    "Input-z dialog",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("show message after scoreboard");
        assert!(!messages.scoreboard_is_above_all_messages());
        let layout = current_scoreboard_test_layout(&mut messages);
        let point = GuiPoint::new(
            (layout.client.x + layout.client.w / 2) as f32,
            (layout.client.y + layout.client.h / 2) as f32,
        );
        assert!(!GameApp::point_in_message_dialog_bounds(
            point,
            &messages
                .top_message_dialog_layout()
                .expect("message layout"),
        ));
        messages
            .handle_cursor_moved(PhysicalPosition::new(
                f64::from(point.x),
                f64::from(point.y),
            ))
            .expect("hover exposed scoreboard");
        messages
            .handle_mouse_button(ElementState::Pressed)
            .expect("left activation moves default-z scoreboard last");
        messages
            .handle_mouse_button(ElementState::Released)
            .expect("release scoreboard body");
        assert!(messages.scoreboard_is_above_all_messages());

        messages
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Second message",
                    "Later input-z dialog",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("later z+1 show overtakes activated scoreboard");
        assert!(!messages.scoreboard_is_above_all_messages());
        assert!(matches!(
            messages.running_dialog_stack.last(),
            Some(RunningDialogStackEntry::Message(_))
        ));
    }

    #[test]
    fn scoreboard_close_restores_the_chat_exposed_beneath_its_activation() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        toggle_scoreboard(&mut app, ModifiersState::empty());
        app.start_running_chat(RunningChatMode::All);
        assert!(app.running_chat_active());

        let layout = current_scoreboard_test_layout(&mut app);
        let body = PhysicalPosition::new(
            f64::from(layout.client.x + layout.client.w / 2),
            f64::from(layout.client.y + layout.client.h / 2),
        );
        let chat = app.game_option_input_layout().expect("chat layout");
        assert!(!GameApp::point_in_input_dialog_bounds(
            GuiPoint::new(body.x as f32, body.y as f32),
            &chat,
        ));
        app.handle_cursor_moved(body)
            .expect("hover the exposed scoreboard body");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("activate scoreboard over chat");
        app.handle_mouse_button(ElementState::Released)
            .expect("release scoreboard activation");
        assert!(!app.running_chat_active());
        assert_eq!(
            app.running_active_dialog,
            Some(RunningDialogStackEntry::Scoreboard),
        );

        let close = current_scoreboard_test_layout(&mut app)
            .close_button
            .expect("scoreboard close button");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(close.x + close.w / 2),
            f64::from(close.y + close.h / 2),
        ))
        .expect("hover scoreboard close");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press scoreboard close");
        app.handle_mouse_button(ElementState::Released)
            .expect("close scoreboard and expose chat");
        assert!(app.scoreboard_dialog.is_none());
        assert!(app.running_chat_active());
        assert_eq!(
            app.running_active_dialog,
            Some(RunningDialogStackEntry::Chat),
        );
    }

    #[test]
    fn activated_chat_under_list_top_scoreboard_does_not_gain_keyboard_focus() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        toggle_scoreboard(&mut app, ModifiersState::empty());
        app.start_running_chat(RunningChatMode::All);

        let scoreboard = current_scoreboard_test_layout(&mut app);
        let scoreboard_body = PhysicalPosition::new(
            f64::from(scoreboard.client.x + scoreboard.client.w / 2),
            f64::from(scoreboard.client.y + scoreboard.client.h / 2),
        );
        app.handle_cursor_moved(scoreboard_body)
            .expect("hover exposed scoreboard body");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("move default-z scoreboard to list top");
        app.handle_mouse_button(ElementState::Released)
            .expect("release scoreboard activation");

        let chat = app.game_option_input_layout().expect("chat layout");
        let chat_point = GuiPoint::new(
            (chat.bounds.x + chat.bounds.w / 2) as f32,
            (chat.bounds.y + chat.bounds.h / 2) as f32,
        );
        assert!(app.scoreboard_pointer_target_cached(chat_point).is_none());
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(chat_point.x),
            f64::from(chat_point.y),
        ))
        .expect("hover exposed chat");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("activate z+2 chat without reordering it");
        app.handle_mouse_button(ElementState::Released)
            .expect("release chat activation");
        assert!(app.running_chat_active());
        assert!(!app.running_chat_keyboard_active());
        assert_eq!(
            app.running_dialog_stack.last(),
            Some(&RunningDialogStackEntry::Scoreboard),
        );

        app.handle_text_input('x')
            .expect("list-top nonexclusive dialog suppresses GUI text");
        assert_eq!(app.running_chat_text(), Some(""));

        assert!(app.close_scoreboard_dialog());
        assert!(app.running_chat_keyboard_active());
        app.handle_text_input('x')
            .expect("chat accepts text once it is list-top again");
        assert_eq!(app.running_chat_text(), Some("x"));
    }

    #[test]
    fn ordinary_message_behind_scoreboard_does_not_suppress_gamepad_gameplay() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        toggle_scoreboard(&mut app, ModifiersState::empty());
        route_primary_gamepad_to_local_owner(&mut app);
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Notice",
                "Retained below the scoreboard",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("show ordinary shared-screen message");
        app.activate_running_dialog(RunningDialogStackEntry::Scoreboard);
        assert_eq!(
            app.running_dialog_stack.last(),
            Some(&RunningDialogStackEntry::Scoreboard)
        );
        assert!(!app.message_dialog_owns_gamepad_input());

        app.process_gamepad_event_batch([
            GamepadEvent::Axis {
                slot: GamepadSlot::new(0),
                axis: LegacyGamepadAxis::new(0, true),
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Right,
                state: ElementState::Pressed,
            },
        ])
        .expect("list-top nonexclusive scoreboard leaves gameplay in scope");

        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_RIGHT),
            0,
        );
        assert_eq!(app.message_dialogs.len(), 1);
    }

    #[test]
    fn scoreboard_release_clears_an_occluded_f4_button_capture() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        app.resize(1024, 768).expect("resize shared running screen");
        let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
        app.control_clients
            .replace_snapshot([message_client(0, b"Host")]);
        toggle_scoreboard(&mut app, ModifiersState::empty());
        app.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("show F4 above scoreboard");

        let (preferred, line_height) = app
            .runtime_client_list_input_geometry()
            .expect("F4 geometry");
        let close = app
            .runtime_client_list
            .as_ref()
            .expect("F4 dialog")
            .layout(preferred, line_height)
            .close_button;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(close.x + close.w / 2),
            f64::from(close.y + close.h / 2),
        ))
        .expect("hover F4 close");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("capture F4 close");
        assert!(app
            .runtime_client_list
            .as_ref()
            .expect("F4 remains open")
            .has_pointer_capture());

        let scoreboard = current_scoreboard_test_layout(&mut app);
        app.scoreboard_runtime
            .presentation
            .as_mut()
            .expect("scoreboard presentation")
            .layout_mut()
            .translate(900 - scoreboard.client.x, 50 - scoreboard.client.y);
        let scoreboard = current_scoreboard_test_layout(&mut app);
        let release = PhysicalPosition::new(
            f64::from(scoreboard.client.x + 4),
            f64::from(scoreboard.client.y + 4),
        );
        app.handle_cursor_moved(release)
            .expect("F4 capture retains held move over lower scoreboard");
        app.handle_mouse_button(ElementState::Released)
            .expect("release routes by actual scoreboard hit");
        assert!(app.runtime_client_list.is_some());
        assert!(!app
            .runtime_client_list
            .as_ref()
            .expect("F4 remains open")
            .has_pointer_capture());
        assert!(app.scoreboard_dialog.is_some());
    }

    #[test]
    fn modified_tab_neither_opens_scoreboard_nor_dispatches_rebound_player_control() {
        let mut app = new_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        app.bindings
            .rebind(ControlBindingId::PlayerMenu, VirtualKeyCode::Tab);
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
        ] {
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .expect("modified Tab has no exact C4 binding");
            app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("release modified Tab");
            assert!(app.ingame_menu.is_none());
            assert!(app.message_dialogs.is_empty());
            assert!(!app.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
            assert!(app.scoreboard_dialog.is_none());
        }

        app.bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::Tab);
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("bare rebound Tab uses PRIO_PlrControl");
        assert!(app.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
        );
        app.open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(20.0, 20.0),
        )
        .expect("open context before the modified release");
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("modified release is not the bare control binding");
        assert!(!app.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
            "exact modifier matching suppresses the bare control release callback",
        );
        assert!(app.context_menu.is_some());

        let mut exclusive_release = new_classic_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        exclusive_release
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::Tab);
        exclusive_release
            .engine
            .player_mut(exclusive_release.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        exclusive_release
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("bare rebound press reaches the player control");
        exclusive_release
            .handle_game_over()
            .expect("open an exclusive dialog between key edges");
        exclusive_release
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("exclusive dialog owns the bare release scope");
        exclusive_release.dismiss_game_over_dialog();
        assert!(!exclusive_release
            .pressed_engine_keys
            .contains(&VirtualKeyCode::Tab));
        assert_ne!(
            exclusive_release
                .engine
                .player(exclusive_release.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
            "the out-of-scope release clears only raw repeat bookkeeping",
        );

        let mut dialog_press = new_classic_scoreboard_test_app(
            r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
        );
        dialog_press
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::Tab);
        dialog_press
            .engine
            .player_mut(dialog_press.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        dialog_press
            .handle_game_over()
            .expect("show exclusive dialog before raw press");
        dialog_press
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("exclusive dialog owns the initial raw press");
        assert_eq!(
            dialog_press
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            Some(GameOverFocus::Close)
        );
        assert!(dialog_press.scoreboard_tab_raw_pressed);
        assert!(!dialog_press
            .pressed_engine_keys
            .contains(&VirtualKeyCode::Tab));
        dialog_press.dismiss_game_over_dialog();
        dialog_press
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("held bare repeat enters player scope");
        assert_eq!(
            dialog_press
                .engine
                .player(dialog_press.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
            "AutoStopControl consumes a repeat first seen in another scope",
        );
        dialog_press
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("eventual raw release clears both latches");
        assert!(!dialog_press.scoreboard_tab_raw_pressed);
        assert!(!dialog_press
            .pressed_engine_keys
            .contains(&VirtualKeyCode::Tab));
    }

    #[test]
    fn scoreboard_tab_obeys_dialog_context_and_menu_priority() {
        const BOARD: &str = r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
            }"#;

        let mut message = new_scoreboard_test_app(BOARD);
        message
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::new(
                    "Scoreboard",
                    "Dialog keeps focus",
                    clonk_frontend::message_dialog::MessageDialogButtons::OK
                        | clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                    clonk_frontend::message_dialog::MessageDialogSize::Regular,
                    false,
                ),
                MessageDialogContinuation::None,
            )
            .expect("push running message dialog");
        message
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("shared message dialog leaves ScoreboardToggle in scope");
        message
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release scoreboard Tab under shared message dialog");
        assert_eq!(message.message_dialogs.len(), 1);
        assert!(message.scoreboard_dialog.is_some());
        assert_eq!(
            message.message_dialogs[0].state.focused_button(),
            Some(clonk_frontend::message_dialog::MessageDialogButton::Ok),
        );
        message
            .handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("set modified shared-screen Tab");
        message
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("modified Tab is inert outside KEYSCOPE_Gui");
        message
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release inert modified Tab");
        assert_eq!(
            message.message_dialogs[0].state.focused_button(),
            Some(clonk_frontend::message_dialog::MessageDialogButton::Ok),
        );
        assert!(message.message_dialog_consumed_keys.is_empty());

        let mut game_over = new_classic_scoreboard_test_app(BOARD);
        game_over
            .handle_game_over()
            .expect("show game-over focus dialog");
        game_over
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("game-over owns Tab focus traversal");
        game_over
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("game-over owns the Tab release");
        assert_eq!(
            game_over
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            Some(GameOverFocus::Close)
        );
        assert!(game_over.scoreboard_dialog.is_none());

        let mut context = new_scoreboard_test_app(BOARD);
        context
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open context menu");
        toggle_scoreboard(&mut context, ModifiersState::empty());
        assert!(context.context_menu.is_some());
        assert!(context.scoreboard_dialog.is_some());

        let mut rebound_context = new_scoreboard_test_app(BOARD);
        rebound_context
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open rebound context menu");
        rebound_context
            .bindings
            .rebind(ControlBindingId::PlayerMenu, VirtualKeyCode::Tab);
        rebound_context
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("PRIO_PlrControl precedes the context callback");
        rebound_context
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release rebound context Tab");
        assert!(rebound_context.context_menu.is_some());
        assert!(rebound_context.ingame_menu.is_some());

        let mut game_over_context = new_classic_scoreboard_test_app(BOARD);
        game_over_context
            .handle_game_over()
            .expect("show game-over context dialog");
        game_over_context
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open context over evaluation");
        game_over_context
            .bindings
            .rebind(ControlBindingId::PlayerMenu, VirtualKeyCode::Tab);
        let before_game_over_context = runtime_global_ui_snapshot(&game_over_context);
        game_over_context
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("game-over context leaves only the suppressed generic route");
        game_over_context
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release suppressed game-over context Tab");
        assert_eq!(
            runtime_global_ui_snapshot(&game_over_context),
            before_game_over_context
        );

        let mut object = new_scoreboard_test_app(BOARD);
        assert!(object.open_object_menu().expect("open object menu"));
        toggle_scoreboard(&mut object, ModifiersState::empty());
        assert!(object.object_menu.is_some());
        assert!(object.scoreboard_dialog.is_some());

        let mut player = new_scoreboard_test_app(BOARD);
        player.open_ingame_menu().expect("open player menu");
        toggle_scoreboard(&mut player, ModifiersState::empty());
        assert!(player.ingame_menu.is_some());
        assert!(player.scoreboard_dialog.is_some());
    }

    fn call_scoreboard_function_and_update(app: &mut GameApp, function: &str) {
        app.engine
            .call_scenario_script_function(function, Vec::new())
            .expect("call runtime scoreboard fixture");
        app.update().expect("apply runtime scoreboard request");
        app.snapshot.hud.messages.clear();
    }

    #[test]
    fn synchronous_scoreboard_callback_is_applied_before_render_and_tab_without_a_tick() {
        const CALLBACK_BOARD: &str = r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "PRIVATE_CALLBACK_CELL");
                   }
                   global func ShowNow()
                   {
                       DoScoreboardShow(1);
                   }"#;

        let mut render_app = new_scoreboard_test_app(CALLBACK_BOARD);
        render_app
            .engine
            .call_scenario_script_function("ShowNow", Vec::new())
            .expect("synchronous callback shows scoreboard");
        assert!(render_app.scoreboard_dialog.is_none());
        let mut frame = vec![0x5a; 320 * 200 * 4];
        let sentinel = frame.clone();
        render_app
            .render(&mut frame)
            .expect("render drains and draws a synchronous presentation request");
        assert_ne!(frame, sentinel);
        assert!(render_app.scoreboard_dialog.is_some());

        let mut tab_app = new_scoreboard_test_app(CALLBACK_BOARD);
        tab_app
            .engine
            .call_scenario_script_function("ShowNow", Vec::new())
            .expect("synchronous callback shows scoreboard");
        assert!(tab_app.scoreboard_dialog.is_none());
        tab_app
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Tab first drains and then closes the live pDlg");
        tab_app
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release close toggle");
        assert!(tab_app.scoreboard_dialog.is_none());
        assert_eq!(tab_app.snapshot.hud.scoreboard.show_count(), 1);
        tab_app
            .render(&mut frame)
            .expect("the consumed request cannot reopen after the user close");
    }

    #[test]
    fn scoreboard_restore_uses_saved_refcount_but_not_the_no_save_user_dialog() {
        const RESTORE_BOARD: &str = r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }
                   global func ShowNow()
                   {
                       DoScoreboardShow(1);
                   }"#;

        let mut positive = new_scoreboard_test_app(RESTORE_BOARD);
        call_scoreboard_function_and_update(&mut positive, "ShowNow");
        positive
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("user hides the positive-refcount dialog before save");
        positive
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release user hide");
        assert!(positive.scoreboard_dialog.is_none());
        let saved_positive = positive.engine.capture_state();
        assert_eq!(saved_positive.scoreboard.show_count(), 1);

        positive
            .engine
            .restore_state(&saved_positive)
            .expect("restore positive scoreboard state");
        positive.snapshot = positive.engine.snapshot();
        positive.arm_initial_scoreboard_reconcile();
        let before_surface = positive.graphics.surface().pixels().to_vec();
        let mut frame = vec![0x4c; 320 * 200 * 4];
        let sentinel = frame.clone();
        positive
            .render(&mut frame)
            .expect("load-time DoDlgShow(0) reopens and renders saved positive refcount");
        assert_ne!(frame, sentinel);
        assert_ne!(
            positive.graphics.surface().pixels(),
            before_surface.as_slice()
        );
        assert!(positive.scoreboard_dialog.is_some());
        assert_eq!(
            positive.engine.scoreboard_snapshot(),
            saved_positive.scoreboard
        );

        let mut zero = new_scoreboard_test_app(RESTORE_BOARD);
        let user_open_request = zero.scoreboard_request();
        assert!(zero.snapshot.hud.scoreboard.can_be_shown());
        assert!(!user_open_request.should_be_shown());
        zero.scoreboard_dialog = Some(user_open_request);
        let saved_zero = zero.engine.capture_state();
        assert_eq!(saved_zero.scoreboard.show_count(), 0);

        zero.engine
            .restore_state(&saved_zero)
            .expect("restore zero-refcount scoreboard state");
        zero.snapshot = zero.engine.snapshot();
        zero.arm_initial_scoreboard_reconcile();
        assert!(zero.scoreboard_dialog.is_none());
        zero.render(&mut frame)
            .expect("NO-SAVE user-open pDlg does not survive restoration");
        assert!(zero.scoreboard_dialog.is_none());
        assert_eq!(zero.engine.scoreboard_snapshot(), saved_zero.scoreboard);
    }

    #[test]
    fn script_scoreboard_lifecycle_uses_ordered_requests_not_final_refcount() {
        let mut empty_then_cell = new_scoreboard_test_app(
            r#"global func EmptyThenCell()
                   {
                       DoScoreboardShow(1);
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "late");
                   }"#,
        );
        call_scoreboard_function_and_update(&mut empty_then_cell, "EmptyThenCell");
        assert!(empty_then_cell.snapshot.hud.scoreboard.should_be_shown());
        assert!(
            empty_then_cell.scoreboard_dialog.is_none(),
            "SetCell cannot retroactively open an earlier empty request"
        );
        let mut ordinary = vec![0_u8; 320 * 200 * 4];
        empty_then_cell
            .render(&mut ordinary)
            .expect("a positive final state alone does not imply pDlg");

        let mut open_then_close = new_scoreboard_test_app(
            r#"global func OpenThenClose()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       DoScoreboardShow(1);
                       DoScoreboardShow(-1);
                   }"#,
        );
        call_scoreboard_function_and_update(&mut open_then_close, "OpenThenClose");
        assert_eq!(open_then_close.snapshot.hud.scoreboard.show_count(), 0);
        assert!(open_then_close.scoreboard_dialog.is_none());
        open_then_close
            .render(&mut ordinary)
            .expect("open then close in one tick leaves no render boundary");
    }

    #[test]
    fn later_data_update_collapses_request_time_allocated_empty_title_margin() {
        let mut app = new_scoreboard_test_app(
            r#"global func ShowEmptyThenInvalidate()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "");
                       DoScoreboardShow(1);
                       SetScoreboardData(1, SBRD_Caption, "Row");
                   }"#,
        );
        call_scoreboard_function_and_update(&mut app, "ShowEmptyThenInvalidate");
        let request = app
            .scoreboard_dialog
            .as_ref()
            .expect("request opened scoreboard");
        assert!(!request.title_widget_present);
        assert!(request.layout_revision < app.engine.scoreboard_layout_revision());

        let constructor_layout = current_scoreboard_test_layout(&mut app);
        assert!(constructor_layout.caption.is_none());
        assert!(constructor_layout.client.y > constructor_layout.bounds.y);

        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("Draw applies the invalidated empty-title Update");
        let updated_layout = current_scoreboard_test_layout(&mut app);
        assert!(updated_layout.caption.is_none());
        assert_eq!(updated_layout.client.y, updated_layout.bounds.y);
        assert_eq!(updated_layout.client.h, updated_layout.bounds.h);
    }

    #[test]
    fn visible_script_scoreboard_preflights_live_data_and_user_tab_can_close_it() {
        let mut app = new_scoreboard_test_app(
            r#"global func ShowThenGrow()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       DoScoreboardShow(1);
                       SetScoreboardData(SBRD_Caption, 7, "Value");
                       SetScoreboardData(1, SBRD_Caption, "One");
                       SetScoreboardData(1, 7, "PRIVATE_LATE_CELL", 42);
                   }"#,
        );
        call_scoreboard_function_and_update(&mut app, "ShowThenGrow");
        assert!(app.scoreboard_dialog.is_some());
        assert_eq!(
            (
                app.snapshot.hud.scoreboard.row_count(),
                app.snapshot.hud.scoreboard.column_count(),
            ),
            (2, 2)
        );

        let before_ui = runtime_global_ui_snapshot(&app);
        let before_surface = app.graphics.surface().pixels().to_vec();
        let mut frame = vec![0x6d; 320 * 200 * 4];
        let sentinel = frame.clone();
        for _ in 0..2 {
            app.render(&mut frame)
                .expect("visible scoreboard draws its current live matrix");
            assert_ne!(frame, sentinel);
            assert_ne!(app.graphics.surface().pixels(), before_surface.as_slice());
            assert_eq!(runtime_global_ui_snapshot(&app), before_ui);
        }

        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Tab closes an existing pDlg without needing its renderer");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release close toggle");
        assert!(app.scoreboard_dialog.is_none());
        app.render(&mut frame)
            .expect("positive refcount does not reopen after a user close");
    }

    #[test]
    fn unresolved_scoreboard_font_image_fails_typed_before_pixels() {
        let mut app = new_scoreboard_test_app(
            r#"global func ShowBroken()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       SetScoreboardData(1, SBRD_Caption, "{{NO_SUCH_DEFINITION}}");
                       DoScoreboardShow(1);
                   }"#,
        );
        call_scoreboard_function_and_update(&mut app, "ShowBroken");
        assert!(app.scoreboard_dialog.is_some());
        let before_surface = app.graphics.surface().pixels().to_vec();
        let mut frame = vec![0x71; 320 * 200 * 4];
        let sentinel = frame.clone();

        let error = app
            .render(&mut frame)
            .expect_err("an unresolved FontRegular image must fail closed");
        assert!(matches!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(ClassicParityBoundary::Scoreboard {
                trigger: ClassicScoreboardTrigger::ScriptVisibility,
                rows: 2,
                columns: 1,
                show_count: 1,
            })
        ));
        assert_eq!(frame, sentinel);
        assert_eq!(app.graphics.surface().pixels(), before_surface.as_slice());
        assert!(app.scoreboard_dialog.is_some());
    }

    #[test]
    fn same_tick_game_over_closes_scoreboard_and_continue_does_not_reopen_it() {
        const GAME_OVER_BOARD: &str = r#"global func ShowAndEnd()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                DoScoreboardShow(1);
                GameOver();
            }
            global func Recheck()
            {
                DoScoreboardShow(0);
            }"#;
        let mut app = new_classic_scoreboard_test_app(GAME_OVER_BOARD);
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        app.dispatch_control_event(ControlEvent::Press(ControlButton::Left))
            .expect("hold a player control under the fullscreen menu");
        assert_ne!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
        );
        app.open_ingame_menu().expect("open fullscreen player menu");
        call_scoreboard_function_and_update(&mut app, "ShowAndEnd");
        assert!(app.game_over_dialog.is_some());
        assert!(app.scoreboard_dialog.is_none());
        assert!(app.ingame_menu.is_none());
        assert_eq!(
            app.engine
                .player(app.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0,
            "game-over player-menu close synchronizes ClearPressedComs once",
        );
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("evaluation suppresses same-tick scoreboard creation");

        app.engine
            .call_scenario_script_function("Recheck", Vec::new())
            .expect("runtime reconciliation while evaluation is exclusive");
        app.handle_game_over_action(GameOverAction::Continue)
            .expect("continue evaluation");
        assert!(app.game_over_dialog.is_none());
        assert!(app.scoreboard_dialog.is_none());
        app.render(&mut frame)
            .expect("Continue does not implicitly reconcile the positive count");

        call_scoreboard_function_and_update(&mut app, "Recheck");
        assert!(app.scoreboard_dialog.is_some());
        app.render(&mut frame)
            .expect("a later runtime DoDlgShow may reopen and render after Continue");

        let mut save_browser = new_classic_scoreboard_test_app(GAME_OVER_BOARD);
        save_browser
            .open_save_browser()
            .expect("open fullscreen save-browser descendant");
        assert!(save_browser.save_browser.is_some());
        call_scoreboard_function_and_update(&mut save_browser, "ShowAndEnd");
        assert!(save_browser.game_over_dialog.is_some());
        assert!(save_browser.save_browser.is_none());
        assert!(!save_browser.save_browser_return_to_menu);

        let mut object_menu = new_classic_scoreboard_test_app(GAME_OVER_BOARD);
        assert!(object_menu.open_object_menu().expect("open object menu"));
        call_scoreboard_function_and_update(&mut object_menu, "ShowAndEnd");
        assert!(object_menu.game_over_dialog.is_some());
        assert!(
            object_menu.object_menu.is_some(),
            "C4Player::CloseMenu does not discard synchronized object menus",
        );
    }

    #[test]
    fn game_over_chat_and_mnemonics_use_exact_modes_and_priority() {
        for (key, modifiers, expected_text) in [
            (VirtualKeyCode::Return, ModifiersState::empty(), ""),
            (VirtualKeyCode::F2, ModifiersState::empty(), ""),
            (VirtualKeyCode::Return, ModifiersState::SHIFT, "/team "),
            (VirtualKeyCode::Return, ModifiersState::LOGO, ""),
            (
                VirtualKeyCode::Return,
                ModifiersState::LOGO | ModifiersState::SHIFT,
                "/team ",
            ),
        ] {
            let mut app = new_game_over_keyboard_app();
            app.handle_modifiers_changed(modifiers)
                .expect("set chat shortcut modifiers");
            app.handle_key(key, ElementState::Pressed)
                .expect("open the real running-chat controller");
            assert_eq!(app.running_chat_text(), Some(expected_text));
            assert!(app.game_over_dialog.is_some());
        }

        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::ALT | ModifiersState::SHIFT,
            ModifiersState::LOGO | ModifiersState::ALT,
        ] {
            let mut app = new_game_over_keyboard_app();
            app.handle_modifiers_changed(modifiers)
                .expect("set mnemonic modifiers");
            app.handle_key(VirtualKeyCode::C, ElementState::Pressed)
                .expect("localized Continue mnemonic activates directly");
            assert!(app.game_over_dialog.is_none());
            assert_eq!(app.mode, AppMode::Running);
            assert!(!app
                .ui_sound_log
                .iter()
                .any(|sound| matches!(sound.as_str(), "ArrowHit" | "Click")));
        }

        let mut say = new_game_over_keyboard_app();
        say.game_over_dialog
            .as_mut()
            .expect("evaluation dialog")
            .set_button_content(
                GameOverAction::Restart,
                "Play again".to_string(),
                "Restart without an R mnemonic".to_string(),
            );
        say.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set Say modifiers");
        say.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("unmatched Alt+Return falls through to Say chat");
        assert_eq!(say.running_chat_text(), Some("\""));
        assert!(say.game_over_dialog.is_some());

        for (key, modifiers) in [
            (
                VirtualKeyCode::Return,
                ModifiersState::ALT | ModifiersState::SHIFT,
            ),
            (VirtualKeyCode::Escape, ModifiersState::ALT),
        ] {
            say.handle_modifiers_changed(modifiers)
                .expect("set an unmatched active-chat hotkey");
            say.handle_key(key, ElementState::Pressed)
                .expect("active chat keeps the evaluation callbacks inactive");
            say.handle_key(key, ElementState::Released)
                .expect("active chat owns the matching release");
            assert!(say.game_over_dialog.is_some());
            assert_eq!(say.running_chat_text(), Some("\""));
        }

        let mut app = new_game_over_keyboard_app();
        for modifiers in [
            ModifiersState::CTRL,
            ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::ALT,
            ModifiersState::CTRL | ModifiersState::ALT | ModifiersState::SHIFT,
        ] {
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
                .expect("combined Return has no exact C++ chat binding");
            app.handle_key(VirtualKeyCode::Return, ElementState::Released)
                .expect("combined Return release is consumed");
        }
        for modifiers in [
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::ALT,
        ] {
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
                .expect("modified F2 has no exact C++ chat binding");
            app.handle_key(VirtualKeyCode::F2, ElementState::Released)
                .expect("modified F2 release is consumed");
        }
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::NumpadEnter, ElementState::Pressed)
            .expect("the macOS SDL oracle does not register keypad Enter here");
        app.handle_key(VirtualKeyCode::NumpadEnter, ElementState::Released)
            .expect("keypad Enter release is consumed");
        assert!(app.game_over_dialog.is_some());
    }

    #[test]
    fn game_over_mnemonics_use_active_language_resources() {
        let user_data = tempdir().expect("localized game-over user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "General", "LanguageEx", "DE").expect("select German resources");
        let mut app = new_classic_running_sandbox_app();
        app.app_paths = Some(paths);
        app.reload_application_language_resources()
            .expect("reload German resources after replacing fixture paths");
        app.handle_game_over()
            .expect("show localized evaluation dialog");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt");
        app.handle_key(VirtualKeyCode::W, ElementState::Pressed)
            .expect("German &Weiterspielen mnemonic invokes Continue");

        assert_eq!(app.mode, AppMode::Running);
        assert!(app.game_over_dialog.is_none());
        assert!(app.running_chat_text().is_none());
        assert!(!app
            .ui_sound_log
            .iter()
            .any(|sound| matches!(sound.as_str(), "ArrowHit" | "Click")));
    }

    #[test]
    fn game_over_tab_moves_real_focus_and_controls_activate_or_open_chat() {
        let mut list_focus = new_game_over_keyboard_app();
        for _ in 0..2 {
            list_focus
                .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .expect("advance to the player list");
            list_focus
                .handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("release focus traversal key");
        }
        assert_eq!(
            list_focus
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            Some(GameOverFocus::PlayerList(0))
        );
        list_focus
            .handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("player-list Return falls through to All chat");
        assert_eq!(list_focus.running_chat_text(), Some(""));
        assert!(list_focus.game_over_dialog.is_some());

        let mut keyboard = new_game_over_keyboard_app();
        for _ in 0..4 {
            keyboard
                .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .expect("advance to Continue");
            keyboard
                .handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("release focus traversal key");
        }
        assert_eq!(
            keyboard
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused_action),
            Some(GameOverAction::Continue)
        );
        keyboard
            .handle_key(VirtualKeyCode::Space, ElementState::Pressed)
            .expect("depress focused Continue");
        assert!(keyboard.game_over_dialog.is_some());
        keyboard
            .handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("any shared activation-key release activates Continue");
        assert!(keyboard.game_over_dialog.is_none());
        assert!(keyboard
            .ui_sound_log
            .iter()
            .any(|sound| sound == "ArrowHit"));
        assert!(keyboard.ui_sound_log.iter().any(|sound| sound == "Click"));

        let mut gamepad = new_game_over_keyboard_app();
        for _ in 0..4 {
            gamepad
                .process_gamepad_event_batch([GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                }])
                .expect("advance gamepad focus to Continue");
        }
        gamepad
            .process_gamepad_event_batch([GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::Low,
                state: ElementState::Pressed,
            }])
            .expect("depress focused Continue with gamepad Low");
        assert!(gamepad.game_over_dialog.is_some());
        gamepad
            .process_gamepad_event_batch([GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::Low,
                state: ElementState::Released,
            }])
            .expect("activate focused Continue with gamepad Low");
        assert!(gamepad.game_over_dialog.is_none());
    }

    #[test]
    fn game_over_arrows_and_space_never_activate_a_hovered_button() {
        let mut app = new_game_over_keyboard_app();
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        let mut continue_point = None;
        'find_button: for y in 0..height {
            for x in 0..width {
                let dialog = app.game_over_dialog.as_mut().expect("evaluation dialog");
                dialog.handle_pointer_move(x as f32, y as f32, width, height);
                if dialog.hovered_action() == Some(GameOverAction::Continue) {
                    continue_point = Some(PhysicalPosition::new(f64::from(x), f64::from(y)));
                    break 'find_button;
                }
            }
        }
        let continue_point = continue_point.expect("find the Continue button on the dialog");
        app.handle_cursor_moved(continue_point)
            .expect("hover Continue through the application input path");
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .and_then(GameOverState::hovered_action),
            Some(GameOverAction::Continue),
            "the fixture must distinguish pointer hover from initial keyboard focus"
        );

        for key in [
            VirtualKeyCode::Left,
            VirtualKeyCode::Right,
            VirtualKeyCode::Up,
            VirtualKeyCode::Down,
            VirtualKeyCode::Space,
        ] {
            for modifiers in [
                ModifiersState::empty(),
                ModifiersState::CTRL | ModifiersState::SHIFT,
                ModifiersState::CTRL | ModifiersState::ALT,
                ModifiersState::LOGO,
            ] {
                app.handle_modifiers_changed(modifiers)
                    .expect("set keyboard modifiers");
                app.handle_key(key, ElementState::Pressed)
                    .expect("unfocused game-over navigation key is a no-op");
                app.handle_key(key, ElementState::Released)
                    .expect("unfocused game-over navigation release is consumed");
                assert_eq!(
                    app.game_over_dialog
                        .as_ref()
                        .and_then(GameOverState::hovered_action),
                    Some(GameOverAction::Continue),
                    "{key:?} with {modifiers:?} must neither focus nor activate a hovered button"
                );
            }
        }
        assert!(matches!(app.mode, AppMode::Running));
    }

    fn hover_game_over_action_for_test(app: &mut GameApp, action: GameOverAction) {
        let (width, height) = {
            let surface = app.graphics.surface();
            (surface.width(), surface.height())
        };
        for y in 0..height {
            for x in 0..width {
                let dialog = app.game_over_dialog.as_mut().expect("evaluation dialog");
                dialog.handle_pointer_move(x as f32, y as f32, width, height);
                if dialog.hovered_action() == Some(action) {
                    return;
                }
            }
        }
        panic!("game-over action {action:?} has no pointer target");
    }

    fn assert_game_over_fixture_has_no_sound_activity(app: &GameApp) {
        if let Some(audio) = app.audio.as_ref() {
            assert!(!audio.options.sound_enabled);
            assert!(!audio.options.menu_sound_enabled);
            assert!(
                audio.active_channels.is_empty(),
                "game-over input must not synthesize a UI sound"
            );
        }
    }

    fn assert_only_gamepad_dirty_mark_changed(mut before: RuntimeGlobalUiSnapshot, app: &GameApp) {
        before.menu_render_version = before.menu_render_version.wrapping_add(1);
        assert_eq!(runtime_global_ui_snapshot(app), before);
        assert_game_over_fixture_has_no_sound_activity(app);
    }

    #[test]
    fn game_over_gui_stack_requires_enabled_primary_gamepad_source() {
        for (gamepad_gui_control, gamepad) in [(false, 0), (true, 1)] {
            let slot = GamepadSlot::new(gamepad as u8);
            let mut app = new_game_over_keyboard_app();
            app.gamepad_gui_control = gamepad_gui_control;
            hover_game_over_action_for_test(&mut app, GameOverAction::Continue);
            app.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Top overlay",
                    "Must remain untouched",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("open message over evaluation");
            let before = runtime_global_ui_snapshot(&app);
            let source = |cluster, event| SourcedGamepadEvent {
                gamepad,
                cluster,
                event,
            };
            let gate = app.gamepad_gui_control;

            app.process_sourced_gamepad_event_batch(
                [
                    source(
                        10,
                        GamepadEvent::GuiButton {
                            slot,
                            class: GuiButtonClass::Low,
                            state: ElementState::Pressed,
                        },
                    ),
                    source(
                        10,
                        GamepadEvent::Action {
                            slot,
                            action: GamepadActionType::Cancel,
                            state: ElementState::Pressed,
                        },
                    ),
                    source(
                        10,
                        GamepadEvent::Button {
                            slot,
                            button: LegacyGamepadButton::new(1),
                            state: ElementState::Pressed,
                        },
                    ),
                    source(
                        11,
                        GamepadEvent::GuiButton {
                            slot,
                            class: GuiButtonClass::Low,
                            state: ElementState::Released,
                        },
                    ),
                    source(
                        11,
                        GamepadEvent::Action {
                            slot,
                            action: GamepadActionType::Cancel,
                            state: ElementState::Released,
                        },
                    ),
                    source(
                        11,
                        GamepadEvent::Button {
                            slot,
                            button: LegacyGamepadButton::new(1),
                            state: ElementState::Released,
                        },
                    ),
                ],
                gate,
            )
            .expect("disabled or non-primary evaluation GUI input is inert");

            assert_only_gamepad_dirty_mark_changed(before, &app);
            assert_eq!(app.message_dialogs.len(), 1);
        }
    }

    #[test]
    fn closed_exclusive_message_alias_cluster_yields_later_direction_to_game_over() {
        let mut app = new_game_over_keyboard_app();
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Top overlay",
                "Close first",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::LeagueSurrender,
        )
        .expect("open message over evaluation");

        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::Cancel,
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Left,
                state: ElementState::Pressed,
            },
        ])
        .expect("a later raw direction begins a new receiver cluster");
        assert!(app.message_dialogs.is_empty());
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            Some(GameOverFocus::Button(2))
        );
    }

    #[test]
    fn context_fences_game_over_until_a_post_close_cluster() {
        let open_context = |app: &mut GameApp| {
            app.open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(20.0, 20.0),
            )
            .expect("open context over evaluation");
        };

        let mut axis_transition = new_game_over_keyboard_app();
        open_context(&mut axis_transition);
        axis_transition
            .process_sourced_gamepad_event_batch(
                [
                    SourcedGamepadEvent {
                        gamepad: 0,
                        cluster: 40,
                        event: GamepadEvent::Axis {
                            slot: GamepadSlot::new(0),
                            axis: LegacyGamepadAxis::new(0, false),
                            state: ElementState::Released,
                        },
                    },
                    SourcedGamepadEvent {
                        gamepad: 0,
                        cluster: 40,
                        event: GamepadEvent::Direction {
                            slot: GamepadSlot::new(0),
                            button: ControlButton::Left,
                            state: ElementState::Released,
                        },
                    },
                    SourcedGamepadEvent {
                        gamepad: 0,
                        cluster: 41,
                        event: GamepadEvent::Axis {
                            slot: GamepadSlot::new(0),
                            axis: LegacyGamepadAxis::new(0, true),
                            state: ElementState::Pressed,
                        },
                    },
                    SourcedGamepadEvent {
                        gamepad: 0,
                        cluster: 41,
                        event: GamepadEvent::Direction {
                            slot: GamepadSlot::new(0),
                            button: ControlButton::Right,
                            state: ElementState::Pressed,
                        },
                    },
                ],
                true,
            )
            .expect("axis release and opposite press re-resolve separate receivers");
        assert!(axis_transition.context_menu.is_some());
        assert!(axis_transition.game_over_dialog.is_some());
        assert_eq!(
            axis_transition
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            None
        );

        let mut pass_through = new_game_over_keyboard_app();
        open_context(&mut pass_through);
        pass_through
            .process_gamepad_event_batch([GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Left,
                state: ElementState::Pressed,
            }])
            .expect("root context Left remains fenced from Dialog traversal");
        assert!(pass_through.context_menu.is_some());
        assert_eq!(
            pass_through
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            None
        );

        let mut closed = new_game_over_keyboard_app();
        open_context(&mut closed);
        closed
            .process_gamepad_event_batch([
                GamepadEvent::GuiButton {
                    slot: GamepadSlot::new(0),
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Action {
                    slot: GamepadSlot::new(0),
                    action: GamepadActionType::Cancel,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
            ])
            .expect("a later raw cluster outlives the closed context menu");
        assert!(closed.context_menu.is_none());
        assert_eq!(
            closed
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            Some(GameOverFocus::Close)
        );
    }

    #[test]
    fn game_over_raw_low_opens_all_chat_for_south_and_east_aliases() {
        for (source, action, button) in [
            (
                "South",
                GamepadActionType::Select,
                LegacyGamepadButton::new(0),
            ),
            (
                "East",
                GamepadActionType::Cancel,
                LegacyGamepadButton::new(1),
            ),
        ] {
            let mut app = new_game_over_keyboard_app();
            hover_game_over_action_for_test(&mut app, GameOverAction::Continue);
            assert_game_over_fixture_has_no_sound_activity(&app);
            app.process_gamepad_event_batch([
                GamepadEvent::GuiButton {
                    slot: GamepadSlot::new(0),
                    class: GuiButtonClass::Low,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Action {
                    slot: GamepadSlot::new(0),
                    action,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Button {
                    slot: GamepadSlot::new(0),
                    button,
                    state: ElementState::Pressed,
                },
            ])
            .expect("raw Low opens the classic all-chat child");
            assert_eq!(app.running_chat_text(), Some(""));
            assert!(
                app.game_over_dialog.is_some(),
                "{source} is Low/chat even when its abstract alias is Cancel"
            );
            assert_game_over_fixture_has_no_sound_activity(&app);
        }
    }

    #[test]
    fn game_over_raw_left_and_right_reach_exact_focus_targets() {
        for (button, expected) in [
            (ControlButton::Left, GameOverFocus::Button(2)),
            (ControlButton::Right, GameOverFocus::Close),
        ] {
            let mut app = new_game_over_keyboard_app();
            hover_game_over_action_for_test(&mut app, GameOverAction::Continue);
            app.process_gamepad_event_batch([GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button,
                state: ElementState::Pressed,
            }])
            .expect("horizontal D-pad traverses native focus");
            assert_eq!(
                app.game_over_dialog
                    .as_ref()
                    .and_then(GameOverState::focused),
                Some(expected)
            );
        }
    }

    #[test]
    fn game_over_raw_vertical_releases_clear_and_abstract_aliases_are_inert() {
        let mut app = new_game_over_keyboard_app();
        hover_game_over_action_for_test(&mut app, GameOverAction::Continue);
        let before = runtime_global_ui_snapshot(&app);

        app.process_gamepad_event_batch([
            GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Up,
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Down,
                state: ElementState::Pressed,
            },
            GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Left,
                state: ElementState::Released,
            },
            GamepadEvent::Direction {
                slot: GamepadSlot::new(0),
                button: ControlButton::Right,
                state: ElementState::Released,
            },
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::Low,
                state: ElementState::Released,
            },
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Released,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::Select,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::Cancel,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::MenuToggle,
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(0),
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(0),
                state: ElementState::Released,
            },
        ])
        .expect("non-binding game-over controller events are consumed");
        assert_only_gamepad_dirty_mark_changed(before, &app);

        let clear_before = runtime_global_ui_snapshot(&app);
        app.process_gamepad_event_batch([GamepadEvent::Clear {
            slot: GamepadSlot::new(0),
        }])
        .expect("standalone Clear is inert while game over owns the screen");
        assert_only_gamepad_dirty_mark_changed(clear_before, &app);

        let direct_before = runtime_global_ui_snapshot(&app);
        for action in [
            GamepadActionType::Select,
            GamepadActionType::Cancel,
            GamepadActionType::MenuToggle,
        ] {
            app.handle_gamepad_action(GamepadSlot::new(0), action, ElementState::Pressed)
                .expect("abstract gamepad actions cannot activate evaluation buttons");
        }
        app.handle_gamepad_button(
            GamepadSlot::new(0),
            LegacyGamepadButton::new(0),
            ElementState::Pressed,
        )
        .expect("abstract commands are swallowed by game over");
        for button in [
            ControlButton::Left,
            ControlButton::Right,
            ControlButton::Up,
            ControlButton::Down,
        ] {
            app.handle_gamepad_direction(GamepadSlot::new(0), button, ElementState::Pressed)
                .expect("only the raw batch route owns game-over directions");
        }
        assert_eq!(runtime_global_ui_snapshot(&app), direct_before);
        assert_game_over_fixture_has_no_sound_activity(&app);

        let mut cancelled = new_game_over_keyboard_app();
        for _ in 0..4 {
            cancelled
                .process_gamepad_event_batch([GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                }])
                .expect("focus Continue before cancellation");
        }
        cancelled
            .process_gamepad_event_batch([GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::Low,
                state: ElementState::Pressed,
            }])
            .expect("depress focused Continue");
        assert_eq!(
            cancelled
                .ui_sound_log
                .iter()
                .filter(|sound| sound.as_str() == "ArrowHit")
                .count(),
            1
        );
        cancelled
            .process_gamepad_event_batch([GamepadEvent::Clear {
                slot: GamepadSlot::new(0),
            }])
            .expect("Clear cancels the retained button latch");
        cancelled
            .process_gamepad_event_batch([GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::Low,
                state: ElementState::Released,
            }])
            .expect("the post-Clear release cannot activate Continue");
        assert!(cancelled.game_over_dialog.is_some());
        assert!(!cancelled.ui_sound_log.iter().any(|sound| sound == "Click"));
    }

    #[test]
    fn game_over_raw_high_ends_and_consumes_aliases_after_dialog_close() {
        let mut app = new_game_over_keyboard_app();
        assert_game_over_fixture_has_no_sound_activity(&app);

        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::MenuToggle,
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(8),
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(9),
                state: ElementState::Pressed,
            },
            GamepadEvent::Clear {
                slot: GamepadSlot::new(0),
            },
        ])
        .expect("raw High ends the round and owns its contiguous aliases");

        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::MainMenu);
        assert!(app.game_over_dialog.is_none());
        assert!(app.ingame_menu.is_none());
        assert!(app.status_text.is_empty());
        assert!(
            !app.exit_requested,
            "the paired MenuToggle alias must not reach the exposed main menu"
        );
        assert_game_over_fixture_has_no_sound_activity(&app);
    }

    #[test]
    fn game_over_high_capture_ends_at_the_next_raw_physical_cluster() {
        let mut app = new_game_over_keyboard_app();

        let source = |cluster, event| SourcedGamepadEvent {
            gamepad: 0,
            cluster,
            event,
        };
        let activate_network_game = |cluster: u64| {
            [
                // D-pad Down moves focus from Start Game to Start Network Game.
                source(
                    cluster,
                    GamepadEvent::Direction {
                        slot: GamepadSlot::new(0),
                        button: ControlButton::Down,
                        state: ElementState::Pressed,
                    },
                ),
                // A new South cluster presses and releases the focused button.
                source(
                    cluster + 1,
                    GamepadEvent::GuiButton {
                        slot: GamepadSlot::new(0),
                        class: GuiButtonClass::Low,
                        state: ElementState::Pressed,
                    },
                ),
                source(
                    cluster + 1,
                    GamepadEvent::Action {
                        slot: GamepadSlot::new(0),
                        action: GamepadActionType::Select,
                        state: ElementState::Pressed,
                    },
                ),
                source(
                    cluster + 1,
                    GamepadEvent::Button {
                        slot: GamepadSlot::new(0),
                        button: LegacyGamepadButton::new(0),
                        state: ElementState::Pressed,
                    },
                ),
                source(
                    cluster + 2,
                    GamepadEvent::GuiButton {
                        slot: GamepadSlot::new(0),
                        class: GuiButtonClass::Low,
                        state: ElementState::Released,
                    },
                ),
                source(
                    cluster + 2,
                    GamepadEvent::Action {
                        slot: GamepadSlot::new(0),
                        action: GamepadActionType::Select,
                        state: ElementState::Released,
                    },
                ),
                source(
                    cluster + 2,
                    GamepadEvent::Button {
                        slot: GamepadSlot::new(0),
                        button: LegacyGamepadButton::new(0),
                        state: ElementState::Released,
                    },
                ),
            ]
        };
        app.process_sourced_gamepad_event_batch(
            [
                // Select: High plus MenuToggle. The first alias is owned by the
                // evaluation dialog even though High immediately returns to the
                // main screen.
                source(
                    20,
                    GamepadEvent::GuiButton {
                        slot: GamepadSlot::new(0),
                        class: GuiButtonClass::High,
                        state: ElementState::Pressed,
                    },
                ),
                source(
                    20,
                    GamepadEvent::Action {
                        slot: GamepadSlot::new(0),
                        action: GamepadActionType::MenuToggle,
                        state: ElementState::Pressed,
                    },
                ),
                source(
                    20,
                    GamepadEvent::Clear {
                        slot: GamepadSlot::new(0),
                    },
                ),
            ]
            .into_iter()
            // Capture ends at the next physical cluster, but the newly started
            // startup fade must suppress every later cluster in this batch.
            .chain(activate_network_game(21)),
            true,
        )
        .expect("raw High returns to startup and the fade owns later clusters");

        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::MainMenu);
        assert!(app.startup_dialog_fade_active());
        let mut frame = vec![0_u8; 320 * 200 * 4];
        for _ in 0..STARTUP_DIALOG_FADE_STEPS {
            app.render(&mut frame)
                .expect("complete the post-round startup fade");
        }
        assert!(!app.startup_dialog_fade_active());

        app.process_sourced_gamepad_event_batch(activate_network_game(24), true)
            .expect("later physical clusters route to the newly exposed main menu");

        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::NetworkGame);
        assert!(!app.exit_requested);
        assert!(app.game_over_dialog.is_none());
    }

    #[test]
    fn exclusive_message_dialog_raw_gamepad_clusters_precede_game_over() {
        let mut app = new_game_over_keyboard_app();
        hover_game_over_action_for_test(&mut app, GameOverAction::Continue);
        let open_message = |app: &mut GameApp, caption: &str| {
            app.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    caption,
                    "Top overlay",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::LeagueSurrender,
            )
            .expect("open message above evaluation");
        };

        open_message(&mut app, "Low");
        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::Low,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::Select,
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(0),
                state: ElementState::Pressed,
            },
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::Low,
                state: ElementState::Released,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::Select,
                state: ElementState::Released,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(0),
                state: ElementState::Released,
            },
        ])
        .expect("message Low activates the top overlay, not game-over chat");
        assert!(app.message_dialogs.is_empty());

        open_message(&mut app, "High");
        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::Cancel,
                state: ElementState::Pressed,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(1),
                state: ElementState::Pressed,
            },
            GamepadEvent::GuiButton {
                slot: GamepadSlot::new(0),
                class: GuiButtonClass::High,
                state: ElementState::Released,
            },
            GamepadEvent::Action {
                slot: GamepadSlot::new(0),
                action: GamepadActionType::Cancel,
                state: ElementState::Released,
            },
            GamepadEvent::Button {
                slot: GamepadSlot::new(0),
                button: LegacyGamepadButton::new(1),
                state: ElementState::Released,
            },
        ])
        .expect("message High closes only the top overlay");

        assert!(app.message_dialogs.is_empty());
        assert_eq!(app.mode, AppMode::Running);
        assert_eq!(
            app.game_over_dialog
                .as_ref()
                .and_then(GameOverState::hovered_action),
            None,
            "closing the top modal clears pointer hover without closing evaluation"
        );
        assert!(app.status_text.is_empty());
        assert_game_over_fixture_has_no_sound_activity(&app);
    }

    #[test]
    fn game_over_tab_and_escape_use_exact_modifier_masks() {
        for (modifiers, expected) in [
            (ModifiersState::empty(), GameOverFocus::Close),
            (ModifiersState::SHIFT, GameOverFocus::Button(2)),
            (ModifiersState::LOGO, GameOverFocus::Close),
            (
                ModifiersState::LOGO | ModifiersState::SHIFT,
                GameOverFocus::Button(2),
            ),
        ] {
            let mut app = new_game_over_keyboard_app();
            app.handle_modifiers_changed(modifiers)
                .expect("set focus modifiers");
            app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .expect("traverse native evaluation focus");
            app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("release evaluation traversal key");
            assert_eq!(
                app.game_over_dialog
                    .as_ref()
                    .and_then(GameOverState::focused),
                Some(expected)
            );
        }
        for modifiers in [
            ModifiersState::CTRL,
            ModifiersState::CTRL | ModifiersState::ALT,
            ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::ALT | ModifiersState::SHIFT,
        ] {
            let mut app = new_game_over_keyboard_app();
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .expect("other-modified Tab has no exact C++ focus binding");
            app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("other-modified Tab release is consumed");
            assert_eq!(
                app.game_over_dialog
                    .as_ref()
                    .and_then(GameOverState::focused),
                None
            );
        }

        for modifiers in [
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::CTRL | ModifiersState::ALT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
        ] {
            let mut app = new_game_over_keyboard_app();
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
                .expect("game-over releases are inert");
            app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
                .expect("modified Escape has no exact C++ End binding");
            assert!(app.game_over_dialog.is_some());
        }

        for modifiers in [ModifiersState::empty(), ModifiersState::LOGO] {
            let mut ending_app = new_game_over_keyboard_app();
            ending_app
                .handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            ending_app
                .handle_key(VirtualKeyCode::Escape, ElementState::Released)
                .expect("Escape release cannot end evaluation");
            assert!(ending_app.game_over_dialog.is_some());
            ending_app
                .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
                .expect("bare Escape invokes End");
            assert!(ending_app.game_over_dialog.is_none());
            assert!(matches!(ending_app.mode, AppMode::Menu));
        }
    }

    #[test]
    fn game_over_pending_network_result_preserves_cpp_button_and_escape_latches() {
        let pending_host = || {
            let mut app = new_classic_running_sandbox_app();
            configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
            app.network_is_league = true;
            app.handle_game_over()
                .expect("show pending host evaluation");
            app
        };

        let mut host = pending_host();
        let dialog = host.game_over_dialog.as_ref().expect("host evaluation");
        assert_eq!(dialog.network_result_label(), Some(""));
        assert!(!dialog.is_net_done());
        assert!(!dialog.allows_escape_close());
        assert!(dialog.actions().contains(&GameOverAction::End));
        assert!(dialog.actions().contains(&GameOverAction::Continue));
        host.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("pending host Escape is consumed");
        assert!(host.game_over_dialog.is_some());
        host.handle_game_over_gamepad_event(GamepadEvent::GuiButton {
            slot: GamepadSlot::new(0),
            class: GuiButtonClass::High,
            state: ElementState::Pressed,
        })
        .expect("pending host High is consumed");
        assert!(host.game_over_dialog.is_some());

        let mut clickable = pending_host();
        clickable
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("enable native mnemonic mask");
        clickable
            .handle_key(VirtualKeyCode::C, ElementState::Pressed)
            .expect("visible pending Continue remains clickable like C++");
        assert!(clickable.game_over_dialog.is_none());
        assert_eq!(clickable.mode, AppMode::Running);

        let mut resolved = pending_host();
        resolved.snapshot.round_results.network_result =
            Some(clonk_engine::RoundResultsNetworkResult::LeagueOk);
        resolved.snapshot.round_results.network_result_message = b"evaluated".to_vec();
        assert!(resolved.sec1_timer().expect("refresh final network result"));
        let dialog = resolved
            .game_over_dialog
            .as_ref()
            .expect("resolved host evaluation");
        assert_eq!(dialog.network_result_label(), Some("evaluated"));
        assert!(dialog.is_net_done());
        assert!(dialog.allows_escape_close());
        resolved
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("resolved host Escape ends the round");
        assert!(resolved.game_over_dialog.is_none());

        let mut client = new_classic_running_sandbox_app();
        configure_runtime_network_role(&mut client, RuntimeNetworkRole::Client);
        client.network_is_league = true;
        client
            .handle_game_over()
            .expect("show pending client evaluation");
        assert!(client
            .game_over_dialog
            .as_ref()
            .is_some_and(GameOverState::allows_escape_close));
        client
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("pending client Escape is allowed");
        assert!(client.game_over_dialog.is_none());
    }

    #[test]
    fn game_over_show_and_continue_use_offline_pause_lifecycle() {
        // C4GameOverDlg::OnShown invokes Game.Pause, while only an accepted
        // Continue close invokes Game.Unpause. Raw dialog teardown does not
        // resume the round (src/C4GameOverDlg.cpp:349-381;
        // src/C4Game.cpp:1045-1084).
        let mut app = new_classic_running_sandbox_app();
        assert_eq!(app.offline_halt_count, 0);

        app.handle_game_over().expect("show offline evaluation");
        assert_eq!(
            app.offline_halt_count, 1,
            "OnShown acquires the native offline game halt"
        );
        app.handle_game_over_action(GameOverAction::Continue)
            .expect("continue the evaluated round");
        assert_eq!(app.offline_halt_count, 0);
        assert!(app.game_over_dialog.is_none());

        let mut raw_teardown = new_classic_running_sandbox_app();
        raw_teardown
            .handle_game_over()
            .expect("show evaluation before raw teardown");
        raw_teardown.dismiss_game_over_dialog();
        assert_eq!(
            raw_teardown.offline_halt_count, 1,
            "destroying the dialog without Continue must not call Unpause"
        );
    }

    #[test]
    fn game_over_network_pause_lifecycle_is_host_authoritative() {
        // Evaluated network games skip league voting and preserve the ordinary
        // host-only Pause/Start authority: OnShown requests GS_Pause and
        // Continue requests GS_Go on the host, while both calls are consumed
        // no-ops on clients (src/C4Game.cpp:1045-1084;
        // src/C4Network2.cpp:527-541).
        let mut host = new_classic_running_sandbox_app();
        let (_events, mut host_commands) = install_running_network_stub(&mut host, 0, 0, 2);

        host.handle_game_over().expect("show host evaluation");
        let pause_changes = host_commands
            .take_runtime_status_commands()
            .into_iter()
            .filter_map(|command| match command {
                network::TestRuntimeStatusCommand::Change(status) => Some(status),
                network::TestRuntimeStatusCommand::Reached { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(pause_changes.len(), 1);
        assert_eq!(pause_changes[0].state, clonk_network::NETWORK_STATE_PAUSE);

        host.handle_game_over_action(GameOverAction::Continue)
            .expect("continue host evaluation");
        let go_changes = host_commands
            .take_runtime_status_commands()
            .into_iter()
            .filter_map(|command| match command {
                network::TestRuntimeStatusCommand::Change(status) => Some(status),
                network::TestRuntimeStatusCommand::Reached { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(go_changes.len(), 1);
        assert_eq!(go_changes[0].state, clonk_network::NETWORK_STATE_GO);

        let mut client = new_classic_running_sandbox_app();
        let (_events, mut client_commands) = install_running_network_stub(&mut client, 7, 0, 2);
        client.handle_game_over().expect("show client evaluation");
        assert!(client_commands.take_runtime_status_commands().is_empty());

        // Model the host's committed Pause. Closing the local dialog must not
        // let a client resume synchronized control independently.
        client.network_control_running = false;
        client
            .handle_game_over_action(GameOverAction::Continue)
            .expect("close client evaluation");
        assert!(client_commands.take_runtime_status_commands().is_empty());
        assert!(!client.network_control_running);
    }

    #[test]
    fn runtime_f3_obeys_player_modifier_game_over_and_key_config_priority() {
        let mut player = new_running_sandbox_app();
        player
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        player
            .engine
            .player_mut(player.local_owner)
            .expect("local player")
            .control
            .control_style = true;
        player
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("PRIO_PlrControl owns bare F3");
        assert!(player.runtime_flash_message.is_none());
        assert_ne!(
            player
                .engine
                .player(player.local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_LEFT),
            0
        );

        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL,
            ModifiersState::CTRL | ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
        ] {
            let mut modified = new_running_sandbox_app();
            modified
                .bindings
                .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
            modified
                .handle_modifiers_changed(modifiers)
                .expect("set F3 modifiers");
            let mut before = runtime_global_ui_snapshot(&modified);
            modified
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("modified F3 falls through without player dispatch");
            modified
                .handle_key(VirtualKeyCode::F3, ElementState::Released)
                .expect("modified F3 release falls through");
            before.menu_render_version = before.menu_render_version.wrapping_add(2);
            assert_eq!(runtime_global_ui_snapshot(&modified), before);
        }

        let mut logo_music = new_running_sandbox_app();
        let configured_before_logo = logo_music
            .audio
            .as_ref()
            .expect("test audio")
            .options
            .music_enabled;
        logo_music
            .handle_modifiers_changed(ModifiersState::LOGO)
            .expect("set Logo");
        logo_music
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("Logo is absent from C4KeyCodeEx modifier masks");
        assert!(logo_music.runtime_flash_message.is_some());
        assert_eq!(
            logo_music
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .music_enabled,
            configured_before_logo
        );

        let mut logo_sound = new_running_sandbox_app();
        let sound_before_logo = logo_sound
            .audio
            .as_ref()
            .expect("test audio")
            .options
            .sound_enabled;
        logo_sound
            .handle_modifiers_changed(ModifiersState::CTRL | ModifiersState::LOGO)
            .expect("set Ctrl+Logo");
        logo_sound
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("Ctrl+Logo retains exact Ctrl+F3");
        assert_eq!(
            logo_sound
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .sound_enabled,
            !sound_before_logo
        );
        assert!(logo_sound.runtime_flash_message.is_none());

        let mut sound = new_running_sandbox_app();
        let before_sound = sound
            .audio
            .as_ref()
            .map(|audio| audio.options.sound_enabled);
        sound
            .handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set Ctrl+F3");
        sound
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("Ctrl+F3 uses SoundToggle, not flash");
        assert!(sound.runtime_flash_message.is_none());
        if let (Some(before), Some(audio)) = (before_sound, sound.audio.as_ref()) {
            assert_eq!(audio.options.sound_enabled, !before);
        }

        let mut existing_sound = new_running_sandbox_app();
        let audio = existing_sound.audio.as_mut().expect("test audio");
        let handle = audio
            .system
            .load_sound(&silent_pcm_wav(1_000))
            .expect("test sound handle");
        let duration_ms = handle.duration_ms().expect("test sound duration");
        audio.active_channels.insert(
            SoundInstanceKey::new("Loop", None),
            ChannelInfo {
                channel: Some(ChannelId(999, 1)),
                handle,
                duration_ms,
                sample_key: "loop".to_string(),
                sample_name: "loop.wav".to_string(),
                sample_order: 0,
                instance_order: 1,
                looped: true,
                target: None,
                volume: 100,
                custom_falloff: None,
                started_at: Instant::now(),
                detached_mix: None,
            },
        );
        existing_sound
            .handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set Ctrl+F3");
        existing_sound
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("disable future effects only");
        assert!(
            existing_sound
                .audio
                .as_ref()
                .expect("test audio")
                .active_channels
                .contains_key(&SoundInstanceKey::new("Loop", None)),
            "C4SoundSystem::ToggleOnOff does not halt existing instances"
        );

        let mut game_over = new_game_over_keyboard_app();
        game_over
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        game_over
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("exclusive GUI suppresses Control but retains Generic music");
        assert!(game_over.runtime_flash_message.is_some());

        let mut custom = new_running_sandbox_app();
        custom.runtime_key_config_cache = OnceLock::new();
        custom
            .runtime_key_config_cache
            .set(Err("Extra.c4g/KeyConfig.txt override".to_string()))
            .expect("empty key-config cache");
        for state in [ElementState::Pressed, ElementState::Released] {
            let error = custom
                .handle_key(VirtualKeyCode::F3, state)
                .expect_err("custom global F3 ownership must fail closed");
            assert!(error.to_string().contains("timed flash-message resources"));
            assert!(custom.runtime_flash_message.is_none());
        }
    }

    #[test]
    fn older_runtime_f4_dialog_renders_inactive_below_new_game_over_dialog() {
        let mut app = new_classic_running_sandbox_app();
        configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
        app.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("open F4 before evaluation");
        assert!(app.runtime_client_list_mouse_active());
        assert!(!app.runtime_client_list_keyboard_active());
        assert!(app.runtime_client_list_draw_active());

        app.handle_game_over()
            .expect("show newer game-over dialog above F4");
        assert!(app.runtime_client_list.is_some());
        assert!(app.game_over_dialog.is_some());
        assert!(!app.runtime_client_list_above_game_over);
        assert!(!app.runtime_client_list_mouse_active());
        assert!(!app.runtime_client_list_keyboard_active());
        assert!(!app.runtime_client_list_draw_active());
    }

    #[test]
    fn runtime_f4_precedes_game_over_message_and_ingame_menus() {
        let mut game_over = new_game_over_keyboard_app();
        let (_events, mut game_over_commands) = install_running_network_stub(&mut game_over, 0, 40, 4);
        route_primary_gamepad_to_local_owner(&mut game_over);
        game_over
            .handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("F4 opens above the older equal-z game-over dialog");
        assert!(game_over.runtime_client_list.is_some());
        assert!(game_over.game_over_dialog.is_some());
        assert!(game_over.runtime_client_list_above_game_over);
        game_over
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("client list keeps evaluation traversal inactive");
        game_over
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("client list owns the traversal release");
        game_over
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt over the client list");
        game_over
            .handle_key(VirtualKeyCode::R, ElementState::Pressed)
            .expect("client list keeps the Restart mnemonic inactive");
        game_over
            .handle_key(VirtualKeyCode::R, ElementState::Released)
            .expect("client list owns the mnemonic release");
        game_over
            .handle_modifiers_changed(ModifiersState::empty())
            .expect("release Alt");
        game_over
            .process_gamepad_event_batch([
                GamepadEvent::Axis {
                    slot: GamepadSlot::new(0),
                    axis: LegacyGamepadAxis::new(0, true),
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
                GamepadEvent::GuiButton {
                    slot: GamepadSlot::new(0),
                    class: GuiButtonClass::Low,
                    state: ElementState::Pressed,
                },
            ])
            .expect("client-list gamepad input cannot reach evaluation");
        assert_eq!(
            game_over
                .game_over_dialog
                .as_ref()
                .and_then(GameOverState::focused),
            None
        );
        let submitted = game_over_commands.take_submitted_local();
        assert_eq!(submitted.len(), 1);
        assert!(matches!(
            submitted[0].1,
            ControlEvent::Press(ControlButton::Right)
        ));
        assert!(game_over.running_chat_text().is_none());
        assert!(game_over.runtime_client_list.is_some());
        game_over
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("active client-list Escape closes only the client list");
        assert!(game_over.runtime_client_list.is_none());
        assert!(game_over.game_over_dialog.is_some());

        let mut message = new_running_sandbox_app();
        configure_runtime_network_role(&mut message, RuntimeNetworkRole::Host);
        message
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Network",
                    "Modal remains open",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("push running modal");
        message
            .handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("F4 opens below existing message dialog");
        assert!(message.runtime_client_list.is_some());
        assert_eq!(message.message_dialogs.len(), 1);

        let mut ingame = new_running_sandbox_app();
        configure_runtime_network_role(&mut ingame, RuntimeNetworkRole::Host);
        ingame.open_ingame_menu().expect("open in-game menu");
        ingame
            .handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("F4 opens over in-game menu");
        assert!(ingame.runtime_client_list.is_some());
        assert!(ingame.ingame_menu.is_some());
    }

    #[test]
    fn runtime_pause_is_game_over_noop_but_precedes_other_running_dialogs() {
        let mut game_over = new_game_over_keyboard_app();
        game_over
            .handle_modifiers_changed(ModifiersState::LOGO)
            .expect("set keyboard modifiers");
        let before_game_over = runtime_global_ui_snapshot(&game_over);
        for state in [
            ElementState::Pressed,
            ElementState::Pressed,
            ElementState::Released,
        ] {
            game_over
                .handle_key(VirtualKeyCode::Pause, state)
                .expect("C4 disables Pause throughout round evaluation");
            assert_eq!(runtime_global_ui_snapshot(&game_over), before_game_over);
            assert_eq!(
                game_over.offline_halt_count, 1,
                "the Pause key cannot release OnShown's evaluation halt"
            );
        }

        let mut message = new_running_sandbox_app();
        message
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Pause",
                    "Modal remains open",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .expect("push running modal");
        message
            .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("Pause precedes an ordinary running modal");
        assert_ne!(message.offline_halt_count, 0);
        assert_eq!(message.message_dialogs.len(), 1);

        let mut ingame = new_running_sandbox_app();
        ingame.open_ingame_menu().expect("open in-game menu");
        ingame
            .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("Pause precedes the fullscreen in-game menu");
        assert_ne!(ingame.offline_halt_count, 0);
        assert!(ingame.ingame_menu.is_some());
    }

    #[test]
    fn modified_runtime_globals_retain_higher_priority_game_over_mnemonics() {
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::LOGO | ModifiersState::ALT,
        ] {
            let mut app = new_game_over_keyboard_app();
            app.handle_modifiers_changed(modifiers)
                .expect("set mnemonic modifiers");
            app.handle_key(VirtualKeyCode::C, ElementState::Pressed)
                .expect("Continue mnemonic precedes the runtime Alt+C owner");
            assert!(app.game_over_dialog.is_none());
            assert_eq!(app.mode, AppMode::Running);
            assert!(app.running_chat_text().is_none());
        }

        for key in [
            VirtualKeyCode::F1,
            VirtualKeyCode::F4,
            VirtualKeyCode::Pause,
        ] {
            for modifiers in [
                ModifiersState::ALT,
                ModifiersState::LOGO | ModifiersState::ALT,
            ] {
                let mut app = new_game_over_keyboard_app();
                app.handle_modifiers_changed(modifiers)
                    .expect("set unmatched mnemonic modifiers");
                app.handle_key(key, ElementState::Pressed)
                    .expect("exclusive evaluation swallows lower runtime globals");
                assert!(app.game_over_dialog.is_some());
                assert!(app.running_chat_text().is_none());
                assert!(!app.runtime_help_visible);
                assert!(app.runtime_client_list.is_none());
            }
        }
    }

    #[test]
    fn l002_abort_confirmation_declines_confirms_and_restarts() {
        let mut declined = new_running_sandbox_app();
        declined.update().expect("advance round before declining");
        let declined_frame = declined.engine.frame();
        assert!(declined_frame > 0);
        let declined_scenario = declined
            .active_scenario
            .as_ref()
            .expect("active sandbox scenario")
            .identifier
            .clone();
        declined
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("open abort confirmation for decline");
        finish_abort_dialog(
            &mut declined,
            clonk_frontend::message_dialog::MessageDialogResult::No,
        );
        assert!(declined.ingame_menu.is_none());
        assert!(declined.message_dialogs.is_empty());
        assert!(matches!(declined.mode, AppMode::Running));
        assert_eq!(
            declined
                .active_scenario
                .as_ref()
                .map(|active| active.identifier.as_str()),
            Some(declined_scenario.as_str())
        );
        assert_eq!(declined.engine.frame(), declined_frame);

        let mut confirmed = new_running_sandbox_app();
        confirmed
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("open abort confirmation for Yes");
        finish_abort_dialog(
            &mut confirmed,
            clonk_frontend::message_dialog::MessageDialogResult::Yes,
        );
        assert!(matches!(confirmed.mode, AppMode::Menu));
        assert!(confirmed.active_scenario.is_none());
        assert!(confirmed.ingame_menu.is_none());

        let mut restarted = new_running_sandbox_app();
        restarted.update().expect("advance round before restarting");
        assert!(restarted.engine.frame() > 0);
        let scenario = restarted
            .active_scenario
            .as_ref()
            .expect("active sandbox scenario")
            .identifier
            .clone();
        restarted
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("open abort confirmation for restart");
        finish_abort_dialog(
            &mut restarted,
            clonk_frontend::message_dialog::MessageDialogResult::Restart,
        );
        wait_for_running(&mut restarted);
        assert_eq!(
            restarted
                .active_scenario
                .as_ref()
                .map(|active| active.identifier.as_str()),
            Some(scenario.as_str())
        );
        assert_eq!(restarted.engine.frame(), 0);
        assert!(restarted.ingame_menu.is_none());
        assert!(restarted.message_dialogs.is_empty());
    }

    #[test]
    fn l002_restart_is_control_host_only_and_game_over_suppresses_abort() {
        let mut client = new_running_sandbox_app();
        client.engine.set_control_host(false);
        client
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("open client abort confirmation");
        let client_dialog = client.message_dialogs.last().expect("client abort dialog");
        assert_eq!(
            client_dialog.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
        );
        assert_eq!(
            client_dialog.state.size(),
            clonk_frontend::message_dialog::MessageDialogSize::Small
        );

        let mut film_client = new_running_sandbox_app();
        film_client.engine.set_control_host(false);
        set_test_scenario_head_flags(&mut film_client, 0, 2);
        let (_film_events, _film_commands) = install_running_network_stub(&mut film_client, 7, 0, 1);
        film_client
            .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("Film2 client opens abort confirmation");
        let film_dialog = film_client
            .message_dialogs
            .last()
            .expect("Film2 abort dialog");
        assert_eq!(
            film_dialog.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::YES_RESTART_NO
        );
        assert_eq!(
            film_dialog.state.size(),
            clonk_frontend::message_dialog::MessageDialogSize::Fixed(400)
        );
        film_client.loader_render_error = Some("test restart blocker".to_string());
        finish_abort_dialog(
            &mut film_client,
            clonk_frontend::message_dialog::MessageDialogResult::Restart,
        );
        assert!(!film_client.abort_restart_pending);
        assert_eq!(
            film_client.scenario_selector_mode,
            ScenarioSelectorMode::NetworkHost,
            "C++ preserves NetworkActive for a Film2 client's NextMission"
        );

        let mut game_over = new_game_over_keyboard_app();
        game_over
            .apply_ingame_menu_action(MenuAction::Abort)
            .expect("suppressed abort request is non-fatal");
        assert!(game_over.game_over_dialog.is_some());
        assert!(game_over.message_dialogs.is_empty());
        assert!(matches!(game_over.mode, AppMode::Running));
    }

    #[test]
    fn modified_escape_does_not_match_the_abort_binding() {
        let mut app = new_running_sandbox_app();
        app.status_text.clear();
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::CTRL,
            ModifiersState::SHIFT,
            ModifiersState::ALT | ModifiersState::CTRL | ModifiersState::SHIFT,
        ] {
            app.handle_modifiers_changed(modifiers)
                .expect("set keyboard modifiers");
            app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
                .expect("modified Escape has no default C++ binding");
            app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
                .expect("release modified Escape");
            assert!(app.ingame_menu.is_none());
            assert!(app.object_menu.is_none());
            assert!(app.status_text.is_empty());
        }
        app.handle_modifiers_changed(ModifiersState::LOGO)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("Logo is outside C++'s Alt/Ctrl/Shift modifier mask");
        assert!(app.message_dialogs.last().is_some_and(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::AbortGame { .. }
        )));
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
    }

    #[test]
    fn host_vote_timeout_is_strict_and_restarts_on_the_oldest_subject() {
        // The host rejects the first stored vote only when wall time is
        // strictly greater than iVoteStartTime + 10, then immediately resets
        // iVoteStartTime while the synchronized VoteEnd is pending
        // (src/C4Network2.cpp:723-731; src/C4Network2.h:69-72).
        let kick = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 2,
        };
        let cancel = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_CANCEL,
            approve: true,
            data: 0,
            by_client: 3,
        };
        let mut votes = LeagueVoteState::default();
        votes.add_at(kick, 100);
        votes.add_at(cancel, 105);

        assert_eq!(votes.take_timed_out_subject_at(110), None);
        assert_eq!(
            votes.take_timed_out_subject_at(111),
            Some(LeagueVoteSubject::from(kick))
        );
        assert_eq!(votes.take_timed_out_subject_at(121), None);
        assert_eq!(
            votes.take_timed_out_subject_at(122),
            Some(LeagueVoteSubject::from(kick))
        );
    }

    #[test]
    fn ending_vote_restarts_timeout_for_the_next_subject() {
        // EndVote resets iVoteStartTime even when another subject remains in
        // Votes, so that subject gets a fresh strict ten-second window
        // (src/C4Network2.cpp:2888-2903).
        let kick = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 2,
        };
        let cancel = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_CANCEL,
            approve: true,
            data: 0,
            by_client: 3,
        };
        let mut votes = LeagueVoteState::default();
        votes.add_at(kick, 100);
        votes.add_at(cancel, 105);

        assert_eq!(
            votes.end_at(LeagueVoteSubject::from(kick), false, None, 106),
            Some(2)
        );
        assert_eq!(votes.take_timed_out_subject_at(116), None);
        assert_eq!(
            votes.take_timed_out_subject_at(117),
            Some(LeagueVoteSubject::from(cancel))
        );
    }

    #[test]
    fn next_mission_action_launches_the_catalog_target() {
        // C4GameOverDlg's Next button passes Game.NextMission through
        // C4Application::SetNextMission/QuitGame and starts that scenario
        // (C4GameOverDlg.cpp:335-382; C4Application.cpp:373-399).
        let fixture = tempdir().expect("next-mission fixture");
        let user_data = tempdir().expect("isolated user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        configure_test_startup_participant(&paths, user_data.path());
        let mut app = new_menu_app_with_paths(320, 200, &paths);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start sandbox scenario");
        wait_for_running(&mut app);
        let target_path = fixture.path().join("Tutorial02.c4s");
        let carried_definition = fixture.path().join("Carry.c4d");
        fs::create_dir_all(&target_path).expect("target scenario");
        fs::create_dir_all(&carried_definition).expect("carried definition");
        fs::write(
            target_path.join("Scenario.txt"),
            "[Head]\nTitle=The First Hut\n",
        )
        .expect("target Scenario.txt");
        fs::write(
            carried_definition.join("DefCore.txt"),
            "[DefCore]\nid=CARY\nName=Carry\nCategory=1\n",
        )
        .expect("carried DefCore.txt");
        fs::write(carried_definition.join("Script.c"), "// carried\n").expect("carried Script.c");
        write_test_definition_graphics(&carried_definition);
        let mut target = FrontendScenario::fallback();
        target.identifier = "Tutorial.c4f/Tutorial02.c4s".to_string();
        target.title = "The First Hut".to_string();
        target.path = Some(target_path);
        app.scenario_catalog
            .insert(target.identifier.clone(), target.clone());
        app.active_definition_load = Some(ScenarioDefinitionLoad::Fixed {
            modules: vec![carried_definition.to_string_lossy().into_owned()],
            definition_root: None,
        });

        let mut state = app.engine.capture_state();
        state.next_mission = clonk_engine::NextMissionState {
            path: "Tutorial.c4f\\Tutorial02.c4s".to_string(),
            text: "Next tutorial".to_string(),
            description: "Continue learning".to_string(),
        };
        app.engine.restore_state(&state).expect("state restores");

        app.handle_game_over_action(GameOverAction::NextMission)
            .expect("next mission starts");
        wait_for_running(&mut app);

        assert_eq!(
            app.active_scenario
                .as_ref()
                .map(|scenario| scenario.identifier.as_str()),
            Some("Tutorial.c4f/Tutorial02.c4s")
        );
        assert!(matches!(
            app.active_definition_load.as_ref(),
            Some(ScenarioDefinitionLoad::Fixed {
                modules,
                definition_root: None,
            }) if modules == &[carried_definition.to_string_lossy().as_ref()]
        ));
        assert!(app.game_over_dialog.is_none());
    }

    #[test]
    fn game_over_restart_and_next_mission_follow_control_host_film_policy() {
        // C4GameOverDlg admits these two controls only for the control host
        // or exact cinematic Film 2. Ordinary Film 1 is intentionally not an
        // override, and 1280 is the first width that retains Restart beside a
        // configured Next Mission (C4GameOverDlg.cpp:115-142,232-258).
        let mut app = new_classic_running_sandbox_app();
        let mut state = app.engine.capture_state();
        state.next_mission = clonk_engine::NextMissionState {
            path: "Tutorial.c4f\\Tutorial02.c4s".into(),
            text: "Next tutorial".into(),
            description: "Continue learning".into(),
        };
        app.engine
            .restore_state(&state)
            .expect("restore next mission");

        for (control_host, film, width, expected) in [
            (
                false,
                0,
                1280,
                vec![GameOverAction::End, GameOverAction::Continue],
            ),
            (
                false,
                1,
                1280,
                vec![GameOverAction::End, GameOverAction::Continue],
            ),
            (
                true,
                0,
                1279,
                vec![
                    GameOverAction::End,
                    GameOverAction::Continue,
                    GameOverAction::NextMission,
                ],
            ),
            (
                true,
                0,
                1280,
                vec![
                    GameOverAction::End,
                    GameOverAction::Continue,
                    GameOverAction::Restart,
                    GameOverAction::NextMission,
                ],
            ),
            (
                false,
                2,
                1279,
                vec![
                    GameOverAction::End,
                    GameOverAction::Continue,
                    GameOverAction::NextMission,
                ],
            ),
            (
                false,
                2,
                1280,
                vec![
                    GameOverAction::End,
                    GameOverAction::Continue,
                    GameOverAction::Restart,
                    GameOverAction::NextMission,
                ],
            ),
        ] {
            app.dismiss_game_over_dialog();
            app.resize(width, 720).expect("resize evaluation fixture");
            set_test_scenario_head_flags(&mut app, 0, film);
            app.engine.set_control_host(control_host);
            app.finish_game_over_after_league()
                .expect("construct evaluation dialog");
            assert_eq!(
                app.game_over_dialog
                    .as_ref()
                    .expect("evaluation dialog")
                    .actions(),
                expected,
                "control_host={control_host}, Film={film}, width={width}"
            );
        }
    }

    #[test]
    fn evaluation_dialog_joins_frozen_results_by_player_info_id() {
        // C4GameOverDlg consumes C4RoundResults goals/player records, and
        // C4PlayerInfoListBox joins each record through C4PlayerInfo::GetID,
        // never C4Player::Number (C4GameOverDlg.cpp:145-220;
        // C4PlayerInfoListBox.cpp:132-143,344-425,1529-1592).
        let mut snapshot = make_snapshot(Vec::new(), Vec::new());
        snapshot.players = vec![
            PlayerState {
                id: 99,
                player_info_id: 41,
                name: "Player".into(),
                status: PlayerStatus::Eliminated,
                won: true,
                color: Some(RgbColor::new(0xe8, 0, 0)),
                ..PlayerState::default()
            },
            PlayerState {
                id: 41,
                player_info_id: 7,
                name: "Decoy".into(),
                status: PlayerStatus::Active,
                won: false,
                ..PlayerState::default()
            },
        ];
        snapshot.round_results = clonk_engine::RoundResultsState {
            goals: vec!["SCRG".into()],
            fulfilled_goals: vec!["SCRG".into()],
            players: vec![
                clonk_engine::RoundResultsPlayerState {
                    player_info_id: 41,
                    total_playing_time: 3_661,
                    score_old: 10,
                    score_new: Some(110),
                    league_progress_data: None,
                    league_performance: 0,
                    custom_evaluation_strings: String::new(),
                    ..clonk_engine::RoundResultsPlayerState::default()
                },
                clonk_engine::RoundResultsPlayerState {
                    player_info_id: 99,
                    total_playing_time: 9,
                    score_old: 900,
                    score_new: Some(901),
                    league_progress_data: None,
                    league_performance: 0,
                    custom_evaluation_strings: "wrong runtime-number join".into(),
                    ..clonk_engine::RoundResultsPlayerState::default()
                },
            ],
            ..clonk_engine::RoundResultsState::default()
        };
        let next_mission = clonk_engine::NextMissionState {
            path: "Tutorial.c4f\\Tutorial02.c4s".into(),
            text: "Next tutorial".into(),
            description: "Continue learning".into(),
        };
        let picture = ImageData::new(1, 1, vec![12, 34, 56, 255]);
        let player_icon = ImageData::new(1, 1, vec![65, 43, 21, 255]);

        let dialog = build_game_over_dialog(
            &snapshot,
            &[],
            false,
            99,
            1024,
            true,
            "decoy title".into(),
            &next_mission,
            |definition_id, fulfilled| {
                (
                    (definition_id == "SCRG").then(|| picture.clone()),
                    if fulfilled {
                        "Goal Scenario goal fulfilled: Complete the scenario".into()
                    } else {
                        "Goal Scenario goal not fulfilled: Complete the scenario".into()
                    },
                )
            },
            |player_info_id| (player_info_id == 41).then(|| player_icon.clone()),
        |_player_info_id| None, |_player_info_id| None,
        );

        assert_eq!(
            dialog.actions(),
            vec![
                GameOverAction::End,
                GameOverAction::Continue,
                GameOverAction::NextMission,
            ],
            "Tutorial02 is exposed through the classic next-mission button"
        );
        assert_eq!(dialog.evaluation().goals().len(), 1);
        let goal = &dialog.evaluation().goals()[0];
        assert_eq!(goal.definition_id, "SCRG");
        assert!(goal.fulfilled);
        assert_eq!(
            goal.tooltip,
            "Goal Scenario goal fulfilled: Complete the scenario"
        );
        assert_eq!(
            goal.picture.as_ref().map(|image| image.pixels().to_vec()),
            Some(vec![12, 34, 56, 255])
        );
        let player = dialog
            .evaluation()
            .player_by_info_id(41)
            .expect("result joins the profile ID");
        assert_eq!(player.name, "Player");
        assert!(player.won, "won comes from frozen player info, not Active");
        assert_eq!(player.color_dw, 0x00e8_0000);
        assert_eq!(player.total_playing_time, 3_661);
        assert_eq!((player.score_old, player.score_new), (10, Some(110)));
        assert_eq!(player.big_icon.as_ref(), Some(&player_icon));
        assert_eq!(
            dialog.evaluation().players().count(),
            1,
            "a result keyed like the runtime number must not attach to the player"
        );

        snapshot.round_results.hide_settlement_score = true;
        let hidden = build_game_over_dialog(
            &snapshot,
            &[],
            false,
            99,
            1024,
            true,
            "decoy title".into(),
            &next_mission,
            |_, _| (None, String::new()),
            |_| None, |_player_info_id| None, |_player_info_id| None,
        );
        let player = hidden
            .evaluation()
            .player_by_info_id(41)
            .expect("hidden result still joins the profile");
        assert_eq!(
            (player.score_old, player.score_new),
            (-1, None),
            "HideSettlementScoreInEvaluation suppresses the score line"
        );
    }

    #[test]
    fn evaluation_dialog_sources_global_text_and_fixed_team_context() {
        let mut snapshot = make_snapshot(Vec::new(), Vec::new());
        snapshot.players = vec![
            PlayerState {
                id: 20,
                player_info_id: 200,
                name: "Blue".into(),
                team: Some(2),
                won: false,
                ..PlayerState::default()
            },
            PlayerState {
                id: 10,
                player_info_id: 100,
                name: "Red winner".into(),
                team: Some(1),
                won: true,
                ..PlayerState::default()
            },
            PlayerState {
                id: 11,
                player_info_id: 101,
                name: "Red teammate".into(),
                team: Some(1),
                won: false,
                ..PlayerState::default()
            },
        ];
        snapshot.round_results = clonk_engine::RoundResultsState {
            custom_evaluation_strings: "Global summary|Second line".into(),
            players: [200, 100, 101]
                .into_iter()
                .map(|player_info_id| clonk_engine::RoundResultsPlayerState {
                    player_info_id,
                    custom_evaluation_strings: if player_info_id == 101 {
                        "Personal note".to_string()
                    } else {
                        String::new()
                    },
                    ..clonk_engine::RoundResultsPlayerState::default()
                })
                .collect(),
            ..clonk_engine::RoundResultsState::default()
        };
        let teams = [
            clonk_engine::TeamInfo::new(1, "Red", 0x00f4_0000),
            clonk_engine::TeamInfo::new(2, "Blue", 0x0000_00f4),
        ];
        let dialog = build_game_over_dialog(
            &snapshot,
            &teams,
            false,
            10,
            1024,
            true,
            "Scenario".into(),
            &clonk_engine::NextMissionState::default(),
            |_, _| (None, String::new()),
            |_| None, |_player_info_id| None, |_player_info_id| None,
        );

        assert_eq!(
            dialog.evaluation().custom_evaluation_strings(),
            "Global summary|Second line"
        );
        assert_eq!(dialog.evaluation().separate_team_ids(), Some([1, 2]));
        let players = dialog.evaluation().players().collect::<Vec<_>>();
        assert_eq!(
            players
                .iter()
                .map(|player| (player.player_info_id, player.team_id, player.won))
                .collect::<Vec<_>>(),
            vec![
                (200, Some(2), false),
                (100, Some(1), true),
                (101, Some(1), true),
            ],
            "fixed-team context retains source order and applies team-level victory"
        );
        assert_eq!(players[2].custom_evaluation_strings, "Personal note");
        let fonts = new_classic_running_sandbox_app()
            .assets
            .clonk_fonts
            .clone()
            .expect("classic fonts");
        let split_layout = dialog.classic_evaluation_layout(1024, 600, &fonts);
        assert_eq!(split_layout.player_lists.len(), 2);
        assert_eq!(
            split_layout
                .players
                .iter()
                .map(|player| (player.player_list_index, player.player_index))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0, 2), (1, 0)]
        );

        let generated = build_game_over_dialog(
            &snapshot,
            &teams,
            true,
            10,
            1024,
            true,
            "Scenario".into(),
            &clonk_engine::NextMissionState::default(),
            |_, _| (None, String::new()),
            |_| None, |_player_info_id| None, |_player_info_id| None,
        );
        assert_eq!(generated.evaluation().separate_team_ids(), None);
        assert_eq!(
            generated
                .classic_evaluation_layout(1024, 600, &fonts)
                .player_lists
                .len(),
            1
        );
    }
