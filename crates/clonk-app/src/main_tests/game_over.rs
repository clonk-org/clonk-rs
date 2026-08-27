// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! game_over_fixture {
    (host: $port:expr, $player_name:expr, $prepared:expr $(,)?) => {
        HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], $port)),
            player_name: $player_name,
            prepared: $prepared,
        }
    };
    (client_update: $update_type:expr, $client_id:expr, $data:expr, $by_client:expr $(,)?) => {
        clonk_engine::ClientUpdateControlData {
            update_type: $update_type,
            client_id: $client_id,
            data: $data,
            by_client: $by_client,
        }
    };
    (player_update: $client_id:expr, $flags:expr, $players:expr $(,)?) => {
        clonk_network::PlayerInfoUpdateRequest {
            client_id: $client_id,
            flags: $flags,
            players: $players,
        }
    };
    (vote: $vote_type:expr, $approve:expr, $data:expr, $by_client:expr $(,)?) => {
        clonk_engine::VoteControlData {
            vote_type: $vote_type,
            approve: $approve,
            data: $data,
            by_client: $by_client,
        }
    };
    (gui_button: $slot:expr, $class:expr, $state:expr $(,)?) => {
        GamepadEvent::GuiButton {
            slot: $slot,
            class: $class,
            state: $state,
        }
    };
    (direction: $slot:expr, $button:expr, $state:expr $(,)?) => {
        GamepadEvent::Direction {
            slot: $slot,
            button: $button,
            state: $state,
        }
    };
    (action: $slot:expr, $action:expr, $state:expr $(,)?) => {
        GamepadEvent::Action {
            slot: $slot,
            action: $action,
            state: $state,
        }
    };
    (button: $slot:expr, $button:expr, $state:expr $(,)?) => {
        GamepadEvent::Button {
            slot: $slot,
            button: $button,
            state: $state,
        }
    };
    (axis: $slot:expr, $axis:expr, $state:expr $(,)?) => {
        GamepadEvent::Axis {
            slot: $slot,
            axis: $axis,
            state: $state,
        }
    };
}

#[test]
fn console_lobby_start_is_host_only_and_restarts_countdown() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "Lobby", "CountdownTime", "7").test_value();
    let mut host = new_menu_app(640, 480);
    host.app_paths = Some(paths);
    let (_events, mut commands) = install_classic_host_network_stub(&mut host);
    // C4Application::OnCommand forwards non-/start lobby commands through
    // C4MessageInput::ProcessInput, whose /set maxplayer branch accepts
    // the network control host (oracle src/C4Application.cpp:622-644;
    // src/C4MessageInput.cpp:472-490).
    host.process_console_command("/set maxplayer 24")
        .test_value();
    main_assert_eq!(commands.take_submitted_control_sets() => vec![clonk_network::LegacyControlSet {value_type: 2, data: 24, by_client: 0,}]);
    host.process_console_command("/start").test_value();
    main_assert_eq!(commands.take_submitted_lobby_countdowns() => vec![clonk_network::LobbyCountdownPacket::new(7)]);
    host.process_console_command("/abort").test_value();
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert_eq!(host.host_lobby_countdown => Some(HostLobbyCountdown::with_seconds(7)));
    main_assert!(host
        .classic_host_lobby
        .as_ref()
        .expect("classic host lobby")
        .controller
        .logs()
        .last()
        .is_some_and(|line| line.text.contains("Unknown command: \"abort\"")));
    host.process_console_command("/starter 12junk").test_value();
    main_assert_eq!(commands.take_submitted_lobby_countdowns() => vec![clonk_network::LobbyCountdownPacket::new(-1), clonk_network::LobbyCountdownPacket::new(12),]);
    host.sound.ui_log.clear();
    host.process_console_command("/start ").test_value();
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(host.sound.ui_log.is_empty(), "native console validation only logs the usage error");
    main_assert_eq!(
        host.classic_host_lobby
            .as_ref()
            .expect("classic host lobby")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()) =>
        Some("Usage: /start [timer]")
    );
    host.process_console_command("/start 0").test_value();
    main_assert_eq!(commands.take_submitted_lobby_countdowns() => vec![clonk_network::LobbyCountdownPacket::new(-1), clonk_network::LobbyCountdownPacket::new(0),]);
    main_assert_eq!(host.host_lobby_countdown => Some(HostLobbyCountdown::with_seconds(0)));
    main_assert_eq!(host.mode => AppMode::Menu);

    install_message_fixture(&mut host);
    host.snapshot = host.engine.snapshot();
    host.process_console_command("/private Sender secret")
        .test_value();
    main_assert!(commands.take_submitted_messages().is_empty());
    main_assert!(host
        .classic_host_lobby
        .as_ref()
        .expect("classic host lobby")
        .controller
        .logs()
        .last()
        .is_some_and(|line| line.text.contains("Unknown command: \"private\"")));
    host.process_console_command("\"hello").test_value();
    main_assert_eq!(
        commands.take_submitted_messages() =>
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: LegacyCString::from_bytes(b"\"hello".to_vec())
                .expect("fixture message is NUL-free"),
            by_client: 0,
        }]
    );
    main_assert!(host.engine.set_team_distribution(4));
    host.process_console_command("^hidden").test_value();
    main_assert!(commands.take_submitted_messages().is_empty());
    main_assert_eq!(
        host.classic_host_lobby
            .as_ref()
            .expect("classic host lobby")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()) =>
        Some("Can't send team message: Teams not known.")
    );

    let mut client = new_menu_app(640, 480);
    client.startup.view = StartupView::NetworkLobby;
    client.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    client.process_console_command("/start 3").test_value();
    main_assert_eq!(client.network_lobby.as_ref().expect("generic client lobby").logs.last().map(|line| line.text.as_str()) => Some("Host only!"));
}

#[test]
fn muted_loop_releases_channel_but_survives_and_restarts_on_unmute() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(1_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    snapshot.audio.push(test_sound_command(true));
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    snapshot.audio.clear();

    let original_channel = audio.active_channels[&key].channel.test_value();
    main_assert!(audio.system.channel_is_playing(original_channel));

    audio.options.sound_enabled = false;
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(audio.active_channels.contains_key(&key));
    main_assert!(audio.active_channels[&key].channel.is_none());
    main_assert!(!audio.system.channel_is_playing(original_channel));

    audio.options.sound_enabled = true;
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    let restored_channel = audio.active_channels[&key].channel.test_value();
    main_assert!(audio.system.channel_is_playing(restored_channel));
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

    main_assert_eq!(resolve_next_mission_scenario(&catalog, "tutorial.c4f\\TUTORIAL02.c4s").map(|scenario| scenario.title) => Some("The First Hut".to_string()));
}

#[test]
fn placeholder_preview_has_expected_dimensions() {
    let preview = generate_preview_placeholder(ScenarioKind::Scenario, "Alpha");
    main_assert_eq!(preview.width() => PLACEHOLDER_PREVIEW_WIDTH);
    main_assert_eq!(preview.height() => PLACEHOLDER_PREVIEW_HEIGHT);
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
    main_assert!(varied, "placeholder preview should contain color variation");
}

#[test]
fn local_scenario_load_failure_returns_to_remembered_selector_with_error_log() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
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
        .test_value();

    app.poll_loading().test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::Local);
    main_assert_eq!(app.last_startup_dialog => StartupDialog::ScenarioBrowser(ScenarioSelectorMode::Local));
    main_assert_eq!(app.startup.scenario_back_dialog => None);
    main_assert!(app.loading_state.is_none());
    // The return through PreInit re-initializes the loader screen for the
    // next game (src/C4Application.cpp:242-247,373-389).
    main_assert!(app.loader_screen.is_some());
    main_assert!(app.loader_error.is_none());
    main_assert!(app.active_scenario.is_none());
    main_assert!(app.active_definition_load.is_none());
    main_assert!(app.active_global_gui_failures.is_empty());
    main_assert!(app.dialogs.client_list.is_none());
    assert_startup_error_log(&app, "controlled local load failure");
    main_assert_eq!(app.startup_restart_diagnostics => StartupRestartDiagnostics::default());

    let mut frame = vec![0x4c; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.dialogs.messages.is_empty());
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::Local);
    main_assert_eq!(app.startup.scenario_back_dialog => None);
    reset_cached_app_paths();
}

#[test]
fn restart_diagnostics_bound_order_deduplicate_and_reset() {
    let mut diagnostics = StartupRestartDiagnostics::default();
    diagnostics.mark_quit_with_error();
    for index in 0..=STARTUP_RESTART_LOG_CAPACITY {
        diagnostics.add_log_entry(format!("entry-{index:03}"));
    }
    main_assert_eq!(
        diagnostics.take_presentation() =>
        Some(StartupRestartPresentation::Ringbuffer(
            (1..=STARTUP_RESTART_LOG_CAPACITY)
                .map(|index| format!("entry-{index:03}"))
                .collect()
        ))
    );
    main_assert_eq!(diagnostics => StartupRestartDiagnostics::default());

    diagnostics.add_fatal_error("fatal");
    diagnostics.add_fatal_error("fatal");
    diagnostics.begin_game_init();
    diagnostics.add_log_entry("ordinary");
    main_assert_eq!(diagnostics.take_presentation() => Some(StartupRestartPresentation::Fatal("fatal".to_string())));
    main_assert_eq!(diagnostics => StartupRestartDiagnostics::default());

    diagnostics.mark_quit_with_error();
    main_assert_eq!(diagnostics.take_presentation() => Some(StartupRestartPresentation::Empty));
    main_assert_eq!(diagnostics => StartupRestartDiagnostics::default());
}

#[test]
fn disconnected_startup_worker_reaches_ringbuffer_only_restart_branch() {
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

    app.poll_startup_network_connection().test_value();

    let info = app.dialogs.client_list.test_ref();
    main_assert!(info.is_static_info_only());
    main_assert_eq!(info.info_lines() => ["network worker disconnected before reporting readiness"]);
    main_assert!(app.dialogs.messages.is_empty());
    main_assert!(app.status_text.is_empty());
    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let bottom_close = app
        .dialogs.client_list
        .as_ref()
        .and_then(|dialog| dialog.info_layout(preferred, line_height))
        .and_then(|layout| layout.bottom_close_button)
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(bottom_close.x + bottom_close.w / 2),
        f64::from(bottom_close.y + bottom_close.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(app.dialogs.client_list.is_none());
    main_assert_eq!(app.startup_restart_diagnostics => StartupRestartDiagnostics::default());
}

#[test]
fn restart_ringbuffer_uses_static_ten_line_error_log_info_dialog() {
    let mut app = new_real_classic_menu_app(800, 600);
    attach_l040_network_dialog(&mut app);
    app.startup_network_dialog
        .test_mut()
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
        .test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert!(app.startup_network_dialog.is_some());
    main_assert!(app.dialogs.messages.is_empty());
    main_assert!(app.status_text.is_empty());
    let info = app.dialogs.client_list.test_ref();
    main_assert!(info.is_info_only());
    main_assert!(info.info_is_open());
    main_assert_eq!(info.info_client_id() => None);
    main_assert_eq!(info.info_caption() => "Error Log");
    main_assert_eq!(info.info_requested_line_count() => 10);
    main_assert_eq!(info.info_lines() => entries);

    let (preferred, _) = app.runtime_client_list_input_geometry().test_value();
    let fonts = app.assets.clonk_fonts.clone().test_value();
    main_assert_eq!(
        app.dialogs.client_list
            .as_ref()
            .expect("Error Log info")
            .visible_info_lines(preferred, &fonts.text)
            .first()
            .map(String::as_str) =>
        Some("retained-log-00")
    );
    main_assert!(app
        .dialogs.client_list
        .as_ref()
        .expect("Error Log info")
        .info_scroll_metrics(preferred, &fonts.text)
        .is_some_and(|metrics| metrics.max_scroll > 0));
    let mut frame = vec![0x4c; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));

    app.test_key(VirtualKeyCode::End, ElementState::Pressed);
    main_assert!(app
        .dialogs.client_list
        .as_ref()
        .expect("scrolled Error Log info")
        .visible_info_lines(preferred, &fonts.text)
        .last()
        .is_some_and(|line| line.ends_with("TAIL")));
    app.test_key(VirtualKeyCode::End, ElementState::Released);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.dialogs.client_list.is_none());
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert!(app.startup_network_dialog.is_some());
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert_eq!(app.startup_restart_diagnostics => StartupRestartDiagnostics::default());
}

#[test]
fn empty_restart_log_uses_regular_error_modal_over_restored_host_selector() {
    let mut app = new_real_classic_menu_app(800, 600);
    app.open_network_game_dialog();
    app.open_network_host_scenario_browser();
    app.status_text = "stale generic status".to_string();

    app.startup_restart_diagnostics.mark_quit_with_error();
    app.finish_startup_network_restart(StartupNetworkPurpose::StagedHost)
        .test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert!(app.dialogs.client_list.is_none());
    assert_startup_error_log(&app, "(no error)");
    let mut frame = vec![0x4c; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.dialogs.messages.is_empty());
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.startup_restart_diagnostics => StartupRestartDiagnostics::default());
}

#[test]
fn restart_restore_team_submits_full_player_packet_on_roster_construction() {
    let mut app = new_menu_app(640, 480);
    let (mut chooser, companion) = install_test_classic_host_team_lobby(&mut app);
    chooser.forced_name = LegacyCString::from_bytes(b"Restart Alias".to_vec()).test_value();
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Host(
        game_over_fixture!(host: 0, "Host".to_string(), None),
    ));
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
    app.players.restart_restore_infos
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
        .test_value();
    main_assert_eq!(app.engine.restart_restore_info_mask() => 2);
    app.retain_restart_restore_mask_for_restart();

    app.sync_classic_lobby_roster();

    let mut restored = chooser.clone();
    restored.team = 2;
    let mut restored_companion = companion.clone();
    restored_companion.team = 5;
    main_assert_eq!(
        commands.take_player_info_updates() =>
        vec![
            game_over_fixture!(player_update:
                0,
                clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                vec![restored.clone(), companion],
            ),
            game_over_fixture!(player_update:
                0,
                clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                vec![restored, restored_companion],
            ),
        ],
        "each synchronous PlayerListItem update carries earlier restored teammates forward"
    );
    main_assert!(
        app.players.team_assignment
            .as_ref()
            .unwrap()
            .teams()
            .teams
            .iter()
            .any(|team| team.id == 5),
        "GetGenerateTeamByID creates a missing restored team before submission"
    );
    main_assert_eq!(app.control_player_infos.client_update_request(0).unwrap().players[0].team => 1, "the roster waits for the authoritative PlayerInfo echo");

    app.sync_classic_lobby_roster();
    main_assert!(commands.take_player_info_updates().is_empty(), "an existing PlayerListItem does not rerun its constructor hook");
}

#[test]
fn host_round_restart_returns_to_network_lobby_staging() {
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
        .test_value();

    app.restart_current_scenario().test_value();

    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost, "a hosted round must rebuild its lobby instead of launching locally");
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.players.restart_restore_infos.what => RESTART_RESTORE_PLAYER_TEAMS, "the lobby handoff retains the raw SetRestoreInfos mask");
    // The pathless sandbox fixture reaches host staging and fails there. A
    // failed OpenGame returns through QuitGame to the remembered startup
    // dialog and reports its fatal error in the Error Log instead of leaving a
    // status overlay behind (src/C4Application.cpp:373-405,438-450).
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert_eq!(app.dialogs.messages[0].state.caption() => "Error Log");
    main_assert!(app.dialogs.messages[0].state.message().starts_with("Cannot host"));
}

#[test]
fn host_round_restart_announces_itself_before_tearing_the_session_down() {
    // Restarting re-hosts from scratch, exactly as C4Application::QuitGame does
    // for a NextMission (src/C4Application.cpp:373-405). Every client therefore
    // sees its host connection close, which native cannot distinguish from a
    // dead host (src/C4Network2.cpp:1826-1832). The port states the intent on
    // the wire first so clients can follow the host into the new lobby.
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 1);

    app.restart_current_scenario().test_value();

    main_assert_eq!(
        commands.take_host_restart_broadcasts() =>
        vec![clonk_network::DEFAULT_HOST_RESTART_REJOIN_SECONDS],
        "a restarting host must announce the restart while it can still be heard"
    );
}

#[test]
fn host_round_restart_keeps_the_session_up_and_rebuilds_its_own_lobby() {
    // The session-preserving restart, and the reason issue clonk-org/clonk-rs#241
    // exists: re-hosting from scratch costs every client a whole new connection
    // to reach a lobby it was already entitled to. With a scenario this host can
    // prepare, nothing is torn down but the round.
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.active_scenario = Some(tutorial_frontend(repository));
    app.active_definition_load = Some(ScenarioDefinitionLoad::Seed {
        modules: vec!["Objects.c4d".to_string()],
        definition_root: None,
    });
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(
        game_over_fixture!(host: 11_112, "Exact Host".to_string(), None),
    ));

    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        main_assert!(restart.complete(Ok(())));
        commands
    });

    app.mode = AppMode::Running;
    main_assert!(app.show_abort_dialog(app.players.local_owner));
    finish_abort_dialog(
        &mut app,
        clonk_frontend::message_dialog::MessageDialogResult::Restart,
    );
    let mut commands = restart_completion.join().test_value();

    main_assert!(app.network.is_some(), "the session every client is connected to must outlive the round");
    main_assert!(app.network_mode.is_some(), "a retained session keeps the host mode that describes it");
    main_assert!(commands.take_host_round_lobby_restarts().is_empty(), "the synchronous restart command was consumed exactly once by the completion worker");
    main_assert!(commands.take_host_restart_broadcasts().is_empty(), "the reconnect notice would send every client to re-dial a host that never left");
    main_assert!(app.classic_host_lobby.is_some(), "the host lands back in its own lobby");
    main_assert_eq!(app.mode => AppMode::Menu);
}

#[test]
fn running_host_round_restart_keeps_connected_clients_in_the_rebuilt_lobby() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let host_name = staged.lobby.local_name.clone();
    app.active_scenario = Some(scenario);
    app.active_definition_load = Some(activated_definition_load(
        Some(staged.effective_definition_modules.clone()),
        staged.definition_load,
    ));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(
        game_over_fixture!(host: 11_112, host_name, None),
    ));
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
            name: LegacyCString::from_bytes(b"Connected Client".to_vec()).test_value(),
            ..Default::default()
        },
    ]);
    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        main_assert!(restart.complete(Ok(())));
        commands
    });

    app.restart_current_network_scenario().test_value();
    let _commands = restart_completion.join().test_value();

    main_assert!(app.network.is_some(), "the live session must survive restart");
    main_assert!(app.classic_host_lobby.is_some(), "the running scenario's effective definitions must rebuild its lobby");
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    let hosted_resource_localities = app
        .admission_resources
        .resources
        .values()
        .filter_map(|resource| match resource {
            AdmissionResourceState::Complete { local, .. } => Some(*local),
            AdmissionResourceState::Loading { .. }
            | AdmissionResourceState::Unavailable(_) => None,
        })
        .collect::<Vec<_>>();
    main_assert!(!hosted_resource_localities.is_empty(), "the rebuilt host lobby must install its prepared local files");
    main_assert!(hosted_resource_localities.into_iter().all(|local| local), "temporary ownership must not relabel a host-prepared local file as remote");
    main_assert!(
        app.classic_host_lobby
            .test_ref()
            .controller
            .rows()
            .iter()
            .any(|row| row.id() == LobbyRosterId::Client(7)),
        "an already-connected client must be present without rejoining"
    );
}

#[test]
fn running_host_round_restart_refreshes_retained_advertising() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let reserved_reference = std::net::TcpListener::bind("[::]:0").test_value();
    let reference_port = reserved_reference.local_addr().test_value().port();
    drop(reserved_reference);
    persist_config_value(
        &paths,
        "Network",
        "PortRefServer",
        reference_port.to_string(),
    )
    .test_value();
    persist_config_value(&paths, "Network", "PortDiscovery", "0").test_value();

    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let host_name = staged.lobby.local_name.clone();
    let host_nick = staged.lobby.nick.clone();
    app.active_scenario = Some(scenario);
    app.active_definition_load = Some(activated_definition_load(
        Some(staged.effective_definition_modules.clone()),
        staged.definition_load,
    ));
    let (manager, _events, mut commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(game_over_fixture!(
        host: 11_112,
        host_name.clone(),
        None,
    )));
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: LegacyCString::from_bytes(host_name.into_bytes()).test_value(),
            nick: LegacyCString::from_bytes(host_nick.into_bytes()).test_value(),
            ..Default::default()
        }]);
    let (_snapshot, reference) = default_exact_host_reference();
    app.start_network_game_advertiser_with_reference(
        clonk_network::NetworkGameAdvertiserConfig {
            discovery_port: 0,
            reference_port: Some(reference_port),
            language_charset: String::new(),
        },
        reference,
    );
    let original_reference_addr = app
        .network_game_advertiser
        .test_ref()
        .reference_addr();
    let mut retained_reference_connection =
        std::net::TcpStream::connect(("127.0.0.1", reference_port)).test_value();
    retained_reference_connection
        .set_read_timeout(Some(Duration::from_secs(5)))
        .test_value();
    retained_reference_connection
        .set_write_timeout(Some(Duration::from_secs(5)))
        .test_value();
    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        main_assert!(restart.complete(Ok(())));
        commands
    });

    app.restart_current_network_scenario().test_value();
    let mut commands = restart_completion.join().test_value();

    let restarted_advertiser = app
        .network_game_advertiser
        .test_ref();
    main_assert_eq!(restarted_advertiser.reference_addr() => original_reference_addr, "the retained host must keep its bound reference endpoint while replacing the round metadata");
    main_assert_eq!(app.advertised_game_reference.test_ref().summary().state => "Lobby");
    retained_reference_connection
        .write_all(b"GET / HTTP/1.0\r\n\r\n")
        .test_value();
    let mut response = String::new();
    retained_reference_connection
        .read_to_string(&mut response)
        .test_value();
    main_assert!(response.contains("Title=\"Fixture\"\r\n"), "the connection accepted before restart must serve the rebuilt round reference: {response:?}");
    main_assert_eq!(commands.take_league_update_effects().1 => 1, "the retained masterserver session must publish the fresh lobby reference without waiting for its ordinary heartbeat");
}

#[test]
fn running_host_round_restart_keeps_live_password_and_comment() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let host_name = staged.lobby.local_name.clone();
    let host_nick = staged.lobby.nick.clone();
    app.active_scenario = Some(scenario);
    app.active_definition_load = Some(activated_definition_load(
        Some(staged.effective_definition_modules.clone()),
        staged.definition_load,
    ));
    let (manager, _events, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(game_over_fixture!(
        host: 11_112,
        host_name.clone(),
        None,
    )));
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: LegacyCString::from_bytes(host_name.into_bytes()).test_value(),
            nick: LegacyCString::from_bytes(host_nick.into_bytes()).test_value(),
            ..Default::default()
        }]);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    let password_completion = thread::spawn(move || {
        let (password, completion) = commands.receive_host_password();
        main_assert_eq!(password.as_bytes() => b"live secret");
        completion.send(Ok(())).test_value();
        commands
    });
    app.set_running_network_password(b"live secret");
    let mut commands = password_completion.join().test_value();
    app.set_running_network_comment(b"live comment");
    let _ = commands.take_league_update_effects();
    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        let password = restart.config.password.as_bytes().to_vec();
        main_assert!(restart.complete(Ok(())));
        password
    });

    app.restart_current_network_scenario().test_value();
    let restarted_password = restart_completion.join().test_value();

    main_assert_eq!(restarted_password => b"live secret");
    main_assert!(app.advertised_game_reference.test_ref().summary().password_needed);
    main_assert_eq!(app.advertised_game_reference.test_ref().metadata().comment.as_bytes() => b"live comment");
}

