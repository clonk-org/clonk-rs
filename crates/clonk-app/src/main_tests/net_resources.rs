// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! netresources_fixture {
    (resource_resource_type_id_loadable_filename: $resource_type:expr, $id:expr, $loadable:expr, $filename:expr, $base:expr $(,)?) => {
        clonk_engine::NetworkResourceCore {
            resource_type: $resource_type,
            id: $id,
            loadable: $loadable,
            filename: $filename,
            ..$base
        }
    };
    (join_envelope: $client_id:expr, $status:expr, $dynamic:expr, $parameters:expr $(,)?) => {
        clonk_network::JoinDataEnvelope {
            client_id: $client_id,
            start_control_tick: 23,
            status: $status,
            dynamic: $dynamic,
            parameters: $parameters,
        }
    };
    (client: $client_id:expr, $activated:expr $(,)?) => {
        clonk_engine::ClientCoreControlData {
            client_id: $client_id,
            activated: $activated,
            ..Default::default()
        }
    };
    (join_player_filename_at_client_info_id_source: $filename:expr, $at_client:expr, $info_id:expr, $source:expr $(,)?) => {
        clonk_engine::JoinPlayerControlData {
            filename: $filename,
            at_client: $at_client,
            info_id: $info_id,
            source: $source,
            by_client: 0,
        }
    };
    (resource_id: $id:expr $(,)?) => {
        clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: $id,
            loadable: true,
            ..Default::default()
        }
    };
    (ready_tick: $tick:expr, $controls:expr $(,)?) => {
        NetworkEvent::ReadyTick {
            tick: $tick,
            controls: $controls,
        }
    };
    (join_player_info_id_source: $info_id:expr, $source:expr $(,)?) => {
        clonk_engine::JoinPlayerControlData {
            at_client: 0,
            info_id: $info_id,
            source: $source,
            by_client: 1,
            ..Default::default()
        }
    };
}

#[test]
fn completion_matches_win32_and_gtk_function_layout() {
    let mut engine = Engine::new();
    main_assert_eq!(engine.install_global_scripts(&[("CompletionGlobals.c".to_string(), "global func EngineProbe() { return true; }".to_string(),)]) => 1,);
    engine
        .install_scenario_script(
            "Scenario",
            "func ScenarioAlpha() { return true; }\n\
                     protected func ScenarioHidden() { return true; }\n\
                     global func ScenarioGlobal() { return true; }",
        )
        .test_value();

    let catalog = engine.console_script_completion_catalog();
    main_assert!(catalog.engine_functions.iter().any(|name| name == "Abs"));
    main_assert!(catalog.engine_functions.iter().any(|name| name == "EngineProbe"));
    main_assert!(catalog.engine_functions.iter().any(|name| name == "ScenarioGlobal"));
    main_assert!(!catalog.engine_functions.iter().any(|name| name == "SetContactDensity"));
    for hidden in [
        "ScoreboardCol",
        "CastInt",
        "CastBool",
        "CastC4ID",
        "CastAny",
    ] {
        main_assert!(!catalog.engine_functions.iter().any(|name| name == hidden));
    }
    main_assert_eq!(catalog.scenario_functions => ["ScenarioHidden".to_string(), "ScenarioAlpha".to_string()]);

    let win32 =
        developer_console_completion_entries(&catalog, DeveloperConsoleCompletionStyle::Win32);
    let separator = win32
        .iter()
        .position(|entry| *entry == DeveloperConsoleCompletionEntry::Separator)
        .test_value();
    main_assert_eq!(separator => catalog.scenario_functions.len());
    main_assert_eq!(
        &win32[..separator] =>
        &[
            DeveloperConsoleCompletionEntry::Function("ScenarioAlpha()".to_string()),
            DeveloperConsoleCompletionEntry::Function("ScenarioHidden()".to_string()),
        ]
    );
    main_assert!(win32[separator + 1..].iter().any(|entry| {entry == &DeveloperConsoleCompletionEntry::Function("EngineProbe()".to_string())}));

    let gtk = developer_console_completion_entries(&catalog, DeveloperConsoleCompletionStyle::Gtk);
    main_assert!(!gtk.contains(&DeveloperConsoleCompletionEntry::Separator));
    main_assert_eq!(
        &gtk[gtk.len() - catalog.scenario_functions.len()..] =>
        &[
            DeveloperConsoleCompletionEntry::Function("ScenarioHidden".to_string()),
            DeveloperConsoleCompletionEntry::Function("ScenarioAlpha".to_string()),
        ]
    );
    main_assert!(gtk.iter().any(|entry| {entry == &DeveloperConsoleCompletionEntry::Function("EngineProbe".to_string())}));
}

#[test]
fn nonhost_console_packet_uses_console_active_policy() {
    let packet = || {
        NetworkControl::Script(clonk_engine::ScriptControlData {
            target_object: clonk_engine::SCRIPT_SCOPE_CONSOLE,
            strictness: clonk_engine::ScriptStrictness::Strict3,
            script: LegacyCString::from_bytes(b"SetGravity(77)".to_vec()).test_value(),
            by_client: 7,
        })
    };

    let mut inactive = new_state_only_running_sandbox_app();
    let initial_gravity = inactive.engine.physics().gravity;
    inactive
        .apply_ready_controls(0, vec![packet()])
        .test_value();
    main_assert_eq!(inactive.engine.physics().gravity => initial_gravity);

    let mut active = new_state_only_running_sandbox_app();
    active.console_mode = true;
    active.apply_ready_controls(0, vec![packet()]).test_value();
    main_assert_eq!(active.engine.physics().gravity => 77);
}

#[test]
fn blocking_resource_stall_timeout_resets_only_when_percent_changes() {
    let started = Instant::now();
    let mut stuck = BlockingResourceWait::new_at(
        BlockingResourceScope::ClientStart,
        7,
        None,
        "Scenario".to_string(),
        25,
        started,
    );
    main_assert!(!stuck.observe_at(25, started + BLOCKING_RESOURCE_STALL_TIMEOUT));
    main_assert!(stuck.observe_at(25, started + BLOCKING_RESOURCE_STALL_TIMEOUT + Duration::from_millis(1)));

    let mut advancing = BlockingResourceWait::new_at(
        BlockingResourceScope::ClientStart,
        7,
        None,
        "Scenario".to_string(),
        25,
        started,
    );
    let changed_at = started + BLOCKING_RESOURCE_STALL_TIMEOUT - Duration::from_millis(1);
    main_assert!(!advancing.observe_at(26, changed_at));
    main_assert!(!advancing.observe_at(26, changed_at + BLOCKING_RESOURCE_STALL_TIMEOUT));
    main_assert!(advancing.observe_at(26, changed_at + BLOCKING_RESOURCE_STALL_TIMEOUT + Duration::from_millis(1)));
}

#[test]
fn client_resource_timeout_closes_progress_and_shows_fatal_error_log() {
    let mut app = new_menu_app(800, 600);
    let core = clonk_engine::NetworkResourceCore {
        id: 7,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).test_value(),
        ..Default::default()
    };
    app.admission_resources.register_lobby_resource(&core);
    let started = Instant::now();
    app.begin_blocking_resource_wait_at(
        BlockingResourceScope::ClientStart,
        core.id,
        None,
        "Scenario".to_string(),
        started,
    )
    .test_value();

    app.poll_blocking_resource_wait_at(started + BLOCKING_RESOURCE_STALL_TIMEOUT)
        .test_value();
    main_assert!(app.blocking_resource_wait.is_some());
    app.poll_blocking_resource_wait_at(
        started + BLOCKING_RESOURCE_STALL_TIMEOUT + Duration::from_millis(1),
    )
    .test_value();

    main_assert!(app.blocking_resource_wait.is_none());
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert_eq!(app.dialogs.messages[0].state.caption() => "Error Log");
    main_assert_eq!(app.dialogs.messages[0].state.message() => "Waiting for Scenario: Timeout!");
    main_assert_eq!(app.dialogs.messages[0].state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::ERROR);
}

#[test]
fn runtime_join_data_tracks_slow_resource_then_cancel_aborts_without_status_packet() {
    // HandleJoinData installs the reference-form GS_Go before C4Game checks
    // isLobbyActive and proceeds directly into InitGame. UpdateChaseTarget
    // may send a newer PID_Status five seconds later, but loading does not
    // wait for that timer (7d43b47b src/C4Game.cpp:400-417;
    // src/C4Network2.cpp:1574-1592,1820-1850,2161-2183).
    let mut app = new_menu_app(800, 600);
    let (manager, event_tx, _commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));

    let resource = |resource_type: clonk_network::HostResourceType, id, name: &[u8]| {
        netresources_fixture!(
            resource_resource_type_id_loadable_filename:
                resource_type as u8,
                id,
                true,
                clonk_engine::LegacyCString::from_bytes(name.to_vec()).test_value(),
                Default::default(),
        )
    };
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.scenario = resource(
        clonk_network::HostResourceType::Scenario,
        70,
        b"Scenario.c4s",
    );
    snapshot.dynamic = resource(clonk_network::HostResourceType::Dynamic, 71, b"Dynamic.c4s");
    snapshot.parameters.game_resources.clear();
    let reference_status = clonk_network::NetworkStatus::new(
        clonk_network::NETWORK_STATE_GO,
        host_config.initial_status.control_mode,
        -1,
    );
    event_tx
        .send(NetworkEvent::JoinData(netresources_fixture!(join_envelope: 7, reference_status, snapshot.dynamic, snapshot.parameters)))
        .test_value();
    app.test_network_events();

    main_assert_eq!(app.mode => AppMode::Loading);
    main_assert!(app.network_lobby.is_none(), "a running host never enters DoLobby");
    main_assert_eq!(app.pending_client_start_status => Some(reference_status));

    let progress = app
        .dialogs.messages
        .iter()
        .find(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::BlockingResourceWait {
                    scope: BlockingResourceScope::ClientStart,
                    resource_id: 70,
                }
            )
        })
        .test_value();
    main_assert_eq!(progress.state.message() => "Waiting for Scenario...");
    main_assert_eq!(progress.state.progress() => Some(0));
    main_assert_eq!(progress.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::CANCEL);
    main_assert_eq!(progress.state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Standard(3));
    main_assert_eq!(progress.state.focused_button() => None);
    main_assert_eq!(progress.state.button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel) => "Cancel");

    for present_percent in [17, 63] {
        event_tx
            .send(NetworkEvent::ResourceProgress {
                resource_id: 70,
                present_percent,
            })
            .test_value();
        app.test_update();
        main_assert_eq!(
            app.dialogs.messages
                .iter()
                .find(|dialog| matches!(
                    dialog.continuation,
                    MessageDialogContinuation::BlockingResourceWait { .. }
                ))
                .and_then(|dialog| dialog.state.progress()) =>
            Some(present_percent)
        );
    }

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();

    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.pending_network_join_data.is_none());
    main_assert!(app.pending_client_start_status.is_none());
    main_assert!(app.blocking_resource_wait.is_none());
    main_assert!(app.admission_resources.resources.is_empty());
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert!(app.dialogs.messages.iter().all(|dialog| !matches!(dialog.continuation, MessageDialogContinuation::BlockingResourceWait { .. })));
    let [failure] = app.dialogs.messages.as_slice() else {
        panic!("Cancel should report one startup-network failure");
    };
    main_assert_eq!(failure.state.caption() => "Error Log");
    main_assert_eq!(failure.state.message() => "Waiting for Scenario was aborted.");
}

#[test]
fn ordinary_client_go_tracks_slow_resource_then_cancel_aborts() {
    // A lobby join starts RetrieveScenario only after the host's ordinary Go
    // status closes DoLobby; cancelling that wait aborts the join and restores
    // startup (7d43b47b src/C4Game.cpp:400-417;
    // src/C4Network2.cpp:475-515,619-671,2017-2057).
    let mut app = new_menu_app(800, 600);
    let (manager, event_tx, _commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));

    let resource = |resource_type: clonk_network::HostResourceType, id, name: &[u8]| {
        netresources_fixture!(
            resource_resource_type_id_loadable_filename:
                resource_type as u8,
                id,
                true,
                clonk_engine::LegacyCString::from_bytes(name.to_vec()).test_value(),
                Default::default(),
        )
    };
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.scenario = resource(
        clonk_network::HostResourceType::Scenario,
        70,
        b"Scenario.c4s",
    );
    snapshot.dynamic = resource(clonk_network::HostResourceType::Dynamic, 71, b"Dynamic.c4s");
    snapshot.parameters.game_resources.clear();
    let mut reference_status = host_config.initial_status;
    reference_status.target_tick = -1;
    let go = clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_GO, 2, 23);
    event_tx
        .send(NetworkEvent::JoinData(netresources_fixture!(join_envelope: 7, reference_status, snapshot.dynamic, snapshot.parameters)))
        .test_value();
    event_tx
        .send(NetworkEvent::StatusRequested(go))
        .test_value();
    app.test_network_events();

    let progress = app
        .dialogs.messages
        .iter()
        .find(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::BlockingResourceWait {
                    scope: BlockingResourceScope::ClientStart,
                    resource_id: 70,
                }
            )
        })
        .test_value();
    main_assert_eq!(progress.state.message() => "Waiting for Scenario...");
    main_assert_eq!(progress.state.progress() => Some(0));
    main_assert_eq!(progress.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::CANCEL);

    for present_percent in [17, 63] {
        event_tx
            .send(NetworkEvent::ResourceProgress {
                resource_id: 70,
                present_percent,
            })
            .test_value();
        app.test_update();
        main_assert_eq!(
            app.dialogs.messages
                .iter()
                .find(|dialog| matches!(
                    dialog.continuation,
                    MessageDialogContinuation::BlockingResourceWait { .. }
                ))
                .and_then(|dialog| dialog.state.progress()) =>
            Some(present_percent)
        );
    }

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();

    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.pending_network_join_data.is_none());
    main_assert!(app.pending_client_start_status.is_none());
    main_assert!(app.blocking_resource_wait.is_none());
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    let [failure] = app.dialogs.messages.as_slice() else {
        panic!("Cancel should report one startup-network failure");
    };
    main_assert_eq!(failure.state.caption() => "Error Log");
    main_assert_eq!(failure.state.message() => "Waiting for Scenario was aborted.");
}

#[test]
fn ordinary_client_go_completes_nonpreloaded_resource_merge_before_acknowledging() {
    // RetrieveScenario merges the synchronized scenario and dynamic groups
    // only after DoLobby closes, then FinalInit acknowledges the ordinary Go
    // barrier (7d43b47b src/C4Network2.cpp:475-515,558-671,2017-2057;
    // src/C4Game.cpp:455-483).
    let directory = tempdir();
    let scenario_path = directory.path().join("Scenario.c4s");
    let dynamic_path = directory.path().join("Dynamic.c4s");
    let combined_path = directory.path().join("Combined7.c4s");
    let mut scenario_group = MutableGroup::new("Scenario.c4s");
    scenario_group
        .add_file(
            "Scenario.txt",
            b"[Head]\nTitle=Ordinary client start\nNetworkGame=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n"
                .to_vec(),
        )
        .test_value();
    scenario_group
        .add_child("Defs.c4d", packed_network_definition("Defs.c4d", "CLNK"))
        .test_value();
    fs::write(&scenario_path, scenario_group.pack().test_value()).test_value();
    let mut dynamic_group = MutableGroup::new("Dynamic.c4s");
    dynamic_group
        .add_file("Dynamic.txt", b"ordinary".to_vec())
        .test_value();
    fs::write(&dynamic_path, dynamic_group.pack().test_value()).test_value();

    let resource = |resource_type: clonk_network::HostResourceType, id, name: &[u8]| {
        netresources_fixture!(
            resource_resource_type_id_loadable_filename:
                resource_type as u8,
                id,
                true,
                clonk_engine::LegacyCString::from_bytes(name.to_vec()).test_value(),
                Default::default(),
        )
    };
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.scenario = resource(
        clonk_network::HostResourceType::Scenario,
        70,
        b"Scenario.c4s",
    );
    snapshot.dynamic = resource(clonk_network::HostResourceType::Dynamic, 71, b"Dynamic.c4s");
    snapshot.parameters.game_resources.clear();
    snapshot
        .parameters
        .clients
        .clients
        .push(clonk_engine::ClientCoreControlData {
            client_id: 7,
            activated: false,
            observer: true,
            name: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec()).test_value(),
            nick: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec()).test_value(),
            lobby_ready: false,
        });
    snapshot.parameters.clients.local_client_id = Some(7);
    let mut reference_status = host_config.initial_status;
    reference_status.target_tick = -1;
    let go = clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_GO, 2, 23);
    let join_data = netresources_fixture!(join_envelope:
        7,
        reference_status,
        snapshot.dynamic,
        snapshot.parameters
    );

    let mut app = new_menu_app(800, 600);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let mut settings = client_network_settings();
    settings.resource_directory = directory.path().to_path_buf();
    settings.group_maker =
        clonk_engine::LegacyCString::from_bytes(b"M\x81ker".to_vec()).test_value();
    app.network_mode = Some(NetworkMode::Client(settings));
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    event_tx
        .send(NetworkEvent::JoinData(join_data.clone()))
        .test_value();
    app.test_network_events();
    let lobby_reached = reference_status.with_target_tick(23);
    main_assert_eq!(
        commands.take_framed_status_acknowledgements() => vec![(lobby_reached, 0)]
    );
    event_tx
        .send(NetworkEvent::StatusCommitted(lobby_reached))
        .test_value();
    app.test_network_events();

    event_tx
        .send(NetworkEvent::StatusRequested(go))
        .test_value();
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 23,
            controls: Vec::new(),
        })
        .test_value();
    app.test_network_events();
    main_assert_eq!(
        app.blocking_resource_wait.as_ref().map(|wait| wait.resource_id) => Some(70)
    );

    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id: 70,
            core: join_data.parameters.scenario.clone(),
            path: scenario_path,
            local: false,
        })
        .test_value();
    app.test_network_events();
    main_assert_eq!(
        app.blocking_resource_wait.as_ref().map(|wait| wait.resource_id) => Some(71)
    );

    let (removed_tx, removed_rx) = mpsc::channel();
    let removal_observer = thread::spawn(move || {
        let (resource_id, completion) = commands.receive_resource_removal();
        completion.send(Ok(())).test_value();
        removed_tx.send(resource_id).test_value();
        commands
    });
    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id: 71,
            core: join_data.dynamic,
            path: dynamic_path,
            local: false,
        })
        .test_value();
    app.test_network_events();
    main_assert_eq!(
        removed_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("client did not retire the ordinary merged dynamic") => 71
    );
    let mut commands = removal_observer.test_join();
    main_assert!(combined_path.exists());
    main_assert_eq!(Group::open(&combined_path).test_value().maker_bytes() => Some(&b"M\x81ker"[..]));

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll_loading().test_value();
        let waiting_for_start = app.dialogs.messages.iter().any(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::NetworkClientStartWait
            )
        });
        if app.mode == AppMode::Running || waiting_for_start {
            break;
        }
        main_assert!(Instant::now() < deadline, "ordinary client loader stalled");
        thread::yield_now();
    }
    main_assert_eq!(app.mode => AppMode::Loading);
    main_assert!(!app.network_control_running);
    main_assert!(app.network_ticks.ready.contains_key(&23));
    main_assert_eq!(commands.take_framed_status_acknowledgements() => vec![(go, 0)]);
    event_tx
        .send(NetworkEvent::StatusCommitted(go))
        .test_value();
    app.test_network_events();
    main_assert_eq!(app.mode => AppMode::Running);
    main_assert!(app.network_control_running);
    app.update().test_value();
    main_assert!(!app.network_ticks.ready.contains_key(&23));
    main_assert_eq!(app.expected_network_control_tick() => 24);
}