#[test]
fn rejected_live_round_restart_falls_back_to_announced_rehosting() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let host_name = staged.lobby.local_name.clone();
    app.active_scenario = Some(scenario);
    app.active_definition_load = Some(activated_definition_load(
        Some(staged.effective_definition_modules.clone()),
        staged.definition_load,
    ));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(
        game_over_fixture!(host: 11_112, host_name, None),
    ));
    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        main_assert!(restart.complete(Err(
            "a retained client does not support atomic round restart".to_string()
        )));
        commands
    });

    app.restart_current_network_scenario().test_value();
    let mut commands = restart_completion.join().test_value();

    main_assert_eq!(
        commands.take_host_restart_broadcasts() =>
        vec![clonk_network::DEFAULT_HOST_RESTART_REJOIN_SECONDS],
        "the compatibility fallback must announce the reconnect before dropping the old session"
    );
    main_assert!(app.network.is_none(), "the rejected retained session must not survive as the next host");
    main_assert!(app.startup_network_connection.is_some(), "the same scenario must immediately begin re-hosting");
}

#[test]
fn host_round_restart_does_not_resurrect_disconnected_player_rows() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let scenario_text_path = scenario.path.test_ref().join("Scenario.txt");
    let scenario_text = fs::read_to_string(&scenario_text_path).test_value();
    fs::write(
        &scenario_text_path,
        scenario_text.replace("MaxPlayer=1", "MaxPlayer=3"),
    )
    .test_value();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let host_name = staged.lobby.local_name.clone();
    app.active_scenario = Some(scenario);
    app.active_definition_load = Some(activated_definition_load(
        Some(staged.effective_definition_modules.clone()),
        staged.definition_load,
    ));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(
        game_over_fixture!(host: 11_112, host_name, None),
    ));
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
            name: LegacyCString::from_bytes(b"Connected Client".to_vec()).test_value(),
            ..Default::default()
        },
    ]);
    app.control_player_infos.replace_snapshot(
        9,
        [
            clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 7,
                    name: LegacyCString::from_bytes(b"Connected Player".to_vec()).test_value(),
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                }],
                by_client: 0,
            },
            clonk_engine::PlayerInfoControlData {
                client_id: 9,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 9,
                    name: LegacyCString::from_bytes(b"Departed Player".to_vec()).test_value(),
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                        | clonk_engine::PLAYER_INFO_FLAG_REMOVED
                        | clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED,
                    ..Default::default()
                }],
                by_client: 0,
            },
        ],
    );
    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        main_assert!(restart.complete(Ok(())));
        commands
    });

    app.restart_current_network_scenario().test_value();
    let _commands = restart_completion.join().test_value();

    main_assert_eq!(app.control_player_infos.client_info_ids(7) => vec![7]);
    main_assert!(
        app.control_player_infos.client_packet(9).is_none(),
        "a PlayerInfo row whose client socket is gone must not be revived in the next lobby"
    );
}

#[test]
fn host_round_restart_without_restore_mask_resets_remote_teams() {
    // Native only reapplies prior team selections when RESTORE_PlayerTeams is
    // present (src/C4PlayerInfoListBox.cpp:170-181; src/C4Game.cpp:2390-2397).
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let scenario_text_path = scenario.path.test_ref().join("Scenario.txt");
    let scenario_text = fs::read_to_string(&scenario_text_path).test_value();
    fs::write(
        &scenario_text_path,
        scenario_text.replace("MaxPlayer=1", "MaxPlayer=2"),
    )
    .test_value();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    configure_test_startup_participant(&paths, user_data.path());
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let host_name = staged.lobby.local_name.clone();
    app.active_scenario = Some(scenario);
    app.active_definition_load = Some(activated_definition_load(
        Some(staged.effective_definition_modules.clone()),
        staged.definition_load,
    ));
    let (manager, _events, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(game_over_fixture!(
        host: 11_112,
        host_name,
        None,
    )));
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
            name: LegacyCString::from_bytes(b"Connected Client".to_vec()).test_value(),
            ..Default::default()
        },
    ]);
    app.control_player_infos.replace_snapshot(
        7,
        [clonk_engine::PlayerInfoControlData {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 7,
                name: LegacyCString::from_bytes(b"Remote Player".to_vec()).test_value(),
                team: 5,
                ..Default::default()
            }],
            by_client: 0,
        }],
    );
    main_assert_eq!(app.engine.restart_restore_info_mask() => 0);
    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        main_assert!(restart.complete(Ok(())));
    });

    app.restart_current_network_scenario().test_value();
    restart_completion.join().test_value();

    let remote_teams = app
        .control_player_infos
        .client_packet(7)
        .test_value()
        .players
        .iter()
        .map(|player| player.team)
        .collect::<Vec<_>>();
    main_assert_eq!(remote_teams => vec![0], "without RESTORE_PlayerTeams the remote row must not retain a team that the fresh host row lost");
}

#[test]
fn observer_host_round_restart_without_profile_does_not_open_first_player_dialog() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let host_name = staged.lobby.local_name.clone();
    app.active_scenario = Some(scenario);
    app.active_definition_load = Some(activated_definition_load(
        Some(staged.effective_definition_modules.clone()),
        staged.definition_load,
    ));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(
        game_over_fixture!(host: 11_112, host_name, None),
    ));
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            observer: true,
            name: LegacyCString::from_bytes(b"Observer Host".to_vec()).test_value(),
            ..Default::default()
        }]);
    let restart_completion = thread::spawn(move || {
        let restart = commands.receive_host_round_lobby_restart();
        main_assert!(restart.complete(Ok(())));
        commands
    });

    app.restart_current_network_scenario().test_value();
    let _commands = restart_completion.join().test_value();

    main_assert!(app.classic_host_lobby.is_some());
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(
        app.startup.player_properties_dialog.is_none(),
        "the retained observer lobby must not inherit main menu's first-profile creation modal"
    );
}

fn pump_live_restart_apps_until(
    host: &mut GameApp,
    client: &mut GameApp,
    description: &str,
    mut completed: impl FnMut(&GameApp, &GameApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !completed(host, client) {
        host.test_update();
        client.test_update();
        main_assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}: host={:?}/{:?} client={:?}/{:?}; host status={:?}; client status={:?}; host clients={:?}; client clients={:?}; host lobby ack={}; client lobby ack={}; client JoinData={}; host resource progress={:?}; client resource progress={:?}",
            host.mode,
            host.startup.view,
            client.mode,
            client.startup.view,
            host.status_text,
            client.status_text,
            host.control_clients.snapshot(),
            client.control_clients.snapshot(),
            host.initial_lobby_status_ack_pending,
            client.initial_lobby_status_ack_pending,
            client.pending_network_join_data.is_some(),
            host.admission_resources.present_percent,
            client.admission_resources.present_percent,
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn pump_live_restart_three_apps_until(
    host: &mut GameApp,
    retained_client: &mut GameApp,
    joining_client: &mut GameApp,
    description: &str,
    mut completed: impl FnMut(&GameApp, &GameApp, &GameApp) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !completed(host, retained_client, joining_client) {
        host.test_update();
        retained_client.test_update();
        joining_client.test_update();
        main_assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}: host={:?}/{:?} retained={:?}/{:?} joining={:?}/{:?}; host status={:?}; retained status={:?}; joining status={:?}; host clients={:?}; retained clients={:?}; joining clients={:?}; joining JoinData={}",
            host.mode,
            host.startup.view,
            retained_client.mode,
            retained_client.startup.view,
            joining_client.mode,
            joining_client.startup.view,
            host.status_text,
            retained_client.status_text,
            joining_client.status_text,
            host.control_clients.snapshot(),
            retained_client.control_clients.snapshot(),
            joining_client.control_clients.snapshot(),
            joining_client.pending_network_join_data.is_some(),
        );
        thread::sleep(Duration::from_millis(2));
    }
}

fn without_round_player_lifecycle(
    packet: &clonk_engine::PlayerInfoControlData,
) -> clonk_engine::PlayerInfoControlData {
    let mut packet = packet.clone();
    for player in &mut packet.players {
        player.flags &= !(clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_REMOVED);
        player.game_number = -1;
        player.game_join_frame = -1;
        player.game_part_frame = -1;
    }
    packet
}

#[test]
fn host_restart_keeps_real_peer_in_same_scenario_lobby_and_starts_again() {
    // Native schedules Game.ScenarioFilename before Abort and restores it with
    // fLobby/NetworkActive, bypassing the selector
    // (src/C4GameDialogs.cpp:94-117;
    // src/C4Application.cpp:232-295,373-399). Native then drops every
    // connection in C4Network2::Clear (src/C4Game.cpp:544-654;
    // src/C4Network2.cpp:746-790), so retaining this live session and roster
    // is the port's intentional improvement.
    let _lock = env_lock().lock();
    let host_user_data = tempdir();
    let client_user_data = tempdir();
    let joining_client_user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let scenario_text_path = scenario.path.test_ref().join("Scenario.txt");
    let scenario_text = fs::read_to_string(&scenario_text_path).test_value();
    let three_player_scenario = scenario_text.replace("MaxPlayer=1", "MaxPlayer=3");
    main_assert_ne!(three_player_scenario => scenario_text, "the E2E fixture must actually admit the retained peer and a new peer after restart");
    fs::write(&scenario_text_path, three_player_scenario).test_value();
    let (_host_guard, host_paths) =
        exact_loader_test_paths(host_user_data.path(), Some(content.path()));
    let (_client_guard, client_paths) =
        exact_loader_test_paths(client_user_data.path(), Some(content.path()));
    let (_joining_client_guard, joining_client_paths) =
        exact_loader_test_paths(joining_client_user_data.path(), Some(content.path()));
    configure_test_startup_participant(&host_paths, host_user_data.path());
    configure_test_startup_participant(&client_paths, client_user_data.path());
    configure_test_startup_participant(&joining_client_paths, joining_client_user_data.path());

    let client_listener = std::net::TcpListener::bind("127.0.0.1:0").test_value();
    let client_port = client_listener.local_addr().test_value().port();
    let joining_client_listener = std::net::TcpListener::bind("127.0.0.1:0").test_value();
    let joining_client_port = joining_client_listener.local_addr().test_value().port();
    for paths in [&host_paths, &client_paths, &joining_client_paths] {
        persist_config_value(paths, "Network", "PortUDP", "0").test_value();
        persist_config_value(paths, "Network", "PortDiscovery", "0").test_value();
        persist_config_value(paths, "Network", "PortRefServer", "0").test_value();
        persist_config_value(paths, "Network", "EnableUPnP", "0").test_value();
        persist_config_value(paths, "Network", "MasterServerSignUp", "0").test_value();
        persist_config_value(paths, "General", "Preloading", "0").test_value();
    }
    for paths in [&host_paths, &client_paths] {
        persist_config_value(paths, "Network", "PortTCP", client_port.to_string()).test_value();
    }
    persist_config_value(
        &joining_client_paths,
        "Network",
        "PortTCP",
        joining_client_port.to_string(),
    )
    .test_value();
    persist_config_value(&client_paths, "Network", "LocalName", "Connected Client").test_value();
    persist_config_value(
        &joining_client_paths,
        "Network",
        "LocalName",
        "Joining Client",
    )
    .test_value();

    let mut host = new_menu_app_with_paths(800, 600, &host_paths);
    let staged = prepare_minimal_host_lobby(&host, scenario.clone());
    host.staged_network_host_scenario = Some(staged);
    host.activate_prepared_network_host(
        scenario.clone(),
        SocketAddr::from(([127, 0, 0, 1], 0)),
    );
    let host_deadline = Instant::now() + Duration::from_secs(30);
    while host.network.is_none() {
        host.test_update();
        main_assert!(
            Instant::now() < host_deadline,
            "timed out starting live host: {}",
            host.status_text,
        );
        thread::sleep(Duration::from_millis(2));
    }
    let host_endpoint = host
        .network
        .test_ref()
        .local_addresses()
        .into_iter()
        .find(|address| address.protocol == clonk_network::NetworkProtocol::Tcp)
        .test_value()
        .endpoint;
    drop(client_listener);

    let mut client = new_menu_app_with_paths(800, 600, &client_paths);
    client.players.local_name = "Connected Client".to_string();
    client
        .activate_network_join(host_endpoint.to_string())
        .test_value();
    pump_live_restart_apps_until(
        &mut host,
        &mut client,
        "the real client to enter the host lobby",
        |host, client| {
            let Some(client_id) = client
                .network
                .as_ref()
                .and_then(|network| i32::try_from(network.local_client_id()).ok())
            else {
                return false;
            };
            host.classic_host_lobby.is_some()
                && client.network_lobby.is_some()
                && host.control_clients.contains(client_id)
                && !host.control_player_infos.client_info_ids(client_id).is_empty()
        },
    );
    let client_id = i32::try_from(client.network.test_ref().local_client_id()).test_value();
    host.start_network_game_now().test_value();
    pump_live_restart_apps_until(
        &mut host,
        &mut client,
        "the initial synchronized round",
        |host, client| matches!(host.mode, AppMode::Running) && matches!(client.mode, AppMode::Running),
    );
    for _ in 0..64 {
        host.test_update();
        client.test_update();
        thread::sleep(Duration::from_millis(2));
    }

    let host_scenario = host.active_scenario.clone().test_value();
    let client_scenario = client.active_scenario.clone().test_value();
    main_assert_eq!(client_scenario.title => host_scenario.title);
    let host_player_ids = host.control_player_infos.client_info_ids(client_id);
    let client_player_ids = client.control_player_infos.client_info_ids(client_id);
    main_assert!(!host_player_ids.is_empty(), "the connected player's authoritative row must exist before restart");
    main_assert!(!client_player_ids.is_empty(), "the connected player must see its row before restart");
    let host_player_packet = host
        .control_player_infos
        .client_packet(client_id)
        .test_value();
    let client_player_packet = client
        .control_player_infos
        .client_packet(client_id)
        .test_value();
    let host_addresses = host.network.test_ref().local_addresses();
    let host_local_id = host.network.test_ref().local_client_id();
    let client_local_id = client.network.test_ref().local_client_id();
    let route_keys = |app: &GameApp| {
        let mut routes = app
            .network
            .test_ref()
            .runtime_connections()
            .test_value()
            .into_iter()
            .map(|route| {
                (
                    route.connection_id,
                    route.client_id,
                    route.protocol,
                    route.peer_address,
                )
            })
            .collect::<Vec<_>>();
        routes.sort_by_key(|route| route.0);
        routes
    };
    let host_routes = route_keys(&host);
    let client_routes = route_keys(&client);
    main_assert!(!host_routes.is_empty() && !client_routes.is_empty(), "the E2E must observe both live workers' real routes");
    main_assert!(host.show_abort_dialog(host.players.local_owner));
    finish_abort_dialog(
        &mut host,
        clonk_frontend::message_dialog::MessageDialogResult::Restart,
    );
    main_assert!(host.network.is_some(), "restart dropped the live host before its peer could follow: status={:?} loader={:?}", host.status_text, host.loader_render_error);
    main_assert!(host.classic_host_lobby.is_some(), "restart did not rebuild the host lobby: status={:?} loader={:?}", host.status_text, host.loader_render_error);
    pump_live_restart_apps_until(
        &mut host,
        &mut client,
        "both retained peers to return to the lobby",
        |host, client| {
            matches!(host.mode, AppMode::Menu)
                && matches!(client.mode, AppMode::Menu)
                && host.classic_host_lobby.is_some()
                && client.network_lobby.is_some()
                && host.control_clients.is_activated(client_id)
                && client.control_clients.is_activated(client_id)
        },
    );
    main_assert_eq!(host.startup.view => StartupView::NetworkLobby);
    main_assert_eq!(client.startup.view => StartupView::NetworkLobby);
    main_assert!(matches!(host.network_mode, Some(NetworkMode::Host(_))));
    main_assert!(matches!(client.network_mode, Some(NetworkMode::Client(_))));
    main_assert_eq!(host.network.test_ref().local_client_id() => host_local_id);
    main_assert_eq!(client.network.test_ref().local_client_id() => client_local_id);
    main_assert_eq!(host.network.test_ref().local_addresses() => host_addresses);
    main_assert!(host.startup_network_connection.is_none());
    main_assert!(client.startup_network_connection.is_none(), "the retained peer must not re-dial the host");
    main_assert!(client.pending_host_rejoin.is_none());
    let host_routes_after = route_keys(&host);
    let client_routes_after = route_keys(&client);
    main_assert_eq!(host_routes_after => host_routes, "the host must reuse exactly its original connection IDs and peer endpoints");
    main_assert_eq!(client_routes_after => client_routes, "the client must reuse exactly its original connection IDs and peer endpoints");
    main_assert!(host.classic_host_lobby.test_ref().controller.rows().iter().any(|row| row.id() == LobbyRosterId::Client(client_id)), "the connected client must remain in the host lobby");
    main_assert!(client.network_lobby.test_ref().participants.contains_key(&0));
    main_assert!(client.network_lobby.test_ref().participants.contains_key(&client_local_id));
    main_assert_eq!(host.control_player_infos.client_info_ids(client_id) => host_player_ids, "the host must retain the connected player's row");
    main_assert_eq!(client.control_player_infos.client_info_ids(client_id) => client_player_ids, "the client must retain its player row");
    let rebuilt_host_packet = host
        .control_player_infos
        .client_packet(client_id)
        .test_value();
    let rebuilt_client_packet = client
        .control_player_infos
        .client_packet(client_id)
        .test_value();
    main_assert_eq!(rebuilt_client_packet => rebuilt_host_packet, "the rebuilt lobby must leave both peers with the same authoritative PlayerInfo packet");
    main_assert_eq!(without_round_player_lifecycle(&rebuilt_host_packet) => without_round_player_lifecycle(&host_player_packet), "the rebuilt host lobby must retain the remote player's full identity and resource packet");
    main_assert_eq!(without_round_player_lifecycle(&rebuilt_client_packet) => without_round_player_lifecycle(&client_player_packet), "the rebuilt client lobby must retain the local player's full identity and resource packet");
    main_assert_eq!(host.staged_network_host_scenario.test_ref().frontend.identifier => host_scenario.identifier);
    // A client executes a local Combined<ID>.c4s artifact, while the title and
    // network resource core below identify it with the host's scenario.
    main_assert_eq!(client.network_lobby.test_ref().selected_identifier() => Some(client_scenario.identifier.as_str()));
    main_assert_eq!(client.network_lobby.test_ref().scenario_label() => host_scenario.title);

    let mut joining_client = new_menu_app_with_paths(800, 600, &joining_client_paths);
    joining_client.players.local_name = "Joining Client".to_string();
    drop(joining_client_listener);
    joining_client
        .activate_network_join(host_endpoint.to_string())
        .test_value();
    pump_live_restart_three_apps_until(
        &mut host,
        &mut client,
        &mut joining_client,
        "a new real client to enter the restarted lobby",
        |host, retained_client, joining_client| {
            let Some(joining_client_network_id) = joining_client
                .network
                .as_ref()
                .map(NetworkManager::local_client_id)
            else {
                return false;
            };
            let Ok(joining_client_id) = i32::try_from(joining_client_network_id) else {
                return false;
            };
            host.classic_host_lobby.is_some()
                && joining_client.network_lobby.is_some()
                && host.control_clients.contains(joining_client_id)
                && retained_client.control_clients.contains(joining_client_id)
                && !host
                    .control_player_infos
                    .client_info_ids(joining_client_id)
                    .is_empty()
                && !retained_client
                    .control_player_infos
                    .client_info_ids(joining_client_id)
                    .is_empty()
        },
    );
    let joining_client_id =
        i32::try_from(joining_client.network.test_ref().local_client_id()).test_value();
    let joining_client_network_id = u32::try_from(joining_client_id).test_value();
    main_assert_ne!(joining_client_id => client_id);
    main_assert!(client
        .network_lobby
        .test_ref()
        .participants
        .contains_key(&joining_client_network_id));
    let host_joining_player_ids = host
        .control_player_infos
        .client_info_ids(joining_client_id);
    let retained_joining_player_ids = client
        .control_player_infos
        .client_info_ids(joining_client_id);
    main_assert!(!host_joining_player_ids.is_empty());
    main_assert_eq!(retained_joining_player_ids => host_joining_player_ids);
    main_assert_eq!(client.control_player_infos.client_packet(joining_client_id) => host.control_player_infos.client_packet(joining_client_id));
    main_assert_eq!(joining_client.network_lobby.test_ref().scenario_label() => host_scenario.title);

    pump_live_restart_three_apps_until(
        &mut host,
        &mut client,
        &mut joining_client,
        "both clients to finish loading the restarted lobby",
        |_host, retained_client, joining_client| {
            [&retained_client, &joining_client]
                .into_iter()
                .all(|client| {
                    let progress = &client.admission_resources.present_percent;
                    !progress.is_empty() && progress.values().all(|present| *present == 100)
                })
        },
    );
    let round_two_host_routes = route_keys(&host);
    let round_two_client_routes = route_keys(&client);
    let round_two_joining_client_routes = route_keys(&joining_client);
    main_assert!(
        round_two_host_routes.len() > host_routes.len(),
        "the restarted host must own an additional real route for the newly admitted client"
    );
    main_assert!(
        !round_two_joining_client_routes.is_empty(),
        "the newly admitted client must own a real route before round two"
    );
    let host_scenario_core = host
        .admission_resources
        .resource_cores
        .values()
        .find(|core| {
            core.resource_type == clonk_network::HostResourceType::Scenario as u8
                && core.filename.as_bytes() == host_scenario.identifier.as_bytes()
        })
        .cloned()
        .test_value();
    main_assert_eq!(client.admission_resources.resource_cores.get(&host_scenario_core.id) => Some(&host_scenario_core));
    main_assert_eq!(joining_client.admission_resources.resource_cores.get(&host_scenario_core.id) => Some(&host_scenario_core));

    host.start_network_game_now().test_value();
    main_assert!(
        !host
            .status_text
            .starts_with("Unable to start prepared host"),
        "the restarted lobby must own a fresh round bootstrap: {}",
        host.status_text
    );
    pump_live_restart_three_apps_until(
        &mut host,
        &mut client,
        &mut joining_client,
        "the restarted synchronized round",
        |host, retained_client, joining_client| {
            matches!(host.mode, AppMode::Running)
                && matches!(retained_client.mode, AppMode::Running)
                && matches!(joining_client.mode, AppMode::Running)
        },
    );
    let network_progress = |app: &GameApp| {
        (
            app.engine.frame(),
            app.network_control_clock
                .map(NetworkControlClock::current_tick)
                .test_value(),
        )
    };
    let host_round_two_start = network_progress(&host);
    let client_round_two_start = network_progress(&client);
    let joining_client_round_two_start = network_progress(&joining_client);
    pump_live_restart_three_apps_until(
        &mut host,
        &mut client,
        &mut joining_client,
        "all three peers to execute synchronized round-two controls",
        |host, retained_client, joining_client| {
            let progressed = |app: &GameApp, start: (u64, i32)| {
                let current = network_progress(app);
                current.0 > start.0 && current.1 > start.1
            };
            progressed(host, host_round_two_start)
                && progressed(retained_client, client_round_two_start)
                && progressed(joining_client, joining_client_round_two_start)
        },
    );

    main_assert!(matches!(host.network_mode, Some(NetworkMode::Host(_))));
    main_assert!(matches!(client.network_mode, Some(NetworkMode::Client(_))));
    main_assert!(matches!(
        joining_client.network_mode,
        Some(NetworkMode::Client(_))
    ));
    main_assert_eq!(host.network.test_ref().local_client_id() => host_local_id);
    main_assert_eq!(client.network.test_ref().local_client_id() => client_local_id);
    main_assert_eq!(host.network.test_ref().local_addresses() => host_addresses);
    main_assert_eq!(route_keys(&host) => round_two_host_routes, "starting round two must preserve every retained and newly admitted host route");
    main_assert_eq!(route_keys(&client) => round_two_client_routes, "starting round two must preserve the retained client's route");
    main_assert_eq!(route_keys(&joining_client) => round_two_joining_client_routes, "starting round two must preserve the newly admitted client's route");
    main_assert!(host.startup_network_connection.is_none());
    main_assert!(client.startup_network_connection.is_none(), "round two must still use the retained worker instead of dialing again");
    main_assert!(client.pending_host_rejoin.is_none());
    let round_two_host_scenario = host.active_scenario.test_ref();
    let round_two_client_scenario = client.active_scenario.test_ref();
    let round_two_joining_client_scenario = joining_client.active_scenario.test_ref();
    main_assert_eq!(round_two_client_scenario.identifier => client_scenario.identifier);
    main_assert_eq!(round_two_client_scenario.title => round_two_host_scenario.title);
    // Fresh clients execute a local Combined<ID>.c4s transport artifact; the
    // matching network resource core above is their scenario identity.
    main_assert_eq!(round_two_joining_client_scenario.title => round_two_host_scenario.title);
    let round_two_host_packet = host
        .control_player_infos
        .client_packet(client_id)
        .test_value();
    let round_two_client_packet = client
        .control_player_infos
        .client_packet(client_id)
        .test_value();
    main_assert_eq!(without_round_player_lifecycle(&round_two_host_packet) => without_round_player_lifecycle(&host_player_packet), "round two may update lifecycle fields but must preserve the remote player's full identity and resource packet");
    main_assert_eq!(without_round_player_lifecycle(&round_two_client_packet) => without_round_player_lifecycle(&client_player_packet), "round two may update lifecycle fields but must preserve the local player's full identity and resource packet");
}