#[test]
fn player_resource_abort_releases_only_the_waiting_join() {
    let mut app = new_synthetic_running_sandbox_app();
    let (manager, _event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.engine.set_network_game(true);
    app.control_clients.register(0, true, false);
    let core = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            clonk_network::HostResourceType::Player as u8,
            9,
            true,
            clonk_engine::LegacyCString::from_bytes(b"Player.c4p".to_vec()).test_value(),
            Default::default(),
    );
    app.admission_resources.register_lobby_resource(&core);
    app.begin_blocking_resource_wait_at(
        BlockingResourceScope::PlayerJoin,
        core.id,
        Some(99),
        "player file for Ada".to_string(),
        Instant::now(),
    )
    .test_value();

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();

    main_assert!(app.blocking_resource_wait.is_none());
    main_assert_eq!(app.admission_resources.status(core.id) => Some(&AdmissionResourceState::Loading { removed: false }));
    main_assert!(app.aborted_player_resource_joins.contains(&(core.id, 99)));
    let join = |info_id| {
        vec![NetworkControl::JoinPlayer(
            clonk_engine::JoinPlayerControlData {
                at_client: 0,
                info_id,
                source: clonk_engine::JoinPlayerSource::Resource(core.clone()),
                ..Default::default()
            },
        )]
    };
    let mut clients = ControlClientRegistry::default();
    clients.register(0, true, false);
    main_assert!(pending_admission_resource(&mut app.admission_resources, &clients, &join(99), &app.aborted_player_resource_joins,).is_none());
    main_assert_eq!(
        pending_admission_resource(
            &mut app.admission_resources,
            &clients,
            &join(100),
            &app.aborted_player_resource_joins,
        )
        .map(|pending| pending.info_id) =>
        Some(100),
        "a later caller still waits on the active backend transfer"
    );
    main_assert!(app.dialogs.messages.is_empty());

    let player_path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    app.admission_resources.mark_complete(core.id, player_path);
    app.apply_ready_controls(
        0,
        vec![
            NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData::new(
                0,
                0,
                vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Ada".to_vec()).unwrap(),
                    id: 99,
                    ..Default::default()
                }],
                1,
            )),
            join(99).pop().expect("resource join"),
        ],
    )
    .test_value();
    main_assert!(!app.control_player_infos.get(99).expect("player info was still applied").is_joined());
    main_assert!(!app.engine.snapshot().players.iter().any(|player| player.player_info_id == 99));
    main_assert!(!app.aborted_player_resource_joins.contains(&(core.id, 99)));
}

#[test]
fn failed_client_start_resource_aborts_instead_of_stalling_silently() {
    let mut app = new_menu_app(800, 600);
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    let core = clonk_engine::NetworkResourceCore {
        id: 11,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).test_value(),
        ..Default::default()
    };
    app.admission_resources.register_lobby_resource(&core);
    app.begin_blocking_resource_wait_at(
        BlockingResourceScope::ClientStart,
        core.id,
        None,
        "Scenario".to_string(),
        Instant::now(),
    )
    .test_value();
    event_tx
        .send(NetworkEvent::ResourceLoadFailed {
            resource_id: core.id,
        })
        .test_value();

    app.test_network_events();

    main_assert!(app.network.is_none());
    main_assert!(app.blocking_resource_wait.is_none());
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert_eq!(app.dialogs.messages[0].state.caption() => "Error Log");
    main_assert_eq!(app.dialogs.messages[0].state.message() => "Unable to retrieve Scenario.");
}

#[test]
fn fresh_install_shutdown_persists_fullscreen_default() {
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet")).test_value();
    fs::write(install.path().join("planet/System.c4g"), b"stub").test_value();
    let config_file = user_data.path().join("custom/fresh.config");
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("LC_CONFIG_FILE", None),
    ]);
    let paths = AppPaths::discover_with_config_file(Some(&config_file)).test_value();
    paths.ensure_user_dirs().test_value();
    main_assert!(!config_file.exists());

    let mut display = DisplayOptions::load(Some(&paths));
    main_assert_eq!(display.mode => DisplayMode::Fullscreen);
    display.persist_if_dirty(&paths);

    let persisted = Config::load(&config_file).test_value();
    main_assert_eq!(persisted.get_in(Some("Graphics"), "DisplayMode") => Some("0"));

    fs::write(&config_file, "[General]\nSentinel=keep\n").test_value();
    let mut missing_key = DisplayOptions::load(Some(&paths));
    main_assert_eq!(missing_key.mode => DisplayMode::Fullscreen);
    missing_key.persist_if_dirty(&paths);
    let persisted = Config::load(&config_file).test_value();
    main_assert_eq!(persisted.get_in(Some("General"), "Sentinel") => Some("keep"));
    main_assert_eq!(persisted.get_in(Some("Graphics"), "DisplayMode") => Some("0"));
}

#[test]
fn fully_disabled_test_audio_skips_install_resource_discovery() {
    let audio = AudioContext::try_new(disabled_audio_options()).test_value();

    main_assert!(audio.resolver.global.is_empty());
    main_assert!(audio.resolver.base_sample_loads.is_empty());
    main_assert!(audio.music_resolver.global.assets.is_empty());
    main_assert!(audio.music_resolver.extra.is_none());
}

#[test]
fn install_walker_registers_defcoreless_c4d_sound_groups() {
    let dir = tempdir();
    let root = dir.path().join("Objects.c4d");
    let pure_sounds = root.join("Potions.c4d");
    fs::create_dir_all(&pure_sounds).test_value();
    fs::write(pure_sounds.join("Drink.wav"), silent_pcm_wav(20)).test_value();

    let group = Group::open(&root).test_value();
    let mut engine = Engine::new();
    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.resolver = SoundResolver::empty();
    audio.refresh_sound_catalog();
    load_definitions_from_group(
        &mut engine,
        &group,
        Some(NonNull::from(&mut audio)),
        &mut HashSet::new(),
        &mut None,
    )
    .test_value();

    main_assert_eq!(audio.available_sound_samples() => ["drink.wav"]);
    main_assert!(audio.ensure_sound_with_key("Drink").expect("decode install pure-container sample").is_some());
}

#[test]
fn message_dialog_buttons_use_active_language_resources() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "LanguageEx", "DE").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);

    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::new(
            "Nachricht",
            "Titel",
            clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL,
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            clonk_frontend::message_dialog::MessageDialogSize::Regular,
            false,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();

    let fonts = app.assets.clonk_fonts.clone().test_value();
    let dialog = &mut app.dialogs.messages[0].state;
    main_assert_eq!(dialog.button_label(clonk_frontend::message_dialog::MessageDialogButton::Ok) => "&OK");
    main_assert_eq!(dialog.button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel) => "&Abbrechen");
    main_assert_eq!(dialog.handle_hotkey('A') => Some(clonk_frontend::message_dialog::MessageDialogResult::Cancel));
    main_assert_eq!(dialog.handle_hotkey('C') => None);
    let layout = dialog.layout(640, 480, &fonts.text);
    let close = layout.close_button.test_value();
    let close_point = GuiPoint::new((close.x + 1) as f32, (close.y + 1) as f32);
    dialog.handle_pointer_move(close_point, &layout);
    main_assert_eq!(dialog.tooltip_state(Some(close_point), &layout).expect("localized close tooltip").text => "Schließen");
    reset_cached_app_paths();
}

#[test]
fn plrclr_submits_full_owner_packet_and_authoritative_rows_recolor() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let fred = clonk_engine::ControlPlayerInfoEntry {
        id: 4,
        name: clonk_engine::LegacyCString::from_bytes(b"Fred".to_vec()).test_value(),
        color: 0x0000_ff00,
        original_color: 0x0000_ff00,
        ..Default::default()
    };
    app.control_clients
        .replace_snapshot([message_client(0, b"Exact Host")]);
    app.control_player_infos.replace_snapshot(
        4,
        [clonk_engine::PlayerInfoControlData::new(
            0,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![fred.clone()],
            0,
        )],
    );
    app.sync_classic_lobby_roster();

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
        "/plrclr Fred FF0000".to_string(),
    ))
    .test_value();

    let updates = commands.take_player_info_updates();
    main_assert_eq!(updates.len() => 1);
    let mut expected = fred.clone();
    expected.original_color = 0x00ff_0000;
    main_assert_eq!(
        updates[0] =>
        clonk_network::PlayerInfoUpdateRequest::new(
            0,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![expected.clone()]
        ),
        "the complete owner packet is cloned and only OriginalColor changes"
    );

    expected.color = expected.original_color;
    let authoritative = clonk_engine::PlayerInfoControlData::new(
        0,
        clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
        vec![expected],
        0,
    );
    app.control_player_infos
        .replace_snapshot(4, [authoritative.clone()]);
    app.sync_classic_lobby_roster();
    let expected_color = [0xff, 0x17, 0x17, 0xff];
    main_assert!(app
                .classic_host_lobby
                .as_ref()
                .unwrap()
                .controller
                .rows()
                .iter()
                .any(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 4 && player.color == expected_color)));

    let mut client = new_menu_app(640, 480);
    client.startup.view = StartupView::NetworkLobby;
    client.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    client.control_clients.replace_snapshot([
        message_client(0, b"Exact Host"),
        message_client(7, b"Client"),
    ]);
    client
        .control_player_infos
        .replace_snapshot(4, [authoritative]);
    client.sync_classic_lobby_roster();
    main_assert!(client
                .network_lobby
                .as_ref()
                .unwrap()
                .roster_rows
                .iter()
                .any(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 4 && player.color == expected_color)));
}

#[test]
fn generic_client_resource_save_hit_target_emits_the_resource_id() {
    let root = tempdir();
    let work = root.path().join("Network");
    fs::create_dir(&work).test_value();
    let source = work.join("Downloaded.c4s");
    fs::write(&source, b"payload").test_value();

    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    let mut settings = ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client");
    settings.resource_directory = work.clone();
    app.network_mode = Some(NetworkMode::Client(settings));
    let (network, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(network);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let core = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            clonk_network::HostResourceType::Scenario as u8,
            23,
            true,
            LegacyCString::from_bytes(b"Remote/Downloaded.c4s".to_vec()).test_value(),
            Default::default(),
    );
    app.admission_resources.register_lobby_resource(&core);
    app.admission_resources
        .mark_complete_with_locality(core.id, source.clone(), false);
    app.register_classic_lobby_resource(&core, 100);
    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Resources))
        .test_value();
    main_assert!(app.network_lobby.as_ref().unwrap().resource_rows[&core.id].save_possible);

    {
        let lobby = app.network_lobby.as_mut().test_value();
        let rect = lobby
            .update_layout(640.0, 480.0)
            .resource_save_buttons
            .first()
            .test_value()
            .1;
        lobby.handle_panel_pointer_move(GuiPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        ));
    }
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(
        fs::read(root.path().join("Downloaded.c4s")).expect("saved copy") =>
        b"payload",
        "the routed SaveResourceRequested reaches request_lobby_resource_save"
    );
    main_assert_eq!(app.dialogs.messages.last().expect("save feedback dialog").state.caption() => "Resource saved");
}

#[test]
fn takeover_selection_submits_full_local_packet_with_savegame_association() {
    let mut app = new_menu_app(640, 480);
    install_test_free_savegame_player_row(&mut app, 50);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let chosen = clonk_engine::ControlPlayerInfoEntry {
        id: 31,
        name: LegacyCString::from_bytes(b"Chooser".to_vec()).test_value(),
        color: 0x0012_3456,
        original_color: 0x0065_4321,
        team: 3,
        extra_data: [1, 2, 3, 4],
        ..Default::default()
    };
    let sibling = clonk_engine::ControlPlayerInfoEntry {
        id: 32,
        name: LegacyCString::from_bytes(b"Sibling".to_vec()).test_value(),
        flags: clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED,
        color: 0x0000_00aa,
        ..Default::default()
    };
    let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
    app.control_player_infos.replace_snapshot(
        99,
        [clonk_engine::PlayerInfoControlData::new(
            7,
            packet_flags,
            vec![chosen.clone(), sibling.clone()],
            7,
        )],
    );

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
        row: LobbyRosterId::Player(50),
        position: GuiPoint::new(200.0, 150.0),
    }])
    .test_value();
    let root = app.context_menu.as_ref().test_value().layout().panels[0].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((root.x + 1) as f32, (root.y + 1) as f32))
        .test_value();

    let mut live_sibling = sibling.clone();
    live_sibling.color = 0x0000_00bb;
    app.control_player_infos.replace_snapshot(
        99,
        [clonk_engine::PlayerInfoControlData::new(
            7,
            packet_flags,
            vec![chosen.clone(), live_sibling.clone()],
            7,
        )],
    );
    let child = app.context_menu.as_ref().test_value().layout().panels[1].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((child.x + 1) as f32, (child.y + 1) as f32))
        .test_value();
    main_assert!(app.handle_context_menu_pointer_button(ElementState::Pressed, ContextMenuPointerButton::Left,).expect("activate takeover child"));
    main_assert!(app.context_menu.is_none());

    let mut expected_chosen = chosen.clone();
    expected_chosen.savegame_player = 50;
    main_assert_eq!(commands.take_player_info_updates() => vec![clonk_network::PlayerInfoUpdateRequest::new(7, packet_flags, vec![expected_chosen, live_sibling])]);
    main_assert_eq!(
        app.control_player_infos
            .client_update_request(7)
            .unwrap()
            .players[0]
            .savegame_player =>
        0,
        "takeover waits for the authoritative PlayerInfo echo"
    );
    main_assert!(app.handle_context_menu_pointer_button(ElementState::Released, ContextMenuPointerButton::Left,).expect("consume takeover activation release"));
}

#[test]
fn new_color_resets_only_current_color_in_full_packet() {
    let mut app = new_menu_app(640, 480);
    let (mut chooser, companion) = install_test_classic_host_team_lobby(&mut app);
    chooser.color = 0x00ab_cdef;
    app.control_player_infos.replace_snapshot(
        9,
        [clonk_engine::PlayerInfoControlData::new(
            0,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![chooser.clone(), companion.clone()],
            0,
        )],
    );
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
        row: LobbyRosterId::Player(chooser.id),
        position: GuiPoint::new(200.0, 150.0),
    }])
    .test_value();
    main_assert!(app.handle_context_menu_key(VirtualKeyCode::KeyC, ElementState::Pressed).expect("activate New Color hotkey"));

    let mut reset = chooser.clone();
    reset.color = reset.original_color;
    main_assert_eq!(
        commands.take_player_info_updates() =>
        vec![clonk_network::PlayerInfoUpdateRequest::new(
            0,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![reset, companion]
        )]
    );
    main_assert_eq!(app.control_player_infos.client_update_request(0).unwrap().players[0].color => chooser.color, "the roster waits for the authoritative echo");
}

#[test]
fn invisible_random_teams_sheet_uses_one_lazy_header_and_client_packet_order() {
    let mut clients = ControlClientRegistry::default();
    clients.replace_snapshot([
        netresources_fixture!(client: 0, true),
        netresources_fixture!(client: 7, false),
        netresources_fixture!(client: 8, true),
    ]);
    let player = |id, flags, player_type| clonk_engine::ControlPlayerInfoEntry {
        id,
        flags,
        player_type,
        name: LegacyCString::from_bytes(format!("Player {id}").into_bytes()).test_value(),
        ..Default::default()
    };
    let mut infos = ControlPlayerInfoRegistry::default();
    infos.replace_snapshot(
        1,
        [
            clonk_engine::PlayerInfoControlData::new(
                0,
                0,
                vec![
                    player(10, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                    player(
                        11,
                        clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                        clonk_engine::PLAYER_INFO_TYPE_USER,
                    ),
                    player(
                        12,
                        clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                        clonk_engine::PLAYER_INFO_TYPE_USER,
                    ),
                ],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                7,
                0,
                vec![player(20, 0, clonk_engine::PLAYER_INFO_TYPE_USER)],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                8,
                0,
                vec![
                    player(30, 0, clonk_engine::PLAYER_INFO_TYPE_SCRIPT),
                    player(31, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                ],
                -1,
            ),
        ],
    );
    let metadata = clonk_engine::InitialNetworkTeamMetadata {
        active: true,
        custom: true,
        allow_hostility_change: false,
        allow_team_switch: false,
        auto_generate_teams: false,
        last_team_id: 0,
        team_distribution: clonk_engine::InitialNetworkTeamDistribution::RandomInvisible,
        team_colors: false,
        max_script_players: 0,
        script_player_names: LegacyCString::default(),
        random_team_count: 0,
        teams: Vec::new(),
    };
    let rows =
        classic_lobby_roster_projection(&clients, &infos, Some(&metadata), 0, LobbySheet::Teams).0;
    main_assert_eq!(
        rows.iter().map(LobbyRosterRow::id).collect::<Vec<_>>() =>
        vec![
            LobbyRosterId::Header(LobbyRosterHeader::RandomTeam),
            LobbyRosterId::Player(10),
            LobbyRosterId::Player(12),
            LobbyRosterId::Player(30),
            LobbyRosterId::Player(31),
        ]
    );
    let [LobbyRosterRow::Header(header), ..] = rows.as_slice() else {
        panic!("random-team projection must start with one header");
    };
    main_assert_eq!(header.label => "Random team");
    main_assert_eq!(header.icon => LobbyRosterIcon::Standard(19));

    infos.replace_snapshot(
        2,
        [
            clonk_engine::PlayerInfoControlData::new(
                0,
                0,
                vec![player(
                    40,
                    clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                    clonk_engine::PLAYER_INFO_TYPE_USER,
                )],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                7,
                0,
                vec![player(41, 0, clonk_engine::PLAYER_INFO_TYPE_USER)],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                8,
                0,
                vec![player(
                    42,
                    clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                    clonk_engine::PLAYER_INFO_TYPE_USER,
                )],
                -1,
            ),
        ],
    );
    main_assert!(
        classic_lobby_roster_projection(&clients, &infos, Some(&metadata), 0, LobbySheet::Teams,)
            .0
            .is_empty(),
        "the Random team header is created lazily only for a visible active-client player"
    );
}

#[test]
fn scensel_search_context_transfer_and_paste_match_edit_callbacks() {
    let mut edit = SearchEditState::default();
    edit.set_text("alpha beta");
    edit.anchor = 0;
    edit.caret = 5;
    let mut copied = String::new();
    main_assert_eq!(transfer_edit_selection(&mut edit, false, |selection| {copied = selection.to_string(); Ok::<(), ()>(())}) => Ok(true));
    main_assert_eq!(copied => "alpha");
    main_assert_eq!(edit.text() => "alpha beta", "Copy does not mutate text");

    main_assert!(transfer_edit_selection(&mut edit, true, |_| Err("clipboard")).is_err());
    main_assert_eq!(edit.text() => "alpha beta", "failed Cut must retain the selection");
    transfer_edit_selection(&mut edit, true, |_| Ok::<(), ()>(())).test_value();
    main_assert_eq!(edit.text() => " beta");

    edit.set_text("replace me");
    edit.select_all();
    main_assert!(apply_scensel_search_paste(&mut edit, "\r\nleft|right\r\nignored"));
    main_assert_eq!(edit.text() => "left¦right", "leading blank lines are skipped and the first real newline submits/aborts");

    edit.set_text("");
    main_assert!(!apply_scensel_search_paste(&mut edit, &"x".repeat(300)));
    main_assert_eq!(edit.text().len() => SEARCH_EDIT_MAX_BYTES);

    edit.set_text("selection");
    edit.select_all();
    main_assert!(!apply_scensel_search_paste(&mut edit, "\n"));
    main_assert_eq!(edit.selected_text() => Some("selection"), "blank-only paste does not delete the selection");
}

#[test]
fn scensel_rename_pointer_completion_cancels_target_focus_transfer() {
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "Pointer.c4s".to_string();
    scenario.title = "Pointer".to_string();
    let scenarios = vec![scenario];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app(800, 600);
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);
    app.open_scenario_browser();
    app.sync_scenario_game_option_bounds();

    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    let record = app
        .scenario_game_options
        .layout()
        .rect(GameOptionButton::Record)
        .test_value();
    app.test_cursor(PhysicalPosition::new(
        f64::from(record.x + record.w / 2),
        f64::from(record.y + record.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search);
    app.test_left_button(ElementState::Released);
    main_assert!(app.scenario_game_options.values().record);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search, "empty FinishRename focus survives the complete mouse gesture");

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    let fonts = app.assets.clonk_fonts.test_ref();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, fonts);
    let search = GuiPoint::new(
        (layout.search_edit.x + layout.search_edit.w / 2) as f32,
        (layout.search_edit.y + layout.search_edit.h / 2) as f32,
    );
    app.test_touch(TouchPhase::Started, search);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::List);
    app.test_touch(TouchPhase::Ended, search);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::List, "RR_Deleted list focus survives the complete touch gesture");
}

#[test]
fn startup_tooltip_app_uses_the_shared_cmouse_clock_and_runtime_resources() {
    let mut app = new_real_classic_menu_app(640, 480);
    let button = clonk_frontend::main_menu_layout(640, 480).buttons[0];
    let point = GuiPoint::new(
        (button.x + button.w / 2) as f32,
        (button.y + button.h / 2) as f32,
    );
    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));

    let target = app.startup_element_tooltip_target_at(point).test_value();
    main_assert_eq!(target => StartupTooltip::resource("IDS_DLGTIP_STARTGAME"));
    main_assert_eq!(app.resolve_startup_tooltip_text(target) => "Start a local game without network support.");
    main_assert_eq!(app.resolve_startup_tooltip_text(StartupTooltip::resource("IDS_L022_MISSING_RESOURCE")) => "[Undefined: IDS_L022_MISSING_RESOURCE]");

    // Render the exact hovered base first with mouse input suppressed, so
    // no tooltip can become due.
    app.startup_tooltip.note_non_pointer_input();
    let mut base = vec![0; 640 * 480 * 4];
    main_assert!(app.render(&mut base).expect("render suppressed base"));

    // Re-arm the one process-level clock far enough in the past to make
    // the inclusive 500ms boundary eligible, so the final overlay changes
    // pixels against that base.
    let started = Instant::now()
        .checked_sub(clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY + Duration::from_millis(1))
        .test_value();
    app.startup_tooltip = ClassicTooltipTracker::new_at(started);
    app.startup_tooltip.note_pointer_move_at(point, started);
    main_assert!(app.startup_element_tooltip_pending());
    main_assert_eq!(app.startup_tooltip.eligible_pointer() => Some(point));
    let mut tipped = vec![0; 640 * 480 * 4];
    main_assert!(app.render(&mut tipped).expect("render eligible tooltip"));
    main_assert_ne!(tipped => base);

    // A physical key clears active mouse input before any downstream key
    // owner. Same-pixel motion remains suppressed; a genuinely different
    // ceil-quantized pixel starts a fresh delay.
    app.test_key(VirtualKeyCode::KeyZ, ElementState::Pressed);
    main_assert!(!app.startup_element_tooltip_pending());
    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x) - 0.25,
        f64::from(point.y) - 0.25,
    ));
    main_assert!(!app.startup_element_tooltip_pending());
    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x) + 0.25,
        f64::from(point.y),
    ));
    main_assert!(app.startup_element_tooltip_pending());
    main_assert_eq!(app.startup_tooltip.eligible_pointer() => None);

    app.open_options_menu();
    main_assert_eq!(app.startup_tooltip.pointer_position() => None);
    main_assert!(!app.startup_element_tooltip_pending());

    app.test_cursor(PhysicalPosition::new(
        f64::from(point.x),
        f64::from(point.y),
    ));
    main_assert!(app.startup_tooltip.pointer_position().is_some());
    app.resize(800, 600).test_value();
    main_assert_eq!(app.startup_tooltip.pointer_position() => None);
}

#[test]
fn dialog_titles_use_the_process_global_tooltip_delay_and_close_resource() {
    use clonk_frontend::startup_options_advanced::{
        AdvancedConfigController, AdvancedConfigLabels,
    };
    use clonk_frontend::startup_options_dlg::OptionsSheet;

    fn assert_delayed_target(app: &mut GameApp, point: GuiPoint, expected: StartupTooltip) {
        let started = Instant::now();
        app.startup_tooltip = ClassicTooltipTracker::new_at(started);
        app.startup_tooltip.note_pointer_move_at(point, started);
        main_assert!(app
            .startup_tooltip
            .eligible_pointer_at(
                started + clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY
                    - Duration::from_millis(1),
            )
            .and_then(|point| app.classic_dialog_title_tooltip_target_at(point))
            .is_none());
        main_assert_eq!(
            app.startup_tooltip
                .eligible_pointer_at(started + clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY,)
                .and_then(|point| app.classic_dialog_title_tooltip_target_at(point)) =>
            Some(expected)
        );
    }

    let mut app = new_real_classic_menu_app(640, 480);
    app.open_options_menu();
    let mut controller = AdvancedConfigController::new(Vec::new());
    controller.resize(640, 480);
    controller.set_labels(AdvancedConfigLabels {
        caption: "Advanced settings with a live title".into(),
        ..AdvancedConfigLabels::default()
    });
    let layout = controller.layout();
    let title_point = GuiPoint::new(
        (layout.caption.x + 8) as f32,
        (layout.caption.y + layout.caption.h / 2) as f32,
    );
    let _ = controller.handle_pointer_move(title_point);
    app.startup.options_advanced_dialog = Some(PendingOptionsAdvancedDialog {
        controller,
        return_sheet: OptionsSheet::Program,
    });
    assert_delayed_target(
        &mut app,
        title_point,
        StartupTooltip::text("Advanced settings with a live title"),
    );

    let close_point = GuiPoint::new(
        (layout.close_button.x + 1) as f32,
        (layout.close_button.y + 1) as f32,
    );
    let _ = app
        .startup.options_advanced_dialog
        .test_mut()
        .controller
        .handle_pointer_move(close_point);
    assert_delayed_target(
        &mut app,
        close_point,
        StartupTooltip::resource("IDS_MNU_CLOSE"),
    );

    use clonk_frontend::runtime_client_list::{
        RuntimeClientListDialog, RuntimeClientListStatus, RuntimeClientRow, RuntimeClientStatusIcon,
    };
    let row = RuntimeClientRow {
        client_id: 7,
        name: "Remote".into(),
        nick: "Nick".into(),
        host: false,
        local: false,
        activated: true,
        observer: false,
        muted: false,
        has_players: false,
        player_names: Vec::new(),
        addresses: Vec::new(),
        status: RuntimeClientStatusIcon::Ready,
        wait_ms: None,
        connections: Vec::new(),
        can_moderate: false,
        unacknowledged: false,
    };
    let line_height = app
        .assets
        .clonk_fonts
        .as_deref()
        .test_value()
        .text
        .line_height;
    let mut preferred = scoreboard_preferred_rect(
        app.graphics
            .preferred_dialog_rect(app.mouse_control.then_some(app.local_owner)),
    );
    let mut runtime = RuntimeClientListDialog::new(
        "Network clients",
        Vec::new(),
        vec![row.clone()],
        RuntimeClientListStatus::default(),
    );
    let runtime_layout = runtime.layout(preferred, line_height);
    let runtime_title = GuiPoint::new(
        (runtime_layout.caption.expect("an ordinary dialog owns its title widgets").x + 8) as f32,
        (runtime_layout.caption.expect("an ordinary dialog owns its title widgets").y + runtime_layout.caption.expect("an ordinary dialog owns its title widgets").h / 2) as f32,
    );
    main_assert!(runtime.handle_pointer_move(runtime_title, preferred, line_height));
    app.mode = AppMode::Running;
    app.dialogs.client_list = Some(runtime);
    assert_delayed_target(
        &mut app,
        runtime_title,
        StartupTooltip::text("Network clients"),
    );
    let runtime_close = GuiPoint::new(
        (runtime_layout.close_button.expect("an ordinary dialog owns its title widgets").x + 1) as f32,
        (runtime_layout.close_button.expect("an ordinary dialog owns its title widgets").y + 1) as f32,
    );
    main_assert!(app.dialogs.client_list.as_mut().expect("runtime list").handle_pointer_move(runtime_close, preferred, line_height));
    assert_delayed_target(
        &mut app,
        runtime_close,
        StartupTooltip::resource("IDS_MNU_CLOSE"),
    );

    let dragged_point = GuiPoint::new(runtime_title.x + 15.0, runtime_title.y - 4.0);
    main_assert!(app.dialogs.client_list.as_mut().expect("runtime list").handle_pointer_down(runtime_title, preferred, line_height));
    let before_layered_move = app
        .dialogs.client_list
        .test_ref()
        .layout(preferred, line_height)
        .bounds;
    app.chat.external_dialog_visible = true;
    app.test_cursor(PhysicalPosition::new(
        f64::from(dragged_point.x),
        f64::from(dragged_point.y),
    ));
    main_assert_ne!(
        app.dialogs.client_list
            .as_ref()
            .expect("runtime list")
            .layout(preferred, line_height)
            .bounds =>
        before_layered_move,
        "CMouse updates its retained drag element before z-order routing"
    );
    main_assert!(app.dialogs.client_list.as_ref().expect("runtime list").has_positional_pointer_drag());
    app.test_left_button(ElementState::Released);
    main_assert!(!app.dialogs.client_list.as_ref().expect("runtime list").has_positional_pointer_drag());
    app.chat.external_dialog_visible = false;

    let dragged_layout = app
        .dialogs.client_list
        .test_ref()
        .layout(preferred, line_height);
    let resize_drag_start = GuiPoint::new(
        (dragged_layout.caption.expect("an ordinary dialog owns its title widgets").x + 8) as f32,
        (dragged_layout.caption.expect("an ordinary dialog owns its title widgets").y + dragged_layout.caption.expect("an ordinary dialog owns its title widgets").h / 2) as f32,
    );
    main_assert!(app.dialogs.client_list.as_mut().expect("runtime list").handle_pointer_down(resize_drag_start, preferred, line_height));
    main_assert!(app
        .dialogs.client_list
        .as_mut()
        .expect("runtime list")
        .handle_pointer_move(
            GuiPoint::new(resize_drag_start.x + 7.0, resize_drag_start.y + 3.0),
            preferred,
            line_height,
        ));
    app.resize(641, 481).test_value();
    main_assert!(!app.dialogs.client_list.as_ref().expect("runtime list").has_positional_pointer_drag());
    preferred = scoreboard_preferred_rect(
        app.graphics
            .preferred_dialog_rect(app.mouse_control.then_some(app.local_owner)),
    );
    let retained_after_resize = app
        .dialogs.client_list
        .test_ref()
        .layout(preferred, line_height)
        .bounds;
    let _ = app.dialogs.client_list.test_mut().handle_pointer_move(
        GuiPoint::new(3.0, 3.0),
        preferred,
        line_height,
    );
    main_assert_eq!(
        app.dialogs.client_list
            .as_ref()
            .expect("runtime list")
            .layout(preferred, line_height)
            .bounds =>
        retained_after_resize,
        "resize cancels capture without discarding the retained offset"
    );

    let mut info =
        RuntimeClientListDialog::new_info("Client information", row.client_id, Some(row));
    let info_layout = info.info_layout(preferred, line_height).test_value();
    let info_title = GuiPoint::new(
        (info_layout.caption.expect("an ordinary dialog owns its title widgets").x + 8) as f32,
        (info_layout.caption.expect("an ordinary dialog owns its title widgets").y + info_layout.caption.expect("an ordinary dialog owns its title widgets").h / 2) as f32,
    );
    main_assert!(info.handle_pointer_move(info_title, preferred, line_height));
    app.mode = AppMode::Menu;
    app.dialogs.client_list = Some(info);
    assert_delayed_target(
        &mut app,
        info_title,
        StartupTooltip::text("Client information"),
    );
    let info_close = GuiPoint::new(
        (info_layout.close_button.expect("an ordinary dialog owns its title widgets").x + 1) as f32,
        (info_layout.close_button.expect("an ordinary dialog owns its title widgets").y + 1) as f32,
    );
    main_assert!(app.dialogs.client_list.as_mut().expect("client info").handle_pointer_move(info_close, preferred, line_height));
    assert_delayed_target(
        &mut app,
        info_close,
        StartupTooltip::resource("IDS_MNU_CLOSE"),
    );

    app.dialogs.client_list = None;
    app.startup.options_advanced_dialog = None;
    let mut definition =
        clonk_frontend::definition_sel::DefinitionSelController::new("", Vec::new(), Vec::new());
    let (definition_width, definition_height) = {
        let surface = app.graphics.surface();
        (surface.width() as i32, surface.height() as i32)
    };
    let definition_layout = definition.layout(
        definition_width,
        definition_height,
        &app.assets.clonk_fonts.as_deref().test_value().text,
    );
    let definition_title = GuiPoint::new(
        (definition_layout.caption.x + 8) as f32,
        (definition_layout.caption.y + definition_layout.caption.h / 2) as f32,
    );
    let _ = definition.handle_pointer_move(definition_title, &definition_layout);
    let definition_caption = definition.caption();
    app.definition_selector = Some(definition);
    assert_delayed_target(
        &mut app,
        definition_title,
        StartupTooltip::text(definition_caption),
    );
    let definition_close = GuiPoint::new(
        (definition_layout.close_button.x + 1) as f32,
        (definition_layout.close_button.y + 1) as f32,
    );
    let _ = app
        .definition_selector
        .test_mut()
        .handle_pointer_move(definition_close, &definition_layout);
    assert_delayed_target(
        &mut app,
        definition_close,
        StartupTooltip::resource("IDS_MNU_CLOSE"),
    );
}

#[test]
fn startup_main_missing_classic_resources_fails_before_rendering() {
    let mut app = new_real_classic_menu_app(320, 200);
    let assets = Arc::get_mut(&mut app.assets).test_value();
    assets.menu_background = None;
    assets.logo = None;
    assets.button_textures = None;
    let mut frame = vec![0_u8; 320 * 200 * 4];
    let error = app
        .render(&mut frame)
        .expect_err("main menu must not use bitmap/solid fallbacks");
    main_assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::StartupMainResources { missing })
            if missing.contains(&"LoaderGoldmine1.png")
                && missing.contains(&"Logo.png")
                && missing.contains(&"StartupBigButton.png/StartupBigButtonDown.png")
    ));
}

#[test]
fn global_gui_bootstrap_issues_are_aggregated_in_cpp_init_order() {
    let mut app = new_classic_menu_app(320, 200);
    let assets = Arc::get_mut(&mut app.assets).test_value();
    assets.clonk_fonts = None;
    assets.global_tooltip_font = None;
    assets.startup_dialog_images.remove("GUISpinBoxArrow.png");
    assets.startup_dialog_images.remove("GUIButtonDown.png");
    assets.startup_dialog_images.insert(
        "GUIIcons.png".to_string(),
        ImageData::new(0, 360, Vec::new()),
    );
    assets.startup_dialog_images.remove("GUIBigArrows.png");
    // Insert active-set failures in deliberately reverse/non-oracle
    // order. C4GUI initialization order, never HashMap insertion order,
    // owns the aggregate presented to the caller.
    let mut failures = HashMap::new();
    failures.insert(
        "GUISpinBoxArrow",
        "Scenario.c4s/Graphics.c4g:GUISpinBoxArrow.bmp: unreadable".to_string(),
    );
    failures.insert(
        "FontTitle",
        "Scenario.c4s/Graphics.c4g:Endeavour.ttf: unreadable".to_string(),
    );
    failures.insert(
        "GUIContext",
        "Folder.c4f/Graphics.c4g:GUIContext.jpg: unreadable".to_string(),
    );

    let error = assets
        .require_classic_global_gui_bootstrap_resources(&failures)
        .expect_err("incomplete global GUI bundle must fail as one aggregate");
    main_assert_eq!(
        error =>
        ClassicParityBoundary::GlobalGuiBootstrapResources {
            issues: vec![
                ClassicGuiBootstrapIssue::missing("FontRegular"),
                ClassicGuiBootstrapIssue::malformed(
                    "FontTitle",
                    "the exact active RX font source",
                    "Scenario.c4s/Graphics.c4g:Endeavour.ttf: unreadable",
                ),
                ClassicGuiBootstrapIssue::missing("FontCaption"),
                ClassicGuiBootstrapIssue::missing("FontTiny"),
                ClassicGuiBootstrapIssue::missing("FontTooltip"),
                ClassicGuiBootstrapIssue::missing("GUIButtonDown"),
                ClassicGuiBootstrapIssue::malformed(
                    "GUIIcons",
                    "a non-empty decoded RGBA surface",
                    "0x360 with 0 bytes",
                ),
                ClassicGuiBootstrapIssue::malformed(
                    "GUIContext",
                    "a readable selected bmp/jpeg/jpg/png RGBA surface",
                    "Folder.c4f/Graphics.c4g:GUIContext.jpg: unreadable",
                ),
                ClassicGuiBootstrapIssue::missing("GUIBigArrows"),
                ClassicGuiBootstrapIssue::malformed(
                    "GUISpinBoxArrow",
                    "a readable selected bmp/jpeg/jpg/png RGBA surface",
                    "Scenario.c4s/Graphics.c4g:GUISpinBoxArrow.bmp: unreadable",
                ),
            ],
        }
    );
}

#[test]
fn loading_refresh_failure_latches_before_resources_finished_or_pixels() {
    let mut app = new_classic_menu_app(320, 200);
    app.mode = AppMode::Loading;
    app.loader_error = Some("lower-priority loader failure".to_string());
    remove_global_gui_sheet(&mut app, "GUIBigArrows.png");
    let expected = vec![ClassicGuiBootstrapIssue::missing("GUIBigArrows")];
    let mut frame = vec![0x91; 320 * 200 * 4];
    let error = app
        .render(&mut frame)
        .expect_err("global bundle precedes logical loader errors");
    assert_global_gui_boundary(&error, expected.clone());
    main_assert!(frame.iter().all(|byte| *byte == 0x91));
    let mut native = vec![0x57; 640 * 400 * 4];
    let error = app
        .render_native_loader_text(&mut native, 640, 400)
        .expect_err("global bundle precedes native loader errors");
    assert_global_gui_boundary(&error, expected);
    main_assert!(native.iter().all(|byte| *byte == 0x57));

    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(320, 200, &paths);
    app.mode = AppMode::Loading;
    let loader_state_before = app.loader_screen.test_ref().state().clone();
    let loader_gui_before = app
        .loader_screen
        .test_ref()
        .resources()
        .gui_progress()
        .clone();
    let loader_fonts_before = app.loader_screen.test_ref().resources().fonts().clone();
    let resources = app.assets.loader_resources().test_value();
    let (sender, receiver) = mpsc::channel();
    let mut failures = HashMap::new();
    failures.insert(
        "GUISpinBoxArrow",
        "Scenario.c4s/Graphics.c4g:GUISpinBoxArrow.bmp: unreadable".to_string(),
    );
    app.loading_state = Some(ScenarioLoadingState::new(
        FrontendScenario::fallback(),
        resources,
        failures.clone(),
        Vec::new(),
        receiver,
    ));
    sender
        .send(ScenarioLoadingEvent::RefreshResources)
        .test_value();
    sender
        .send(ScenarioLoadingEvent::Finished(Err(
            "finished must remain queued".to_string(),
        )))
        .test_value();
    let boundary = ClassicParityBoundary::GlobalGuiBootstrapResources {
        issues: vec![ClassicGuiBootstrapIssue::malformed(
            "GUISpinBoxArrow",
            "a readable selected bmp/jpeg/jpg/png RGBA surface",
            failures["GUISpinBoxArrow"].clone(),
        )],
    };
    let error = app
        .update()
        .expect_err("refresh failure must fail before resource replacement");
    assert_engine_parity_boundary(error, boundary.clone());
    let state = app.loading_state.test_ref();
    main_assert!(state.refresh_requested);
    main_assert!(state.refreshed_resources.is_some());
    main_assert_eq!(state.refreshed_global_gui_failures.as_ref() => Some(&failures));
    main_assert!(app.active_global_gui_failures.is_empty());
    main_assert_eq!(app.mode => AppMode::Loading);
    let loader = app.loader_screen.test_ref();
    main_assert_eq!(loader.state() => &loader_state_before);
    main_assert_eq!(loader.resources().gui_progress() => &loader_gui_before);
    main_assert!(Arc::ptr_eq(loader.resources().fonts(), &loader_fonts_before));

    let error = app
        .update()
        .expect_err("latched failure must guard the next update at ingress");
    assert_engine_parity_boundary(error, boundary);
    let mut frame = vec![0x62; 320 * 200 * 4];
    let error = app
        .render(&mut frame)
        .expect_err("latched failure must guard logical loader render");
    main_assert!(matches!(error.downcast_ref::<ClassicParityBoundary>(), Some(ClassicParityBoundary::GlobalGuiBootstrapResources { .. })));
    main_assert!(frame.iter().all(|byte| *byte == 0x62));
}