#[test]
fn a_league_round_restart_still_clears_the_live_session() {
    // Dropping the manager is what sends the league End, so a league session
    // that outlived its round would still be registered when the next
    // LeagueStart asked the same server to register it again
    // (src/C4Network2.cpp:259-272,748-763,2292-2303). The connection saving is
    // not worth a rejected Start.
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 0, 1);
    app.network_is_league = true;

    main_assert!(!app.network_round_restart_preserves_session(), "a league round must re-host so its registration is released");

    app.restart_current_scenario().test_value();

    main_assert!(app.network.is_none(), "the league session must be torn down before the next one registers");
}

#[test]
fn host_round_restart_clears_the_live_session_before_staging_the_next_host() {
    // C4Network2StartWaitDlg::OnBtnRestart queues the next mission and closes
    // its dialog as aborted; C4Network2::FinalInit answers that abort with
    // C4Network2::Clear, which sends the league End and closes NetIO before
    // the queued mission ever reaches InitNetworkHost and LeagueStart
    // (src/C4Network2Dialogs.cpp:580-584; src/C4Network2.cpp:591-604,748-763).
    // Dropping the manager is what sends that End here, so a session that
    // survives the restart leaves this host registered while the next Start
    // asks the same server to register it again.
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 0, 1);

    app.restart_current_scenario().test_value();

    main_assert!(app.network.is_none(), "the abandoned host session must be torn down before the next one registers");
    main_assert!(app.network_mode.is_none(), "a torn-down session leaves no host mode behind");
}

#[test]
fn restart_restore_team_obeys_mask_user_and_equal_team_guards() {
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
        app.network_mode = Some(NetworkMode::Host(
            game_over_fixture!(host: 0, "Host".to_string(), None),
        ));
        app.players.restart_restore_infos.what = mask;
        app.players.restart_restore_infos.players.insert(
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

    main_assert!(submitted(0, clonk_engine::PLAYER_INFO_TYPE_USER, 1, 2).is_empty(), "RESTORE_PlayerTeams must be selected");
    main_assert!(submitted(RESTART_RESTORE_PLAYER_TEAMS, clonk_engine::PLAYER_INFO_TYPE_SCRIPT, 1, 2,).is_empty(), "only current User rows run the restore hook");
    main_assert!(submitted(RESTART_RESTORE_PLAYER_TEAMS, clonk_engine::PLAYER_INFO_TYPE_USER, 2, 2,).is_empty(), "an already-restored team is a no-op");
}

#[test]
fn frontend_music_uses_catalog_once_per_startup_entry_and_toggle_restarts() {
    let _lock = env_lock().lock();
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    // The decoder sniffs the payload; naming valid WAV bytes `.mid` lets
    // this test exercise catalog-extension handling without FluidSynth.
    fs::write(global.join("Frontend.mid"), silent_pcm_wav(1_000)).test_value();

    let mut app = new_menu_app(320, 200);
    {
        let mut audio = app.test_audio_mut();
        audio.stop_music();
        audio.music_resolver =
            MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
        audio.options.menu_music_enabled = false;
        audio.set_scenario_music_level(Some(25));
        let stale_recent = Arc::clone(
            &audio
                .music_resolver
                .resolve("Frontend.mid")
                .test_value()
                .identity,
        );
        lock_unpoisoned(&audio.music_control).most_recently_played = Some(stale_recent);
    }

    app.begin_frontend_music_entry();
    main_assert!(app.sound.frontend_attempted_for_entry);
    {
        let audio = app.test_audio_ref();
        main_assert_eq!(audio.music_resolver.playlist.as_deref() => Some("Frontend.*"));
        main_assert_eq!(
            audio
                .music_resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()) =>
            Some(b"Frontend.mid".as_slice()),
            "all C++ music extensions resolve through the frontend playlist"
        );
        main_assert_eq!(lock_unpoisoned(&audio.music_control).scenario_level => None);
        main_assert!(lock_unpoisoned(&audio.music_control).most_recently_played.is_none());
        main_assert!(!audio.music_is_playing());
    }

    app.set_frontend_music_option(true).test_value();
    let first_generation = {
        let audio = app.test_audio_ref();
        let generation = lock_unpoisoned(&audio.music_control).generation;
        generation
    };

    let wait_for_mixer_start = |app: &GameApp| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.test_audio_ref().system.music_is_playing() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        app.test_audio_ref().system.music_is_playing()
    };
    main_assert!(wait_for_mixer_start(&app), "Frontend.mid starts playback");

    let mixer = Arc::clone(app.test_audio_ref().system.mixer());
    let mut output = vec![0_i16; mixer.sample_rate() as usize * 2 * 2];
    mixer.mix_i16(&mut output);
    main_assert!(!app.test_audio_ref().system.music_is_playing(), "draining past the asset end proves frontend music is non-looping");

    app.test_update();
    app.ensure_menu_music();
    main_assert_eq!(
        lock_unpoisoned(&app.test_audio_ref().music_control).generation =>
        first_generation,
        "frontend navigation does not pump an ended track"
    );

    app.return_to_menu();
    main_assert!(app.sound.frontend_attempted_for_entry);
    main_assert!(wait_for_mixer_start(&app), "a new startup entry restarts frontend music");

    app.set_frontend_music_option(false).test_value();
    main_assert!(!app.test_audio_ref().options.menu_music_enabled);
    main_assert!(!app.test_audio_ref().system.music_is_playing());
    app.set_frontend_music_option(true).test_value();
    main_assert!(wait_for_mixer_start(&app), "FEMusic re-enable restarts the frontend playlist");

    app.sound.runtime_music_enabled = false;
    app.play_sandbox_audio();
    main_assert_eq!(app.test_audio_ref().music_resolver.playlist => None, "game entry restores the default playlist");
}

#[test]
fn playlist_restart_selection_randomizes_new_matches_and_named_lookup_bypasses_filter() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    for name in ["A.ogg", "B.ogg", "C.ogg"] {
        fs::write(global.join(name), name.as_bytes()).test_value();
    }
    let mut resolver =
        MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    resolver.set_playlist(Some("B.*;C.*".to_string()));

    main_assert_eq!(
        resolver
            .resolve("A")
            .map(|asset| asset.file_name_bytes.as_slice()) =>
        Some(b"A.ogg".as_slice()),
        "an explicit Music(\"Name\") lookup ignores the default playlist"
    );
    let selected = resolver
        .select_default_with(None, |range| {
            main_assert_eq!(range => 2);
            1
        })
        .test_value();
    main_assert_eq!(selected.file_name_bytes => b"C.ogg", "a restarted playlist uses the random choice instead of its first match");
}

#[test]
fn set_music_playlist_command_restarts_only_when_enabled_at_its_event_position() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Frontend.ogg"), silent_pcm_wav(20)).test_value();

    let group = Group::open(&global).test_value();
    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.music_resolver = MusicResolver::with_global_group(group).test_value();
    let fixture = audio.system.load_music(&silent_pcm_wav(20)).test_value();
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
    main_assert_eq!(
        lock_unpoisoned(&audio.music_control).generation =>
        initial_generation,
        "a restart request must not start music while Game.IsMusicEnabled is false"
    );
    main_assert_eq!(
        audio
            .music_resolver
            .first_default()
            .map(|asset| asset.file_name_bytes.as_slice()) =>
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
    main_assert_ne!(lock_unpoisoned(&audio.music_control).generation => initial_generation, "an enabled restart replaces the current music generation");
    main_assert!(audio.complete_next_controlled_music_load().expect("complete enabled playlist restart"));

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
    main_assert!(!runtime_music_enabled);
    main_assert_eq!(
        lock_unpoisoned(&audio.music_control).generation =>
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
    main_assert!(runtime_music_enabled);
    main_assert_eq!(
        lock_unpoisoned(&audio.music_control).generation =>
        before_stop_restart_play + 1,
        "StopMusic suppresses the intervening restart before the later PlayMusic"
    );
}

#[test]
fn queued_playlist_restart_uses_its_command_time_filter() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    for name in ["A.ogg", "B.ogg", "C.ogg"] {
        fs::write(global.join(name), name.as_bytes()).test_value();
    }

    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.music_resolver =
        MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    let b_identity = Arc::clone(&audio.music_resolver.resolve("B").test_value().identity);
    let fixture = audio.system.load_music(&silent_pcm_wav(20)).test_value();
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
    main_assert_eq!(audio.queued_music_starts.len() => 1);
    main_assert_eq!(audio.music_resolver.playlist.as_deref() => Some("C.*"));

    main_assert!(audio.complete_next_controlled_music_load().expect("complete named predecessor"));
    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    main_assert!(controlled.requests.front().and_then(|request| request.identity.as_ref()).is_some_and(|identity| Arc::ptr_eq(identity, &b_identity)));
}

#[test]
fn running_global_gui_guard_precedes_scoreboard_and_root_overlay_pixels() {
    let check = |mut app: GameApp, label: &str| {
        app.dialogs.scoreboard_initial_reconcile_pending = true;
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
        main_assert_eq!(runtime_global_ui_snapshot(&app) => before, "{label}");
        main_assert!(frame.iter().all(|byte| *byte == 0x73), "{label}");
    };

    check(new_running_sandbox_app(), "base running view");

    let mut context = new_running_sandbox_app();
    context
        .open_context_menu_at(Vec::new(), GuiPoint::new(20.0, 20.0))
        .test_value();
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
        .test_value();
    check(message, "running message");

    let mut menu = new_running_sandbox_app();
    menu.ingame_menu.replace(
        menu.players.local_owner,
        Some(IngameMenuState::surrender_menu(&IngameMenuLabels::default())),
    );
    check(menu, "running player menu");

    let mut evaluation = new_classic_running_sandbox_app();
    evaluation.handle_game_over().test_value();
    check(evaluation, "running evaluation");
}

#[test]
fn abort_action_opens_confirmation_with_control_host_restart() {
    let mut app = new_menu_app(320, 200);
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    app.apply_ingame_menu_action(MenuAction::Abort).test_value();
    let dialog = app.dialogs.messages.last().test_value();
    main_assert_eq!(dialog.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::YES_RESTART_NO);
    main_assert_eq!(dialog.state.size() => clonk_frontend::message_dialog::MessageDialogSize::Fixed(400));
    main_assert_eq!(dialog.state.focused_button() => Some(clonk_frontend::message_dialog::MessageDialogButton::Yes));
    main_assert!(matches!(app.mode, AppMode::Running));
}

/// `C4Game::ShowGameOverDlg` builds no dialog in a console engine — it waits
/// out any pending stream and calls `Application.QuitGame()` directly
/// (src/C4Game.cpp:3679-3690), where the graphical build instead shows
/// `C4GameOverDlg` and pauses behind it. An unattended server has no renderer
/// to draw that dialog and no input to dismiss it, so pausing there would
/// wedge the round forever.
#[test]
fn headless_round_end_quits_instead_of_pausing_behind_an_evaluation_dialog() {
    // The graphical path is the control: it does open the dialog and pause.
    let mut windowed = running_browser_sandbox(ScenarioSelectorMode::Local);
    windowed.handle_game_over().test_value();
    main_assert!(windowed.game_over_dialog.is_some());
    main_assert!(windowed.runtime_halt_active());
    main_assert!(!windowed.take_exit_request());

    // A server launched with a command-line scenario has no startup
    // generation to return to: `ParseCommandLine` clears `UseStartupDialog`
    // for a nonempty `ScenarioFilename` (C4Game.cpp:3299), so `QuitGame`
    // falls through to `Quit()` (C4Application.cpp:373-405).
    let mut server = running_browser_sandbox(ScenarioSelectorMode::Local);
    server.headless = true;
    server.classic_command_line.scenario = Some(PathBuf::from("Server.c4s"));
    server.handle_game_over().test_value();
    main_assert!(server.game_over_dialog.is_none(), "a dedicated server draws no evaluation dialog");
    main_assert!(!server.runtime_halt_active(), "and must not pause behind the dialog it never drew");
    main_assert!(server.take_exit_request());
}

#[test]
fn a_console_opened_round_parks_the_server_for_the_next_open() {
    // `/open` sets `UseStartupDialog` back (C4Application.cpp:598-612), so the
    // round end's `QuitGame` reconstructs the startup state rather than
    // reaching `Quit()` (C4Application.cpp:373-405) — and a console engine
    // then "just stay[s] in this state until aborted or new commands arrive on
    // stdin" (C4Application.cpp:428-429). Exiting instead forces an operator to
    // supervise the process for every round.
    let mut server = running_browser_sandbox(ScenarioSelectorMode::Local);
    server.headless = true;
    server.console_restored_startup_dialog = true;

    server.handle_game_over().test_value();

    main_assert!(server.game_over_dialog.is_none(), "a dedicated server still draws no evaluation dialog");
    main_assert!(!server.take_exit_request(), "but it parks for the next /open instead of ending the process");
    main_assert!(server.console_startup_active(), "and it parks in the startup state that /open is accepted from");

    // The finished round must not be left on the command line, or the parked
    // server would relaunch it instead of waiting.
    main_assert_eq!(server.classic_command_line.scenario => None);

    // A second round starts in this same process. Boot deferral stands in for
    // the scenario load here, the way `console_open_close_and_message_fallback_
    // follow_app_state` does: the point is that `/open` is accepted and its
    // parameters land, not that the file exists.
    let (_boot_sender, boot_receiver) = mpsc::channel();
    server.boot_loading = Some(BootLoadingState::new(boot_receiver));
    server
        .process_console_command("/open \"Missions/Second Round/Scenario.txt\"")
        .test_value();
    main_assert_eq!(server.classic_command_line.scenario => Some(PathBuf::from("Missions/Second Round")));
    main_assert!(server.auto_start_classic_command_line_scenario, "the operator's next round is queued without restarting the process");
    main_assert!(!server.take_exit_request());
}

#[test]
fn local_round_abort_and_evaluation_end_restore_fresh_browser() {
    let mut aborted = running_browser_sandbox(ScenarioSelectorMode::Local);
    confirm_abort_dialog(&mut aborted);
    assert_l038_browser_return(&aborted, ScenarioSelectorMode::Local);

    let mut evaluated = running_browser_sandbox(ScenarioSelectorMode::Local);
    evaluated.handle_game_over().test_value();
    main_assert!(evaluated.game_over_dialog.is_some());
    evaluated
        .handle_game_over_action(GameOverAction::End)
        .test_value();
    assert_l038_browser_return(&evaluated, ScenarioSelectorMode::Local);
}

#[test]
fn reload_button_and_f5_restart_and_repopulate_search() {
    fn exercise(use_f5: bool, title: &str) {
        let listener = std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0)).test_value();
        listener.set_nonblocking(true).test_value();
        let master_address = listener.local_addr().test_value();

        let discovery_port = std::net::UdpSocket::bind((std::net::Ipv6Addr::LOCALHOST, 0))
            .expect("reserve discovery port")
            .local_addr()
            .test_value()
            .port();
        let mut app = new_classic_menu_app(800, 600);
        let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(
            app.assets.clonk_fonts.as_deref().test_value(),
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
        app.startup.view = StartupView::NetworkGame;
        app.startup_network_dialog = Some(dialog);
        app.startup_game_search = Some(
            clonk_network::StartupGameSearch::start(clonk_network::NetworkGameSearchConfig {
                internet_enabled: true,
                use_alternate_server: false,
                master_server_url: format!("http://{master_address}/"),
                discovery_port,
            })
            .test_value(),
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
        main_assert_eq!(app.startup_network_dialog.as_ref().unwrap().games().len() => 2);

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
                        Err(error) => panic!("accept masterserver request: {error}"),
                    }
                };
                stream.set_nonblocking(false).test_value();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .test_value();
                let mut request = [0_u8; 4096];
                let size = stream.read(&mut request).test_value();
                main_assert!(String::from_utf8_lossy(&request[..size]).starts_with("GET / HTTP/1.1"));
                let body = format!(
                        "[Reference]\nTitle=\"{server_title}\"\nState=Lobby\nJoinAllowed=1\nAddress=TCP:\"127.0.0.1:31112\"\nGame=LegacyClonk\nVersion=4,9,11,0\nBuild=362\n"
                    );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .test_value();
                stream.write_all(body.as_bytes()).test_value();
                true
            })
        };

        if use_f5 {
            app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
            app.test_key(VirtualKeyCode::F5, ElementState::Released);
        } else {
            let reload = layout.buttons[1];
            app.test_cursor(PhysicalPosition::new(
                f64::from(reload.x + reload.w / 2),
                f64::from(reload.y + reload.h / 2),
            ));
            app.test_left_button(ElementState::Pressed);
            app.test_left_button(ElementState::Released);
        }
        // The listener is already bound, so a promptly scheduled client
        // can wait in the kernel backlog. Arm the fixture deadline only
        // after the refresh command has been delivered to the worker.
        let server = start_server();

        main_assert!(app.startup_game_references.is_empty());
        main_assert!(app.startup_direct_reference_queries.is_empty());
        main_assert!(app.startup_network_dialog.as_ref().unwrap().games().is_empty());
        main_assert!(app.netdlg_last_click.is_none());
        main_assert!(app.status_text.is_empty(), "query presentation belongs to the native masterserver row");
        main_assert!(!app.take_exit_request());

        let deadline = Instant::now() + Duration::from_secs(14);
        while !app
            .startup_game_references
            .iter()
            .any(|reference| reference.title == title)
            && Instant::now() < deadline
        {
            app.poll_startup_game_search().test_value();
            // Hosts without a usable multicast route report the explicit
            // LAN probe failure in a modal, and the modal's Sec1 timer freeze
            // holds the queued masterserver result until it is dismissed.
            if app
                .dialogs.messages
                .last()
                .is_some_and(|dialog| dialog.state.caption() == "Search Error")
            {
                app.finish_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogResult::Cancel,
                )
                .test_value();
            }
            thread::sleep(Duration::from_millis(10));
        }
        main_assert!(server.join().expect("masterserver fixture thread"));
        main_assert_eq!(app.startup_game_references.iter().map(|reference| reference.title.as_str()).collect::<Vec<_>>() => [title]);
        main_assert_eq!(app.startup_network_dialog.as_ref().unwrap().games().len() => 1);
        main_assert!(app.status_text.is_empty(), "result presentation belongs to the native query/game rows");
        main_assert_eq!(app.startup.view => StartupView::NetworkGame);
        main_assert!(!app.take_exit_request());
    }

    exercise(false, "Reload button result");
    exercise(true, "F5 result");
}

#[test]
fn subsecond_refresh_only_plays_error_and_preserves_rows() {
    let sound_root = tempdir();
    let scenario = sound_root.path().join("Cooldown.c4s");
    fs::create_dir(&scenario).test_value();
    fs::write(scenario.join("Error.wav"), silent_pcm_wav(1_000)).test_value();

    let mut app = new_classic_menu_app(800, 600);
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(
        app.assets.clonk_fonts.as_deref().test_value(),
    );
    let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig::default(),
        metrics,
    );
    dialog.resize(800, 600);
    app.startup.view = StartupView::NetworkGame;
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
        .test_value()
        .games()
        .to_vec();
    {
        let mut audio = app.test_audio_mut();
        audio.options.menu_sound_enabled = true;
        audio.configure_scenario(Some(&scenario));
        audio.missing_sounds.clear();
    }

    app.request_startup_network_refresh_at(now + Duration::from_millis(999))
        .test_value();

    main_assert_eq!(app.startup_network_last_refresh => Some(now));
    main_assert_eq!(app.startup_game_references => expected_references);
    main_assert_eq!(app.startup_direct_reference_queries => expected_queries);
    main_assert_eq!(app.startup_network_dialog.as_ref().unwrap().games() => expected_games);
    main_assert_eq!(app.status_text => "Retained status");
    main_assert_eq!(app.netdlg_last_click => Some((0, now)));
    main_assert!(app.dialogs.messages.is_empty());
    {
        let audio = app.test_audio_ref();
        main_assert!(audio.loaded_sounds.keys().any(|key| key.to_ascii_lowercase().contains("error.wav")), "the rejected refresh must request only the Error GUI sound");
        main_assert!(audio.missing_sounds.is_empty());
    }
    main_assert!(!app.take_exit_request());
}