#[test]
fn accepted_loading_reaches_100_only_after_successful_activation() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = test_repository_root();

    let make_resources = |app: &GameApp, pixel: [u8; 4]| {
        let mut pixels = Vec::new();
        for _ in 0..3 {
            pixels.extend_from_slice(&pixel);
        }
        LoaderResources::new(
            app.assets
                .clonk_fonts
                .clone()
                .expect("classic loader fonts"),
            ImageData::new(3, 1, pixels),
        )
        .test_value()
    };

    let mut success = new_menu_app_with_paths(320, 200, &paths);
    let staged = prepare_tutorial_host_lobby(&success, repository);
    let StagedNetworkHostScenario {
        frontend,
        scenario,
        loader_screen,
        ..
    } = staged;
    success.loader_screen = loader_screen;
    let refreshed = make_resources(&success, [0x11, 0x22, 0x33, 0xff]);
    let expected_progress = refreshed.progress_bar().test_value().pixels().to_vec();
    let (sender, receiver) = mpsc::channel();
    success.loading_state = Some(ScenarioLoadingState::new(
        frontend,
        refreshed,
        HashMap::new(),
        Vec::new(),
        receiver,
    ));
    success.mode = AppMode::Loading;
    sender
        .send(ScenarioLoadingEvent::RefreshResources)
        .test_value();
    sender
        .send(ScenarioLoadingEvent::Finished(Ok(scenario)))
        .test_value();
    success.poll_loading().test_value();
    main_assert_eq!(success.mode => AppMode::Running);
    main_assert!(success.loading_state.is_none());
    main_assert_eq!(success.loader_screen.as_ref().expect("loader retained").state().progress() => 100);
    main_assert!(success.active_global_gui_failures.is_empty());
    main_assert_eq!(
        success
            .loader_screen
            .as_ref()
            .expect("loader retained")
            .resources()
            .progress_bar()
            .expect("installed refreshed progress")
            .pixels() =>
        expected_progress
    );

    let mut failure = new_menu_app_with_paths(320, 200, &paths);
    let staged = prepare_tutorial_host_lobby(&failure, repository);
    let StagedNetworkHostScenario {
        scenario,
        loader_screen,
        ..
    } = staged;
    failure.loader_screen = loader_screen;
    let refreshed = make_resources(&failure, [0x44, 0x55, 0x66, 0xff]);
    let (sender, receiver) = mpsc::channel();
    failure.loading_state = Some(ScenarioLoadingState::new(
        FrontendScenario::fallback(),
        refreshed,
        HashMap::new(),
        Vec::new(),
        receiver,
    ));
    failure.mode = AppMode::Loading;
    sender
        .send(ScenarioLoadingEvent::RefreshResources)
        .test_value();
    sender
        .send(ScenarioLoadingEvent::Finished(Ok(scenario)))
        .test_value();
    failure.poll_loading().test_value();
    main_assert_eq!(failure.mode => AppMode::Menu);
    main_assert_eq!(failure.startup.view => StartupView::MainMenu);
    main_assert!(failure.loading_state.is_none());
    // The return through PreInit re-initializes the loader screen for the
    // next game (src/C4Application.cpp:242-247,373-389).
    main_assert!(failure.loader_screen.is_some());
    main_assert!(failure.loader_error.is_none());
    main_assert!(failure.active_global_gui_failures.is_empty());
    assert_startup_error_log(
        &failure,
        "Scenario `Rust Sandbox` is missing a filesystem path",
    );
    main_assert_eq!(failure.startup_restart_diagnostics => StartupRestartDiagnostics::default());
}

#[test]
fn visible_ingame_menu_without_exact_resources_fails_before_rendering() {
    let mut app = new_menu_app(320, 200);
    app.start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    let assets = Arc::get_mut(&mut app.assets).test_value();
    for name in ["Menu.png", "Options.png", "Control.png", "Player.png"] {
        assets.startup_dialog_images.remove(name);
    }
    Arc::make_mut(&mut assets.hud_graphics).captain = None;
    app.ingame_menu.replace(
        app.local_owner,
        Some(IngameMenuState::surrender_menu(&IngameMenuLabels::default())),
    );
    app.dialogs.scoreboard_initial_reconcile_pending = true;
    let before = runtime_global_ui_snapshot(&app);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    let error = app
        .render(&mut frame)
        .expect_err("text-only in-game menu fallback must not render");
    main_assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::IngameMenuResources { missing })
            if missing.contains(&"Menu.png")
                && missing.contains(&"Options.png")
                && missing.contains(&"Control.png")
                && missing.contains(&"Player.png")
                && missing.contains(&"Captain.png")
    ));
    main_assert_eq!(runtime_global_ui_snapshot(&app) => before);
}

#[test]
fn screenshot_folder_override_falls_back_to_install_root() {
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        b"[General]\nName=M\xe4ker\nScreenshotFolder=Configured Screenshots\n",
    )
    .test_value();
    let blocked = install.path().join("Configured Screenshots");
    fs::write(&blocked, b"not a directory").test_value();

    let (path, result) = prepare_numbered_screenshot_path(Some(&paths));

    result.test_value();
    main_assert_eq!(path => install.path().join("Screenshot001.png"));
}

#[test]
fn scenario_head_font_installs_the_pre_definition_size_twenty_loader_bundle() {
    let _lock = env_lock().lock();
    let root = tempdir();
    install_global_gui_and_loader_test_root(root.path());
    let scenario_path = root.path().join("content/FontScenario.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Font scenario\nLoader=LoaderFont.png\nFont=SomeFace,20\n",
    )
    .test_value();
    write_preview_png(
        &scenario_path.join("LoaderFont.png"),
        [0x12, 0x34, 0x56, 0xff],
    );
    fs::copy(
        root.path().join("planet/System.c4g/Endeavour.ttf"),
        scenario_path.join("SomeFace.ttf"),
    )
    .test_value();
    let user = root.path().join("user");
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(root.path())),
        (
            "LC_CONTENT_DIR",
            Some(root.path().join("content").as_path()),
        ),
        ("LC_USER_DATA_DIR", Some(user.as_path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();
    let assets = FrontendAssets::load(Some(&paths));
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "FontScenario.c4s".to_string();
    scenario.title = "Font scenario".to_string();
    scenario.kind = ScenarioKind::Scenario;
    scenario.path = Some(scenario_path.clone());
    let definition_load = ScenarioDefinitionLoad::Seed {
        modules: Vec::new(),
        definition_root: None,
    };

    let setup = build_scenario_loader(&scenario, &definition_load, &paths, &assets).test_value();
    let fonts = setup.screen.resources().fonts();
    for (name, line_height) in [
        ("Log", fonts.mini.line_height),
        ("MainSmall", fonts.main_small.line_height),
        ("Main", fonts.text.line_height),
        ("Caption", fonts.caption.line_height),
        ("Title", fonts.title.line_height),
    ] {
        main_assert_eq!(line_height => 31, "explicit ,20 must override {name} size");
    }
    let tooltip = setup.initial_tooltip_font.as_deref().test_value();
    main_assert_eq!(tooltip.line_height => 31);
    main_assert_eq!(tooltip.h_space => 0);
    // `Font=...,20` collapses every role onto one explicit size. The
    // native builder used to refuse any recipe that was not its hard-coded
    // 22/16/14/13/12 map; it now carries the resolved sizes, so it serves
    // this one — but it must report exactly the uniform map the loader
    // resolved, never a different one.
    let uniform_twenty = clonk_frontend::clonk_fonts::NativeFontSizes {
        title: 20,
        caption: 20,
        text: 20,
        main_small: 20,
        mini: 20,
    };
    main_assert_eq!(setup.initial_native_font_source.as_ref().expect("explicit Head.Font size is serviceable").sizes => uniform_twenty);
    main_assert_eq!(setup.refreshed_native_font_source.as_ref().expect("the refresh keeps the same explicit recipe").sizes => uniform_twenty);

    // A definition root is not registered until the later full resource
    // refresh. It cannot rescue a face missing during InitLoaderScreen.
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Font scenario\nLoader=LoaderFont.png\nFont=DefinitionOnly,20\n",
    )
    .test_value();
    let definition_root = tempdir();
    let objects = definition_root.path().join("Objects.c4d");
    fs::create_dir_all(&objects).test_value();
    fs::copy(
        root.path().join("planet/System.c4g/Endeavour.ttf"),
        objects.join("DefinitionOnly.ttf"),
    )
    .test_value();
    let error = build_scenario_loader(
        &scenario,
        &ScenarioDefinitionLoad::Fixed {
            modules: vec!["Objects.c4d".to_string()],
            definition_root: Some(path_with_trailing_native_separator(definition_root.path())),
        },
        &paths,
        &assets,
    )
    .err()
    .test_value();
    main_assert!(error.to_string().contains("DefinitionOnly"), "unexpected pre-definition font error: {error:#}");
}

#[test]
fn installed_startup_loader_renders_before_boot_completion() {
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    main_assert_eq!(app.mode => AppMode::Loading);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.chunks_exact(4).any(|pixel| pixel != [0, 0, 0, 0]));
    main_assert_eq!(app.loader_screen.as_ref().expect("loader").selection().context() => clonk_frontend::loader_screen::LoaderContext::Startup);
    let state = app.loader_screen.test_ref().state();
    main_assert_eq!(state.title() => "Loading...");
    main_assert_eq!(state.progress() => 0);
    main_assert_eq!(state.log() => &clonk_frontend::loader_screen::LoaderLog::Hidden);
}

#[test]
fn installed_scenario_loader_uses_recursive_folder_resource_tier() {
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    paths.ensure_user_dirs().test_value();
    let mut config = Config::new();
    config.set_in(Some("General"), "LanguageEx", "US");
    config.save(paths.config_file()).test_value();
    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    let scenario =
        resolve_next_mission_scenario(&app.scensel.catalog, "Fantasy.c4f/Crystalvalley.c4s")
            .test_value();
    let setup = build_scenario_loader(
        &scenario,
        &ScenarioDefinitionLoad::Seed {
            modules: vec!["Objects.c4d".to_string()],
            definition_root: None,
        },
        &paths,
        app.assets.as_ref(),
    )
    .test_value();
    main_assert_eq!(setup.screen.selection().context() => clonk_frontend::loader_screen::LoaderContext::Scenario);
    main_assert_eq!(setup.screen.selection().effective_specification() => "LoaderFantasy*");
    main_assert!(setup.screen.selection().selected_filename().starts_with("LoaderFantasy"));

    let initial_source = setup.initial_native_font_source.clone().test_value();
    let refreshed_source = setup.refreshed_native_font_source.clone().test_value();
    app.configure_native_startup_fonts(1.5, false);
    app.install_active_classic_fonts(
        setup.screen.resources().fonts().clone(),
        setup.initial_tooltip_font.clone(),
        Some(initial_source),
    );
    main_assert!(app.can_defer_native_loader_text(1.5));

    app.install_active_classic_fonts(
        setup.refreshed_resources.fonts().clone(),
        setup.refreshed_tooltip_font.clone(),
        Some(refreshed_source),
    );
    app.mode = AppMode::Running;
    main_assert!(app.can_present_ordered_native_text(1.5));
}

#[test]
fn client_network_settings_supply_the_local_system_resource_candidate() {
    // GameRes.InitNetwork resolves the host's non-loadable System core
    // against the client's installed System.c4g before DoLobby
    // (src/C4GameParameters.cpp:125-160;
    // src/C4Network2.cpp:329-344).
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let address = SocketAddr::from(([127, 0, 0, 1], 11_112));

    let settings = client_settings_for_paths(address, "Client".to_string(), Some(&paths))
        .test_value();

    main_assert_eq!(
        settings.server_addresses =>
        [
            clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Tcp, address),
            clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Udp, address),
        ]
    );
    main_assert_eq!(settings.resource_directory => paths.cache_dir().join("Network"));
    main_assert_eq!(settings.local_system_path.as_deref() => Some(paths.system_group_path()));
    main_assert_eq!(settings.mesh_tcp_bind_address => Some(SocketAddr::from(([0_u16; 8], 11_112))));
    main_assert_eq!(settings.mesh_udp_bind_address => Some(SocketAddr::from(([0_u16; 8], 11_113))));
    main_assert!(settings.local_resource_roots.iter().any(|root| Some(root.as_path()) == paths.content_dir()));
}

#[test]
fn player_context_menu_missing_global_resources_fails_typed_without_selection_mutation() {
    let mut app = new_menu_app(640, 480);
    app.startup.player_models
        .push(clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: "No Assets".to_string(),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: String::new(),
        });
    app.open_player_selection_dialog();
    let layout = clonk_frontend::startup_plrsel::plrsel_layout(640, 480);
    app.startup.player_dialog
        .test_mut()
        .set_pointer_position(Some(GuiPoint::new(
            (layout.list_client.x + 2) as f32,
            (layout.list_client.y + layout.item_height / 2) as f32,
        )));

    remove_global_gui_sheet(&mut app, "GUISpinBoxArrow.png");
    let selected_before = app.startup.player_dialog.test_ref().selected_index();
    let error = app
        .open_startup_player_context_menu(false)
        .expect_err("missing process-global resource must fail typed");
    main_assert!(matches!(error, EngineError::ClassicMenuParityBoundary { ref detail } if detail.contains("GUISpinBoxArrow")));
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.startup.player_dialog.as_ref().expect("player controller").selected_index() => selected_before);
}

#[test]
fn resource_join_record_copies_player_group_for_replay() {
    let directory = tempdir();
    let player_path = directory.path().join("Alice.c4p");
    let mut player_group = MutableGroup::new("Alice.c4p");
    player_group
        .add_file(
            "Player.txt",
            b"[Player]\nName=Alice\n[Preferences]\nColorDw=255\n".to_vec(),
        )
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();
    let output_path = directory.path().join("001-Resource.c4s");
    let mut app = new_state_only_running_sandbox_app();
    install_test_recording_template(&mut app, output_path.clone());
    app.admission_resources.mark_complete(17, player_path);
    app.start_recording(true).test_value();
    let core = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            clonk_network::HostResourceType::Player as u8,
            17,
            true,
            LegacyCString::from_bytes(b"Players/Alice.c4p".to_vec()).test_value(),
            clonk_engine::NetworkResourceCore::default(),
    );
    let packet = clonk_engine::ControlPacket::JoinPlayer(netresources_fixture!(
        join_player_filename_at_client_info_id_source:
            LegacyCString::from_bytes(b"Alice.c4p".to_vec()).test_value(),
            0,
            1,
            clonk_engine::JoinPlayerSource::Resource(core.clone()),
    ));

    app.record_control_packet(&packet);
    main_assert!(app.finish_recording().is_none());

    let record = Group::open(&output_path).test_value();
    let copied = record.open_child("17-Alice.c4p").test_value();
    main_assert!(copied.exists("Player.txt"));
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(output_path);
    app.active_scenario = Some(scenario);
    app.control_playback = Some(
        ControlRecordPlayback::from_bytes(&record.read_file("CtrlRec.c4b").test_value())
            .test_value(),
    );
    main_assert_eq!(app.replay_record_player_file(&core).expect("reload copied player").name => "Alice");
}

#[test]
fn recreated_savegame_record_copies_current_profile_under_saved_info_id() {
    // Direct RecreatePlayers joins have no JoinPlayer control to record, so
    // C++ adds the current profile itself as Recreate-<saved ID>.c4p
    // (C4PlayerInfo.cpp:1594-1598).
    let directory = tempdir();
    let player_path = directory.path().join("Alice.c4p");
    let mut player_group = MutableGroup::new("Alice.c4p");
    player_group
        .add_file("Player.txt", b"[Player]\nName=Alice\n".to_vec())
        .test_value();
    fs::write(&player_path, player_group.pack().test_value()).test_value();
    let output_path = directory.path().join("001-Recreate.c4s");
    let mut app = new_state_only_running_sandbox_app();
    install_test_recording_template(&mut app, output_path.clone());
    app.start_recording(true).test_value();

    app.record_recreated_player_file(7, &player_path);
    main_assert!(app.finish_recording().is_none());

    let record = Group::open(&output_path).test_value();
    let copied = record.open_child("Recreate-7.c4p").test_value();
    main_assert!(copied.exists("Player.txt"));
}

#[test]
fn recreated_malformed_player_file_is_recorded_as_opaque_bytes() {
    // C4Record::AddFile copies the external source before Players.Join opens
    // it, so a malformed/non-group source is still present for the failed
    // join (C4PlayerInfo.cpp:1594-1603).
    let directory = tempdir();
    let player_path = directory.path().join("Malformed.c4p");
    let payload = b"not a C4Group\0\x80\xff";
    fs::write(&player_path, payload).test_value();
    let output_path = directory.path().join("001-Malformed.c4s");
    let mut app = new_state_only_running_sandbox_app();
    install_test_recording_template(&mut app, output_path.clone());
    app.start_recording(true).test_value();

    app.record_recreated_player_file(7, &player_path);
    main_assert!(app.finish_recording().is_none());

    let record = Group::open(&output_path).test_value();
    main_assert_eq!(record.read_entry_bytes("Recreate-7.c4p").test_value() => payload,);
}

#[test]
fn offline_recreation_shares_filename_ledger_across_one_source_calls() {
    // The saves entry point calls the engine once per row, while native
    // RecreatePlayers keeps FileInUse state across the whole walk
    // (C4PlayerList.cpp:288-303,433-448).
    let directory = tempdir();
    let scenario_path = directory.path().join("Scenario.c4s");
    fs::create_dir(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Game.txt"),
        b"[Player7]\nStatus=1\nIndex=2\nID=7\n\n[Player8]\nStatus=1\nIndex=3\nID=8\n",
    )
    .test_value();
    let profile_path = directory.path().join("Shared.c4p");
    fs::create_dir(&profile_path).test_value();
    fs::write(
        profile_path.join("Player.txt"),
        b"[Player]\nName=Shared\n[Preferences]\nControl=0\nMouse=0\n",
    )
    .test_value();
    let alias_path = profile_path
        .parent()
        .test_ref()
        .join(".")
        .join("Shared.c4p");
    let source = |id| clonk_engine::RuntimeJoinPlayerSource {
        client_id: 0,
        at_client_name: "Local".to_string(),
        info: clonk_engine::ControlPlayerInfoEntry {
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            id,
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            ..Default::default()
        },
        load_unnamed_portraits: true,
    };
    let savegame = OfflineSavegameStartup {
        initial_game_data: None,
        runtime_players: vec![source(7), source(8)],
        external_player_paths: HashMap::from([(7, profile_path), (8, alias_path)]),
        recreation_record_paths: HashMap::new(),
        embedded_player_info_ids: std::collections::HashSet::new(),
        recording_player_info: Default::default(),
        recording_last_player_id: 0,
        unassociated_restore_players: Vec::new(),
        save_game: false,
        wild_takeovers: Vec::new(),
    };
    let mut app = new_state_only_running_sandbox_app();
    let mut engine = clonk_engine::Engine::new();
    engine.set_max_players(4);

    let (local_players, _, _) = app
        .restore_offline_savegame_engine_players(&mut engine, &scenario_path, &savegame)
        .test_value();

    main_assert_eq!(local_players => [2]);
    main_assert!(engine.player(2).is_some());
    main_assert!(engine.player(3).is_none());
}