#[test]
fn running_chat_raw_gamepad_owner_outranks_game_over_source_eligibility() {
    let mut app = new_game_over_keyboard_app();
    let mut config = Config::new();
    config.set_in(
        Some("Gamepad1"),
        "Button7",
        input::legacy_gamepad_axis_key(1, 0, false)
            .test_value()
            .to_string(),
    );
    app.gamepad_bindings = GamepadBindings::from_config(&config);
    app.local_controls.remove(app.players.local_owner);
    app.local_controls.initialize(LocalControlInit {
        owner: app.players.local_owner,
        preferred_set: GamepadSlot::new(1).control_set(),
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.start_running_chat(RunningChatMode::All);
    app.engine
        .test_player_mut(app.players.local_owner)
        .control
        .pressed_coms = 0;

    app.process_sourced_gamepad_event_batch(
        [
            SourcedGamepadEvent {
                gamepad: 1,
                cluster: 17,
                event: game_over_fixture!(axis:
                           GamepadSlot::new(1),
                           LegacyGamepadAxis::new(0, false),
                           ElementState::Pressed,
                       ),
            },
            SourcedGamepadEvent {
                gamepad: 1,
                cluster: 17,
                event: game_over_fixture!(direction: GamepadSlot::new(1), ControlButton::Left, ElementState::Pressed),
            },
        ],
        false,
    )
    .test_value();

    main_assert_ne!(app.engine.player(app.players.local_owner).expect("local sandbox player").control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0);
    main_assert!(app.game_over_dialog.is_some());
    main_assert!(app.chat.running.is_some());
    main_assert!(app.ingame_menu.is_none());
}

#[test]
fn named_remaps_drive_chat_scoreboard_abort_menu_and_player_candidates() {
    let config = parse_runtime_key_config(
                b"[Keys]\nChatOpen=G,Joy2A\nScoreboardToggle=H\nGameAbort=B\nFullscreenMenuDown=J\nKbd1Key1=Shift+T\nKbd1Key2=\\x0042000a\n",
            ).test_value();
    let mut app = new_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache.set(Ok(config)).test_value();

    main_assert!(app.handle_running_chat_open_key(VirtualKeyCode::KeyG, ElementState::Pressed));
    main_assert!(app.running_chat_active());
    app.close_running_chat().test_value();
    main_assert!(!app.handle_running_chat_open_key(VirtualKeyCode::Enter, ElementState::Pressed,));
    app.handle_gamepad_direction(
        GamepadSlot::new(1),
        ControlButton::Left,
        ElementState::Pressed,
    )
    .test_value();
    main_assert!(app.running_chat_active());
    app.close_running_chat().test_value();

    main_assert!(app.handle_scoreboard_key(VirtualKeyCode::KeyH, ElementState::Pressed).expect("custom scoreboard callback"));
    main_assert!(!app.handle_scoreboard_key(VirtualKeyCode::Tab, ElementState::Pressed).expect("replaced scoreboard default"));

    let shifted =
        app.runtime_control_candidates_for_keyboard(VirtualKeyCode::KeyT, ElementState::Pressed);
    main_assert!(shifted.is_empty(), "the custom chord requires Shift");
    app.live_input.modifiers = ModifiersState::SHIFT;
    main_assert_eq!(
        app.runtime_control_candidates_for_keyboard(VirtualKeyCode::KeyT, ElementState::Pressed,) =>
        vec![KeyboardBindings::control_candidate_for_set(
            0,
            ControlBindingId::CursorLeft,
            ElementState::Pressed,
        )
        .expect("first keyboard callback")]
    );
    app.live_input.modifiers = ModifiersState::empty();
    main_assert_eq!(
        app.runtime_control_candidates_for_gamepad_button(0, 0, ElementState::Pressed,) =>
        vec![KeyboardBindings::control_candidate_for_set(
            0,
            ControlBindingId::CursorToggle,
            ElementState::Pressed,
        )
        .expect("second keyboard callback")]
    );

    app.ingame_menu.replace(
        OWNER_NONE,
        IngameMenuState::main_menu(
            &MainMenuConditions {
                has_player: false,
                player_count: 2,
                ..MainMenuConditions::default()
            },
            &IngameMenuLabels::default(),
        ),
    );
    let before = app.ingame_menu.get(OWNER_NONE).test_value().selection();
    main_assert!(app.handle_runtime_fullscreen_menu_key(VirtualKeyCode::KeyJ, ElementState::Pressed,).expect("custom ownerless menu callback"));
    main_assert_ne!(app.ingame_menu.get(OWNER_NONE).expect("ownerless menu remains").selection() => before);
    app.ingame_menu.clear();

    app.test_key(VirtualKeyCode::KeyB, ElementState::Pressed);
    main_assert!(app.dialogs.messages.last().is_some_and(|dialog| matches!(dialog.continuation, MessageDialogContinuation::AbortGame { .. })));

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
        .test_value();
    context_priority
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(20.0, 20.0),
        )
        .test_value();
    context_priority.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(context_priority.context_menu.is_none());
    main_assert!(context_priority.dialogs.scoreboard.is_none());
    context_priority
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(20.0, 20.0),
        )
        .test_value();
    context_priority.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(context_priority.dialogs.scoreboard.is_none());
    context_priority.close_context_menu_silently();
    context_priority
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new("Remain open").with_hotkey('R')],
            GuiPoint::new(20.0, 20.0),
        )
        .test_value();
    context_priority.test_key(VirtualKeyCode::KeyR, ElementState::Pressed);
    main_assert!(context_priority.dialogs.scoreboard.is_none());

    let mut gamepad_priority = new_running_sandbox_app();
    gamepad_priority.runtime_key_config_cache = OnceLock::new();
    gamepad_priority
        .runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(b"[Keys]\nChatOpen=Joy1A\n")
            .expect("parse colliding gamepad chat chord")))
        .test_value();
    let mut gamepad_config = Config::new();
    gamepad_config.set_in(
        Some("Gamepad0"),
        "Button7",
        input::legacy_gamepad_axis_key(0, 0, false)
            .test_value()
            .to_string(),
    );
    gamepad_priority.gamepad_bindings = GamepadBindings::from_config(&gamepad_config);
    gamepad_priority.local_controls = LocalControlRegistry::default();
    gamepad_priority
        .local_controls
        .initialize(LocalControlInit {
            owner: gamepad_priority.players.local_owner,
            preferred_set: 4,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
    gamepad_priority.test_gamepad_events([
        game_over_fixture!(axis: GamepadSlot::new(0), LegacyGamepadAxis::new(0, false), ElementState::Pressed),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Left, ElementState::Pressed),
    ]);
    main_assert!(!gamepad_priority.running_chat_active());
    main_assert_ne!(
        gamepad_priority
            .engine
            .player(gamepad_priority.players.local_owner)
            .expect("local gamepad player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT) =>
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
        .test_value();
    chat_priority.start_running_chat(RunningChatMode::All);
    chat_priority.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    main_assert_eq!(chat_priority.running_chat_text() => Some(""));

    let mut game_over_chat = new_game_over_keyboard_app();
    game_over_chat.runtime_key_config_cache = OnceLock::new();
    game_over_chat
        .runtime_key_config_cache
        .set(Ok(
            parse_runtime_key_config(b"[Keys]\nChatOpen=G\n").expect("parse game-over chat remap")
        ))
        .test_value();
    game_over_chat.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(game_over_chat.running_chat_active());
    main_assert_eq!(game_over_chat.running_chat_text() => Some(""));
}

#[test]
fn observer_and_game_over_use_the_ownerless_fullscreen_camera() {
    let mut observer = new_running_sandbox_app();
    observer.snapshot.hud.local_players.clear();
    let observer_inputs = collect_viewport_inputs(&observer.snapshot).test_value();
    main_assert_eq!(observer_inputs.len() => 1);
    main_assert_eq!(observer_inputs[0].owner => OWNER_NONE);

    let mut game_over_observer = new_classic_running_sandbox_app();
    game_over_observer
        .assets
        .require_classic_game_over_resources()
        .test_value();
    game_over_observer.handle_game_over().test_value();
    game_over_observer.status_text.clear();
    game_over_observer.snapshot.hud.local_players.clear();
    main_assert!(game_over_observer.game_over_dialog.is_some());
    let game_over_inputs = collect_viewport_inputs(&game_over_observer.snapshot).test_value();
    main_assert_eq!(game_over_inputs.len() => 1);
    main_assert_eq!(game_over_inputs[0].owner => OWNER_NONE);
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
        result_string: clonk_engine::LegacyCString::from_bytes(b"evaluated".to_vec()).test_value(),
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
        .test_value();

    app.test_network_events();

    let info = app.control_player_infos.get(10).test_value();
    main_assert_eq!((info.league_score, info.league_rank, info.league_rank_symbol) => (0, 0, 0), "EvaluateLeague does not overwrite live PlayerInfo");
    let engine_snapshot = app.engine.snapshot();
    main_assert_eq!(app.snapshot.round_results => engine_snapshot.round_results);
    main_assert_eq!(engine_snapshot.round_results.network_result => Some(clonk_engine::RoundResultsNetworkResult::LeagueOk));
    main_assert_eq!(engine_snapshot.round_results.network_result_message.as_slice() => &b"evaluated"[..]);
    let result = engine_snapshot
        .round_results
        .players
        .iter()
        .find(|result| result.player_info_id == 10)
        .test_value();
    main_assert_eq!((result.score_old, result.score_new) => (-1, None));
    main_assert_eq!((result.league_score_new, result.league_score_gain, result.league_rank_new, result.league_rank_symbol_new,) => (80, 5, 3, 4));
    main_assert_eq!(result.league_progress_data.as_deref() => Some(&b"progress"[..]));
}

#[test]
fn synchronized_activation_restarts_playerless_activity_window() {
    // C4Client::SetActivated(true) stamps the current FrameCounter. The
    // strict delay begins at synchronized execution, not at connection or
    // request time (src/C4Client.cpp:104-110; src/C4Control.cpp:589-602).
    let mut app = new_running_sandbox_app();
    for _ in 0..400 {
        app.engine.test_tick();
    }
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(
        game_over_fixture!(host: 0, "Host".to_string(), None),
    ));
    app.network_control_running = false;
    app.control_clients.register(3, false, false);
    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientUpdate(
            game_over_fixture!(client_update: clonk_engine::CLIENT_UPDATE_ACTIVATE, 3, 1, 0),
        )],
    )
    .test_value();

    for _ in 0..500 {
        app.engine.test_tick();
    }
    app.test_update();
    main_assert!(commands.take_submitted_client_updates().is_empty());

    app.engine.test_tick();
    app.test_update();
    main_assert_eq!(commands.take_submitted_client_updates() => vec![game_over_fixture!(client_update: clonk_engine::CLIENT_UPDATE_ACTIVATE, 3, 0, 0)]);
}

// C4GoalDisplay::GoalPicture resolves the goal as a *live object* through
// Game.Objects.FindInternal and draws through
// `C4Def::Draw(Picture, false, 0, pGoalObj)`, so the picture carries that
// object's graphics rather than the bare definition picture; only then is
// the unfulfilled grayscale applied
// (src/C4GameOverDlg.cpp:52-63; src/C4GameObjects.cpp:264-268).
#[test]
fn game_over_goal_picture_includes_live_goal_object_overlays() {
    let mut app = new_classic_running_sandbox_app();
    let mut definition = test_definition("GOAL", "Goal", "#strict 3\n");
    definition.set_picture(Some(clonk_engine::DefinitionPicture {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }));
    definition.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
        width: 2,
        height: 1,
        pixels: Arc::from([0x11, 0x22, 0x33, 0xff, 0x44, 0x55, 0x66, 0xff]),
        color_mask: None,
    }));
    app.engine.register_test_definition(definition);
    app.snapshot.round_results.goals = vec!["GOAL".into()];
    app.snapshot.round_results.fulfilled_goals = vec!["GOAL".into()];

    // This fixture has no cached definition Picture facet, so a
    // definition-only freeze produces nothing to draw.
    app.snapshot.objects.clear();
    main_assert!(app.engine.definition_picture_image("GOAL").is_none());
    app.handle_game_over().test_value();
    main_assert!(
        app.game_over_dialog
            .as_ref()
            .expect("evaluation dialog")
            .evaluation()
            .goals()[0]
            .picture
            .is_none(),
        "without a live goal object there is only the definition picture"
    );
    app.game_over_dialog = None;
    app.game_over_handled = false;

    // A live goal object draws through its own graphics, which this
    // definition does have.
    app.engine.spawn_test_object(SpawnConfig::new("GOAL"));
    app.snapshot = app.engine.snapshot();
    app.snapshot.round_results.goals = vec!["GOAL".into()];
    app.snapshot.round_results.fulfilled_goals = vec!["GOAL".into()];
    let object = app
        .snapshot
        .objects
        .iter()
        .find(|candidate| candidate.definition_id.as_str() == "GOAL")
        .test_value();
    let expected = app.engine.object_picture_image(object).test_value();
    app.handle_game_over().test_value();
    let picture = app.game_over_dialog.test_ref().evaluation().goals()[0]
        .picture
        .clone()
        .test_value();
    main_assert_eq!(picture.width() => expected.width());
    main_assert_eq!(picture.height() => expected.height());
    main_assert_eq!(picture.pixels() => expected.pixels().as_ref());
}

#[test]
fn game_over_goal_hover_uses_localized_cpp_tooltips_and_shared_delay() {
    let mut app = new_classic_running_sandbox_app();
    for (id, name, description) in [
        ("GFDN", "Build the %s bridge", "Reach the other side"),
        ("GOPN", "Find the gold", "Recover the treasure"),
    ] {
        let mut definition = test_definition(id, name, "#strict 3\n");
        definition.set_description(Some(description.to_string()));
        app.engine.register_test_definition(definition);
    }
    app.snapshot.round_results.goals = vec!["GFDN".into(), "GOPN".into()];
    app.snapshot.round_results.fulfilled_goals = vec!["GFDN".into()];
    app.handle_game_over().test_value();

    let goal_rects = {
        let surface = app.graphics.surface();
        let dialog = app.game_over_dialog.test_ref();
        main_assert_eq!(
            dialog
                .evaluation()
                .goals()
                .iter()
                .map(|goal| goal.tooltip.as_str())
                .collect::<Vec<_>>() =>
            vec![
                "Goal Build the %s bridge fulfilled: Reach the other side",
                "Goal Find the gold not fulfilled: Recover the treasure",
            ]
        );
        let layout = dialog.classic_evaluation_layout(
            surface.width(),
            surface.height(),
            app.assets.clonk_fonts.as_deref().test_value(),
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
        app.test_cursor(PhysicalPosition::new(
            f64::from(rect.x + rect.w / 2),
            f64::from(rect.y + rect.h / 2),
        ));
        main_assert_eq!(app.game_over_dialog.as_ref().expect("evaluation dialog").hovered_description() => expected);
    }

    let first = goal_rects[0];
    let first_center = GuiPoint::new(
        (first.x + first.w / 2) as f32,
        (first.y + first.h / 2) as f32,
    );
    app.test_cursor(PhysicalPosition::new(
        f64::from(first_center.x),
        f64::from(first_center.y),
    ));
    let started = Instant::now() - clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY;
    app.startup_tooltip = ClassicTooltipTracker::new_at(started);
    app.startup_tooltip
        .note_pointer_move_at(first_center, started);
    main_assert_eq!(app.startup_tooltip.eligible_pointer() => Some(first_center));
    main_assert!(app.render_game_over_tooltip(Some(startup_gamma())).expect("draw classic delayed tooltip"));

    // A newer dialog can consume motion and then close before the shared
    // delay expires. Re-resolve at the tracker's current pointer instead
    // of drawing this dialog's stale cached goal hover there.
    let consumed_pointer = GuiPoint::new(0.0, 0.0);
    let started = Instant::now() - clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY;
    app.startup_tooltip = ClassicTooltipTracker::new_at(started);
    app.startup_tooltip
        .note_pointer_move_at(consumed_pointer, started);
    main_assert_eq!(
        app.game_over_dialog
            .as_ref()
            .expect("evaluation dialog")
            .hovered_description() =>
        "Goal Build the %s bridge fulfilled: Reach the other side",
        "the lower dialog intentionally retains its last routed hover"
    );
    main_assert!(!app.render_game_over_tooltip(Some(startup_gamma())).expect("ignore stale lower-dialog hover"));
}

#[test]
fn game_over_custom_text_wheel_uses_app_routing_and_stays_below_newer_dialogs() {
    let mut app = new_classic_running_sandbox_app();
    app.snapshot.round_results.custom_evaluation_strings = (0..40)
        .map(|index| format!("Line {index}"))
        .collect::<Vec<_>>()
        .join("|");
    app.handle_game_over().test_value();
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    let custom = app
        .game_over_dialog
        .test_ref()
        .classic_evaluation_layout(
            width,
            height,
            app.assets.clonk_fonts.as_deref().expect("classic fonts"),
        )
        .custom_evaluation
        .test_value();
    main_assert!(custom.scrollable);
    app.test_cursor(PhysicalPosition::new(
        f64::from(custom.viewport.x + 1),
        f64::from(custom.viewport.y + 1),
    ));

    let mut before = vec![0_u8; (width * height * 4) as usize];
    app.test_render(&mut before);
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert_eq!(app.game_over_dialog.as_ref().expect("evaluation dialog").custom_evaluation_scroll() => 60);
    let mut after = vec![0_u8; (width * height * 4) as usize];
    app.test_render(&mut after);
    main_assert_ne!(before => after, "routing the wheel into the evaluation must change the rendered frame");

    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    main_assert!(app.runtime_client_list_owns_game_over());
    app.live_input.running_pointer = Some(GuiPoint::new(0.0, 0.0));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert_eq!(app.game_over_dialog.as_ref().expect("evaluation dialog").custom_evaluation_scroll() => 60);
}

/// `C4GraphicsResource::Init` refuses the whole graphics load for any missing
/// mandatory game/HUD file (C4GraphicsResource.cpp:200-231), so those facets
/// are reported by the earlier global gate rather than by the game-over
/// dialog's own recursive presentation inventory.
fn assert_hud_resource_boundary(error: &anyhow::Error, expected_missing: Vec<&'static str>) {
    main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&ClassicParityBoundary::HudResources {missing: expected_missing}));
    main_assert!(error.to_string().contains("refusing generic Rust fallback"), "boundary must explain why the fallback is unreachable: {error:#}");
}

fn assert_game_over_resource_boundary(error: &anyhow::Error, expected_missing: Vec<&'static str>) {
    let expected = ClassicParityBoundary::GameOverResources {
        missing: expected_missing.into_iter().map(str::to_string).collect(),
    };
    main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&expected));
    main_assert!(error.to_string().contains("refusing generic Rust fallback"), "boundary must explain why the fallback is unreachable: {error:#}");
}

fn assert_startup_game_over_boundary(error: &anyhow::Error, view: StartupView) {
    let expected = ClassicParityBoundary::StartupGameOver { view };
    main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&expected));
    main_assert!(error.to_string().contains("running-mode only"), "boundary must identify the invalid lifecycle state: {error:#}");
}

#[test]
fn game_over_missing_resources_fail_typed_before_touching_output_frame() {
    let mut app = new_game_over_keyboard_app();
    let assets = Arc::get_mut(&mut app.assets).test_value();
    assets
        .startup_dialog_images
        .remove("Player.png")
        .test_value();
    let hud = Arc::make_mut(&mut assets.hud_graphics);
    hud.player = None;
    hud.score = None;
    app.dialogs.scoreboard_initial_reconcile_pending = true;
    let before = runtime_global_ui_snapshot(&app);
    let mut frame = vec![0x5a; 320 * 200 * 4];
    let sentinel = frame.clone();

    let error = app
        .render(&mut frame)
        .expect_err("asset-less game over must not render a fallback");

    assert_hud_resource_boundary(&error, vec!["Score.png", "Player.png"]);
    main_assert_eq!(frame => sentinel, "preflight must precede every output write");
    main_assert_eq!(runtime_global_ui_snapshot(&app) => before);
}