#[test]
fn offline_recreation_captures_malformed_source_before_failed_join() {
    // C4Record::AddFile copies the source before Players.Join opens it, so a
    // malformed external file remains available after the failed join
    // (C4PlayerInfo.cpp:1594-1603; C4Record.cpp:273-315).
    let directory = tempdir();
    let scenario_path = directory.path().join("Scenario.c4s");
    fs::create_dir(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Game.txt"),
        b"[Player7]\nStatus=1\nIndex=2\nID=7\n",
    )
    .test_value();
    let profile_path = directory.path().join("Malformed.c4p");
    let payload = b"opaque malformed source\0\x80\xff";
    fs::write(&profile_path, payload).test_value();
    let source = clonk_engine::RuntimeJoinPlayerSource {
        client_id: 0,
        at_client_name: "Local".to_string(),
        info: clonk_engine::ControlPlayerInfoEntry {
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            id: 7,
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            ..Default::default()
        },
        load_unnamed_portraits: true,
    };
    let savegame = OfflineSavegameStartup {
        initial_game_data: None,
        runtime_players: vec![source],
        external_player_paths: HashMap::from([(7, profile_path.clone())]),
        recreation_record_paths: HashMap::from([(7, profile_path.clone())]),
        embedded_player_info_ids: std::collections::HashSet::new(),
        recording_player_info: Default::default(),
        recording_last_player_id: 0,
        unassociated_restore_players: Vec::new(),
        save_game: false,
        wild_takeovers: Vec::new(),
    };
    let mut app = new_state_only_running_sandbox_app();
    app.recording_enabled = true;
    let mut engine = clonk_engine::Engine::new();

    let (_, _, captured) = app
        .restore_offline_savegame_engine_players(&mut engine, &scenario_path, &savegame)
        .test_value();

    main_assert_eq!(captured => vec![(7, payload.to_vec())]);
    main_assert!(engine.player(2).is_none());

    let output_path = directory.path().join("001-OfflineMalformed.c4s");
    install_test_recording_template(&mut app, output_path.clone());
    app.start_recording(true).test_value();
    fs::remove_file(&profile_path).test_value();
    app.record_recreated_player_file_with_fallback(7, &profile_path, Some(&captured[0].1));
    main_assert!(app.finish_recording().is_none());
    let record = Group::open(&output_path).test_value();
    main_assert_eq!(record.read_entry_bytes("Recreate-7.c4p").test_value() => payload);
}

#[test]
fn synchronized_player_file_with_empty_filename_never_resolves_the_install_root() {
    // C4Player::Save on a filename-less player fails at its EraseItem/
    // C4Group_MoveItem calls without ever renaming the installation
    // (C4Player.cpp:406-462). The Rust fallback used to resolve the empty
    // filename to `install_root.join("")` — the install root itself — and
    // then swap the whole installation aside for the staged commit.
    let install = tempdir();
    let planet = install.path().join("planet");
    fs::create_dir_all(planet.join("System.c4g")).test_value();
    fs::write(
        install.path().join("Sentinel.txt"),
        b"install root survives",
    )
    .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_CONTENT_DIR", None),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();

    let mut app = new_state_only_synthetic_crew_running_sandbox_app();
    app.app_paths = Some(paths);
    let player_number = app.local_owner;
    let info_id = 603;
    let mut state = app.engine.capture_state();
    let player = state
        .players
        .iter_mut()
        .find(|player| player.id == player_number)
        .test_value();
    player.player_info_id = info_id;
    player.status = clonk_engine::PlayerStatus::Active;
    player.script_player = false;
    app.engine.restore_state(&state).test_value();
    app.control_player_infos.replace_snapshot(
        info_id,
        [clonk_engine::PlayerInfoControlData::new(
            0,
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: info_id,
                game_number: player_number,
                ..clonk_engine::ControlPlayerInfoEntry::default()
            }],
            0,
        )],
    );

    let info = app.control_player_infos.get(info_id).cloned().test_value();
    main_assert_eq!(info.filename.as_bytes() => b"");
    main_assert_eq!(app.synchronized_player_profile_path(&info) => None);

    main_assert!(!app.persist_synchronized_local_player_files());
    main_assert_eq!(fs::read(install.path().join("Sentinel.txt")).expect("install root intact") => b"install root survives");
    let residue = fs::read_dir(install.path().parent().expect("install parent"))
        .test_value()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains("lc-rewrite"))
        .count();
    main_assert_eq!(residue => 0, "no staged/backup rewrite residue may appear");
}

#[test]
fn network_set_pre_send_applies_at_packet_position_before_change_to_local() {
    let mut app = new_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 7, 0, 2);
    app.control_clients.replace_snapshot([
        message_client(0, b"Host"),
        message_client(7, b"Client Alice"),
    ]);
    app.engine.set_network_game(true);
    app.engine.set_network_control_mode(true);

    app.apply_ready_controls(
        0,
        vec![
            NetworkControl::Script(clonk_engine::ScriptControlData {
                target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                strictness: clonk_engine::ScriptStrictness::Strict3,
                script: legacy_cstring(b"SetPreSend(76, \"client a*\")"),
                by_client: 0,
            }),
            NetworkControl::ClientRemove(clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: clonk_engine::LegacyCString::default(),
                by_client: 0,
            }),
        ],
    )
    .test_value();

    main_assert!(app.network.is_none());
    main_assert!(app.network_control_clock.is_none());
    main_assert_eq!(runtime_flash_text(&app) => Some("TargetFPS: 76"));
}

#[test]
fn adaptive_presend_uses_live_target_and_emits_the_exact_classic_flash() {
    let mut app = new_running_sandbox_app();
    let mut clock = NetworkControlClock::new(0, 1);
    clock.set_target_fps(76);
    // At 76 fps a frame is 13.2ms, so a 20ms link is one frame plus C++'s
    // one-frame floor and a 30ms link is two.
    clock.observe_control_send_time_ms(20);
    let change = clock.calculate_performance().test_value();
    app.apply_control_presend_change(change).test_value();
    main_assert_eq!(runtime_flash_text(&app) => Some("PreSend: 2  - TargetFPS: 76"));
    clock.complete_control_frame();

    clock.observe_control_send_time_ms(30);
    let change = clock.calculate_performance().test_value();
    app.apply_control_presend_change(change).test_value();
    main_assert_eq!(runtime_flash_text(&app) => Some("PreSend: 3  - TargetFPS: 76"));
}

#[test]
fn console_script_strictness_matches_native_tokens_and_reaches_packets() {
    use clonk_engine::ScriptStrictness::{NonStrict, Strict1, Strict2, Strict3};

    for (config, expected) in [
        (
            "[Developer]\nConsoleScriptStrictness=NonStrict\n",
            NonStrict,
        ),
        ("[Developer]\nConsoleScriptStrictness=Strict1\n", Strict1),
        ("[Developer]\nConsoleScriptStrictness=Strict2\n", Strict2),
        ("[Developer]\nConsoleScriptStrictness=Strict3\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=MaxStrict\n", Strict3),
        ("[Developer]\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=0\n", NonStrict),
        ("[Developer]\nConsoleScriptStrictness=1\n", Strict1),
        ("[Developer]\nConsoleScriptStrictness=2\n", Strict2),
        ("[Developer]\nConsoleScriptStrictness=3\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=4\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=254\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=255\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=256\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=-1\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness=+2\n", Strict2),
        ("[Developer]\nConsoleScriptStrictness=02\n", Strict2),
        ("[Developer]\nConsoleScriptStrictness=0x2\n", Strict2),
        ("[Developer]\nConsoleScriptStrictness=2suffix\n", Strict2),
        ("[Developer]\nConsoleScriptStrictness=-0\n", NonStrict),
        (
            "[Developer]\nConsoleScriptStrictness=\"Strict2\"\n",
            Strict3,
        ),
        ("[Developer]\nConsoleScriptStrictness =Strict2\n", Strict3),
        ("[Developer]\nConsoleScriptStrictness= Strict2\n", Strict2),
        (
            "[Developer]\nConsoleScriptStrictness=Strict2 # comment\n",
            Strict2,
        ),
    ] {
        main_assert_eq!(configured_console_script_strictness(config.as_bytes()) => expected, "config {config:?}");
    }
    let wide_unsigned_long = std::mem::size_of::<std::os::raw::c_ulong>() > 4;
    for (value, expected) in [
        (
            "4294967296",
            if wide_unsigned_long {
                NonStrict
            } else {
                Strict3
            },
        ),
        (
            "4294967298",
            if wide_unsigned_long { Strict2 } else { Strict3 },
        ),
    ] {
        let config = format!("[Developer]\nConsoleScriptStrictness={value}\n");
        main_assert_eq!(configured_console_script_strictness(config.as_bytes()) => expected, "native unsigned-long conversion for {value}");
    }

    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::write(
        paths.config_file(),
        b"[Developer]\nConsoleScriptStrictness=Strict2\n",
    )
    .test_value();

    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths);
    app.engine.set_debug_mode(true);
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);

    app.process_running_chat_text("/script return 1");

    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::Script(script), _)] = decided.as_slice() else {
        panic!("expected one script command, got {decided:?}");
    };
    main_assert_eq!(script.strictness => Strict2);
}

#[test]
fn client_retains_exact_join_data_until_resource_bootstrap_can_apply_it() {
    // HandleJoinData retains the synchronized parameters and dynamic core;
    // InitNetworkFromReference retrieves and overlays those resources later
    // (src/C4Network2.cpp:281-344,1574-1623).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.control_rate = 3;
    let join_data = netresources_fixture!(
        join_envelope:
            3,
            host_config.initial_status,
            snapshot.dynamic,
            snapshot.parameters,
    );
    event_tx
        .send(NetworkEvent::JoinData(join_data.clone()))
        .test_value();

    app.test_network_events();

    main_assert_eq!(app.pending_network_join_data => Some(join_data));
    main_assert_eq!(app.network_control_clock => Some(NetworkControlClock::new(23, 3)));
}

#[test]
fn runtime_join_data_arms_the_client_start_barrier_without_a_status_request() {
    // HandleJoinData hands the JoinData status to HandleStatus, so the joiner
    // begins loading from that packet alone. UpdateChaseTarget may replace it
    // five seconds later, but FinalInit reaches this initial barrier when the
    // load finishes first (7d43b47b src/C4Network2.cpp:558-561,1574-1592,
    // 2017-2057,2161-2183).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    let host_config = clonk_network::HostConfig::default();
    let snapshot = host_config.initial_join_snapshot.test_value();
    // The reference form omits TargetTick, so a running host's JoinData status
    // arrives with the -1 default rather than a usable control target.
    let running = clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_GO, 2, -1);
    event_tx
        .send(NetworkEvent::JoinData(netresources_fixture!(join_envelope: 3, running, snapshot.dynamic, snapshot.parameters)))
        .test_value();

    app.test_network_events();

    // CheckStatusReached retargets to the tick the client actually reached
    // before sending PID_StatusAck (src/C4Network2.cpp:2050-2052).
    main_assert_eq!(
        app.client_start_barrier.local_initialized_at(23) =>
            Some(clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_GO, 2, 23)),
        "the JoinData status must arm the initial runtime-join barrier"
    );
}

#[test]
fn catalog_host_selection_change_discards_and_rearms_preload_state() {
    let mut app = new_state_only_menu_app(320, 200);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    for (identifier, title) in [("Old.c4s", "Old"), ("New.c4s", "New")] {
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = identifier.to_string();
        scenario.title = title.to_string();
        scenario.path = Some(PathBuf::from(identifier));
        app.scensel.catalog
            .insert(identifier.to_string(), scenario);
    }
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario("Old.c4s", "Old");
    lobby.preload.record_result(true);
    app.network_lobby = Some(lobby);

    main_assert!(app.select_network_lobby_scenario("New.c4s", "New"));

    let preload = app.network_lobby.as_ref().test_value().preload;
    main_assert!(!preload.spent);
    main_assert!(preload.manual_button_present);
    main_assert!(preload.eligible);
    main_assert_eq!(app.network_lobby.as_ref().and_then(NetworkLobbyState::selected_identifier) => Some("New.c4s"));
}

#[test]
fn direct_and_synchronized_player_info_register_loadable_resources() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let player_info = |player_id, resource_id| {
        let core = netresources_fixture!(resource_id: resource_id);
        clonk_engine::PlayerInfoControlData::new(
            0,
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: player_id,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core),
                ..Default::default()
            }],
            0,
        )
    };

    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            player_info(1, 48),
        )))
        .test_value();
    app.test_network_events();
    main_assert_eq!(app.admission_resources.status(48) => Some(&AdmissionResourceState::Loading { removed: false }));

    app.apply_ready_controls(0, vec![NetworkControl::PlayerInfo(player_info(2, 49))])
        .test_value();
    main_assert_eq!(app.admission_resources.status(49) => Some(&AdmissionResourceState::Loading { removed: false }));
}

#[test]
fn missing_join_client_does_not_start_or_stall_a_resource_load() {
    let resource_id = 50;
    let controls = vec![NetworkControl::JoinPlayer(
        clonk_engine::JoinPlayerControlData {
            at_client: 9,
            info_id: 1,
            source: clonk_engine::JoinPlayerSource::Resource(
                netresources_fixture!(resource_id: resource_id),
            ),
            ..Default::default()
        },
    )];
    let mut resources = AdmissionResourceStore::default();

    main_assert!(preflight_admission_resources(&mut resources, &ControlClientRegistry::default(), &controls, &HashSet::new(),));
    main_assert_eq!(resources.status(resource_id) => None);
}

#[test]
fn host_direct_player_info_rebalances_random_teams_and_broadcasts_changed_packet() {
    // HandlePlayerInfo runs RecheckPlayers followed by the control-host
    // RecheckTeams. The third unjoined player makes a 3/0 split, so the
    // first member moves once. The incoming Updated flag and the move
    // both mark the same owner, but SendUpdatedPlayers emits that
    // client's complete, clean packet only once
    // (src/C4Network2Players.cpp:245-275; src/C4Teams.cpp:688-730).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));

    let mut metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10, 20], 0),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.control_player_infos.replace_snapshot(
        30,
        [clonk_engine::PlayerInfoControlData::new(
            3,
            0,
            vec![
                set_control_test_player(10, 1, 0),
                set_control_test_player(20, 1, 0),
            ],
            -1,
        )],
    );

    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData::new(
                3,
                clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED,
                vec![
                    set_control_test_player(10, 1, 0),
                    set_control_test_player(20, 1, 0),
                    set_control_test_player(30, 1, 0),
                ],
                0,
            ),
        )))
        .test_value();
    app.test_network_events();

    let teams = app.network_team_assignment.test_ref().teams();
    main_assert_eq!(teams.teams[0].player_ids => vec![20, 30]);
    main_assert_eq!(teams.teams[1].player_ids => vec![10]);
    main_assert_eq!(app.control_player_infos.get(10).unwrap().team => 2);
    main_assert_eq!(app.control_player_infos.get(20).unwrap().team => 1);
    main_assert_eq!(app.control_player_infos.get(30).unwrap().team => 1);

    let broadcasts = commands.take_broadcast_player_infos();
    let [updated] = broadcasts.as_slice() else {
        panic!("expected one rebalanced PlayerInfo packet, got {broadcasts:?}");
    };
    main_assert_eq!((updated.client_id, updated.by_client) => (3, 0));
    main_assert_eq!(updated.flags & (clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED) => 0,);
    main_assert_eq!(updated.players.iter().map(|player| (player.id, player.team)).collect::<Vec<_>>() => vec![(10, 2), (20, 1), (30, 1)],);
}

#[test]
fn running_host_queues_remote_join_before_player_resource_completes() {
    // HandlePlayerInfo starts loading the advertised resource and queues
    // JoinPlayer immediately. Resource completeness is checked later by
    // the synchronized control's PreExecute
    // (src/C4Network2Players.cpp:245-269,353-388;
    // src/C4Control.cpp:811-825).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_clients.register(3, true, false);
    let tick = app.local_control_submission_tick();
    let resource = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            clonk_network::HostResourceType::Player as u8,
            61,
            true,
            clonk_engine::LegacyCString::from_bytes(b"Remote.c4p".to_vec()).test_value(),
            Default::default(),
    );

    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData::new(
                3,
                0,
                vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 41,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(resource.clone()),
                    ..Default::default()
                }],
                0,
            ),
        )))
        .test_value();

    app.test_network_events();

    main_assert_eq!(app.admission_resources.status(resource.id) => Some(&AdmissionResourceState::Loading { removed: false }));
    main_assert_eq!(
        commands.take_submitted_join_players() =>
        vec![(
            tick,
            netresources_fixture!(
                join_player_filename_at_client_info_id_source:
                    resource.filename.clone(),
                    3,
                    41,
                    clonk_engine::JoinPlayerSource::Resource(resource),
            ),
        )]
    );
}

#[test]
fn synchronized_client_remove_rebalances_random_teams_and_broadcasts_changed_packet() {
    // OnClientPart first drops the departing client's unjoined infos and
    // rechecks memberships. The host then redistributes the first
    // relocatable member from the resulting 3/0 split and sends its
    // owner's updated packet (src/C4Network2Players.cpp:425-459;
    // src/C4Teams.cpp:688-730).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients.register(4, true, false);

    let mut metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10, 20, 30], 0),
            set_control_test_team(2, vec![40], 0),
        ],
    );
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let player = |id, team, color, original_color, projected_gain, name: &[u8], forced: &[u8]| {
        let mut player = set_control_test_player(id, team, 0);
        player.color = color;
        player.original_color = original_color;
        player.league_projected_gain = projected_gain;
        player.name = clonk_engine::LegacyCString::from_bytes(name.to_vec()).test_value();
        player.forced_name = clonk_engine::LegacyCString::from_bytes(forced.to_vec()).test_value();
        player
    };
    let mut gain_only = player(50, 0, 0x0000_00f4, 0x0000_00f4, 4, b"History", b"");
    gain_only.flags =
        clonk_engine::PLAYER_INFO_FLAG_JOINED | clonk_engine::PLAYER_INFO_FLAG_REMOVED;
    app.control_player_infos.replace_snapshot(
        50,
        [
            clonk_engine::PlayerInfoControlData::new(
                3,
                0,
                vec![
                    player(10, 1, 0x0000_f400, 0x00f4_0000, 6, b"Alice", b"Alice (2)"),
                    player(20, 1, 0x0000_00f4, 0x0000_00f4, -1, b"Bob", b""),
                    player(30, 1, 0x00f4_f400, 0x00f4_f400, 0, b"Cara", b""),
                ],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                4,
                0,
                vec![player(40, 2, 0x00f4_0000, 0x00f4_0000, 9, b"Alice", b"")],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(5, 0, vec![gain_only], -1),
        ],
    );

    app.apply_ready_controls(
        0,
        vec![NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 4,
                reason: clonk_engine::LegacyCString::default(),
                by_client: 0,
            },
        )],
    )
    .test_value();

    main_assert!(!app.control_clients.contains(4));
    main_assert!(app.control_player_infos.get(40).is_none());
    let teams = app.network_team_assignment.test_ref().teams();
    main_assert_eq!(teams.teams[0].player_ids => vec![20, 30]);
    main_assert_eq!(teams.teams[1].player_ids => vec![10]);
    main_assert_eq!(app.control_player_infos.get(10).unwrap().team => 2);
    main_assert_eq!(app.control_player_infos.get(10).unwrap().color => 0x00f4_0000);
    main_assert!(app.control_player_infos.get(10).unwrap().forced_name.is_empty());
    main_assert_eq!(
        app.control_player_infos
            .client_packet(3)
            .unwrap()
            .players
            .iter()
            .map(|player| player.league_projected_gain)
            .collect::<Vec<_>>() =>
        vec![-1, -1, -1],
    );

    let broadcasts = commands.take_broadcast_player_infos();
    let [updated, gain_only] = broadcasts.as_slice() else {
        panic!("expected two final PlayerInfo packets, got {broadcasts:?}");
    };
    main_assert_eq!((updated.client_id, updated.by_client) => (3, 0));
    main_assert_eq!(updated.flags & (clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED) => 0,);
    main_assert_eq!(
        updated
            .players
            .iter()
            .map(|player| {
                (
                    player.id,
                    player.team,
                    player.color,
                    player.forced_name.as_bytes().to_vec(),
                    player.league_projected_gain,
                )
            })
            .collect::<Vec<_>>() =>
        vec![
            (10, 2, 0x00f4_0000, Vec::new(), -1),
            (20, 1, 0x0000_00f4, Vec::new(), -1),
            (30, 1, 0x00f4_f400, Vec::new(), -1),
        ],
    );
    main_assert_eq!((gain_only.client_id, gain_only.by_client) => (5, 0));
    main_assert_eq!(gain_only.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED => 0);
    main_assert_eq!(gain_only.players.len() => 1);
    main_assert_eq!(gain_only.players[0].id => 50);
    main_assert_eq!(gain_only.players[0].league_projected_gain => -1);
}