#[test]
fn game_over_recursive_inventory_covers_global_sheets_crew_and_frozen_images() {
    let mut app = new_game_over_keyboard_app();
    let (gui_icons2, gui_scroll) = {
        let assets = Arc::get_mut(&mut app.assets).test_value();
        let gui_icons2 = assets
            .startup_dialog_images
            .remove("GUIIcons2.png")
            .test_value();
        let gui_scroll = assets
            .startup_dialog_images
            .remove("GUIScroll.png")
            .test_value();
        Arc::make_mut(&mut assets.hud_graphics).crew = None;
        (gui_icons2, gui_scroll)
    };
    let error = app
        .assets
        .require_classic_game_over_resources()
        .expect_err("recursive direct inventory rejects missing child resources");
    main_assert_eq!(
        error =>
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
    let boundary = error.downcast_ref::<ClassicParityBoundary>().test_value();
    main_assert!(matches!(boundary, ClassicParityBoundary::GlobalGuiBootstrapResources { .. }));
    main_assert_eq!(frame => sentinel);

    // Once the process-global sheets are restored, the recursive child
    // inventory owns Crew and must still fail before any output pixels.
    {
        let assets = Arc::get_mut(&mut app.assets).test_value();
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
    assert_hud_resource_boundary(&error, vec!["Crew.png"]);
    main_assert_eq!(frame => sentinel);

    let mut app = new_game_over_keyboard_app();
    let invalid = ImageData::new(1, 1, Vec::new());
    app.game_over_dialog
        .test_mut()
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
                league_rank_symbol: None,
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
    main_assert_eq!(frame => sentinel);
}

#[test]
fn game_over_freezes_cached_player_big_icon_when_portraits_are_hidden() {
    let mut app = new_classic_running_sandbox_app();
    app.display_flags.portraits = false;
    let player_info_id = app.snapshot.players.first().test_value().player_info_id;
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
    app.startup.player_files.insert(
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

    app.handle_game_over().test_value();
    main_assert_eq!(app.runtime_player_big_icons.get(&player_info_id) => Some(&icon), "evaluation hydration must ignore the viewport portrait switch");
    app.runtime_player_big_icons.remove(&player_info_id);
    main_assert_eq!(
        app.game_over_dialog
            .as_ref()
            .expect("evaluation dialog")
            .evaluation()
            .player_by_info_id(player_info_id)
            .and_then(|player| player.big_icon.as_ref()) =>
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
    let player_info_id = app.snapshot.players.first().test_value().player_info_id;
    let icon = ImageData::new(1, 1, vec![9, 8, 7, 255]);
    let file_name = "Departed.c4p".to_string();
    app.control_player_infos.replace_snapshot(
        player_info_id,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: player_info_id,
                name: LegacyCString::from_bytes(b"Departed".to_vec()).expect("fixture player name"),
                filename: LegacyCString::from_bytes(file_name.as_bytes().to_vec())
                    .expect("fixture player filename"),
                ..Default::default()
            }],
            by_client: 0,
        }],
    );
    app.startup.player_files.insert(
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
    main_assert_eq!(app.runtime_player_big_icons.get(&player_info_id) => Some(&icon), "evaluation copies the icon while the player still exists");

    // Its player file and resource then depart, so nothing can supply the
    // icon any more.
    app.startup.player_files.clear();
    app.control_player_infos
        .replace_snapshot(player_info_id + 1, []);
    app.snapshot.round_results.players = vec![clonk_engine::RoundResultsPlayerState {
        player_info_id,
        ..clonk_engine::RoundResultsPlayerState::default()
    }];

    app.handle_game_over().test_value();
    main_assert_eq!(
        app.game_over_dialog
            .as_ref()
            .expect("evaluation dialog")
            .evaluation()
            .player_by_info_id(player_info_id)
            .and_then(|player| player.big_icon.as_ref()) =>
        Some(&icon),
        "the dialog consumes the elimination-time snapshot"
    );
}

#[test]
fn game_over_rejects_an_extended_sheet_without_the_native_league_facet() {
    let mut app = new_game_over_keyboard_app();
    let invalid = ImageData::new(64, 64, vec![0xff; 64 * 64 * 4]);
    Arc::get_mut(&mut app.assets)
        .test_value()
        .startup_dialog_images
        .insert("GUIIcons2.png".to_string(), invalid);

    let error = app
        .assets
        .require_classic_game_over_resources()
        .expect_err("evaluation resources must contain the native league facet");
    main_assert_eq!(error => ClassicParityBoundary::GameOverResources {missing: vec!["GUIIcons2.png (Ico:League)".to_string()],});
}

#[test]
fn every_game_over_icon_source_obeys_global_then_overlay_preflight() {
    let mut app = new_game_over_keyboard_app();
    app.assets
        .require_classic_game_over_resources()
        .test_value();

    let gui_icons = Arc::get_mut(&mut app.assets)
        .test_value()
        .startup_dialog_images
        .remove("GUIIcons.png")
        .test_value();
    let mut frame = vec![0xa5; 320 * 200 * 4];
    let sentinel = frame.clone();
    let error = app
        .render(&mut frame)
        .expect_err("shared GUIIcons must fail at the process-global preflight");
    assert_global_gui_boundary(&error, vec![ClassicGuiBootstrapIssue::missing("GUIIcons")]);
    main_assert_eq!(frame => sentinel);
    Arc::get_mut(&mut app.assets)
        .test_value()
        .startup_dialog_images
        .insert("GUIIcons.png".to_string(), gui_icons);

    {
        let name = "Player.png";
        let (image, hud_player) = {
            let assets = Arc::get_mut(&mut app.assets).test_value();
            let image = assets.startup_dialog_images.remove(name).test_value();
            let hud_player = Arc::make_mut(&mut assets.hud_graphics)
                .player
                .take()
                .test_value();
            (image, hud_player)
        };
        let mut frame = vec![0xa5; 320 * 200 * 4];
        let sentinel = frame.clone();

        let error = app
            .render(&mut frame)
            .expect_err("missing game-over icon source must fail typed");
        // Removing the HUD facet too now trips the earlier mandatory-graphics
        // gate, which is where `C4GraphicsResource::Init` would have refused.
        assert_hud_resource_boundary(&error, vec![name]);
        main_assert_eq!(frame => sentinel, "{name} guard must run before pixels");

        let assets = Arc::get_mut(&mut app.assets).test_value();
        assets.startup_dialog_images.insert(name.to_string(), image);
        Arc::make_mut(&mut assets.hud_graphics).player = Some(hud_player);
    }

    let score = {
        let assets = Arc::get_mut(&mut app.assets).test_value();
        Arc::make_mut(&mut assets.hud_graphics)
            .score
            .take()
            .test_value()
    };
    let mut frame = vec![0x3c; 320 * 200 * 4];
    let sentinel = frame.clone();
    let error = app
        .render(&mut frame)
        .expect_err("missing score source must fail typed");
    assert_hud_resource_boundary(&error, vec!["Score.png"]);
    main_assert_eq!(frame => sentinel, "Score.png guard must run before pixels");
    let assets = Arc::get_mut(&mut app.assets).test_value();
    Arc::make_mut(&mut assets.hud_graphics).score = Some(score);
}

#[test]
fn game_over_with_complete_classic_resources_renders_without_fallback() {
    let mut app = new_game_over_keyboard_app();
    app.assets
        .require_classic_game_over_resources()
        .test_value();
    let mut frame = vec![0x5a; 320 * 200 * 4];
    let sentinel = frame.clone();

    app.test_render(&mut frame);

    main_assert_ne!(frame => sentinel, "classic renderer must compose an output frame");
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
    main_assert_eq!(
        app.startup.options_dialog
            .as_ref()
            .expect("retained Options model")
            .active_sheet() =>
        clonk_frontend::startup_options_dlg::OptionsSheet::Graphics
    );
    main_assert_eq!(app.startup.about_dialog.as_ref().expect("retained About model").current_page() => clonk_frontend::startup_about_dlg::AboutPage::Licenses);
    main_assert_eq!(app.startup_network_dialog.as_ref().expect("retained Network model").mode() => clonk_frontend::startup_netdlg::NetDlgMode::Chat);
    main_assert!(app.startup.player_dialog.is_some());
    app.handle_game_over().test_value();
    app.assets
        .require_classic_game_over_resources()
        .test_value();

    for view in StartupView::ALL {
        // Exhaustive arms force future startup roots into this lifecycle
        // invariant instead of silently omitting the evaluation dialog.
        match view {
            StartupView::MainMenu => app.startup.view = StartupView::MainMenu,
            StartupView::ScenarioBrowser => {
                app.startup.view = StartupView::ScenarioBrowser;
            }
            StartupView::NetworkLobby => {
                app.startup.view = StartupView::NetworkLobby;
                app.classic_host_lobby = None;
            }
            StartupView::NetworkGame => app.startup.view = StartupView::NetworkGame,
            StartupView::Options => app.startup.view = StartupView::Options,
            StartupView::About => app.startup.view = StartupView::About,
            StartupView::PlayerSelection => {
                app.startup.view = StartupView::PlayerSelection;
            }
        }
        app.status_text = format!("lower-priority status for {view:?}");

        let mut frame = vec![0xc3; 640 * 480 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("stale evaluation must precede every startup pixel");
        assert_startup_game_over_boundary(&error, view);
        main_assert!(frame.iter().all(|byte| *byte == 0xc3));

        let mut native = vec![0x6d; 1280 * 960 * 4];
        let error = app
            .render_native_main_menu_text(&mut native, 1280, 960)
            .expect_err("native pass must enforce the same lifecycle boundary");
        assert_startup_game_over_boundary(&error, view);
        main_assert!(native.iter().all(|byte| *byte == 0x6d));
    }
}

#[test]
fn stale_menu_game_over_lifecycle_boundary_precedes_missing_resources() {
    let mut app = new_real_classic_menu_app(320, 200);
    app.handle_game_over().test_value();
    app.status_text.clear();
    app.assets = Arc::new(FrontendAssets::load(None));
    let mut frame = vec![0xc3; 320 * 200 * 4];
    let sentinel = frame.clone();

    let error = app
        .render(&mut frame)
        .expect_err("invalid lifecycle must win without resource lookup");
    assert_startup_game_over_boundary(&error, StartupView::MainMenu);
    main_assert_eq!(frame => sentinel, "startup preflight must precede every pixel");

    let mut native = vec![0x47; 640 * 400 * 4];
    let error = app
        .render_native_main_menu_text(&mut native, 640, 400)
        .expect_err("native pass must reject stale evaluation before resources");
    assert_startup_game_over_boundary(&error, StartupView::MainMenu);
    main_assert!(native.iter().all(|byte| *byte == 0x47));
}

fn current_scoreboard_test_layout(
    app: &mut GameApp,
) -> clonk_frontend::scoreboard::ScoreboardLayout {
    app.materialize_scoreboard_presentation().test_value();
    app.dialogs.scoreboard_runtime
        .presentation
        .test_ref()
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
        .test_value()
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
    empty.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    empty.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert_eq!(runtime_global_ui_snapshot(&empty) => before_empty);

    let mut dimensionless_positive =
        new_scoreboard_test_app("global func Initialize() { DoScoreboardShow(1); }");
    main_assert_eq!(dimensionless_positive.snapshot.hud.scoreboard.show_count() => 1);
    main_assert_eq!(dimensionless_positive.snapshot.hud.scoreboard.row_count() => 0);
    let before_dimensionless = runtime_global_ui_snapshot(&dimensionless_positive);
    dimensionless_positive.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    dimensionless_positive.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert_eq!(runtime_global_ui_snapshot(&dimensionless_positive) => before_dimensionless);

    let mut negative = new_scoreboard_test_app(
        r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       DoScoreboardShow(-1);
                   }"#,
    );
    main_assert_eq!((negative.snapshot.hud.scoreboard.row_count(), negative.snapshot.hud.scoreboard.column_count()) => (1, 1));
    main_assert_eq!(negative.snapshot.hud.scoreboard.show_count() => -1);
    let before_negative = runtime_global_ui_snapshot(&negative);
    negative.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    negative.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert_eq!(runtime_global_ui_snapshot(&negative) => before_negative);

    let mut eligible = new_scoreboard_test_app(
        r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "PRIVATE_CELL_TEXT");
                   }"#,
    );
    eligible.display_flags.scroll_smooth = 1;
    eligible.graphics.set_scroll_smooth(1);
    let mut hidden = vec![0_u8; 320 * 200 * 4];
    eligible.test_render(&mut hidden);
    toggle_scoreboard(&mut eligible, ModifiersState::empty());
    main_assert_eq!(eligible.dialogs.scoreboard => Some(eligible.scoreboard_request()));
    let mut frame = vec![0_u8; 320 * 200 * 4];
    eligible.test_render(&mut frame);
    let layout = current_scoreboard_test_layout(&mut eligible);
    main_assert!(frames_differ_in_rect(&hidden, &frame, 320, layout.bounds,));

    toggle_scoreboard(&mut eligible, ModifiersState::empty());
    main_assert!(eligible.dialogs.scoreboard.is_none());

    // Logo is not represented by C4KeyCodeEx and therefore remains an
    // exact bare-Tab ScoreboardToggle.
    toggle_scoreboard(&mut eligible, ModifiersState::SUPER);
    main_assert_eq!(eligible.dialogs.scoreboard => Some(eligible.scoreboard_request()));
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
    app.test_render(&mut frame);
    let baseline = app.graphics.surface().pixels().to_vec();
    let close = current_scoreboard_test_layout(&mut app)
        .close_button
        .test_value();
    let point = PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    );
    app.test_cursor(point);
    main_assert!(app.dialogs.scoreboard_runtime.close_hovered);
    app.test_render(&mut frame);
    let hovered = app.graphics.surface().pixels().to_vec();
    main_assert!(frames_differ_in_rect(&baseline, &hovered, 320, close));
    let sounds_before_press = app.sound.ui_log.len();
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(&app.sound.ui_log[sounds_before_press..] => &["ArrowHit".to_string()]);
    app.test_render(&mut frame);
    let down = app.graphics.surface().pixels().to_vec();
    main_assert!(frames_differ_in_rect(&hovered, &down, 320, close));
    main_assert!(app.dialogs.scoreboard_close_pointer_capture);
    main_assert!(app.dialogs.scoreboard.is_some());

    let outside = PhysicalPosition::new(0.0, 199.0);
    let sounds_before_leave = app.sound.ui_log.len();
    app.test_cursor(outside);
    main_assert_eq!(&app.sound.ui_log[sounds_before_leave..] => &["ArrowHit".to_string()]);
    main_assert!(app.dialogs.scoreboard_close_pointer_capture);
    main_assert!(app.live_input.ingame_pointer.is_none());
    app.test_left_button(ElementState::Released);
    main_assert!(app.dialogs.scoreboard.is_some());
    main_assert!(!app.dialogs.scoreboard_close_pointer_capture);

    app.test_cursor(point);
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(outside);
    let sounds_before_reentry = app.sound.ui_log.len();
    app.test_cursor(point);
    main_assert_eq!(&app.sound.ui_log[sounds_before_reentry..] => &["ArrowHit".to_string()]);
    let sounds_before_click = app.sound.ui_log.len();
    app.test_left_button(ElementState::Released);
    main_assert_eq!(&app.sound.ui_log[sounds_before_click..] => &["Click".to_string()]);
    main_assert!(app.dialogs.scoreboard.is_none());
    main_assert!(!app.dialogs.scoreboard_close_pointer_capture);
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
    app.test_render(&mut frame);
    let original = current_scoreboard_test_layout(&mut app);
    let caption = original.caption.test_value();
    let title = PhysicalPosition::new(
        f64::from(caption.x + caption.w / 2),
        f64::from(caption.y + caption.h / 2),
    );
    app.test_cursor(title);
    main_assert_eq!(app.classic_dialog_title_tooltip_target_at(GuiPoint::new(title.x as f32, title.y as f32,)) => Some(StartupTooltip::text("A scoreboard title")),);
    app.test_left_button(ElementState::Pressed);
    let moved_pointer = PhysicalPosition::new(title.x + 35.0, title.y + 22.0);
    app.test_cursor(moved_pointer);
    app.test_left_button(ElementState::Released);
    let moved = current_scoreboard_test_layout(&mut app);
    main_assert_eq!(moved.bounds.x => original.bounds.x + 35);
    main_assert_eq!(moved.bounds.y => original.bounds.y + 22);

    app.test_render(&mut frame);
    main_assert_eq!(current_scoreboard_test_layout(&mut app) => moved);

    app.resize(640, 480).test_value();
    let mut resized_frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut resized_frame);
    main_assert_eq!(current_scoreboard_test_layout(&mut app) => moved);

    call_scoreboard_function_and_update(&mut app, "InvalidateLayout");
    app.test_render(&mut resized_frame);
    main_assert_ne!(current_scoreboard_test_layout(&mut app).bounds => moved.bounds);
}

#[test]
fn asynchronously_shown_message_stays_active_during_scoreboard_title_drag() {
    let mut app = new_scoreboard_test_app(
        r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
    );
    app.resize(1024, 768).test_value();
    toggle_scoreboard(&mut app, ModifiersState::empty());
    let mut frame = vec![0_u8; 1024 * 768 * 4];
    app.test_render(&mut frame);
    let before = current_scoreboard_test_layout(&mut app);
    let caption = before.caption.test_value();
    let start = PhysicalPosition::new(
        f64::from(caption.x + caption.w / 2),
        f64::from(caption.y + caption.h / 2),
    );
    app.test_cursor(start);
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.dialogs.scoreboard_runtime.title_drag.is_some());

    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Asynchronous notice",
            "Higher dialog",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    main_assert!(matches!(app.running_active_dialog, Some(RunningDialogStackEntry::Message(_))));

    let moved_pointer = PhysicalPosition::new(start.x - 24.0, start.y + 17.0);
    let message = app.top_message_dialog_layout().test_value();
    main_assert!(!GameApp::point_in_message_dialog_bounds(GuiPoint::new(moved_pointer.x as f32, moved_pointer.y as f32), &message,));
    app.test_cursor(moved_pointer);
    let moved = current_scoreboard_test_layout(&mut app);
    main_assert_eq!(moved.bounds.x => before.bounds.x - 24);
    main_assert_eq!(moved.bounds.y => before.bounds.y + 17);
    main_assert!(matches!(app.running_active_dialog, Some(RunningDialogStackEntry::Message(_))));
    main_assert!(app.live_input.ingame_pointer.is_none());

    app.remove_message_dialog_at(0).test_value();
    main_assert!(app.dialogs.scoreboard_runtime.title_drag.is_none());
    main_assert!(app.dialogs.messages.is_empty());
    let after_close = current_scoreboard_test_layout(&mut app);
    app.test_cursor(PhysicalPosition::new(
        moved_pointer.x + 31.0,
        moved_pointer.y - 9.0,
    ));
    main_assert_eq!(current_scoreboard_test_layout(&mut app) => after_close);
    app.test_left_button(ElementState::Released);
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
    app.test_render(&mut frame);
    let initial = current_scoreboard_test_layout(&mut app);
    let initial_revision = app.dialogs.scoreboard_runtime.layout_revision;
    let point = GuiPoint::new(
        (initial.bounds.x + initial.bounds.w / 2) as f32,
        (initial.bounds.y + initial.bounds.h / 2) as f32,
    );

    app.engine
        .call_scenario_script_function("GrowBetweenFrames", Vec::new())
        .test_value();
    main_assert!(app.scoreboard_pointer_target(point).expect("pointer route").is_some());
    main_assert_eq!(app.snapshot.hud.scoreboard.row_count() => 2);
    main_assert_eq!(app.dialogs.scoreboard_runtime.layout_revision => initial_revision);
    main_assert_eq!(
        app.dialogs.scoreboard_runtime
            .presentation
            .as_ref()
            .expect("retained presentation")
            .layout() =>
        &initial,
        "pointer input retains pre-draw C++ geometry",
    );

    app.test_render(&mut frame);
    main_assert_eq!(app.dialogs.scoreboard_runtime.layout_revision => app.engine.scoreboard_layout_revision(),);
    main_assert_ne!(current_scoreboard_test_layout(&mut app) => initial);
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
    let request_revision = app.dialogs.scoreboard.test_ref().layout_revision;
    main_assert!(request_revision < app.engine.scoreboard_layout_revision());

    let constructor = current_scoreboard_test_layout(&mut app);
    let late_column_point = GuiPoint::new(
        (constructor.bounds.x - 2) as f32,
        (constructor.client.y + constructor.client.h / 2) as f32,
    );
    main_assert!(
        app.scoreboard_pointer_target(late_column_point)
            .expect("pre-draw pointer route")
            .is_none(),
        "the late column is outside the synchronous constructor bounds",
    );
    main_assert_eq!(app.dialogs.scoreboard_runtime.layout_revision => request_revision);
    main_assert_eq!(current_scoreboard_test_layout(&mut app) => constructor);

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let updated = current_scoreboard_test_layout(&mut app);
    main_assert!(updated.bounds.x < late_column_point.x as i32);
    main_assert!(app.scoreboard_pointer_target_cached(late_column_point).is_some());
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
        .test_value();
    main_assert!(app.dialogs.scoreboard.is_none());

    let point = GuiPoint::new(299.0, 50.0);
    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));

    main_assert!(app.dialogs.scoreboard.is_some());
    main_assert!(app.scoreboard_pointer_target_cached(point).is_some());
    main_assert_eq!(app.running_active_dialog => Some(RunningDialogStackEntry::Scoreboard),);
    main_assert!(app.live_input.ingame_pointer.is_none());
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
    let caption = before.caption.test_value();
    let start = PhysicalPosition::new(
        f64::from(caption.x + caption.w / 2),
        f64::from(caption.y + caption.h / 2),
    );
    app.test_cursor(start);
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.dialogs.scoreboard_runtime.title_drag.is_some());

    app.resize(360, 220).test_value();
    main_assert!(app.dialogs.scoreboard_runtime.title_drag.is_none());
    main_assert!(app.dialogs.scoreboard_runtime.pointer.is_none());
    main_assert!(!app.dialogs.scoreboard_runtime.close_hovered);
    main_assert!(!app.dialogs.scoreboard_close_pointer_capture);
    let cached = current_scoreboard_test_layout(&mut app);
    main_assert_eq!(cached => before);

    app.test_cursor(PhysicalPosition::new(start.x + 30.0, start.y + 20.0));
    main_assert_eq!(current_scoreboard_test_layout(&mut app) => cached);
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
        .test_value();
    let close_point = GuiPoint::new(
        (close_button.x + close_button.w / 2) as f32,
        (close_button.y + close_button.h / 2) as f32,
    );
    close.test_touch(TouchPhase::Started, close_point);
    main_assert!(close.dialogs.scoreboard_close_pointer_capture);
    close
        .push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    let message = close.top_message_dialog_layout().test_value();
    let message_point = GuiPoint::new(
        (message.bounds.x + message.bounds.w / 2) as f32,
        (message.bounds.y + message.bounds.h / 2) as f32,
    );
    close.test_touch(TouchPhase::Ended, message_point);
    main_assert!(close.dialogs.scoreboard.is_some());
    main_assert!(!close.dialogs.scoreboard_close_pointer_capture);

    let mut drag = new_scoreboard_test_app(board);
    toggle_scoreboard(&mut drag, ModifiersState::empty());
    let layout = current_scoreboard_test_layout(&mut drag);
    let caption = layout.caption.test_value();
    let title_point = GuiPoint::new(
        (caption.x + caption.w / 2) as f32,
        (caption.y + caption.h / 2) as f32,
    );
    drag.test_touch(TouchPhase::Started, title_point);
    main_assert!(drag.dialogs.scoreboard_runtime.title_drag.is_some());
    drag.push_message_dialog(notice(), MessageDialogContinuation::None)
        .test_value();
    drag.test_touch(TouchPhase::Cancelled, title_point);
    main_assert!(drag.dialogs.scoreboard_runtime.title_drag.is_none());
    main_assert!(!drag.dialogs.scoreboard_close_pointer_capture);
    let after_cancel = current_scoreboard_test_layout(&mut drag);
    drag.test_cursor(PhysicalPosition::new(
        f64::from(title_point.x + 30.0),
        f64::from(title_point.y + 20.0),
    ));
    main_assert_eq!(current_scoreboard_test_layout(&mut drag) => after_cancel);
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
    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));
    app.live_input.ingame_mouse_init_centered = false;

    app.test_right_button(ElementState::Pressed);
    app.test_right_button(ElementState::Released);
    main_assert!(!app.live_input.ingame_mouse_init_centered);
    main_assert!(commands.take_submitted_local().is_empty());

    app.handle_other_mouse_button(ElementState::Pressed)
        .test_value();
    app.handle_other_mouse_button(ElementState::Released)
        .test_value();
    main_assert!(!app.live_input.ingame_mouse_init_centered);
    main_assert!(commands.take_submitted_local().is_empty());

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert!(!app.live_input.ingame_mouse_init_centered);
    main_assert!(commands.take_submitted_local().is_empty());

    let before_touch = current_scoreboard_test_layout(&mut app);
    let caption = before_touch.caption.test_value();
    let touch_title = GuiPoint::new(
        (caption.x + caption.w / 2) as f32,
        (caption.y + caption.h / 2) as f32,
    );
    let touch_moved = GuiPoint::new(touch_title.x + 12.0, touch_title.y + 9.0);
    app.test_touch(TouchPhase::Started, touch_title);
    main_assert!(app.dialogs.scoreboard_runtime.title_drag.is_some());
    app.test_touch(TouchPhase::Moved, touch_moved);
    let after_touch_move = current_scoreboard_test_layout(&mut app);
    main_assert_eq!(after_touch_move.bounds.x => before_touch.bounds.x + 12);
    main_assert_eq!(after_touch_move.bounds.y => before_touch.bounds.y + 9);
    app.test_touch(TouchPhase::Ended, touch_moved);
    main_assert!(app.dialogs.scoreboard_runtime.title_drag.is_none());
    main_assert_eq!(current_scoreboard_test_layout(&mut app) => after_touch_move);
    main_assert!(commands.take_submitted_local().is_empty());
    main_assert!(app.dialogs.scoreboard.is_some());
}

#[test]
fn running_context_menu_routes_before_shared_scoreboard_dialogs() {
    const BOARD: &str = r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "A deliberately wide scoreboard");
                SetScoreboardData(1, 1, "A deliberately wide value");
            }"#;

    let mut overlap = new_scoreboard_test_app(BOARD);
    overlap.resize(1024, 768).test_value();
    toggle_scoreboard(&mut overlap, ModifiersState::empty());
    let mut frame = vec![0_u8; 1024 * 768 * 4];
    overlap.test_render(&mut frame);
    let bounds = current_scoreboard_test_layout(&mut overlap).bounds;
    overlap
        .dialogs.scoreboard_runtime
        .presentation
        .test_mut()
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
        .test_value();
    overlap
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new("Popup row")],
            GuiPoint::new(48.0, 48.0),
        )
        .test_value();
    let popup_row = overlap.context_menu.test_ref().layout().panels[0].rows[0].rect;
    let popup_point = GuiPoint::new(
        (popup_row.x + popup_row.w / 2) as f32,
        (popup_row.y + popup_row.h / 2) as f32,
    );
    main_assert!(overlap.scoreboard_pointer_target_cached(popup_point).is_some());
    main_assert!(!GameApp::point_in_message_dialog_bounds(popup_point, &overlap.top_message_dialog_layout().expect("message layout"),));
    overlap.test_cursor(PhysicalPosition::new(
        f64::from(popup_point.x),
        f64::from(popup_point.y),
    ));
    overlap.test_left_button(ElementState::Pressed);
    main_assert!(overlap.context_menu.is_some());
    main_assert!(!overlap.dialogs.scoreboard_close_pointer_capture);
    main_assert!(matches!(overlap.running_active_dialog, Some(RunningDialogStackEntry::Message(_))));
    overlap.test_left_button(ElementState::Released);

    let mut outside = new_scoreboard_test_app(BOARD);
    toggle_scoreboard(&mut outside, ModifiersState::empty());
    outside
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new("Popup row")],
            GuiPoint::new(10.0, 10.0),
        )
        .test_value();
    let scoreboard = current_scoreboard_test_layout(&mut outside);
    let body = GuiPoint::new(
        (scoreboard.client.x + scoreboard.client.w / 2) as f32,
        (scoreboard.client.y + scoreboard.client.h / 2) as f32,
    );
    main_assert!(!outside.context_menu.as_ref().expect("context menu").captures_point(body));
    outside.test_cursor(PhysicalPosition::new(f64::from(body.x), f64::from(body.y)));
    outside.live_input.ingame_mouse_init_centered = false;
    outside.test_right_button(ElementState::Pressed);
    main_assert!(outside.context_menu.is_none());
    main_assert!(!outside.live_input.ingame_mouse_init_centered);
}