#[test]
fn completed_network_resource_enters_the_control_resource_registry() {
    // C4Network2Res::EndLoad calls OnResComplete; later synchronized
    // controls resolve the resource strictly by ID and use getFile()
    // (src/C4Network2Res.cpp:1113-1122,1701-1707;
    // src/C4Control.cpp:758-764).
    let mut app = new_state_only_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    let resource_id = 61;
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    let core = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            3,
            resource_id,
            true,
            clonk_engine::LegacyCString::from_bytes(b"Player.c4p".to_vec()).test_value(),
            Default::default(),
    );
    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id,
            core,
            path: path.clone(),
            local: false,
        })
        .test_value();

    app.process_network_events().test_value();

    main_assert_eq!(app.admission_resources.complete_path(resource_id) => Some(path.as_path()));
}

#[test]
fn unknown_loadable_resource_join_stalls_until_resource_completion() {
    let mut app = new_synthetic_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.engine.set_network_game(true);
    let tick = u32::try_from(app.engine.frame()).test_value();
    let initial_frame = app.engine.frame();
    let info_id = 18;
    let resource_id = 62;
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    let core = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            clonk_network::HostResourceType::Player as u8,
            resource_id,
            true,
            clonk_engine::LegacyCString::from_bytes(b"Player.c4p".to_vec()).test_value(),
            Default::default(),
    );
    event_tx
        .send(netresources_fixture!(
    ready_tick:
        tick,
        vec![
                        NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData::new(1, 0, vec![clonk_engine::ControlPlayerInfoEntry {
                                id: info_id,
                                name: legacy_cstring(b"Delayed resource"),
                                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                                resource: Some(core.clone()),
                                ..Default::default()
                            }], 1)),
                        NetworkControl::JoinPlayer(netresources_fixture!(
            join_player_info_id_source:
                info_id,
                clonk_engine::JoinPlayerSource::Resource(core.clone()),
        )),
                    ],
))
        .test_value();

    app.test_update();

    main_assert_eq!(app.engine.frame() => initial_frame);
    main_assert!(app.network_ticks.ready.contains_key(&tick));
    main_assert!(app.control_player_infos.get(info_id).is_none());
    main_assert_eq!(app.admission_resources.status(resource_id) => Some(&AdmissionResourceState::Loading { removed: false }));
    let wait = app.blocking_resource_wait.test_ref();
    main_assert_eq!(wait.scope => BlockingResourceScope::PlayerJoin);
    main_assert_eq!(wait.resource_id => resource_id);
    main_assert_eq!(wait.display_name => "player file for Delayed resource");
    let progress = app
        .dialogs.messages
        .iter()
        .find(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::BlockingResourceWait {
                    scope: BlockingResourceScope::PlayerJoin,
                    resource_id: 62,
                }
            )
        })
        .test_value();
    main_assert_eq!(progress.state.progress() => Some(0));
    main_assert_eq!(progress.state.message() => "Waiting for player file for Delayed resource...");

    event_tx
        .send(NetworkEvent::ResourceProgress {
            resource_id,
            present_percent: 47,
        })
        .test_value();
    app.test_update();
    main_assert_eq!(app.engine.frame() => initial_frame);
    main_assert_eq!(app.blocking_resource_wait.as_ref().expect("wait remains active").present_percent() => 47);
    main_assert_eq!(
        app.dialogs.messages
            .iter()
            .find(|dialog| matches!(
                dialog.continuation,
                MessageDialogContinuation::BlockingResourceWait { .. }
            ))
            .and_then(|dialog| dialog.state.progress()) =>
        Some(47)
    );

    event_tx
        .send(NetworkEvent::ResourceComplete {
            resource_id,
            core,
            path: path.clone(),
            local: false,
        })
        .test_value();
    app.test_update();

    main_assert_eq!(app.engine.frame() => initial_frame + 1);
    main_assert!(!app.network_ticks.ready.contains_key(&tick));
    main_assert_eq!(app.admission_resources.complete_path(resource_id) => Some(path.as_path()));
    main_assert!(app.snapshot.players.iter().any(|player| player.player_info_id == info_id));
    main_assert!(app.blocking_resource_wait.is_none());
    main_assert!(!app.dialogs.messages.iter().any(|dialog| matches!(dialog.continuation, MessageDialogContinuation::BlockingResourceWait { .. })));
}

#[test]
fn failed_loadable_resource_releases_the_stalled_tick_as_a_noop() {
    let mut app = new_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    let tick = u32::try_from(app.engine.frame()).test_value();
    let initial_frame = app.engine.frame();
    let resource_id = 63;
    let core = netresources_fixture!(resource_id: resource_id);
    event_tx
        .send(netresources_fixture!(
            ready_tick:
                tick,
                vec![NetworkControl::JoinPlayer(
                                netresources_fixture!(
                    join_player_info_id_source:
                        99,
                        clonk_engine::JoinPlayerSource::Resource(core),
                ),
                            )],
        ))
        .test_value();

    app.test_update();
    main_assert_eq!(app.engine.frame() => initial_frame);
    main_assert_eq!(app.admission_resources.status(resource_id) => Some(&AdmissionResourceState::Loading { removed: false }));

    event_tx
        .send(NetworkEvent::ResourceLoadFailed { resource_id })
        .test_value();
    app.test_update();

    main_assert_eq!(app.engine.frame() => initial_frame + 1);
    main_assert_eq!(app.admission_resources.status(resource_id) => Some(&AdmissionResourceState::Unavailable(AdmissionResourceUnavailable::TransferFailed)));
}

#[test]
fn unloadable_resource_join_is_unavailable_and_does_not_stall_tick() {
    // AddByCore returns null for an unloadable absent resource; PreExecute
    // therefore reports ready and JoinPlr later no-ops, without blocking
    // following controls (src/C4Network2Res.cpp:1499-1515;
    // src/C4Control.cpp:73-109,758-764,811-825).
    let mut app = new_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.engine.set_network_game(true);
    let tick = u32::try_from(app.engine.frame()).test_value();
    let initial_frame = app.engine.frame();
    let info_id = 17;
    let resource_id = 61;
    let local_owner = app.local_owner;
    let resource = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            3,
            resource_id,
            false,
            clonk_engine::LegacyCString::from_bytes(b"Missing.c4p".to_vec()).test_value(),
            Default::default(),
    );
    event_tx
        .send(netresources_fixture!(
    ready_tick:
        tick,
        vec![
                        NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData::new(1, 0, vec![clonk_engine::ControlPlayerInfoEntry {
                                id: info_id,
                                ..Default::default()
                            }], 1)),
                        NetworkControl::JoinPlayer(netresources_fixture!(
            join_player_info_id_source:
                info_id,
                clonk_engine::JoinPlayerSource::Resource(resource),
        )),
                        NetworkControl::Player {
                            owner: local_owner,
                            event: ControlEvent::Press(ControlButton::Right),
                        },
                    ],
))
        .test_value();

    app.test_update();

    main_assert_eq!(app.admission_resources.status(resource_id) => Some(&AdmissionResourceState::Unavailable(AdmissionResourceUnavailable::Unloadable)));
    main_assert!(app.snapshot.players.iter().all(|player| player.player_info_id != info_id), "an unavailable resource cannot create a player");
    main_assert_ne!(
        app.engine
            .player(local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_RIGHT) =>
        0,
        "the later control still executes"
    );
    main_assert_eq!(app.engine.frame() => initial_frame + 1);
}

#[test]
fn complete_resource_join_uses_registry_path() {
    // Rust resolves resource-backed execution by resource ID, including on
    // the authoring host when issuance preceded resource completion. Use
    // the registry path, not packet Filename or the core filename
    // (src/C4Control.cpp:758-764; src/C4Network2Res.cpp:1388-1412).
    let mut app = new_synthetic_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.engine.set_network_game(true);
    let tick = u32::try_from(app.engine.frame()).test_value();
    let info_id = 18;
    let resource_id = 62;
    let resolved_path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    app.admission_resources.resources.insert(
        resource_id,
        AdmissionResourceState::Complete {
            path: resolved_path,
            removed: false,
            local: true,
        },
    );
    let resource = netresources_fixture!(
        resource_resource_type_id_loadable_filename:
            3,
            resource_id,
            true,
            legacy_cstring(b"WrongCorePath.c4p"),
            Default::default(),
    );
    event_tx
        .send(netresources_fixture!(
    ready_tick:
        tick,
        vec![
                        NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData::new(1, 0, vec![clonk_engine::ControlPlayerInfoEntry {
                                name: legacy_cstring(b"Resource Tyler"),
                                id: info_id,
                                ..Default::default()
                            }], 1)),
                        NetworkControl::JoinPlayer(netresources_fixture!(
            join_player_filename_at_client_info_id_source:
                legacy_cstring(b"WrongPacketPath.c4p"),
                0,
                info_id,
                clonk_engine::JoinPlayerSource::Resource(resource),
        )),
                    ],
))
        .test_value();

    app.test_update();

    let joined = app
        .snapshot
        .players
        .iter()
        .find(|player| player.player_info_id == info_id)
        .test_value();
    main_assert_eq!(joined.name => "Resource Tyler");
    main_assert_eq!((joined.score, joined.total_playing_time) => (42, 99));
}

#[test]
fn network_presends_next_tick_on_the_frame_before_execution() {
    // With ControlRate 2 and the default one-frame PreSend, tick 10 is
    // transmitted on non-control frame 1 so a one-frame network trip can
    // return the complete packet before frame 2 executes it
    // (src/C4GameControl.cpp:253-258;
    // src/C4GameControlNetwork.cpp:145-176).
    let mut app = new_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_control_clock = Some(network::NetworkControlClock::new(9, 2));
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(9, 2).test_value(),
    );

    event_tx
        .send(netresources_fixture!(ready_tick: 9, Vec::new()))
        .test_value();
    app.test_update();
    main_assert_eq!(app.engine.frame() => 1);
    main_assert_eq!(commands.take_finalized_ticks() => vec![9]);

    app.test_update();
    main_assert_eq!(app.engine.frame() => 2);
    main_assert_eq!(commands.take_finalized_ticks() => vec![10], "tick 10 must leave one frame before its execution frame");

    // Deliver the echoed aggregate after that one-frame delay. Frame 2
    // must execute immediately rather than first submitting tick 10 and
    // stalling for another round trip.
    event_tx
        .send(netresources_fixture!(ready_tick: 10, Vec::new()))
        .test_value();
    app.test_update();
    main_assert_eq!(app.engine.frame() => 3);
    main_assert_eq!(app.network_control_clock.map(network::NetworkControlClock::current_tick) => Some(11));
    main_assert!(commands.take_finalized_ticks().is_empty());
}

#[test]
fn valid_construction_menu_drop_submits_exact_shift_append_packet() {
    let (mut app, owner, menu_point, valid_point, _invalid, valid_world, raw_c4id) =
        construction_drag_fixture();
    let (manager, _events, mut network_commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();

    begin_construction_drag(&mut app, menu_point, valid_point);
    main_assert!(matches!(app.construction_menu_drag.as_ref(), Some(ConstructionMenuDrag::Active {site_valid: true,..})));
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_left_button(ElementState::Released);

    let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
    main_assert!(controls.is_empty());
    main_assert_eq!(
        commands =>
        vec![(
            tick,
            PlayerCommandControlData {
                player: owner,
                command: CommandId::Construct as i32,
                x: valid_world.x,
                y: valid_world.y,
                target: 0,
                target2: 0,
                data: raw_c4id,
                add_mode: 5,
                by_client: 7,
            },
        )]
    );
    main_assert!(selections.is_empty());
    main_assert!(app.construction_menu_drag.is_none());
}

// `CStdFont::DrawText` and `GetTextExtent` both consume an unknown `{{...}}`
// id with no advance (src/StdFont.cpp:868-890), so a caption carrying one is
// laid out, drawn and hit-tested on the geometry its remaining text gives it.
// The pointer keeps reaching the menu rather than the world, and no entry
// point fails (clonk-org/clonk-rs#1204).
#[test]
fn a_script_menu_with_an_unresolved_image_still_owns_its_pointer() {
    let mut app = new_classic_running_sandbox_app();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let cursor = app.engine.test_crew_cursor(app.local_owner);
    let mut menu = two_item_script_menu(cursor);
    menu.caption = "{{MISS}} unavailable".to_string();
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(menu)),
                ..ObjectUpdate::default()
            },
        )
        .test_value();

    // A point the menu owns, found through the same hit test the renderer's
    // geometry feeds.
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width() as i32, surface.height() as i32)
    };
    let menu_point = (0..height)
        .flat_map(|y| (0..width).map(move |x| GuiPoint::new(x as f32 + 0.5, y as f32 + 0.5)))
        .find(|point| {
            matches!(
                app.script_menu_pointer_target(*point),
                Ok(Some(EngineScriptMenuPointerTarget::Item(_)))
            )
        })
        .test_value();
    let point = PhysicalPosition::new(f64::from(menu_point.x), f64::from(menu_point.y));

    // Hover, left-down and right-up all succeed where they used to raise a
    // typed boundary.
    app.handle_cursor_moved(point)
        .expect("hover is well defined over consumed markup");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("left-down is well defined");

    let (manager, _events) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.handle_right_mouse_button(ElementState::Released)
        .expect("right-up is well defined");

    // And the hit test still names the row, which is what the pointer routes
    // on — the consumed markup costs the caption no advance.
    main_assert!(matches!(
        app.script_menu_pointer_target(menu_point),
        Ok(Some(EngineScriptMenuPointerTarget::Item(_)))
    ));
}

#[test]
fn script_menu_pointer_requires_global_resources_before_fallback_layout() {
    let mut app = new_running_sandbox_app();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(app.local_owner).test_value();
    let point = PhysicalPosition::new(
        f64::from(viewport.x) + f64::from(viewport.width) / 2.0,
        f64::from(viewport.y) + f64::from(viewport.height) / 2.0,
    );
    let cursor = app.engine.test_crew_cursor(app.local_owner);
    app.engine
        .apply_object_update(
            cursor,
            ObjectUpdate {
                menu: Some(Some(two_item_script_menu(cursor))),
                ..ObjectUpdate::default()
            },
        )
        .test_value();
    app.assets = Arc::new(FrontendAssets::load(None));

    let error = app
        .handle_cursor_moved(point)
        .expect_err("pointer layout must reject missing classic global resources");
    main_assert!(matches!(&error, EngineError::ClassicMenuParityBoundary { .. }));
    main_assert!(error.to_string().contains("classic process-global C4GUI bootstrap"));
    main_assert!(error.to_string().contains("FontRegular: missing"));
}

/// `C4GraphicsResource::Init` returns false on the first game/HUD file it
/// cannot load, so each of these is mandatory and none may reach pixels through
/// an optional fallback (C4GraphicsResource.cpp:200-231).
#[test]
fn running_hud_rejects_each_missing_mandatory_graphics_facet() {
    let mut app = new_classic_lightweight_running_sandbox_app();
    // Every mandatory facet, in native `Init` order.
    type MandatoryFacet = (
        &'static str,
        fn(&mut clonk_frontend::HudGraphics) -> &mut Option<ImageData>,
    );
    let facets: [MandatoryFacet; 23] = [
        ("Control.png", |hud| &mut hud.control),
        ("Fire.png", |hud| &mut hud.fire),
        ("Background.png", |hud| &mut hud.background),
        ("Flag.png", |hud| &mut hud.flag),
        ("Crew.png", |hud| &mut hud.crew),
        ("Score.png", |hud| &mut hud.score),
        ("Wealth.png", |hud| &mut hud.wealth),
        ("Player.png", |hud| &mut hud.player),
        ("Rank.png", |hud| &mut hud.rank),
        ("Captain.png", |hud| &mut hud.captain),
        ("SelectMark.png", |hud| &mut hud.select_mark),
        ("Menu.png", |hud| &mut hud.menu),
        ("Logo.png", |hud| &mut hud.logo),
        ("Construction.png", |hud| &mut hud.construction),
        ("Energy.png", |hud| &mut hud.energy),
        ("Magic.png", |hud| &mut hud.magic),
        ("UpperBoard.png", |hud| &mut hud.upper_board),
        ("Arrow.png", |hud| &mut hud.arrow),
        ("Exit.png", |hud| &mut hud.exit),
        ("Hand.png", |hud| &mut hud.hand),
        ("Gamepad.png", |hud| &mut hud.gamepad),
        ("Build.png", |hud| &mut hud.build),
        ("EnergyBars.png", |hud| &mut hud.energy_bars),
    ];

    // With the whole inventory present the frame renders.
    let mut frame = vec![0x5a; app.graphics.surface().pixels().len()];
    app.test_render(&mut frame);
    let _ = &frame;

    for (name, field) in facets {
        let taken = {
            let hud = Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics);
            field(hud).take().unwrap_or_else(|| {
                panic!("the classic fixture must ship {name} for this sweep to mean anything")
            })
        };

        app.graphics.surface_mut().fill(Color::opaque(91, 47, 13));
        let surface_before = app.graphics.surface().pixels().to_vec();
        let mut frame = vec![0x5a; surface_before.len()];
        let frame_before = frame.clone();
        let error = match app.render(&mut frame) {
            Err(error) => error,
            Ok(_) => panic!("a missing {name} must fail closed, not fall back"),
        };
        main_assert_eq!(
            error.downcast_ref::<ClassicParityBoundary>() =>
            Some(&ClassicParityBoundary::HudResources {
                missing: vec![name]
            }),
            "{name} must be reported as the single missing facet"
        );
        main_assert_eq!(frame => frame_before, "{name}: preflight must precede output writes");
        main_assert_eq!(app.graphics.surface().pixels() => surface_before.as_slice(), "{name}: preflight must precede logical-surface writes");

        let hud = Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics);
        *field(hud) = Some(taken);
    }

    // Restoring the whole inventory renders again, so the sweep left no
    // facet behind.
    let mut frame = vec![0x5a; app.graphics.surface().pixels().len()];
    app.test_render(&mut frame);
}

#[test]
fn upper_board_and_message_board_fail_closed_when_resources_missing() {
    let mut app = new_classic_lightweight_running_sandbox_app();
    let assert_refusal = |app: &mut GameApp, missing: Vec<&'static str>| {
        app.graphics.surface_mut().fill(Color::opaque(91, 47, 13));
        let surface_before = app.graphics.surface().pixels().to_vec();
        let mut frame = vec![0x5a; surface_before.len()];
        let frame_before = frame.clone();

        let error = app
            .render(&mut frame)
            .expect_err("missing classic HUD resource must fail closed");
        let expected = ClassicParityBoundary::HudResources { missing };
        main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&expected));
        main_assert!(error.to_string().contains("refusing generic Rust fallback"), "boundary must explain why the fallback is unreachable: {error:#}");
        main_assert_eq!(frame => frame_before, "preflight must precede output writes");
        main_assert_eq!(app.graphics.surface().pixels() => surface_before.as_slice(), "preflight must precede logical-surface writes");
    };

    let upper_board = Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics)
        .upper_board
        .take()
        .test_value();
    assert_refusal(&mut app, vec!["UpperBoard.png"]);
    Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics).upper_board =
        Some(upper_board);

    let background = Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics)
        .background
        .take()
        .test_value();
    assert_refusal(&mut app, vec!["Background.png"]);
    Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics).background =
        Some(background);

    let (background, upper_board) = {
        let hud = Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics);
        (
            hud.background.take().test_value(),
            hud.upper_board.take().test_value(),
        )
    };
    assert_refusal(&mut app, vec!["Background.png", "UpperBoard.png"]);
    let hud = Arc::make_mut(&mut Arc::get_mut(&mut app.assets).test_value().hud_graphics);
    hud.background = Some(background);
    hud.upper_board = Some(upper_board);
}

#[test]
fn runtime_f3_and_ingame_music_action_install_the_localized_flash() {
    let mut app = new_running_sandbox_app();
    let configured_music = app
        .sound.context
        .as_ref()
        .map(|audio| audio.borrow().options.music_enabled);
    let expected_enabled = !app.test_audio_ref().music_is_playing();
    let resources = app.runtime_flash_resources().test_value().clone();
    let expected_text = resources.music_on_off(expected_enabled);
    app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert_eq!(app.sound.runtime_music_enabled => expected_enabled);
    main_assert_eq!(app.sound.context.as_ref().map(|audio| audio.borrow().options.music_enabled) => configured_music, "running global F3 must not change persisted RXMusic");
    let message = app.runtime_flash_message.test_ref();
    main_assert_eq!(message.text => expected_text);
    main_assert_eq!(
        usize::from(message.remaining_draws) =>
        runtime_flash_stored_bytes(&expected_text, resources.charset)
            .expect("encode expected music flash")
            .len()
            * 2
    );
    let after_down = app.runtime_flash_message.clone();
    app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert_eq!(app.sound.runtime_music_enabled => !expected_enabled);
    main_assert_ne!(app.runtime_flash_message => after_down);
    main_assert_eq!(app.sound.context.as_ref().map(|audio| audio.borrow().options.music_enabled) => configured_music);
    let after_repeat = app.runtime_flash_message.clone();
    app.test_key(VirtualKeyCode::F3, ElementState::Released);
    main_assert_eq!(app.runtime_flash_message => after_repeat);

    let mut menu = new_running_sandbox_app();
    let configured_before = menu
        .sound.context
        .as_ref()
        .map(|audio| audio.borrow().options.music_enabled);
    menu.ingame_menu.replace(
        menu.local_owner,
        Some(IngameMenuState::options_menu(
            &menu.option_flags(menu.local_owner),
            1,
            &IngameMenuLabels::default(),
        )),
    );
    menu.apply_ingame_menu_action(MenuAction::ToggleMusic)
        .test_value();
    main_assert!(menu.runtime_flash_message.is_some());
    main_assert_eq!(menu.ingame_menu.as_ref().map(IngameMenuState::page) => Some(ingame_menu::MenuPage::Options));
    if let (Some(before), Some(audio)) = (configured_before, menu.sound.context.as_ref()) {
        let audio = audio.borrow();
        main_assert_eq!(audio.options.music_enabled => !before);
        main_assert_eq!(menu.sound.runtime_music_enabled => !before);
    }

    let mut startup = new_running_sandbox_app();
    startup.return_to_menu();
    startup.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(startup.runtime_flash_message.is_none());
    startup.mode = AppMode::Loading;
    startup.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(startup.runtime_flash_message.is_none());
}

#[test]
fn graphics_resources_validate_liquid_even_when_animation_disabled() {
    let directory = tempdir();
    let base_path = directory.path().join("base.c4g");
    let override_path = directory.path().join("override.c4g");
    fs::create_dir(&base_path).test_value();
    fs::create_dir(&override_path).test_value();

    for stem in [
        "Player",
        "Flag",
        "Crew",
        "Score",
        "Wealth",
        "Rank",
        "Captain",
        "Fire",
        "Menu",
        "UpperBoard",
        "Logo",
        "Construction",
        "Energy",
        "Magic",
        "Arrow",
        "Exit",
        "Hand",
        "Build",
        "EnergyBars",
        "SelectMark",
        "Control",
        "Gamepad",
        "Background",
        "Options",
        "Cursor",
    ] {
        write_preview_png(&base_path.join(format!("{stem}.png")), [9, 8, 7, 255]);
    }
    fs::write(base_path.join("C4.pal"), vec![0_u8; GamePalette::BYTE_LEN]).test_value();
    let cached_cursors = Arc::new(CursorAtlas::new(vec![
        Some(ImageData::new(
            1,
            1,
            vec![1, 2, 3, 255]
        ));
        8
    ]));

    let missing = resolve_game_graphics_resources(
        &[],
        &Group::open(&base_path).test_value(),
        Some(Arc::clone(&cached_cursors)),
        false,
    )
    .err()
    .test_value();
    main_assert_eq!(missing.to_string() => "failed to load game graphics resource `Liquid`");
    main_assert_eq!(FrontendAssets::liquid_animation_issue(&missing) => Some(ClassicGuiBootstrapIssue::missing("Liquid")));

    write_preview_image(
        &base_path.join("Liquid.bmp"),
        [10, 20, 30, 255],
        image::ImageFormat::Bmp,
    );
    fs::write(override_path.join("Liquid.png"), b"not a png").test_value();
    let malformed_registration = [LoaderGroupRegistration {
        priority: 200,
        registration_order: 0,
        group: Group::open(&override_path).test_value(),
    }];
    let malformed = resolve_game_graphics_resources(
        &malformed_registration,
        &Group::open(&base_path).test_value(),
        Some(Arc::clone(&cached_cursors)),
        false,
    )
    .err()
    .test_value();
    main_assert_eq!(malformed.to_string() => "failed to load game graphics resource `Liquid`");
    main_assert!(format!("{malformed:#}").contains("Liquid.png"));
    let malformed_issue = FrontendAssets::liquid_animation_issue(&malformed).test_value();
    main_assert!(matches!(&malformed_issue, ClassicGuiBootstrapIssue {resource: "Liquid", defect: ClassicGuiBootstrapDefect::Malformed { .. },}));
    let mut startup_assets = synthetic_classic_test_assets();
    startup_assets.liquid_animation_issue = Some(malformed_issue.clone());
    main_assert_eq!(
        startup_assets
            .require_classic_global_gui_bootstrap_resources(&HashMap::new())
            .expect_err("startup must reject a malformed selected Liquid resource") =>
        ClassicParityBoundary::GlobalGuiBootstrapResources {
            issues: vec![malformed_issue],
        }
    );

    write_preview_png(&override_path.join("Liquid.png"), [170, 180, 190, 255]);
    let valid_registration = [LoaderGroupRegistration {
        priority: 200,
        registration_order: 0,
        group: Group::open(&override_path).test_value(),
    }];
    let disabled = resolve_game_graphics_resources(
        &valid_registration,
        &Group::open(&base_path).test_value(),
        Some(Arc::clone(&cached_cursors)),
        false,
    )
    .test_value();
    main_assert!(disabled.liquid_animation.is_none());

    let enabled = resolve_game_graphics_resources(
        &valid_registration,
        &Group::open(&base_path).test_value(),
        Some(cached_cursors),
        true,
    )
    .test_value();
    main_assert_eq!(enabled.liquid_animation.as_deref().expect("enabled Liquid animation").pixels() => [170, 180, 190, 255]);
}

#[test]
fn runtime_client_list_renders_with_the_classic_gui_resource_set() {
    let mut app = new_classic_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0));
}

#[test]
fn load_frontend_scenarios_discovers_install_entries() {
    let install_dir = tempdir();

    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let scenario_dir = install_dir.path().join("Scenarios");
    let alpha_dir = scenario_dir.join("Alpha.c4s");
    fs::create_dir_all(&alpha_dir).test_value();
    fs::write(
        alpha_dir.join("Scenario.json"),
        br#"{"name":"Alpha Mission"}"#,
    )
    .test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    main_assert_eq!(scenarios.len() => 1, "expected discovered scenario without fallback");
    let scenario = &scenarios[0];
    main_assert_eq!(scenario.identifier => "Alpha.c4s");
    main_assert_eq!(scenario.title => "Alpha Mission");
    main_assert!(scenario.is_playable);
    main_assert_eq!(scenario.path.as_ref().and_then(|path| path.file_name()).and_then(|name| name.to_str()) => Some("Alpha.c4s"));

    reset_cached_app_paths();
}

#[test]
fn load_frontend_scenarios_discovers_repository_content() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let install_root = manifest_dir.parent().and_then(Path::parent).test_value();

    let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(install_root))]);
    let scenarios = load_frontend_scenarios();

    main_assert!(scenarios.iter().any(|scenario| scenario.identifier != "rust_sandbox"), "expected repository content scenarios to be discoverable");

    reset_cached_app_paths();
}

#[test]
fn load_frontend_scenarios_prefers_user_over_install() {
    reset_cached_app_paths();

    let install_dir = tempdir();

    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let install_scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
    fs::create_dir_all(&install_scenario_dir).test_value();
    fs::write(
        install_scenario_dir.join("Scenario.json"),
        br#"{"name":"System Alpha"}"#,
    )
    .test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let user_scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
    fs::create_dir_all(&user_scenario_dir).test_value();
    fs::write(
        user_scenario_dir.join("Scenario.json"),
        br#"{"name":"User Alpha"}"#,
    )
    .test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    main_assert_eq!(scenarios.len() => 1, "duplicate scenario should be merged");
    let scenario = &scenarios[0];
    main_assert_eq!(scenario.identifier => "Alpha.c4s");
    main_assert_eq!(scenario.title => "User Alpha", "user scenario should override install variant");
    let path = scenario.path.test_ref();
    main_assert!(path.starts_with(&user_dir), "expected scenario path to point at user overrides");

    reset_cached_app_paths();
}

#[test]
fn load_frontend_scenarios_fills_missing_preview_from_install() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();

    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let install_scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
    fs::create_dir_all(&install_scenario_dir).test_value();
    fs::write(
        install_scenario_dir.join("Scenario.json"),
        br#"{"name":"Install Alpha"}"#,
    )
    .test_value();
    write_preview_png(
        &install_scenario_dir.join("Title.png"),
        [0x10, 0x20, 0x30, 0x40],
    );

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let user_scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
    fs::create_dir_all(&user_scenario_dir).test_value();
    fs::write(
        user_scenario_dir.join("Scenario.json"),
        br#"{"name":"User Alpha"}"#,
    )
    .test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    main_assert_eq!(scenarios.len() => 1, "duplicate scenario should be merged");
    let scenario = &scenarios[0];
    main_assert_eq!(scenario.title => "User Alpha");
    let preview = scenario.preview.test_ref();
    main_assert_eq!(preview.width() => 1);
    main_assert_eq!(preview.height() => 1);
    main_assert_eq!(preview.pixels() => &[0x10, 0x20, 0x30, 0x40]);

    reset_cached_app_paths();
}

#[test]
fn load_frontend_scenarios_merges_folder_children_across_roots() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();

    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let install_folder = install_dir.path().join("Scenarios").join("Worlds.c4f");
    fs::create_dir_all(&install_folder).test_value();
    fs::write(install_folder.join("Folder.txt"), "Title=Worlds\n").test_value();
    let install_scenario = install_folder.join("Alpha.c4s");
    fs::create_dir_all(&install_scenario).test_value();
    fs::write(
        install_scenario.join("Scenario.json"),
        br#"{"name":"Alpha Install"}"#,
    )
    .test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let user_folder = user_dir.join("Scenarios").join("Worlds.c4f");
    fs::create_dir_all(&user_folder).test_value();
    fs::write(user_folder.join("Folder.txt"), "Title=Worlds\n").test_value();
    let user_scenario = user_folder.join("Beta.c4s");
    fs::create_dir_all(&user_scenario).test_value();
    fs::write(
        user_scenario.join("Scenario.json"),
        br#"{"name":"Beta User"}"#,
    )
    .test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    main_assert_eq!(scenarios.len() => 1, "duplicate folders should merge instead of duplicating entries");
    let folder = &scenarios[0];
    main_assert_eq!(folder.identifier => "Worlds.c4f");
    main_assert!(matches!(folder.kind, ScenarioKind::Folder), "expected merged entry to remain a folder");
    main_assert_eq!(folder.children.len() => 2, "merged folder should expose children from all roots");
    let identifiers: Vec<_> = folder
        .children
        .iter()
        .map(|child| child.identifier.as_str())
        .collect();
    main_assert_eq!(identifiers => vec!["Worlds.c4f/Alpha.c4s", "Worlds.c4f/Beta.c4s"], "children should be sorted deterministically");
    let user_entry = folder
        .children
        .iter()
        .find(|child| child.identifier == "Worlds.c4f/Beta.c4s")
        .test_value();
    main_assert_eq!(user_entry.title => "Beta User");
    main_assert!(user_entry.path.as_ref().map(|path| path.starts_with(&user_dir)).unwrap_or(false), "user scenario should retain user path");
    let install_entry = folder
        .children
        .iter()
        .find(|child| child.identifier == "Worlds.c4f/Alpha.c4s")
        .test_value();
    main_assert_eq!(install_entry.title => "Alpha Install");
    main_assert!(install_entry.path.as_ref().map(|path| path.starts_with(&install_dir)).unwrap_or(false), "install scenario should retain install path");

    reset_cached_app_paths();
}

#[test]
fn scenario_roots_deduplicates_case_insensitive_variants() {
    reset_cached_app_paths();

    let install_dir = tempdir();

    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let install_scenarios = install_dir.path().join("Scenarios");
    fs::create_dir_all(&install_scenarios).test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let paths = test_app_paths();
    let roots = scenario_roots(&paths);

    let expected_key = scenario_root_key(&install_scenarios);
    let duplicate_count = roots
        .iter()
        .map(|root| scenario_root_key(&root.path))
        .filter(|key| key == &expected_key)
        .count();

    main_assert_eq!(duplicate_count => 1, "install scenarios path should appear once despite case variants");

    reset_cached_app_paths();
}

#[test]
fn start_real_scenario_loads_from_disk() {
    clonk_logging::init();

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
                            "crew_member": true
                        }
                    ]
                }
                "#,
    )
    .test_value();
    fs::write(scripts_dir.join("mover.aul"), walker_script()).test_value();

    let (_guard, paths) = exact_loader_test_paths(&user_dir, None);

    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();

    let scenario = app.scensel.catalog.get("Alpha.c4s").cloned().test_value();
    main_assert_eq!(scenario.title => "Alpha Mission");

    let frontend_music = app
        .test_audio_ref()
        .system
        .load_music(&silent_pcm_wav(5_000))
        .test_value();
    app.test_audio_ref()
        .system
        .play_music(&frontend_music, true)
        .test_value();

    app.start_scenario(scenario).test_value();
    main_assert!(app.test_audio_ref().system.music_is_playing(), "scenario initialization must fade rather than halt frontend music");
    main_assert!(app.sound.resume_frontend_after_fade);
    wait_for_running(&mut app);

    main_assert!(matches!(app.mode, AppMode::Running), "mode should be Running");
    main_assert_eq!(app.scenario_label => "Alpha Mission");
    main_assert_eq!(app.fallback_ground => 72);
    main_assert!(app.snapshot.objects.iter().any(|object| object.definition_id == "Mover"), "expected spawned Mover object");
    main_assert!(app.focus_id.is_some(), "expected focus to be assigned for crew member");
    main_assert_eq!(
        app.active_scenario
            .as_ref()
            .and_then(|active| active.path.as_ref())
            .map(|path| path.as_path()) =>
        Some(scenario_dir.as_path()),
        "active scenario should track disk path"
    );
}

#[test]
fn install_definition_resolver_prefers_global_pack_before_folder_local_collision() {
    fn write_definition(root: &Path, directory: &str, id: &str, value: i32) {
        let definition = root.join(directory);
        fs::create_dir_all(&definition).test_value();
        fs::write(
            definition.join("DefCore.txt"),
            format!("[DefCore]\nid={id}\nName={id}\nCategory=0\nValue={value}\n"),
        )
        .test_value();
        write_test_definition_graphics(&definition);
    }

    let dir = tempdir();
    let content = dir.path().join("content");
    let global = content.join("Objects.c4d");
    let family = content.join("Tutorial.c4f");
    let local = family.join("Objects.c4d");
    let scenario = family.join("Tutorial01.c4s");
    write_definition(&global, "Global.c4d", "GLOB", 1);
    write_definition(&global, "Shared.c4d", "SAME", 1);
    let global_graphics = global.join("Graphics.c4g");
    fs::create_dir_all(&global_graphics).test_value();
    write_preview_png(
        &global_graphics.join("DefinitionSky.png"),
        [0x12, 0x34, 0x56, 0xff],
    );
    write_definition(&local, "Local.c4d", "LOCL", 2);
    write_definition(&local, "Shared.c4d", "SAME", 2);
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Scenario.txt"), "[Head]\nTitle=Collision\n\n[Definitions]\nDefinition1=Objects.c4d\n\n[Landscape]\nSky=DefinitionSky\n").test_value();

    let scenario_group = Group::open(&scenario).test_value();
    let resolver = InstallDefinitionResolver::new(None);
    let groups = resolver
        .resolve_definition_groups(&scenario_group, "Objects.c4d")
        .test_value();
    let roots = groups
        .iter()
        .map(|group| group.root().to_path_buf())
        .collect::<Vec<_>>();

    main_assert_eq!(
            roots.as_slice() =>
            std::slice::from_ref(&global),
            "the resolver returns the one explicit global resource; InitDefs adds folder-local resources separately"
        );

    let loaded = Scenario::load_from_path_with(&scenario, &resolver).test_value();
    main_assert_eq!(loaded.definition_resource_paths() => [global.clone(), family.clone()]);
    main_assert_eq!(
        loaded
            .definition_root_groups()
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect::<Vec<_>>() =>
        [global, family],
        "folder-local definitions are appended to C++'s final NRT_Definitions vector"
    );
    main_assert_eq!(
        &loaded
            .sky()
            .and_then(|sky| sky.surface.as_ref())
            .expect("definition-pack SkyDef surface")
            .pixels()[..4] =>
        &[0x12, 0x34, 0x56, 0xff],
        "the retained definition root participates in the live graphics chain"
    );
    let mut engine = Engine::new();
    loaded.apply(&mut engine).test_value();
    main_assert!(engine.definition_ids().any(|id| id == "GLOB"));
    main_assert!(engine.definition_ids().any(|id| id == "LOCL"));
    main_assert_eq!(engine.definition_value("SAME") => Some(2), "the later folder-local pass overloads the explicit global pack");
}

#[test]
fn install_definition_resolver_falls_back_to_the_folder_local_pack() {
    // C4Game::InitDefs appends every `.c4f` ancestor holding definitions to the
    // same NRT_Definitions vector the explicit entries populate
    // (C4Game.cpp:210-213, FoldersWithLocalsDefs :3961-3994), so a pack nested
    // beside its scenarios is already a definition source. Failing the explicit
    // lookup for that same pack is what forces every mod to be unpacked into
    // the data root; resolve it from the folder chain instead.
    clonk_logging::init();
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let family = planet_dir.join("Mods.c4f");
    let nested = family.join("OnlyNested.c4d").join("Nested.c4d");
    fs::create_dir_all(&nested).test_value();
    fs::write(
        nested.join("DefCore.txt"),
        "[DefCore]\nid=NEST\nName=NEST\nCategory=0\nValue=7\n",
    )
    .test_value();

    let scenario_dir = family.join("Alpha.c4s");
    fs::create_dir_all(&scenario_dir).test_value();
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Nested\n\n[Definitions]\nDefinition1=OnlyNested.c4d\n",
    )
    .test_value();
    let scenario_group = Group::open(&scenario_dir).test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let paths = cached_app_paths().test_value();
    let resolver = InstallDefinitionResolver::new(Some(paths));
    let groups = resolver
        .resolve_definition_groups(&scenario_group, "OnlyNested.c4d")
        .test_value();
    main_assert_eq!(
        groups
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect::<Vec<_>>() =>
        vec![family.join("OnlyNested.c4d")],
        "a pack that exists only inside the scenario's .c4f resolves from there"
    );

    reset_cached_app_paths();
}