#[test]
fn scoreboard_wheel_does_not_scroll_an_overlapped_lower_f4_dialog() {
    let mut app = new_scoreboard_test_app(
        r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
    );
    app.resize(1024, 768).test_value();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot((0..40).map(|client_id| message_client(client_id, b"Remote")));
    toggle_scoreboard(&mut app, ModifiersState::empty());
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    app.activate_running_dialog(RunningDialogStackEntry::Scoreboard);

    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let f4_layout = app
        .dialogs.client_list
        .test_ref()
        .layout(preferred, line_height);
    let scoreboard = current_scoreboard_test_layout(&mut app);
    let dx = f4_layout.list.x + 8 - scoreboard.client.x;
    let dy = f4_layout.list.y + 8 - scoreboard.client.y;
    app.dialogs.scoreboard_runtime
        .presentation
        .test_mut()
        .layout_mut()
        .translate(dx, dy);
    let scoreboard = current_scoreboard_test_layout(&mut app);
    let point = GuiPoint::new(
        (scoreboard.client.x + 4) as f32,
        (scoreboard.client.y + 4) as f32,
    );
    main_assert!(point.x < (f4_layout.list.x + f4_layout.list.w) as f32);
    main_assert!(point.y < (f4_layout.list.y + f4_layout.list.h) as f32);
    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));
    let before = app
        .dialogs.client_list
        .test_ref()
        .scroll_row(preferred, line_height);
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    let after = app
        .dialogs.client_list
        .test_ref()
        .scroll_row(preferred, line_height);
    main_assert_eq!(after => before, "wheel cannot fall through to lower F4");
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
    app.resize(1024, 768).test_value();
    toggle_scoreboard(&mut app, ModifiersState::empty());
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Message remains open",
            "Shared dialog",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();

    let close = current_scoreboard_test_layout(&mut app)
        .close_button
        .test_value();
    let point = GuiPoint::new(
        (close.x + close.w / 2) as f32,
        (close.y + close.h / 2) as f32,
    );
    let message_layout = app.top_message_dialog_layout().test_value();
    main_assert!(!GameApp::point_in_message_dialog_bounds(point, &message_layout,));

    let mut frame = vec![0_u8; 1024 * 768 * 4];
    app.test_render(&mut frame);
    let baseline = app.graphics.surface().pixels().to_vec();

    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));
    main_assert!(matches!(app.running_active_dialog, Some(RunningDialogStackEntry::Message(_))));
    main_assert!(app.dialogs.scoreboard_runtime.close_hovered);
    app.test_render(&mut frame);
    let hovered = app.graphics.surface().pixels().to_vec();
    main_assert!(frames_differ_in_rect(&baseline, &hovered, 1024, close));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert!(app.dialogs.scoreboard.is_none());
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
    f4.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    toggle_scoreboard(&mut f4, ModifiersState::empty());
    main_assert!(f4.scoreboard_is_above_runtime_client_list());
    f4.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    f4.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    main_assert!(!f4.scoreboard_is_above_runtime_client_list());

    let mut messages = new_scoreboard_test_app(BOARD);
    messages.resize(1024, 768).test_value();
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
        .test_value();
    main_assert!(!messages.scoreboard_is_above_all_messages());
    let layout = current_scoreboard_test_layout(&mut messages);
    let point = GuiPoint::new(
        (layout.client.x + layout.client.w / 2) as f32,
        (layout.client.y + layout.client.h / 2) as f32,
    );
    main_assert!(!GameApp::point_in_message_dialog_bounds(point, &messages.top_message_dialog_layout().expect("message layout"),));
    messages.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));
    messages.test_left_button(ElementState::Pressed);
    messages.test_left_button(ElementState::Released);
    main_assert!(messages.scoreboard_is_above_all_messages());

    messages
        .push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Second message",
                "Later input-z dialog",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .test_value();
    main_assert!(!messages.scoreboard_is_above_all_messages());
    main_assert!(matches!(messages.dialogs.stack.last(), Some(RunningDialogStackEntry::Message(_))));
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
    main_assert!(app.running_chat_active());

    let layout = current_scoreboard_test_layout(&mut app);
    let body = PhysicalPosition::new(
        f64::from(layout.client.x + layout.client.w / 2),
        f64::from(layout.client.y + layout.client.h / 2),
    );
    let chat = app.game_option_input_layout().test_value();
    main_assert!(!GameApp::point_in_input_dialog_bounds(GuiPoint::new(body.x as f32, body.y as f32), &chat,));
    app.test_cursor(body);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(!app.running_chat_active());
    main_assert_eq!(app.running_active_dialog => Some(RunningDialogStackEntry::Scoreboard),);

    let close = current_scoreboard_test_layout(&mut app)
        .close_button
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(app.dialogs.scoreboard.is_none());
    main_assert!(app.running_chat_active());
    main_assert_eq!(app.running_active_dialog => Some(RunningDialogStackEntry::Chat),);
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
    app.test_cursor(scoreboard_body);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);

    let chat = app.game_option_input_layout().test_value();
    let chat_point = GuiPoint::new(
        (chat.bounds.x + chat.bounds.w / 2) as f32,
        (chat.bounds.y + chat.bounds.h / 2) as f32,
    );
    main_assert!(app.scoreboard_pointer_target_cached(chat_point).is_none());
    app.test_cursor(PhysicalPosition::new(
        f64::from(chat_point.x),
        f64::from(chat_point.y),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(app.running_chat_active());
    main_assert!(!app.running_chat_keyboard_active());
    main_assert_eq!(app.dialogs.stack.last() => Some(&RunningDialogStackEntry::Scoreboard),);

    app.test_text_input('x');
    main_assert_eq!(app.running_chat_text() => Some(""));

    main_assert!(app.close_scoreboard_dialog());
    main_assert!(app.running_chat_keyboard_active());
    app.test_text_input('x');
    main_assert_eq!(app.running_chat_text() => Some("x"));
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
    .test_value();
    app.activate_running_dialog(RunningDialogStackEntry::Scoreboard);
    main_assert_eq!(app.dialogs.stack.last() => Some(&RunningDialogStackEntry::Scoreboard));
    main_assert!(!app.message_dialog_owns_gamepad_input());

    app.test_gamepad_events([
        game_over_fixture!(axis: GamepadSlot::new(0), LegacyGamepadAxis::new(0, true), ElementState::Pressed),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Right, ElementState::Pressed),
    ]);

    main_assert_ne!(app.engine.player(app.players.local_owner).expect("local player").control.pressed_coms & (1 << clonk_engine::COM_RIGHT) => 0,);
    main_assert_eq!(app.dialogs.messages.len() => 1);
}

#[test]
fn scoreboard_release_clears_an_occluded_f4_button_capture() {
    let mut app = new_scoreboard_test_app(
        r#"global func Initialize()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                   }"#,
    );
    app.resize(1024, 768).test_value();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    toggle_scoreboard(&mut app, ModifiersState::empty());
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);

    let (preferred, line_height) = app.runtime_client_list_input_geometry().test_value();
    let close = app
        .dialogs.client_list
        .test_ref()
        .layout(preferred, line_height)
        .close_button
        .expect("an ordinary dialog owns its title widgets");
    app.test_cursor(PhysicalPosition::new(
        f64::from(close.x + close.w / 2),
        f64::from(close.y + close.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.dialogs.client_list.as_ref().expect("F4 remains open").has_pointer_capture());

    let scoreboard = current_scoreboard_test_layout(&mut app);
    app.dialogs.scoreboard_runtime
        .presentation
        .test_mut()
        .layout_mut()
        .translate(900 - scoreboard.client.x, 50 - scoreboard.client.y);
    let scoreboard = current_scoreboard_test_layout(&mut app);
    let release = PhysicalPosition::new(
        f64::from(scoreboard.client.x + 4),
        f64::from(scoreboard.client.y + 4),
    );
    app.test_cursor(release);
    app.test_left_button(ElementState::Released);
    main_assert!(app.dialogs.client_list.is_some());
    main_assert!(!app.dialogs.client_list.as_ref().expect("F4 remains open").has_pointer_capture());
    main_assert!(app.dialogs.scoreboard.is_some());
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
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
    ] {
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Tab, ElementState::Released);
        main_assert!(app.ingame_menu.is_none());
        main_assert!(app.dialogs.messages.is_empty());
        main_assert!(!app.live_input.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
        main_assert!(app.dialogs.scoreboard.is_none());
    }

    app.bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::Tab);
    app.engine
        .test_player_mut(app.players.local_owner)
        .control
        .control_style = true;
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert!(app.live_input.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
    main_assert_ne!(app.engine.player(app.players.local_owner).expect("local player").control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0,);
    app.open_context_menu_at(
        vec![ContextMenuEntry::<AppContextMenuCommand>::new(
            "Remain open",
        )],
        GuiPoint::new(20.0, 20.0),
    )
    .test_value();
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert!(!app.live_input.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
    main_assert_ne!(
        app.engine
            .player(app.players.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT) =>
        0,
        "exact modifier matching suppresses the bare control release callback",
    );
    main_assert!(app.context_menu.is_some());

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
        .test_player_mut(exclusive_release.players.local_owner)
        .control
        .control_style = true;
    exclusive_release.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    exclusive_release.handle_game_over().test_value();
    exclusive_release.test_key(VirtualKeyCode::Tab, ElementState::Released);
    exclusive_release.dismiss_game_over_dialog();
    main_assert!(!exclusive_release.live_input.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
    main_assert_ne!(
        exclusive_release
            .engine
            .player(exclusive_release.players.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT) =>
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
        .test_player_mut(dialog_press.players.local_owner)
        .control
        .control_style = true;
    dialog_press.handle_game_over().test_value();
    dialog_press.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(dialog_press.game_over_dialog.as_ref().and_then(GameOverState::focused) => Some(GameOverFocus::Close));
    main_assert!(dialog_press.scoreboard_tab_raw_pressed);
    // `C4Game::DoKeyboardInput` records the raw physical edge before the
    // exclusive dialog can claim it (C4Game.cpp:2143-2155), which is what
    // makes the bare repeat below a repeat rather than a fresh press.
    main_assert!(dialog_press.live_input.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
    dialog_press.dismiss_game_over_dialog();
    dialog_press.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(
        dialog_press
            .engine
            .player(dialog_press.players.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT) =>
        0,
        "AutoStopControl consumes a repeat first seen in another scope",
    );
    dialog_press.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert!(!dialog_press.scoreboard_tab_raw_pressed);
    main_assert!(!dialog_press.live_input.pressed_engine_keys.contains(&VirtualKeyCode::Tab));
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
        .test_value();
    message.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    message.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert_eq!(message.dialogs.messages.len() => 1);
    main_assert!(message.dialogs.scoreboard.is_some());
    main_assert_eq!(message.dialogs.messages[0].state.focused_button() => Some(clonk_frontend::message_dialog::MessageDialogButton::Ok),);
    message.test_modifiers(ModifiersState::SHIFT);
    message.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    message.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert_eq!(message.dialogs.messages[0].state.focused_button() => Some(clonk_frontend::message_dialog::MessageDialogButton::Ok),);
    main_assert!(message.message_dialog_consumed_keys.is_empty());

    let mut game_over = new_classic_scoreboard_test_app(BOARD);
    game_over.handle_game_over().test_value();
    game_over.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    game_over.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert_eq!(game_over.game_over_dialog.as_ref().and_then(GameOverState::focused) => Some(GameOverFocus::Close));
    main_assert!(game_over.dialogs.scoreboard.is_none());

    let mut context = new_scoreboard_test_app(BOARD);
    context
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(20.0, 20.0),
        )
        .test_value();
    toggle_scoreboard(&mut context, ModifiersState::empty());
    main_assert!(context.context_menu.is_some());
    main_assert!(context.dialogs.scoreboard.is_some());

    let mut rebound_context = new_scoreboard_test_app(BOARD);
    rebound_context
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(20.0, 20.0),
        )
        .test_value();
    rebound_context
        .bindings
        .rebind(ControlBindingId::PlayerMenu, VirtualKeyCode::Tab);
    rebound_context.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    rebound_context.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert!(rebound_context.context_menu.is_some());
    main_assert!(rebound_context.ingame_menu.is_some());

    let mut game_over_context = new_classic_scoreboard_test_app(BOARD);
    game_over_context.handle_game_over().test_value();
    game_over_context
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(20.0, 20.0),
        )
        .test_value();
    game_over_context
        .bindings
        .rebind(ControlBindingId::PlayerMenu, VirtualKeyCode::Tab);
    let before_game_over_context = runtime_global_ui_snapshot(&game_over_context);
    game_over_context.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    game_over_context.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert_eq!(runtime_global_ui_snapshot(&game_over_context) => before_game_over_context);

    let mut object = new_scoreboard_test_app(BOARD);
    main_assert!(object.open_object_menu().expect("open object menu"));
    toggle_scoreboard(&mut object, ModifiersState::empty());
    main_assert!(object.object_menu.is_some());
    main_assert!(object.dialogs.scoreboard.is_some());

    let mut player = new_scoreboard_test_app(BOARD);
    player.open_ingame_menu().test_value();
    toggle_scoreboard(&mut player, ModifiersState::empty());
    main_assert!(player.ingame_menu.is_some());
    main_assert!(player.dialogs.scoreboard.is_some());
}

fn call_scoreboard_function_and_update(app: &mut GameApp, function: &str) {
    app.engine
        .call_scenario_script_function(function, Vec::new())
        .test_value();
    app.test_update();
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
        .test_value();
    main_assert!(render_app.dialogs.scoreboard.is_none());
    let mut frame = vec![0x5a; 320 * 200 * 4];
    let sentinel = frame.clone();
    render_app.test_render(&mut frame);
    main_assert_ne!(frame => sentinel);
    main_assert!(render_app.dialogs.scoreboard.is_some());

    let mut tab_app = new_scoreboard_test_app(CALLBACK_BOARD);
    tab_app
        .engine
        .call_scenario_script_function("ShowNow", Vec::new())
        .test_value();
    main_assert!(tab_app.dialogs.scoreboard.is_none());
    tab_app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    tab_app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert!(tab_app.dialogs.scoreboard.is_none());
    main_assert_eq!(tab_app.snapshot.hud.scoreboard.show_count() => 1);
    tab_app.test_render(&mut frame);
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
    positive.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    positive.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert!(positive.dialogs.scoreboard.is_none());
    let saved_positive = positive.engine.capture_state();
    main_assert_eq!(saved_positive.scoreboard.show_count() => 1);

    positive.engine.restore_state(&saved_positive).test_value();
    positive.snapshot = positive.engine.snapshot();
    positive.arm_initial_scoreboard_reconcile();
    let before_surface = positive.graphics.surface().pixels().to_vec();
    let mut frame = vec![0x4c; 320 * 200 * 4];
    let sentinel = frame.clone();
    positive.test_render(&mut frame);
    main_assert_ne!(frame => sentinel);
    main_assert_ne!(positive.graphics.surface().pixels() => before_surface.as_slice());
    main_assert!(positive.dialogs.scoreboard.is_some());
    main_assert_eq!(positive.engine.scoreboard_snapshot() => saved_positive.scoreboard);

    let mut zero = new_scoreboard_test_app(RESTORE_BOARD);
    let user_open_request = zero.scoreboard_request();
    main_assert!(zero.snapshot.hud.scoreboard.can_be_shown());
    main_assert!(!user_open_request.should_be_shown());
    zero.dialogs.scoreboard = Some(user_open_request);
    let saved_zero = zero.engine.capture_state();
    main_assert_eq!(saved_zero.scoreboard.show_count() => 0);

    zero.engine.restore_state(&saved_zero).test_value();
    zero.snapshot = zero.engine.snapshot();
    zero.arm_initial_scoreboard_reconcile();
    main_assert!(zero.dialogs.scoreboard.is_none());
    zero.test_render(&mut frame);
    main_assert!(zero.dialogs.scoreboard.is_none());
    main_assert_eq!(zero.engine.scoreboard_snapshot() => saved_zero.scoreboard);
}

/// `C4ScoreboardDlg` is constructed with `fViewportDlg == false`
/// (`C4Scoreboard.cpp:292`), so `Dialog::Show`'s console arm —
/// `if (!Application.isFullScreen && !IsViewportDialog()) CreateConsoleWindow()`
/// (`C4GuiDialogs.cpp:659-661`) — gives it a real child window of the console,
/// and `Dialog::Close` destroys that window again (`:677`). The port's console
/// branch returns before any dialog layer, so the window request is what the
/// console runner reconciles against.
#[test]
fn console_scoreboard_owns_a_child_window_only_while_its_dialog_is_open() {
    let script = r#"global func ShowBoard()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       DoScoreboardShow(1);
                   }"#;

    // Fullscreen draws the scoreboard into the primary window, so it never
    // asks for a child one however the dialog was opened.
    let mut fullscreen = new_scoreboard_test_app(script);
    call_scoreboard_function_and_update(&mut fullscreen, "ShowBoard");
    main_assert!(fullscreen.dialogs.scoreboard.is_some());
    main_assert!(!fullscreen.console_scoreboard_window_open());

    let mut console = new_scoreboard_test_app(script);
    console.console_mode = true;
    main_assert!(!console.console_scoreboard_window_open());

    call_scoreboard_function_and_update(&mut console, "ShowBoard");
    main_assert!(console.dialogs.scoreboard.is_some());
    main_assert!(console.console_scoreboard_window_open());

    // A second script show is another ordered request, not a second window:
    // `CreateConsoleWindow` returns early when the dialog already has one
    // (`C4GuiDialogs.cpp:308`).
    call_scoreboard_function_and_update(&mut console, "ShowBoard");
    main_assert!(console.console_scoreboard_window_open());

    main_assert!(console.close_scoreboard_dialog());
    main_assert!(!console.console_scoreboard_window_open());
}

/// `ScoreboardToggle` is registered at `KEYSCOPE_Generic` (`C4Game.cpp:3427`),
/// so Tab toggles the board in console mode too, and `DialogWinProc` forwards
/// the dialog window's own keys to `Game.DoKeyboardInput`
/// (`C4GuiDialogs.cpp:219-228`) — pressing it again on the scoreboard window
/// closes it. `DoDlgShow(0, true)` is the user toggle, which leaves the
/// reference count alone (`C4Scoreboard.cpp:239`).
#[test]
fn console_scoreboard_tab_toggles_the_child_window_without_moving_the_refcount() {
    let mut app = new_scoreboard_test_app(
        r#"global func FillBoard()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       SetScoreboardData(1, 1, "Ada");
                   }"#,
    );
    app.console_mode = true;
    call_scoreboard_function_and_update(&mut app, "FillBoard");
    let refcount = app.snapshot.hud.scoreboard.show_count();
    main_assert!(!app.console_scoreboard_window_open());

    toggle_scoreboard(&mut app, ModifiersState::empty());
    main_assert!(app.console_scoreboard_window_open());
    main_assert_eq!(app.snapshot.hud.scoreboard.show_count() => refcount, "a user toggle passes DoDlgShow a change of zero");

    toggle_scoreboard(&mut app, ModifiersState::empty());
    main_assert!(!app.console_scoreboard_window_open());
    main_assert_eq!(app.snapshot.hud.scoreboard.show_count() => refcount);
}

/// The console window is sized to the dialog `C4ScoreboardDlg::Update`
/// computes and titled from the caption cell, because `Dialog::SetTitle` puts
/// that text on the window bar instead of in a `WoodenLabel`
/// (`C4GuiDialogs.cpp:390-395`). Live `SetScoreboardData` grows the dialog, and
/// `Dialog::UpdateSize` resizes the window with it (`C4GuiDialogs.cpp:445-473`).
#[test]
fn console_scoreboard_window_takes_its_title_and_size_from_the_live_board() {
    let mut app = new_scoreboard_test_app(
        r#"global func ShowBoard()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       SetScoreboardData(1, 1, "Ada");
                       DoScoreboardShow(1);
                   }
                   global func WidenBoard()
                   {
                       SetScoreboardData(2, 1, "Bert has a very long name indeed");
                   }"#,
    );
    app.console_mode = true;
    main_assert!(app.console_scoreboard_window_chrome().is_none());

    call_scoreboard_function_and_update(&mut app, "ShowBoard");
    let (title, width, height) = app
        .console_scoreboard_window_chrome()
        .test_value();
    main_assert_eq!(title => "Scores".to_owned());
    main_assert!(width > 0 && height > 0);

    // The caption text is on the window bar, so the dialog itself reserves no
    // title band: its whole height is the spreadsheet.
    let surface = app.render_console_scoreboard(width, height).test_value();
    main_assert_eq!(surface.width() => width);
    main_assert_eq!(surface.height() => height);
    main_assert!(surface.pixels().chunks_exact(4).any(|pixel| pixel[3] != 0));

    // A live data update grows the board, and the window follows it.
    call_scoreboard_function_and_update(&mut app, "WidenBoard");
    let (_, wider, taller) = app
        .console_scoreboard_window_chrome()
        .test_value();
    main_assert!(wider > width, "an added row widens the console window");
    main_assert!(taller > height, "an added row heightens the console window");

    // Closing the dialog withdraws the window and everything it drew.
    main_assert!(app.close_scoreboard_dialog());
    main_assert!(app.console_scoreboard_window_chrome().is_none());
    main_assert!(app.render_console_scoreboard(wider, taller).is_none());
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
    main_assert!(empty_then_cell.snapshot.hud.scoreboard.should_be_shown());
    main_assert!(empty_then_cell.dialogs.scoreboard.is_none(), "SetCell cannot retroactively open an earlier empty request");
    let mut ordinary = vec![0_u8; 320 * 200 * 4];
    empty_then_cell.test_render(&mut ordinary);

    let mut open_then_close = new_scoreboard_test_app(
        r#"global func OpenThenClose()
                   {
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       DoScoreboardShow(1);
                       DoScoreboardShow(-1);
                   }"#,
    );
    call_scoreboard_function_and_update(&mut open_then_close, "OpenThenClose");
    main_assert_eq!(open_then_close.snapshot.hud.scoreboard.show_count() => 0);
    main_assert!(open_then_close.dialogs.scoreboard.is_none());
    open_then_close.test_render(&mut ordinary);
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
    let request = app.dialogs.scoreboard.test_ref();
    main_assert!(!request.title_widget_present);
    main_assert!(request.layout_revision < app.engine.scoreboard_layout_revision());

    let constructor_layout = current_scoreboard_test_layout(&mut app);
    main_assert!(constructor_layout.caption.is_none());
    main_assert!(constructor_layout.client.y > constructor_layout.bounds.y);

    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let updated_layout = current_scoreboard_test_layout(&mut app);
    main_assert!(updated_layout.caption.is_none());
    main_assert_eq!(updated_layout.client.y => updated_layout.bounds.y);
    main_assert_eq!(updated_layout.client.h => updated_layout.bounds.h);
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
    main_assert!(app.dialogs.scoreboard.is_some());
    main_assert_eq!((app.snapshot.hud.scoreboard.row_count(), app.snapshot.hud.scoreboard.column_count(),) => (2, 2));

    let before_ui = runtime_global_ui_snapshot(&app);
    let before_surface = app.graphics.surface().pixels().to_vec();
    let mut frame = vec![0x6d; 320 * 200 * 4];
    let sentinel = frame.clone();
    for _ in 0..2 {
        app.test_render(&mut frame);
        main_assert_ne!(frame => sentinel);
        main_assert_ne!(app.graphics.surface().pixels() => before_surface.as_slice());
        main_assert_eq!(runtime_global_ui_snapshot(&app) => before_ui);
    }

    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    main_assert!(app.dialogs.scoreboard.is_none());
    app.test_render(&mut frame);
}

// `CStdFont::DrawText` consumes `{{...}}` markup and `continue`s when the
// image renderer is not hooked or the id is unknown — "printing it out
// wouldn't look better" (src/StdFont.cpp:868-890). The board keeps drawing;
// only the image is lost (clonk-org/clonk-rs#1209). The exact widths that
// follow from `GetTextExtent`'s matching branch are pinned in
// clonk-frontend's own scoreboard tests.
#[test]
fn an_unresolved_scoreboard_font_image_still_draws_the_rest_of_the_board() {
    let show = |cell: &str| {
        format!(
            r#"global func Show()
                   {{
                       SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
                       SetScoreboardData(SBRD_Caption, 1, "Points");
                       SetScoreboardData(1, SBRD_Caption, "{cell}");
                       SetScoreboardData(1, 1, "42");
                       DoScoreboardShow(1);
                   }}"#
        )
    };
    let render = |source: String| {
        let mut app = new_scoreboard_test_app(&source);
        call_scoreboard_function_and_update(&mut app, "Show");
        main_assert!(app.dialogs.scoreboard.is_some());
        let mut frame = vec![0x71; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("an unresolved FontRegular image does not fail the frame");
        (app.graphics.surface().pixels().to_vec(), frame)
    };

    // The frame is drawn rather than refused, and the board reached it.
    let (unresolved, frame) = render(show("{{NO_SUCH_DEFINITION}}"));
    main_assert_ne!(frame => vec![0x71; 320 * 200 * 4], "the board drew over the sentinel");

    // Mixed markup is equally fine, and the board is still a *board*: a
    // resolvable label paints something else.
    render(show("a{{NO_SUCH_DEFINITION}}b"));
    let (plain, _) = render(show("Team"));
    main_assert_ne!(unresolved => plain);
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
        .test_player_mut(app.players.local_owner)
        .control
        .control_style = true;
    app.dispatch_control_event(ControlEvent::Press(ControlButton::Left))
        .test_value();
    main_assert_ne!(app.engine.player(app.players.local_owner).expect("local player").control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0,);
    app.open_ingame_menu().test_value();
    call_scoreboard_function_and_update(&mut app, "ShowAndEnd");
    main_assert!(app.game_over_dialog.is_some());
    main_assert!(app.dialogs.scoreboard.is_none());
    main_assert!(app.ingame_menu.is_none());
    main_assert_eq!(
        app.engine
            .player(app.players.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT) =>
        0,
        "game-over player-menu close synchronizes ClearPressedComs once",
    );
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);

    app.engine
        .call_scenario_script_function("Recheck", Vec::new())
        .test_value();
    app.handle_game_over_action(GameOverAction::Continue)
        .test_value();
    main_assert!(app.game_over_dialog.is_none());
    main_assert!(app.dialogs.scoreboard.is_none());
    app.test_render(&mut frame);

    call_scoreboard_function_and_update(&mut app, "Recheck");
    main_assert!(app.dialogs.scoreboard.is_some());
    app.test_render(&mut frame);

    let mut object_menu = new_classic_scoreboard_test_app(GAME_OVER_BOARD);
    main_assert!(object_menu.open_object_menu().expect("open object menu"));
    call_scoreboard_function_and_update(&mut object_menu, "ShowAndEnd");
    main_assert!(object_menu.game_over_dialog.is_some());
    main_assert!(object_menu.object_menu.is_some(), "C4Player::CloseMenu does not discard synchronized object menus",);
}

#[test]
fn game_over_chat_and_mnemonics_use_exact_modes_and_priority() {
    for (key, modifiers, expected_text) in [
        (VirtualKeyCode::Enter, ModifiersState::empty(), ""),
        (VirtualKeyCode::F2, ModifiersState::empty(), ""),
        (VirtualKeyCode::Enter, ModifiersState::SHIFT, "/team "),
        (VirtualKeyCode::Enter, ModifiersState::SUPER, ""),
        (
            VirtualKeyCode::Enter,
            ModifiersState::SUPER | ModifiersState::SHIFT,
            "/team ",
        ),
    ] {
        let mut app = new_game_over_keyboard_app();
        app.test_modifiers(modifiers);
        app.test_key(key, ElementState::Pressed);
        main_assert_eq!(app.running_chat_text() => Some(expected_text));
        main_assert!(app.game_over_dialog.is_some());
    }

    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::SUPER | ModifiersState::ALT,
    ] {
        let mut app = new_game_over_keyboard_app();
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
        main_assert!(app.game_over_dialog.is_none());
        main_assert_eq!(app.mode => AppMode::Running);
        main_assert!(!app.sound.ui_log.iter().any(|sound| matches!(sound.as_str(), "ArrowHit" | "Click")));
    }

    let mut say = new_game_over_keyboard_app();
    say.game_over_dialog.test_mut().set_button_content(
        GameOverAction::Restart,
        "Play again".to_string(),
        "Restart without an R mnemonic".to_string(),
    );
    say.test_modifiers(ModifiersState::ALT);
    say.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert_eq!(say.running_chat_text() => Some("\""));
    main_assert!(say.game_over_dialog.is_some());

    for (key, modifiers) in [
        (
            VirtualKeyCode::Enter,
            ModifiersState::ALT | ModifiersState::SHIFT,
        ),
        (VirtualKeyCode::Escape, ModifiersState::ALT),
    ] {
        say.test_modifiers(modifiers);
        say.test_key(key, ElementState::Pressed);
        say.test_key(key, ElementState::Released);
        main_assert!(say.game_over_dialog.is_some());
        main_assert_eq!(say.running_chat_text() => Some("\""));
    }

    let mut app = new_game_over_keyboard_app();
    for modifiers in [
        ModifiersState::CONTROL,
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::ALT,
        ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
    ] {
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    }
    for modifiers in [
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::ALT,
    ] {
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
        app.test_key(VirtualKeyCode::F2, ElementState::Released);
    }
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::NumpadEnter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::NumpadEnter, ElementState::Released);
    main_assert!(app.game_over_dialog.is_some());
}

#[test]
fn game_over_mnemonics_use_active_language_resources() {
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "LanguageEx", "DE").test_value();
    let mut app = new_classic_running_sandbox_app();
    app.app_paths = Some(paths);
    app.reload_application_language_resources().test_value();
    app.handle_game_over().test_value();
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyW, ElementState::Pressed);

    main_assert_eq!(app.mode => AppMode::Running);
    main_assert!(app.game_over_dialog.is_none());
    main_assert!(app.running_chat_text().is_none());
    main_assert!(!app.sound.ui_log.iter().any(|sound| matches!(sound.as_str(), "ArrowHit" | "Click")));
}

#[test]
fn game_over_tab_moves_real_focus_and_controls_activate_or_open_chat() {
    let mut list_focus = new_game_over_keyboard_app();
    for _ in 0..2 {
        list_focus.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        list_focus.test_key(VirtualKeyCode::Tab, ElementState::Released);
    }
    main_assert_eq!(list_focus.game_over_dialog.as_ref().and_then(GameOverState::focused) => Some(GameOverFocus::PlayerList(0)));
    list_focus.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert_eq!(list_focus.running_chat_text() => Some(""));
    main_assert!(list_focus.game_over_dialog.is_some());

    let mut keyboard = new_game_over_keyboard_app();
    for _ in 0..4 {
        keyboard.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        keyboard.test_key(VirtualKeyCode::Tab, ElementState::Released);
    }
    main_assert_eq!(keyboard.game_over_dialog.as_ref().and_then(GameOverState::focused_action) => Some(GameOverAction::Continue));
    keyboard.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert!(keyboard.game_over_dialog.is_some());
    keyboard.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert!(keyboard.game_over_dialog.is_none());
    main_assert!(keyboard.sound.ui_log.iter().any(|sound| sound == "ArrowHit"));
    main_assert!(keyboard.sound.ui_log.iter().any(|sound| sound == "Click"));

    let mut gamepad = new_game_over_keyboard_app();
    for _ in 0..4 {
        gamepad.test_gamepad_events([game_over_fixture!(direction:
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )]);
    }
    gamepad.test_gamepad_events([game_over_fixture!(gui_button:
        GamepadSlot::new(0),
        GuiButtonClass::Low,
        ElementState::Pressed,
    )]);
    main_assert!(gamepad.game_over_dialog.is_some());
    gamepad.test_gamepad_events([game_over_fixture!(gui_button:
        GamepadSlot::new(0),
        GuiButtonClass::Low,
        ElementState::Released,
    )]);
    main_assert!(gamepad.game_over_dialog.is_none());
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
            let dialog = app.game_over_dialog.test_mut();
            dialog.handle_pointer_move(x as f32, y as f32, width, height);
            if dialog.hovered_action() == Some(GameOverAction::Continue) {
                continue_point = Some(PhysicalPosition::new(f64::from(x), f64::from(y)));
                break 'find_button;
            }
        }
    }
    let continue_point = continue_point.test_value();
    app.test_cursor(continue_point);
    main_assert_eq!(
        app.game_over_dialog
            .as_ref()
            .and_then(GameOverState::hovered_action) =>
        Some(GameOverAction::Continue),
        "the fixture must distinguish pointer hover from initial keyboard focus"
    );

    for key in [
        VirtualKeyCode::ArrowLeft,
        VirtualKeyCode::ArrowRight,
        VirtualKeyCode::ArrowUp,
        VirtualKeyCode::ArrowDown,
        VirtualKeyCode::Space,
    ] {
        for modifiers in [
            ModifiersState::empty(),
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            ModifiersState::CONTROL | ModifiersState::ALT,
            ModifiersState::SUPER,
        ] {
            app.test_modifiers(modifiers);
            app.test_key(key, ElementState::Pressed);
            app.test_key(key, ElementState::Released);
            main_assert_eq!(
                app.game_over_dialog
                    .as_ref()
                    .and_then(GameOverState::hovered_action) =>
                Some(GameOverAction::Continue),
                "{key:?} with {modifiers:?} must neither focus nor activate a hovered button"
            );
        }
    }
    main_assert!(matches!(app.mode, AppMode::Running));
}

fn hover_game_over_action_for_test(app: &mut GameApp, action: GameOverAction) {
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    for y in 0..height {
        for x in 0..width {
            let dialog = app.game_over_dialog.test_mut();
            dialog.handle_pointer_move(x as f32, y as f32, width, height);
            if dialog.hovered_action() == Some(action) {
                return;
            }
        }
    }
    panic!("game-over action {action:?} has no pointer target");
}

fn assert_game_over_fixture_has_no_sound_activity(app: &GameApp) {
    if let Some(audio) = app.sound.context.as_ref() {
        let audio = audio.borrow();
        main_assert!(!audio.options.sound_enabled);
        main_assert!(!audio.options.menu_sound_enabled);
        main_assert!(audio.active_channels.is_empty(), "game-over input must not synthesize a UI sound");
    }
}

fn assert_no_global_ui_change(before: RuntimeGlobalUiSnapshot, app: &GameApp) {
    main_assert_eq!(runtime_global_ui_snapshot(app) => before);
    assert_game_over_fixture_has_no_sound_activity(app);
}

#[test]
fn game_over_gui_stack_requires_enabled_primary_gamepad_source() {
    for (gamepad_gui_control, gamepad) in [(false, 0), (true, 1)] {
        let slot = GamepadSlot::new(gamepad as u8);
        let mut app = new_game_over_keyboard_app();
        app.config.gamepad_gui_control = gamepad_gui_control;
        hover_game_over_action_for_test(&mut app, GameOverAction::Continue);
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Top overlay",
                "Must remain untouched",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .test_value();
        let before = runtime_global_ui_snapshot(&app);
        let source = |cluster, event| SourcedGamepadEvent {
            gamepad,
            cluster,
            event,
        };
        let gate = app.config.gamepad_gui_control;

        app.process_sourced_gamepad_event_batch(
            [
                source(
                    10,
                    game_over_fixture!(gui_button: slot, GuiButtonClass::Low, ElementState::Pressed),
                ),
                source(
                    10,
                    game_over_fixture!(action: slot, GamepadActionType::Cancel, ElementState::Pressed),
                ),
                source(
                    10,
                    game_over_fixture!(button: slot, LegacyGamepadButton::new(1), ElementState::Pressed),
                ),
                source(
                    11,
                    game_over_fixture!(gui_button: slot, GuiButtonClass::Low, ElementState::Released),
                ),
                source(
                    11,
                    game_over_fixture!(action: slot, GamepadActionType::Cancel, ElementState::Released),
                ),
                source(
                    11,
                    game_over_fixture!(button: slot, LegacyGamepadButton::new(1), ElementState::Released),
                ),
            ],
            gate,
        )
        .test_value();

        assert_no_global_ui_change(before, &app);
        main_assert_eq!(app.dialogs.messages.len() => 1);
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
    .test_value();

    app.test_gamepad_events([
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::High, ElementState::Pressed),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Cancel, ElementState::Pressed),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Left, ElementState::Pressed),
    ]);
    main_assert!(app.dialogs.messages.is_empty());
    main_assert_eq!(app.game_over_dialog.as_ref().and_then(GameOverState::focused) => Some(GameOverFocus::Button(2)));
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
        .test_value();
    };

    let mut axis_transition = new_game_over_keyboard_app();
    open_context(&mut axis_transition);
    axis_transition
        .process_sourced_gamepad_event_batch(
            [
                SourcedGamepadEvent {
                    gamepad: 0,
                    cluster: 40,
                    event: game_over_fixture!(axis:
                        GamepadSlot::new(0),
                        LegacyGamepadAxis::new(0, false),
                        ElementState::Released,
                    ),
                },
                SourcedGamepadEvent {
                    gamepad: 0,
                    cluster: 40,
                    event: game_over_fixture!(direction:
                        GamepadSlot::new(0),
                        ControlButton::Left,
                        ElementState::Released,
                    ),
                },
                SourcedGamepadEvent {
                    gamepad: 0,
                    cluster: 41,
                    event: game_over_fixture!(axis:
                        GamepadSlot::new(0),
                        LegacyGamepadAxis::new(0, true),
                        ElementState::Pressed,
                    ),
                },
                SourcedGamepadEvent {
                    gamepad: 0,
                    cluster: 41,
                    event: game_over_fixture!(direction:
                        GamepadSlot::new(0),
                        ControlButton::Right,
                        ElementState::Pressed,
                    ),
                },
            ],
            true,
        )
        .test_value();
    main_assert!(axis_transition.context_menu.is_some());
    main_assert!(axis_transition.game_over_dialog.is_some());
    main_assert_eq!(axis_transition.game_over_dialog.as_ref().and_then(GameOverState::focused) => None);

    let mut pass_through = new_game_over_keyboard_app();
    open_context(&mut pass_through);
    pass_through.test_gamepad_events([game_over_fixture!(direction:
        GamepadSlot::new(0),
        ControlButton::Left,
        ElementState::Pressed,
    )]);
    main_assert!(pass_through.context_menu.is_some());
    main_assert_eq!(pass_through.game_over_dialog.as_ref().and_then(GameOverState::focused) => None);

    let mut closed = new_game_over_keyboard_app();
    open_context(&mut closed);
    closed.test_gamepad_events([
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::High, ElementState::Pressed),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Cancel, ElementState::Pressed),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Right, ElementState::Pressed),
    ]);
    main_assert!(closed.context_menu.is_none());
    main_assert_eq!(closed.game_over_dialog.as_ref().and_then(GameOverState::focused) => Some(GameOverFocus::Close));
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
        app.test_gamepad_events([
            game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::Low, ElementState::Pressed),
            game_over_fixture!(action: GamepadSlot::new(0), action, ElementState::Pressed),
            game_over_fixture!(button: GamepadSlot::new(0), button, ElementState::Pressed),
        ]);
        main_assert_eq!(app.running_chat_text() => Some(""));
        main_assert!(app.game_over_dialog.is_some(), "{source} is Low/chat even when its abstract alias is Cancel");
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
        app.test_gamepad_events([game_over_fixture!(direction:
            GamepadSlot::new(0),
            button,
            ElementState::Pressed,
        )]);
        main_assert_eq!(app.game_over_dialog.as_ref().and_then(GameOverState::focused) => Some(expected));
    }
}

#[test]
fn game_over_raw_vertical_releases_clear_and_abstract_aliases_are_inert() {
    let mut app = new_game_over_keyboard_app();
    hover_game_over_action_for_test(&mut app, GameOverAction::Continue);
    let before = runtime_global_ui_snapshot(&app);

    app.test_gamepad_events([
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Up, ElementState::Pressed),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Down, ElementState::Pressed),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Left, ElementState::Released),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Right, ElementState::Released),
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::Low, ElementState::Released),
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::High, ElementState::Released),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Select, ElementState::Pressed),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Cancel, ElementState::Pressed),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::MenuToggle, ElementState::Pressed),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(0), ElementState::Pressed),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(0), ElementState::Released),
    ]);
    assert_no_global_ui_change(before, &app);

    let clear_before = runtime_global_ui_snapshot(&app);
    app.test_gamepad_events([GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    }]);
    assert_no_global_ui_change(clear_before, &app);

    let direct_before = runtime_global_ui_snapshot(&app);
    for action in [
        GamepadActionType::Select,
        GamepadActionType::Cancel,
        GamepadActionType::MenuToggle,
    ] {
        app.handle_gamepad_action(GamepadSlot::new(0), action, ElementState::Pressed)
            .test_value();
    }
    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0),
        ElementState::Pressed,
    )
    .test_value();
    for button in [
        ControlButton::Left,
        ControlButton::Right,
        ControlButton::Up,
        ControlButton::Down,
    ] {
        app.handle_gamepad_direction(GamepadSlot::new(0), button, ElementState::Pressed)
            .test_value();
    }
    main_assert_eq!(runtime_global_ui_snapshot(&app) => direct_before);
    assert_game_over_fixture_has_no_sound_activity(&app);

    let mut cancelled = new_game_over_keyboard_app();
    for _ in 0..4 {
        cancelled.test_gamepad_events([game_over_fixture!(direction:
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )]);
    }
    cancelled.test_gamepad_events([game_over_fixture!(gui_button:
        GamepadSlot::new(0),
        GuiButtonClass::Low,
        ElementState::Pressed,
    )]);
    main_assert_eq!(cancelled.sound.ui_log.iter().filter(|sound| sound.as_str() == "ArrowHit").count() => 1);
    cancelled.test_gamepad_events([GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    }]);
    cancelled.test_gamepad_events([game_over_fixture!(gui_button:
        GamepadSlot::new(0),
        GuiButtonClass::Low,
        ElementState::Released,
    )]);
    main_assert!(cancelled.game_over_dialog.is_some());
    main_assert!(!cancelled.sound.ui_log.iter().any(|sound| sound == "Click"));
}

#[test]
fn game_over_raw_high_ends_and_consumes_aliases_after_dialog_close() {
    let mut app = new_game_over_keyboard_app();
    assert_game_over_fixture_has_no_sound_activity(&app);

    app.test_gamepad_events([
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::High, ElementState::Pressed),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::MenuToggle, ElementState::Pressed),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(8), ElementState::Pressed),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(9), ElementState::Pressed),
        GamepadEvent::Clear {
            slot: GamepadSlot::new(0),
        },
    ]);

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::MainMenu);
    main_assert!(app.game_over_dialog.is_none());
    main_assert!(app.ingame_menu.is_none());
    main_assert!(app.status_text.is_empty());
    main_assert!(!app.exit_requested, "the paired MenuToggle alias must not reach the exposed main menu");
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
                game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Down, ElementState::Pressed),
            ),
            // A new South cluster presses and releases the focused button.
            source(
                cluster + 1,
                game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::Low, ElementState::Pressed),
            ),
            source(
                cluster + 1,
                game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Select, ElementState::Pressed),
            ),
            source(
                cluster + 1,
                game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(0), ElementState::Pressed),
            ),
            source(
                cluster + 2,
                game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::Low, ElementState::Released),
            ),
            source(
                cluster + 2,
                game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Select, ElementState::Released),
            ),
            source(
                cluster + 2,
                game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(0), ElementState::Released),
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
                game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::High, ElementState::Pressed),
            ),
            source(
                20,
                game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::MenuToggle, ElementState::Pressed),
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
    .test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::MainMenu);
    main_assert!(app.startup_dialog_fade_active());
    let mut frame = vec![0_u8; 320 * 200 * 4];
    for _ in 0..STARTUP_DIALOG_FADE_STEPS {
        app.test_render(&mut frame);
    }
    main_assert!(!app.startup_dialog_fade_active());

    app.process_sourced_gamepad_event_batch(activate_network_game(24), true)
        .test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert!(!app.exit_requested);
    main_assert!(app.game_over_dialog.is_none());
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
        .test_value();
    };

    open_message(&mut app, "Low");
    app.test_gamepad_events([
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::Low, ElementState::Pressed),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Select, ElementState::Pressed),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(0), ElementState::Pressed),
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::Low, ElementState::Released),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Select, ElementState::Released),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(0), ElementState::Released),
    ]);
    main_assert!(app.dialogs.messages.is_empty());

    open_message(&mut app, "High");
    app.test_gamepad_events([
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::High, ElementState::Pressed),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Cancel, ElementState::Pressed),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(1), ElementState::Pressed),
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::High, ElementState::Released),
        game_over_fixture!(action: GamepadSlot::new(0), GamepadActionType::Cancel, ElementState::Released),
        game_over_fixture!(button: GamepadSlot::new(0), LegacyGamepadButton::new(1), ElementState::Released),
    ]);

    main_assert!(app.dialogs.messages.is_empty());
    main_assert_eq!(app.mode => AppMode::Running);
    main_assert_eq!(
        app.game_over_dialog
            .as_ref()
            .and_then(GameOverState::hovered_action) =>
        None,
        "closing the top modal clears pointer hover without closing evaluation"
    );
    main_assert!(app.status_text.is_empty());
    assert_game_over_fixture_has_no_sound_activity(&app);
}

#[test]
fn game_over_tab_and_escape_use_exact_modifier_masks() {
    for (modifiers, expected) in [
        (ModifiersState::empty(), GameOverFocus::Close),
        (ModifiersState::SHIFT, GameOverFocus::Button(2)),
        (ModifiersState::SUPER, GameOverFocus::Close),
        (
            ModifiersState::SUPER | ModifiersState::SHIFT,
            GameOverFocus::Button(2),
        ),
    ] {
        let mut app = new_game_over_keyboard_app();
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Tab, ElementState::Released);
        main_assert_eq!(app.game_over_dialog.as_ref().and_then(GameOverState::focused) => Some(expected));
    }
    for modifiers in [
        ModifiersState::CONTROL,
        ModifiersState::CONTROL | ModifiersState::ALT,
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
    ] {
        let mut app = new_game_over_keyboard_app();
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Tab, ElementState::Released);
        main_assert_eq!(app.game_over_dialog.as_ref().and_then(GameOverState::focused) => None);
    }

    for modifiers in [
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::ALT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
    ] {
        let mut app = new_game_over_keyboard_app();
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::Escape, ElementState::Released);
        app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
        main_assert!(app.game_over_dialog.is_some());
    }

    for modifiers in [ModifiersState::empty(), ModifiersState::SUPER] {
        let mut ending_app = new_game_over_keyboard_app();
        ending_app.test_modifiers(modifiers);
        ending_app.test_key(VirtualKeyCode::Escape, ElementState::Released);
        main_assert!(ending_app.game_over_dialog.is_some());
        ending_app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
        main_assert!(ending_app.game_over_dialog.is_none());
        main_assert!(matches!(ending_app.mode, AppMode::Menu));
    }
}

#[test]
fn game_over_pending_network_result_preserves_cpp_button_and_escape_latches() {
    let pending_host = || {
        let mut app = new_classic_running_sandbox_app();
        configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
        app.network_is_league = true;
        app.handle_game_over().test_value();
        app
    };

    let mut host = pending_host();
    let dialog = host.game_over_dialog.test_ref();
    main_assert_eq!(dialog.network_result_label() => Some(""));
    main_assert!(!dialog.is_net_done());
    main_assert!(!dialog.allows_escape_close());
    main_assert!(dialog.actions().contains(&GameOverAction::End));
    main_assert!(dialog.actions().contains(&GameOverAction::Continue));
    host.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(host.game_over_dialog.is_some());
    host.handle_game_over_gamepad_event(game_over_fixture!(gui_button:
        GamepadSlot::new(0),
        GuiButtonClass::High,
        ElementState::Pressed,
    ))
    .test_value();
    main_assert!(host.game_over_dialog.is_some());

    let mut clickable = pending_host();
    clickable.test_modifiers(ModifiersState::ALT);
    clickable.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    main_assert!(clickable.game_over_dialog.is_none());
    main_assert_eq!(clickable.mode => AppMode::Running);

    let mut resolved = pending_host();
    resolved.snapshot.round_results.network_result =
        Some(clonk_engine::RoundResultsNetworkResult::LeagueOk);
    resolved.snapshot.round_results.network_result_message = b"evaluated".to_vec();
    main_assert!(resolved.sec1_timer().expect("refresh final network result"));
    let dialog = resolved.game_over_dialog.test_ref();
    main_assert_eq!(dialog.network_result_label() => Some("evaluated"));
    main_assert!(dialog.is_net_done());
    main_assert!(dialog.allows_escape_close());
    resolved.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(resolved.game_over_dialog.is_none());

    let mut client = new_classic_running_sandbox_app();
    configure_runtime_network_role(&mut client, RuntimeNetworkRole::Client);
    client.network_is_league = true;
    client.handle_game_over().test_value();
    main_assert!(client.game_over_dialog.as_ref().is_some_and(GameOverState::allows_escape_close));
    client.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(client.game_over_dialog.is_none());
}

#[test]
fn game_over_show_and_continue_use_offline_pause_lifecycle() {
    // C4GameOverDlg::OnShown invokes Game.Pause, while only an accepted
    // Continue close invokes Game.Unpause. Raw dialog teardown does not
    // resume the round (src/C4GameOverDlg.cpp:349-381;
    // src/C4Game.cpp:1045-1084).
    let mut app = new_classic_running_sandbox_app();
    main_assert_eq!(app.offline_halt_count => 0);

    app.handle_game_over().test_value();
    main_assert_eq!(app.offline_halt_count => 1, "OnShown acquires the native offline game halt");
    app.handle_game_over_action(GameOverAction::Continue)
        .test_value();
    main_assert_eq!(app.offline_halt_count => 0);
    main_assert!(app.game_over_dialog.is_none());

    let mut raw_teardown = new_classic_running_sandbox_app();
    raw_teardown.handle_game_over().test_value();
    raw_teardown.dismiss_game_over_dialog();
    main_assert_eq!(raw_teardown.offline_halt_count => 1, "destroying the dialog without Continue must not call Unpause");
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

    host.handle_game_over().test_value();
    let pause_changes = host_commands
        .take_runtime_status_commands()
        .into_iter()
        .filter_map(|command| match command {
            network::TestRuntimeStatusCommand::Change(status) => Some(status),
            network::TestRuntimeStatusCommand::Reached { .. } => None,
        })
        .collect::<Vec<_>>();
    main_assert_eq!(pause_changes.len() => 1);
    main_assert_eq!(pause_changes[0].state => clonk_network::NETWORK_STATE_PAUSE);

    host.handle_game_over_action(GameOverAction::Continue)
        .test_value();
    let go_changes = host_commands
        .take_runtime_status_commands()
        .into_iter()
        .filter_map(|command| match command {
            network::TestRuntimeStatusCommand::Change(status) => Some(status),
            network::TestRuntimeStatusCommand::Reached { .. } => None,
        })
        .collect::<Vec<_>>();
    main_assert_eq!(go_changes.len() => 1);
    main_assert_eq!(go_changes[0].state => clonk_network::NETWORK_STATE_GO);

    let mut client = new_classic_running_sandbox_app();
    let (_events, mut client_commands) = install_running_network_stub(&mut client, 7, 0, 2);
    client.handle_game_over().test_value();
    main_assert!(client_commands.take_runtime_status_commands().is_empty());

    // Model the host's committed Pause. Closing the local dialog must not
    // let a client resume synchronized control independently.
    client.network_control_running = false;
    client
        .handle_game_over_action(GameOverAction::Continue)
        .test_value();
    main_assert!(client_commands.take_runtime_status_commands().is_empty());
    main_assert!(!client.network_control_running);
}