#[test]
fn install_definition_resolver_falls_back_to_the_pack_beside_the_scenario_pack() {
    // A downloaded mod ships a definition pack and the scenario pack that
    // names it as siblings — `Definition1=Epic.c4d` next to `Epic.c4f` — and
    // is dropped whole into a subdirectory of the data root. C++ opens the
    // explicit name relative to the working directory only
    // (C4GameParameters.cpp:199-210), and FoldersWithLocalsDefs scans just the
    // `.c4f` path components (C4Game.cpp:3961-3994), so neither reaches a
    // sibling one directory down and the scenario refuses to start.
    //
    // Extending the existing folder-chain fallback past the `.c4f` ancestors
    // to the enclosing data-root directories costs no parity: the data root is
    // still searched first, so every name C++ resolves keeps the exact pack it
    // has, and only a name that resolves nowhere — a scenario C++ would refuse
    // to start at all — reaches the sibling.
    clonk_logging::init();
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let mods = planet_dir.join("mods");
    let beside = mods.join("Epic.c4d");
    fs::create_dir_all(&beside).test_value();
    fs::write(
        beside.join("DefCore.txt"),
        "[DefCore]\nid=EPIC\nName=EPIC\nCategory=0\nValue=7\n",
    )
    .test_value();

    let family = mods.join("Epic.c4f");
    let scenario_dir = family.join("Test map.c4s");
    fs::create_dir_all(&scenario_dir).test_value();
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=Test map\n\n[Definitions]\nDefinition1=Epic.c4d\n",
    )
    .test_value();
    let scenario_group = Group::open(&scenario_dir).test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let paths = cached_app_paths().test_value();
    let resolver = InstallDefinitionResolver::new(Some(paths));
    let groups = resolver
        .resolve_definition_groups(&scenario_group, "Epic.c4d")
        .test_value();
    main_assert_eq!(
        groups
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect::<Vec<_>>() =>
        vec![beside],
        "a pack shipped beside the scenario's .c4f resolves from the mod directory"
    );

    reset_cached_app_paths();
}

fn assert_parent_resource_order(scenario: &Group, inner: &Path, outer: &Path) {
    let resolver = InstallDefinitionResolver::new(None);
    let graphics = resolver.resolve_graphics_groups(scenario).test_value();
    main_assert_eq!(graphics.iter().map(|group| group.root().to_path_buf()).collect::<Vec<_>>() => [inner.join("Graphics.c4g"), outer.join("Graphics.c4g")]);
    main_assert_eq!(graphics[0].read_file("Source.txt").expect("inner graphic") => b"inner graphics");
    main_assert_eq!(graphics[1].read_file("Source.txt").expect("outer graphic") => b"outer graphics");

    let materials = resolver.resolve_material_groups(scenario).test_value();
    main_assert_eq!(materials.iter().map(|group| group.root().to_path_buf()).collect::<Vec<_>>() => [inner.join("Material.c4g"), outer.join("Material.c4g")]);
    main_assert_eq!(materials[0].read_file("Source.txt").expect("inner material") => b"inner materials");
    main_assert_eq!(materials[1].read_file("Source.txt").expect("outer material") => b"outer materials");
}

#[test]
fn install_definition_resolver_opens_packed_parent_resource_chain() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();
    let dir = tempdir();
    let inner_png_path = dir.path().join("inner.png");
    let outer_png_path = dir.path().join("outer.png");
    write_preview_png(&inner_png_path, [1, 2, 3, 255]);
    write_preview_png(&outer_png_path, [9, 8, 7, 255]);
    let inner_png = fs::read(inner_png_path).test_value();
    let outer_png = fs::read(outer_png_path).test_value();
    let outer_graphics = packed_test_group(&[
        ("Source.txt", false, b"outer graphics"),
        ("Priority.png", false, outer_png.as_slice()),
    ]);
    let outer_materials = packed_test_group(&[
        ("Source.txt", false, b"outer materials"),
        ("TexMap.txt", false, b"1=Earth-Rough\n"),
        ("Earth.c4m", false, b"[Material]\nName=Earth\nDensity=50\n"),
        ("Rough.png", false, outer_png.as_slice()),
    ]);
    let inner_graphics = packed_test_group(&[
        ("Source.txt", false, b"inner graphics"),
        ("Priority.png", false, inner_png.as_slice()),
    ]);
    let inner_materials = packed_test_group(&[
        ("Source.txt", false, b"inner materials"),
        (
            "TexMap.txt",
            false,
            b"OverloadMaterials\nOverloadTextures\n1=Earth-Rough\n",
        ),
        ("Earth.c4m", false, b"[Material]\nName=Earth\nDensity=100\n"),
        ("Rough.png", false, inner_png.as_slice()),
    ]);
    let definition = packed_test_group(&[
        (
            "DefCore.txt",
            false,
            b"[DefCore]\nid=GOOD\nName=Good\nCategory=0\n",
        ),
        ("Script.c", false, b"// packed definition\n"),
        ("Graphics.png", false, inner_png.as_slice()),
    ]);
    let scenario = packed_test_group(&[
                (
                    "Scenario.txt",
                    false,
                    b"[Head]\nTitle=Packed parents\n\n[Definitions]\nLocalOnly=1\n\n[Landscape]\nSky=Priority\n",
                ),
                ("Good.c4d", true, definition.as_slice()),
            ]);
    let inner = packed_test_group(&[
        ("Graphics.c4g", true, inner_graphics.as_slice()),
        ("Material.c4g", true, inner_materials.as_slice()),
        ("Scen.c4s", true, scenario.as_slice()),
    ]);
    let outer = packed_test_file_group(&[
        ("Graphics.c4g", true, outer_graphics.as_slice()),
        ("Material.c4g", true, outer_materials.as_slice()),
        ("Inner.c4f", true, inner.as_slice()),
    ]);
    let outer_path = dir.path().join("Outer.c4f");
    fs::write(&outer_path, outer).test_value();
    let scenario_group =
        open_group_path_for_folder_map(&outer_path.join("Inner.c4f/Scen.c4s")).test_value();

    assert_parent_resource_order(&scenario_group, &outer_path.join("Inner.c4f"), &outer_path);
    let scenario_path = outer_path.join("Inner.c4f/Scen.c4s");
    preflight_offline_startup(&scenario_path).test_value();
    let loaded = load_scenario_with_definition_load_and_startup_player_count(
        &scenario_path,
        &InstallDefinitionResolver::new(None),
        &["US".to_string()],
        &ScenarioDefinitionLoad::Seed {
            modules: Vec::new(),
            definition_root: None,
        },
        0,
    )
    .test_value();
    main_assert_eq!(&loaded.sky().and_then(|sky| sky.surface.as_ref()).expect("inner parent sky").pixels()[..4] => &[1, 2, 3, 255]);
    main_assert_eq!(
        load_material_render_info(&scenario_path, None).get("earth") =>
        Some(
            &clonk_frontend::MaterialRenderInfo::new([0; 9], [0; 6], None, 0, 100)
                .with_placement(70)
        )
    );
    main_assert_eq!(
        load_scenario_material_textures(&scenario_path, None)
            .get("rough")
            .expect("inner parent material texture")
            .surface32_image()
            .expect("rough texture is PNG-backed")
            .pixels() =>
        &[1, 2, 3, 255]
    );
    reset_cached_app_paths();
}

#[test]
fn install_definition_resolver_keeps_unpacked_parent_resource_chain() {
    let dir = tempdir();
    let outer = dir.path().join("Outer.c4f");
    let inner = outer.join("Inner.c4f");
    let scenario = inner.join("Scen.c4s");
    for (parent, graphic, material) in [
        (
            &outer,
            b"outer graphics".as_slice(),
            b"outer materials".as_slice(),
        ),
        (
            &inner,
            b"inner graphics".as_slice(),
            b"inner materials".as_slice(),
        ),
    ] {
        fs::create_dir_all(parent.join("Graphics.c4g")).test_value();
        fs::create_dir_all(parent.join("Material.c4g")).test_value();
        fs::write(parent.join("Graphics.c4g/Source.txt"), graphic).test_value();
        fs::write(parent.join("Material.c4g/Source.txt"), material).test_value();
    }
    fs::create_dir_all(&scenario).test_value();
    let scenario_group = Group::open(&scenario).test_value();

    assert_parent_resource_order(&scenario_group, &inner, &outer);
}

#[test]
fn install_definition_resolver_prioritizes_scenario_graphics_over_folder() {
    let dir = tempdir();
    let family = dir.path().join("Tutorial.c4f");
    let scenario = family.join("Tutorial01.c4s");
    let scenario_graphics = scenario.join("Graphics.c4g");
    let folder_graphics = family.join("Graphics.c4g");
    fs::create_dir_all(&scenario_graphics).test_value();
    fs::create_dir_all(&folder_graphics).test_value();
    fs::write(scenario_graphics.join("Shared.png"), b"scenario").test_value();
    fs::write(folder_graphics.join("Shared.png"), b"folder").test_value();

    let scenario_group = Group::open(&scenario).test_value();
    let graphics = InstallDefinitionResolver::new(None)
        .resolve_graphics_groups(&scenario_group)
        .test_value();

    main_assert_eq!(graphics.len() => 2);
    main_assert_eq!(graphics[0].root() => scenario_graphics.as_path());
    main_assert_eq!(graphics[1].root() => folder_graphics.as_path());
    main_assert_eq!(graphics[0].read_file("Shared.png").expect("local graphic") => b"scenario");
}

#[test]
fn definition_pack_gui_sheet_wins_the_active_override_selection() {
    let _env_lock = crate::tests::env_lock().lock();
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let scenario = content.join("Scenario.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(
        scenario.join("Scenario.txt"),
        "[Head]\nTitle=GUI Override\n",
    )
    .test_value();

    let definition = content.join("Objects.c4d");
    let definition_graphics = definition.join("Graphics.c4g");
    fs::create_dir_all(&definition_graphics).test_value();
    write_preview_png(
        &definition_graphics.join("GUIBigArrows.png"),
        [0x12, 0x34, 0x56, 0xff],
    );
    let base_graphics = root.path().join("planet/Graphics.c4g");
    fs::create_dir_all(&base_graphics).test_value();
    write_preview_png(
        &base_graphics.join("GUIBigArrows.png"),
        [0xaa, 0xbb, 0xcc, 0xff],
    );

    let scenario_group = Group::open(&scenario).test_value();
    let head = ScenarioLoaderHead::load_from_group(&scenario_group).test_value();
    let registrations = definition_graphics_source_registrations(
        &head,
        &scenario_group,
        &ScenarioDefinitionLoad::Fixed {
            modules: vec!["Objects.c4d".to_string()],
            definition_root: None,
        },
        &paths,
        0,
    )
    .test_value();
    main_assert_eq!(registrations.len() => 1);
    main_assert_eq!(registrations[0].priority => 1);
    main_assert_eq!(registrations[0].group.root() => definition.as_path());

    let resolution = resolve_classic_global_gui_sheet_overrides(
        &registrations,
        &Group::open(&base_graphics).test_value(),
    );
    main_assert!(resolution.failures.is_empty(), "a decodable definition-pack sheet must not fail: {:?}", resolution.failures);
    let sheet = resolution
        .overrides
        .iter()
        .find(|sheet| sheet.stem == "GUIBigArrows")
        .test_value();
    main_assert_eq!(sheet.canonical_name => "GUIBigArrows.png");
    main_assert_eq!(sheet.source => format!("{}:GUIBigArrows.png", definition_graphics.display()));
    main_assert_eq!(&sheet.image.pixels()[..4] => &[0x12, 0x34, 0x56, 0xff], "the applied override carries the winning group's decoded pixels");
}

#[test]
fn install_definition_resolver_handles_case_insensitive_paths() {
    clonk_logging::init();
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let objects_dir = planet_dir.join("objects.ocd").join("clonk.c4d");
    fs::create_dir_all(&objects_dir).test_value();
    fs::write(
        objects_dir.join("DefCore.txt"),
        "[DefCore]\nid=CLNK\nName=Clonk\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
    )
    .test_value();
    fs::write(objects_dir.join("Script.c"), walker_script()).test_value();

    let scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
    fs::create_dir_all(&scenario_dir).test_value();
    let local_shadow = scenario_dir.join("objects.ocd").join("clonk.c4d");
    fs::create_dir_all(&local_shadow).test_value();
    fs::write(
        local_shadow.join("DefCore.txt"),
        "[DefCore]\nid=LOCL\nName=Local shadow\nCategory=1\n",
    )
    .test_value();
    let scenario_group = Group::open(&scenario_dir).test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let paths = cached_app_paths().test_value();
    let resolver = InstallDefinitionResolver::new(Some(paths.clone()));
    let groups = resolver
        .resolve_definition_groups(&scenario_group, "Objects.ocd\\Clonk.c4d")
        .test_value();
    let first_root = groups.first().test_value().root();
    main_assert!(
        first_root
            .to_string_lossy()
            .eq_ignore_ascii_case(&objects_dir.to_string_lossy()),
        "ExePath definitions precede scenario/folder-local collisions: {}",
        first_root.display()
    );
    main_assert!(!first_root.starts_with(&scenario_dir));
    let found_definition = groups.iter().any(|group| {
        group
            .root()
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with("clonk.c4d")
    });
    main_assert!(found_definition, "expected to locate definition group");

    let absolute_groups = resolver
        .resolve_definition_groups(&scenario_group, &objects_dir.to_string_lossy())
        .test_value();
    main_assert_eq!(absolute_groups.len() => 1);
    main_assert_eq!(absolute_groups[0].root() => objects_dir.as_path());

    let local_only = scenario_dir.join("OnlyLocal.c4d");
    fs::create_dir_all(&local_only).test_value();
    main_assert!(matches!(
        resolver.resolve_definition_groups(&scenario_group, "OnlyLocal.c4d"),
        Err(ScenarioError::LegacyDefinitionNotFound { path }) if path == "OnlyLocal.c4d"
    ));

    reset_cached_app_paths();
}

#[test]
fn load_install_definitions_discovers_mixed_case_objects_group() {
    clonk_logging::init();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let objects_dir = planet_dir.join("objects.c4d").join("clonk.c4d");
    fs::create_dir_all(&objects_dir).test_value();
    fs::write(
        objects_dir.join("DefCore.txt"),
        "[DefCore]\nid=Clonk\nName=Invalid\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
    )
    .test_value();
    fs::write(objects_dir.join("Script.c"), walker_script()).test_value();
    let long_id = planet_dir.join("objects.c4d").join("clone.c4d");
    fs::create_dir_all(&long_id).test_value();
    fs::write(
        long_id.join("DefCore.txt"),
        "[DefCore]\nid=WIPFEX\nName=Wipf\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
    )
    .test_value();
    fs::write(long_id.join("Script.c"), walker_script()).test_value();
    write_test_definition_graphics(&long_id);
    let zero_id = planet_dir.join("objects.c4d").join("zero.c4d");
    fs::create_dir_all(&zero_id).test_value();
    fs::write(
        zero_id.join("DefCore.txt"),
        "[DefCore]\nid=0000\nName=Zero\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
    )
    .test_value();
    fs::write(zero_id.join("ActMap.txt"), "not an action map").test_value();
    let canonical = planet_dir.join("objects.c4d").join("canonical.c4d");
    fs::create_dir_all(&canonical).test_value();
    fs::write(
        canonical.join("DefCore.txt"),
        "[DefCore]\nid=CLNK\nName=Clonk\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
    )
    .test_value();
    fs::write(canonical.join("Script.c"), walker_script()).test_value();
    write_test_definition_graphics(&canonical);

    let missing_graphics = planet_dir.join("objects.c4d").join("missing.c4d");
    fs::create_dir_all(&missing_graphics).test_value();
    fs::write(
        missing_graphics.join("DefCore.txt"),
        "[DefCore]\nid=MISS\nName=Missing graphics\nCategory=1\n",
    )
    .test_value();

    let old_gfx = planet_dir.join("objects.c4d").join("oldgfx.c4d");
    fs::create_dir_all(&old_gfx).test_value();
    fs::write(
        old_gfx.join("DefCore.txt"),
        "[DefCore]\nid=OLDG\nName=Old graphics\nCategory=1\nNeededGfxMode=2\n",
    )
    .test_value();
    write_test_definition_graphics(&old_gfx);

    let particle = planet_dir.join("objects.c4d").join("particle.c4d");
    fs::create_dir_all(&particle).test_value();
    fs::write(
        particle.join("DefCore.txt"),
        "[DefCore]\nid=PART\nName=Particle\nCategory=1\n",
    )
    .test_value();
    write_test_definition_graphics(&particle);
    fs::write(
        particle.join("Particle.txt"),
        "[Particle]\n\
                 Name=InstallParticle\n\
                 InitFn=StdInit\n\
                 ExecFn=StdExec\n\
                 DrawFn=Std\n\
                 Face=0,0,1,1,0,0\n",
    )
    .test_value();
    let particle_override = particle.join("Override.c4d");
    fs::create_dir_all(&particle_override).test_value();
    fs::write(
                particle_override.join("Particle.txt"),
                "[Particle]\nName=InstallParticle\nInitFn=StdInit\nExecFn=StdExec\nDrawFn=Std\nFace=0,0,1,1,0,0\n",
            ).test_value();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 8, 7, 255]))
        .save(particle_override.join("Graphics.png"))
        .test_value();
    let invalid_override = particle_override.join("Invalid.c4d");
    fs::create_dir_all(&invalid_override).test_value();
    fs::write(
                invalid_override.join("Particle.txt"),
                "[Particle]\nName=InstallParticle\nInitFn=StdInit\nExecFn=StdExec\nDrawFn=MissingDrawProc\nFace=0,0,1,1,0,0\n",
            ).test_value();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([99, 98, 97, 255]))
        .save(invalid_override.join("Graphics.png"))
        .test_value();

    let bad_overlay = planet_dir.join("objects.c4d").join("overlay.c4d");
    fs::create_dir_all(&bad_overlay).test_value();
    fs::write(
        bad_overlay.join("DefCore.txt"),
        "[DefCore]\nid=OVLY\nName=Bad overlay\nCategory=1\nColorByOwner=1\n",
    )
    .test_value();
    write_test_definition_graphics(&bad_overlay);
    image::RgbaImage::from_pixel(2, 1, image::Rgba([32, 32, 32, 255]))
        .save(bad_overlay.join("Overlay.png"))
        .test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let paths = cached_app_paths().test_value();
    let mut engine = Engine::new();
    let spawn = load_install_definitions(&mut engine, &paths, None).test_value();
    main_assert_eq!(spawn.as_deref() => Some("CLNK"));
    main_assert!(engine.definition_ids().any(|id| id == "CLNK"), "expected Clonk definition to be registered");
    main_assert!(engine.definition_ids().any(|id| id == "WIPF"));
    main_assert!(!engine.definition_ids().any(|id| id == "Clon"));
    for rejected in ["MISS", "OLDG", "OVLY", "PART"] {
        main_assert!(!engine.definition_ids().any(|id| id == rejected));
    }
    let particle = engine
        .particle_system()
        .get_def("InstallParticle")
        .test_value();
    main_assert_eq!(particle.length => 1);
    main_assert_eq!(particle.graphics.as_ref().unwrap().image.pixels() => [9, 8, 7, 255], "later valid overload wins and a later invalid overload preserves it");
    let particle_sprites = particle_sprite_map(&engine);
    main_assert_eq!(particle_sprites["InstallParticle"].image.pixels() => [9, 8, 7, 255], "frontend registry receives the final post-overload image");

    let objects_group = Group::open(planet_dir.join("objects.c4d")).test_value();
    main_assert!(find_definition_in_group(&objects_group, "Clon").expect("lowercase ID lookup skips").is_none());
    main_assert!(find_definition_in_group(&objects_group, "0000").expect("invalid lookup skips").is_none());
    for rejected in ["MISS", "OLDG", "OVLY", "PART"] {
        main_assert!(find_definition_in_group(&objects_group, rejected).expect("load-ladder rejection remains nonfatal").is_none());
    }
    main_assert_eq!(find_definition_in_group(&objects_group, "WIPF").expect("truncated lookup succeeds").expect("WIPF exists").core.id => "WIPF");

    reset_cached_app_paths();
}