#[test]
fn runtime_f3_obeys_player_modifier_game_over_and_key_config_priority() {
    let mut player = new_running_sandbox_app();
    player
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
    player
        .engine
        .test_player_mut(player.players.local_owner)
        .control
        .control_style = true;
    player.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(player.runtime_flash_message.is_none());
    main_assert_ne!(player.engine.player(player.players.local_owner).expect("local player").control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0);

    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL,
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
    ] {
        let mut modified = new_running_sandbox_app();
        modified
            .bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
        modified.test_modifiers(modifiers);
        let before = runtime_global_ui_snapshot(&modified);
        modified.test_key(VirtualKeyCode::F3, ElementState::Pressed);
        modified.test_key(VirtualKeyCode::F3, ElementState::Released);
        main_assert_eq!(runtime_global_ui_snapshot(&modified) => before);
    }

    let mut logo_music = new_running_sandbox_app();
    let configured_before_logo = logo_music.test_audio_ref().options.music_enabled;
    logo_music.test_modifiers(ModifiersState::SUPER);
    logo_music.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(logo_music.runtime_flash_message.is_some());
    main_assert_eq!(logo_music.test_audio_ref().options.music_enabled => configured_before_logo);

    let mut logo_sound = new_running_sandbox_app();
    let sound_before_logo = logo_sound.test_audio_ref().options.sound_enabled;
    logo_sound.test_modifiers(ModifiersState::CONTROL | ModifiersState::SUPER);
    logo_sound.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert_eq!(logo_sound.test_audio_ref().options.sound_enabled => !sound_before_logo);
    main_assert!(logo_sound.runtime_flash_message.is_none());

    let mut sound = new_running_sandbox_app();
    let before_sound = sound
        .sound.context
        .as_ref()
        .map(|audio| audio.borrow().options.sound_enabled);
    sound.test_modifiers(ModifiersState::CONTROL);
    sound.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(sound.runtime_flash_message.is_none());
    if let (Some(before), Some(audio)) = (before_sound, sound.sound.context.as_ref()) {
        let audio = audio.borrow();
        main_assert_eq!(audio.options.sound_enabled => !before);
    }

    let mut existing_sound = new_running_sandbox_app();
    {
        let mut audio = existing_sound.test_audio_mut();
        let handle = audio.system.load_sound(&silent_pcm_wav(1_000)).test_value();
        let duration_ms = handle.duration_ms().test_value();
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
    }
    existing_sound.test_modifiers(ModifiersState::CONTROL);
    existing_sound.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(
        existing_sound
            .test_audio_ref()
            .active_channels
            .contains_key(&SoundInstanceKey::new("Loop", None)),
        "C4SoundSystem::ToggleOnOff does not halt existing instances"
    );

    let mut game_over = new_game_over_keyboard_app();
    game_over
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
    game_over.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(game_over.runtime_flash_message.is_some());

    let mut custom = new_running_sandbox_app();
    custom.runtime_key_config_cache = OnceLock::new();
    custom
        .runtime_key_config_cache
        .set(Err("Extra.c4g/KeyConfig.txt override".to_string()))
        .test_value();
    for state in [ElementState::Pressed, ElementState::Released] {
        let error = custom
            .handle_key(VirtualKeyCode::F3, state)
            .expect_err("custom global F3 ownership must fail closed");
        main_assert!(error.to_string().contains("timed flash-message resources"));
        main_assert!(custom.runtime_flash_message.is_none());
    }
}

#[test]
fn older_runtime_f4_dialog_renders_inactive_below_new_game_over_dialog() {
    let mut app = new_classic_running_sandbox_app();
    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Host);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    main_assert!(app.runtime_client_list_mouse_active());
    main_assert!(!app.runtime_client_list_keyboard_active());
    main_assert!(app.runtime_client_list_draw_active());

    app.handle_game_over().test_value();
    main_assert!(app.dialogs.client_list.is_some());
    main_assert!(app.game_over_dialog.is_some());
    main_assert!(!app.dialogs.client_list_above_game_over);
    main_assert!(!app.runtime_client_list_mouse_active());
    main_assert!(!app.runtime_client_list_keyboard_active());
    main_assert!(!app.runtime_client_list_draw_active());
}

#[test]
fn runtime_f4_precedes_game_over_message_and_ingame_menus() {
    let mut game_over = new_game_over_keyboard_app();
    let (_events, mut game_over_commands) = install_running_network_stub(&mut game_over, 0, 40, 4);
    route_primary_gamepad_to_local_owner(&mut game_over);
    game_over.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    main_assert!(game_over.dialogs.client_list.is_some());
    main_assert!(game_over.game_over_dialog.is_some());
    main_assert!(game_over.dialogs.client_list_above_game_over);
    game_over.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    game_over.test_key(VirtualKeyCode::Tab, ElementState::Released);
    game_over.test_modifiers(ModifiersState::ALT);
    game_over.test_key(VirtualKeyCode::KeyR, ElementState::Pressed);
    game_over.test_key(VirtualKeyCode::KeyR, ElementState::Released);
    game_over.test_modifiers(ModifiersState::empty());
    game_over.test_gamepad_events([
        game_over_fixture!(axis: GamepadSlot::new(0), LegacyGamepadAxis::new(0, true), ElementState::Pressed),
        game_over_fixture!(direction: GamepadSlot::new(0), ControlButton::Right, ElementState::Pressed),
        game_over_fixture!(gui_button: GamepadSlot::new(0), GuiButtonClass::Low, ElementState::Pressed),
    ]);
    main_assert_eq!(game_over.game_over_dialog.as_ref().and_then(GameOverState::focused) => None);
    let submitted = game_over_commands.take_submitted_local();
    main_assert_eq!(submitted.len() => 1);
    main_assert!(matches!(submitted[0].1, ControlEvent::Press(ControlButton::Right)));
    main_assert!(game_over.running_chat_text().is_none());
    main_assert!(game_over.dialogs.client_list.is_some());
    game_over.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(game_over.dialogs.client_list.is_none());
    main_assert!(game_over.game_over_dialog.is_some());

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
        .test_value();
    message.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    main_assert!(message.dialogs.client_list.is_some());
    main_assert_eq!(message.dialogs.messages.len() => 1);

    let mut ingame = new_running_sandbox_app();
    configure_runtime_network_role(&mut ingame, RuntimeNetworkRole::Host);
    ingame.open_ingame_menu().test_value();
    ingame.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    main_assert!(ingame.dialogs.client_list.is_some());
    main_assert!(ingame.ingame_menu.is_some());
}

#[test]
fn runtime_pause_is_game_over_noop_but_precedes_other_running_dialogs() {
    let mut game_over = new_game_over_keyboard_app();
    game_over.test_modifiers(ModifiersState::SUPER);
    let before_game_over = runtime_global_ui_snapshot(&game_over);
    for (state, held) in [
        (ElementState::Pressed, true),
        (ElementState::Pressed, true),
        (ElementState::Released, false),
    ] {
        game_over.test_key(VirtualKeyCode::Pause, state);
        // `C4Game::DoKeyboardInput` records the raw physical edge for every
        // key ahead of any scope decision (C4Game.cpp:2143-2155), including
        // the keys the round-evaluation gate then discards. That latch is
        // the only state this sequence may move.
        let mut after = runtime_global_ui_snapshot(&game_over);
        main_assert_eq!(after.pressed_engine_keys.contains(&VirtualKeyCode::Pause) => held, "the discarded Pause edge still updates the held-key latch");
        after
            .pressed_engine_keys
            .clone_from(&before_game_over.pressed_engine_keys);
        main_assert_eq!(after => before_game_over);
        main_assert_eq!(game_over.offline_halt_count => 1, "the Pause key cannot release OnShown's evaluation halt");
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
        .test_value();
    message.test_key(VirtualKeyCode::Pause, ElementState::Pressed);
    main_assert_ne!(message.offline_halt_count => 0);
    main_assert_eq!(message.dialogs.messages.len() => 1);

    let mut ingame = new_running_sandbox_app();
    ingame.open_ingame_menu().test_value();
    ingame.test_key(VirtualKeyCode::Pause, ElementState::Pressed);
    main_assert_ne!(ingame.offline_halt_count => 0);
    main_assert!(ingame.ingame_menu.is_some());
}

#[test]
fn modified_runtime_globals_retain_higher_priority_game_over_mnemonics() {
    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::SUPER | ModifiersState::ALT,
    ] {
        let mut app = new_game_over_keyboard_app();
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
        main_assert!(app.game_over_dialog.is_none());
        main_assert_eq!(app.mode => AppMode::Running);
        main_assert!(app.running_chat_text().is_none());
    }

    for key in [
        VirtualKeyCode::F1,
        VirtualKeyCode::F4,
        VirtualKeyCode::Pause,
    ] {
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::SUPER | ModifiersState::ALT,
        ] {
            let mut app = new_game_over_keyboard_app();
            app.test_modifiers(modifiers);
            app.test_key(key, ElementState::Pressed);
            main_assert!(app.game_over_dialog.is_some());
            main_assert!(app.running_chat_text().is_none());
            main_assert!(!app.dialogs.help_visible);
            main_assert!(app.dialogs.client_list.is_none());
        }
    }
}

#[test]
fn abort_confirmation_declines_confirms_and_restarts() {
    let mut declined = new_running_sandbox_app();
    declined.test_update();
    let declined_frame = declined.engine.frame();
    main_assert!(declined_frame > 0);
    let declined_scenario = declined.active_scenario.test_ref().identifier.clone();
    declined.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    finish_abort_dialog(
        &mut declined,
        clonk_frontend::message_dialog::MessageDialogResult::No,
    );
    main_assert!(declined.ingame_menu.is_none());
    main_assert!(declined.dialogs.messages.is_empty());
    main_assert!(matches!(declined.mode, AppMode::Running));
    main_assert_eq!(declined.active_scenario.as_ref().map(|active| active.identifier.as_str()) => Some(declined_scenario.as_str()));
    main_assert_eq!(declined.engine.frame() => declined_frame);

    let mut confirmed = new_running_sandbox_app();
    confirmed.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    finish_abort_dialog(
        &mut confirmed,
        clonk_frontend::message_dialog::MessageDialogResult::Yes,
    );
    main_assert!(matches!(confirmed.mode, AppMode::Menu));
    main_assert!(confirmed.active_scenario.is_none());
    main_assert!(confirmed.ingame_menu.is_none());

    let mut restarted = new_running_sandbox_app();
    restarted.test_update();
    main_assert!(restarted.engine.frame() > 0);
    let scenario = restarted.active_scenario.test_ref().identifier.clone();
    restarted.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    finish_abort_dialog(
        &mut restarted,
        clonk_frontend::message_dialog::MessageDialogResult::Restart,
    );
    wait_for_running(&mut restarted);
    main_assert_eq!(restarted.active_scenario.as_ref().map(|active| active.identifier.as_str()) => Some(scenario.as_str()));
    main_assert_eq!(restarted.engine.frame() => 0);
    main_assert!(restarted.ingame_menu.is_none());
    main_assert!(restarted.dialogs.messages.is_empty());
}

#[test]
fn restart_is_control_host_only_and_game_over_suppresses_abort() {
    let mut client = new_running_sandbox_app();
    client.engine.set_control_host(false);
    client.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    let client_dialog = client.dialogs.messages.last().test_value();
    main_assert_eq!(client_dialog.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::YES_NO);
    main_assert_eq!(client_dialog.state.size() => clonk_frontend::message_dialog::MessageDialogSize::Small);

    let mut film_client = new_running_sandbox_app();
    film_client.engine.set_control_host(false);
    set_test_scenario_head_flags(&mut film_client, 0, 2);
    let (_film_events, _film_commands) = install_running_network_stub(&mut film_client, 7, 0, 1);
    film_client.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    let film_dialog = film_client.dialogs.messages.last().test_value();
    main_assert_eq!(film_dialog.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::YES_RESTART_NO);
    main_assert_eq!(film_dialog.state.size() => clonk_frontend::message_dialog::MessageDialogSize::Fixed(400));
    film_client.loader_render_error = Some("test restart blocker".to_string());
    finish_abort_dialog(
        &mut film_client,
        clonk_frontend::message_dialog::MessageDialogResult::Restart,
    );
    main_assert!(!film_client.abort_restart_pending);
    main_assert_eq!(film_client.scensel.mode => ScenarioSelectorMode::NetworkHost, "C++ preserves NetworkActive for a Film2 client's NextMission");

    let mut game_over = new_game_over_keyboard_app();
    game_over
        .apply_ingame_menu_action(MenuAction::Abort)
        .test_value();
    main_assert!(game_over.game_over_dialog.is_some());
    main_assert!(game_over.dialogs.messages.is_empty());
    main_assert!(matches!(game_over.mode, AppMode::Running));
}

#[test]
fn modified_escape_does_not_match_the_abort_binding() {
    let mut app = new_running_sandbox_app();
    app.status_text.clear();
    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
    ] {
        app.test_modifiers(modifiers);
        app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Escape, ElementState::Released);
        main_assert!(app.ingame_menu.is_none());
        main_assert!(app.object_menu.is_none());
        main_assert!(app.status_text.is_empty());
    }
    app.test_modifiers(ModifiersState::SUPER);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.dialogs.messages.last().is_some_and(|dialog| matches!(dialog.continuation, MessageDialogContinuation::AbortGame { .. })));
    app.test_modifiers(ModifiersState::empty());
}

#[test]
fn host_vote_timeout_is_strict_and_restarts_on_the_oldest_subject() {
    // The host rejects the first stored vote only when wall time is
    // strictly greater than iVoteStartTime + 10, then immediately resets
    // iVoteStartTime while the synchronized VoteEnd is pending
    // (src/C4Network2.cpp:723-731; src/C4Network2.h:69-72).
    let kick = game_over_fixture!(vote: clonk_engine::VOTE_TYPE_KICK, true, 7, 2);
    let cancel = game_over_fixture!(vote: clonk_engine::VOTE_TYPE_CANCEL, true, 0, 3);
    let mut votes = LeagueVoteState::default();
    votes.add_at(kick, 100);
    votes.add_at(cancel, 105);

    main_assert_eq!(votes.take_timed_out_subject_at(110) => None);
    main_assert_eq!(votes.take_timed_out_subject_at(111) => Some(LeagueVoteSubject::from(kick)));
    main_assert_eq!(votes.take_timed_out_subject_at(121) => None);
    main_assert_eq!(votes.take_timed_out_subject_at(122) => Some(LeagueVoteSubject::from(kick)));
}

#[test]
fn ending_vote_restarts_timeout_for_the_next_subject() {
    // EndVote resets iVoteStartTime even when another subject remains in
    // Votes, so that subject gets a fresh strict ten-second window
    // (src/C4Network2.cpp:2888-2903).
    let kick = game_over_fixture!(vote: clonk_engine::VOTE_TYPE_KICK, true, 7, 2);
    let cancel = game_over_fixture!(vote: clonk_engine::VOTE_TYPE_CANCEL, true, 0, 3);
    let mut votes = LeagueVoteState::default();
    votes.add_at(kick, 100);
    votes.add_at(cancel, 105);

    main_assert_eq!(votes.end_at(LeagueVoteSubject::from(kick), false, None, 106) => Some(2));
    main_assert_eq!(votes.take_timed_out_subject_at(116) => None);
    main_assert_eq!(votes.take_timed_out_subject_at(117) => Some(LeagueVoteSubject::from(cancel)));
}

/// `FnSetNextMission` resolves an omitted button text or description through
/// `LoadResStr(IDS_BTN_NEXTSCENARIO)` / `IDS_DESC_NEXTSCENARIO`
/// (C4Script.cpp:6244-6259). An explicit empty string is kept, because
/// `C4String::Data.getData()` is non-null for it, and an explicit custom string
/// is kept verbatim.
#[test]
fn set_next_mission_omitted_labels_use_active_runtime_resources() {
    let mut app = new_classic_running_sandbox_app();
    app.startup_tooltip_resources.insert(
        "IDS_BTN_NEXTSCENARIO".to_string(),
        "&Naechstes Szenario".to_string(),
    );
    app.startup_tooltip_resources.insert(
        "IDS_DESC_NEXTSCENARIO".to_string(),
        "Mit dem naechsten Szenario fortfahren.".to_string(),
    );
    // The same host-state pass that seeds every other resource string.
    app.apply_material_library();

    let run = |app: &mut GameApp, script: &str| {
        app.engine
            .execute_script_control(
                &clonk_engine::ScriptControlData {
                    target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                    strictness: clonk_engine::ScriptStrictness::Strict3,
                    script: LegacyCString::from_bytes(script.as_bytes().to_vec())
                        .expect("script has no NUL"),
                    by_client: 0,
                },
                ScriptControlPolicy::live(false),
            )
            .test_value();
    };

    // Omitted arguments take the active language table.
    run(
        &mut app,
        r#"SetNextMission("Tutorial.c4f\\Tutorial02.c4s");"#,
    );
    main_assert_eq!(app.engine.next_mission().text => "&Naechstes Szenario");
    main_assert_eq!(app.engine.next_mission().description => "Mit dem naechsten Szenario fortfahren.");

    // An explicit empty string stays empty.
    run(
        &mut app,
        r#"SetNextMission("Tutorial.c4f\\Tutorial02.c4s", "", "");"#,
    );
    main_assert_eq!(app.engine.next_mission().text => "");
    main_assert_eq!(app.engine.next_mission().description => "");

    // Explicit custom text is verbatim, and one omitted argument still
    // resolves independently of the other.
    run(
        &mut app,
        r#"SetNextMission("Tutorial.c4f\\Tutorial02.c4s", "Weiter");"#,
    );
    main_assert_eq!(app.engine.next_mission().text => "Weiter");
    main_assert_eq!(app.engine.next_mission().description => "Mit dem naechsten Szenario fortfahren.");
}

#[test]
fn next_mission_action_launches_the_catalog_target() {
    // C4GameOverDlg's Next button passes Game.NextMission through
    // C4Application::SetNextMission/QuitGame and starts that scenario
    // (C4GameOverDlg.cpp:335-382; C4Application.cpp:373-399).
    let fixture = tempdir();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    configure_test_startup_participant(&paths, user_data.path());
    let mut app = new_menu_app_with_paths(320, 200, &paths);
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    wait_for_running(&mut app);
    let target_path = fixture.path().join("Tutorial02.c4s");
    let carried_definition = fixture.path().join("Carry.c4d");
    fs::create_dir_all(&target_path).test_value();
    fs::create_dir_all(&carried_definition).test_value();
    fs::write(
        target_path.join("Scenario.txt"),
        "[Head]\nTitle=The First Hut\n",
    )
    .test_value();
    fs::write(
        carried_definition.join("DefCore.txt"),
        "[DefCore]\nid=CARY\nName=Carry\nCategory=1\n",
    )
    .test_value();
    fs::write(carried_definition.join("Script.c"), "// carried\n").test_value();
    write_test_definition_graphics(&carried_definition);
    let mut target = FrontendScenario::fallback();
    target.identifier = "Tutorial.c4f/Tutorial02.c4s".to_string();
    target.title = "The First Hut".to_string();
    target.path = Some(target_path);
    app.scensel.catalog
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
    app.engine.restore_state(&state).test_value();

    app.handle_game_over_action(GameOverAction::NextMission)
        .test_value();
    wait_for_running(&mut app);

    main_assert_eq!(app.active_scenario.as_ref().map(|scenario| scenario.identifier.as_str()) => Some("Tutorial.c4f/Tutorial02.c4s"));
    main_assert!(matches!(
        app.active_definition_load.as_ref(),
        Some(ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root: None,
        }) if modules == &[carried_definition.to_string_lossy().as_ref()]
    ));
    main_assert!(app.game_over_dialog.is_none());
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
    app.engine.restore_state(&state).test_value();

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
        app.resize(width, 720).test_value();
        set_test_scenario_head_flags(&mut app, 0, film);
        app.engine.set_control_host(control_host);
        app.finish_game_over_after_league().test_value();
        main_assert_eq!(app.game_over_dialog.as_ref().expect("evaluation dialog").actions() => expected, "control_host={control_host}, Film={film}, width={width}");
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
        |_player_info_id| None,
        |_player_info_id| None,
        |_icon_spec, _color| None,
        |_player_info_id| None,
        false,
    );

    main_assert_eq!(
        dialog.actions() =>
        vec![
            GameOverAction::End,
            GameOverAction::Continue,
            GameOverAction::NextMission,
        ],
        "Tutorial02 is exposed through the classic next-mission button"
    );
    main_assert_eq!(dialog.evaluation().goals().len() => 1);
    let goal = &dialog.evaluation().goals()[0];
    main_assert_eq!(goal.definition_id => "SCRG");
    main_assert!(goal.fulfilled);
    main_assert_eq!(goal.tooltip => "Goal Scenario goal fulfilled: Complete the scenario");
    main_assert_eq!(goal.picture.as_ref().map(|image| image.pixels().to_vec()) => Some(vec![12, 34, 56, 255]));
    let player = dialog.evaluation().player_by_info_id(41).test_value();
    main_assert_eq!(player.name => "Player");
    main_assert!(player.won, "won comes from frozen player info, not Active");
    main_assert_eq!(player.color_dw => 0x00e8_0000);
    main_assert_eq!(player.total_playing_time => 3_661);
    main_assert_eq!((player.score_old, player.score_new) => (10, Some(110)));
    main_assert_eq!(player.big_icon.as_ref() => Some(&player_icon));
    main_assert_eq!(dialog.evaluation().players().count() => 1, "a result keyed like the runtime number must not attach to the player");

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
        |_| None,
        |_player_info_id| None,
        |_player_info_id| None,
        |_icon_spec, _color| None,
        |_player_info_id| None,
        false,
    );
    let player = hidden.evaluation().player_by_info_id(41).test_value();
    main_assert_eq!((player.score_old, player.score_new) => (-1, None), "HideSettlementScoreInEvaluation suppresses the score line");
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
        |_| None,
        |_player_info_id| None,
        |_player_info_id| None,
        |_icon_spec, _color| None,
        |_player_info_id| None,
        false,
    );

    main_assert_eq!(dialog.evaluation().custom_evaluation_strings() => "Global summary|Second line");
    main_assert_eq!(dialog.evaluation().separate_team_ids() => Some([1, 2]));
    let players = dialog.evaluation().players().collect::<Vec<_>>();
    main_assert_eq!(
        players
            .iter()
            .map(|player| (player.player_info_id, player.team_id, player.won))
            .collect::<Vec<_>>() =>
        vec![
            (200, Some(2), false),
            (100, Some(1), true),
            (101, Some(1), true),
        ],
        "fixed-team context retains source order and applies team-level victory"
    );
    main_assert_eq!(players[2].custom_evaluation_strings => "Personal note");
    let fonts = new_classic_running_sandbox_app()
        .assets
        .clonk_fonts
        .clone()
        .test_value();
    let split_layout = dialog.classic_evaluation_layout(1024, 600, &fonts);
    main_assert_eq!(split_layout.player_lists.len() => 2);
    main_assert_eq!(split_layout.players.iter().map(|player| (player.player_list_index, player.player_index)).collect::<Vec<_>>() => vec![(0, 1), (0, 2), (1, 0)]);

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
        |_| None,
        |_player_info_id| None,
        |_player_info_id| None,
        |_icon_spec, _color| None,
        |_player_info_id| None,
        false,
    );
    main_assert_eq!(generated.evaluation().separate_team_ids() => None);
    main_assert_eq!(generated.classic_evaluation_layout(1024, 600, &fonts).player_lists.len() => 1);
}
