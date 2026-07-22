// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

    #[test]
    fn m10_l046_completion_matches_win32_and_gtk_function_layout() {
        let mut engine = Engine::new();
        assert_eq!(
            engine.install_global_scripts(&[(
                "CompletionGlobals.c".to_string(),
                "global func EngineProbe() { return true; }".to_string(),
            )]),
            1,
        );
        engine
            .install_scenario_script(
                "Scenario",
                "func ScenarioAlpha() { return true; }\n\
                 protected func ScenarioHidden() { return true; }\n\
                 global func ScenarioGlobal() { return true; }",
            )
            .expect("install completion scenario");

        let catalog = engine.console_script_completion_catalog();
        assert!(catalog.engine_functions.iter().any(|name| name == "Abs"));
        assert!(catalog
            .engine_functions
            .iter()
            .any(|name| name == "EngineProbe"));
        assert!(catalog
            .engine_functions
            .iter()
            .any(|name| name == "ScenarioGlobal"));
        assert!(!catalog
            .engine_functions
            .iter()
            .any(|name| name == "SetContactDensity"));
        for hidden in ["ScoreboardCol", "CastInt", "CastBool", "CastC4ID", "CastAny"] {
            assert!(!catalog.engine_functions.iter().any(|name| name == hidden));
        }
        assert_eq!(
            catalog.scenario_functions,
            ["ScenarioHidden".to_string(), "ScenarioAlpha".to_string()]
        );

        let win32 = developer_console_completion_entries(
            &catalog,
            DeveloperConsoleCompletionStyle::Win32,
        );
        let separator = win32
            .iter()
            .position(|entry| *entry == DeveloperConsoleCompletionEntry::Separator)
            .expect("Win32 inserts the scenario divider");
        assert_eq!(separator, catalog.scenario_functions.len());
        assert_eq!(
            &win32[..separator],
            &[
                DeveloperConsoleCompletionEntry::Function("ScenarioAlpha()".to_string()),
                DeveloperConsoleCompletionEntry::Function("ScenarioHidden()".to_string()),
            ]
        );
        assert!(win32[separator + 1..].iter().any(|entry| {
            entry
                == &DeveloperConsoleCompletionEntry::Function("EngineProbe()".to_string())
        }));

        let gtk = developer_console_completion_entries(
            &catalog,
            DeveloperConsoleCompletionStyle::Gtk,
        );
        assert!(!gtk
            .iter()
            .any(|entry| *entry == DeveloperConsoleCompletionEntry::Separator));
        assert_eq!(
            &gtk[gtk.len() - catalog.scenario_functions.len()..],
            &[
                DeveloperConsoleCompletionEntry::Function("ScenarioHidden".to_string()),
                DeveloperConsoleCompletionEntry::Function("ScenarioAlpha".to_string()),
            ]
        );
        assert!(gtk.iter().any(|entry| {
            entry == &DeveloperConsoleCompletionEntry::Function("EngineProbe".to_string())
        }));
    }

    #[test]
    fn m10_l046_nonhost_console_packet_uses_console_active_policy() {
        let packet = || {
            NetworkControl::Script(clonk_engine::ScriptControlData {
                target_object: clonk_engine::SCRIPT_SCOPE_CONSOLE,
                strictness: clonk_engine::ScriptStrictness::Strict3,
                script: LegacyCString::from_bytes(b"SetGravity(77)".to_vec())
                    .expect("fixture script has no NUL"),
                by_client: 7,
            })
        };

        let mut inactive = new_state_only_running_sandbox_app();
        let initial_gravity = inactive.engine.physics().gravity;
        inactive
            .apply_ready_controls(0, vec![packet()])
            .expect("inactive console rejects non-host script without failing the batch");
        assert_eq!(inactive.engine.physics().gravity, initial_gravity);

        let mut active = new_state_only_running_sandbox_app();
        active.console_mode = true;
        active
            .apply_ready_controls(0, vec![packet()])
            .expect("active console executes non-host script");
        assert_eq!(active.engine.physics().gravity, 77);
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
        assert!(!stuck.observe_at(25, started + BLOCKING_RESOURCE_STALL_TIMEOUT));
        assert!(stuck.observe_at(
            25,
            started + BLOCKING_RESOURCE_STALL_TIMEOUT + Duration::from_millis(1)
        ));

        let mut advancing = BlockingResourceWait::new_at(
            BlockingResourceScope::ClientStart,
            7,
            None,
            "Scenario".to_string(),
            25,
            started,
        );
        let changed_at = started + BLOCKING_RESOURCE_STALL_TIMEOUT - Duration::from_millis(1);
        assert!(!advancing.observe_at(26, changed_at));
        assert!(!advancing.observe_at(
            26,
            changed_at + BLOCKING_RESOURCE_STALL_TIMEOUT
        ));
        assert!(advancing.observe_at(
            26,
            changed_at + BLOCKING_RESOURCE_STALL_TIMEOUT + Duration::from_millis(1)
        ));
    }

    #[test]
    fn client_resource_timeout_closes_progress_and_shows_fatal_error_log() {
        let mut app = new_menu_app(800, 600);
        let core = clonk_engine::NetworkResourceCore {
            id: 7,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
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
        .expect("open progress dialog");

        app.poll_blocking_resource_wait_at(started + BLOCKING_RESOURCE_STALL_TIMEOUT)
            .expect("the exact deadline does not time out");
        assert!(app.blocking_resource_wait.is_some());
        app.poll_blocking_resource_wait_at(
            started + BLOCKING_RESOURCE_STALL_TIMEOUT + Duration::from_millis(1),
        )
        .expect("timeout returns to the startup network screen");

        assert!(app.blocking_resource_wait.is_none());
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.message_dialogs[0].state.caption(), "Error Log");
        assert_eq!(
            app.message_dialogs[0].state.message(),
            "Waiting for Scenario: Timeout!"
        );
        assert_eq!(
            app.message_dialogs[0].state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::ERROR
        );
    }

    #[test]
    fn l126_headless_client_join_tracks_slow_resource_then_cancel_aborts() {
        let mut app = new_menu_app(800, 600);
        let (manager, event_tx, _commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(client_network_settings()));

        let resource = |resource_type: clonk_network::HostResourceType, id, name: &[u8]| {
            clonk_engine::NetworkResourceCore {
                resource_type: resource_type as u8,
                id,
                loadable: true,
                filename: clonk_engine::LegacyCString::from_bytes(name.to_vec())
                    .expect("fixture filename is NUL-free"),
                ..Default::default()
            }
        };
        let host_config = clonk_network::HostConfig::default();
        let mut snapshot = host_config
            .initial_join_snapshot
            .expect("default host publishes JoinData");
        snapshot.parameters.scenario =
            resource(clonk_network::HostResourceType::Scenario, 70, b"Scenario.c4s");
        snapshot.dynamic = resource(clonk_network::HostResourceType::Dynamic, 71, b"Dynamic.c4s");
        snapshot.parameters.game_resources.clear();
        let mut reference_status = host_config.initial_status;
        reference_status.target_tick = -1;
        let go = clonk_network::NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 23,
        };
        event_tx
            .send(NetworkEvent::JoinData(clonk_network::JoinDataEnvelope {
                client_id: 7,
                start_control_tick: 23,
                status: reference_status,
                dynamic: snapshot.dynamic,
                parameters: snapshot.parameters,
            }))
            .expect("queue client JoinData");
        event_tx
            .send(NetworkEvent::StatusRequested(go))
            .expect("queue client GO request");
        app.process_network_events()
            .expect("open scenario resource wait");

        let progress = app
            .message_dialogs
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
            .expect("client start progress dialog");
        assert_eq!(progress.state.message(), "Waiting for Scenario...");
        assert_eq!(progress.state.progress(), Some(0));
        assert_eq!(
            progress.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::CANCEL
        );
        assert_eq!(
            progress.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::Standard(3)
        );
        assert_eq!(progress.state.focused_button(), None);
        assert_eq!(
            progress.state.button_label(
                clonk_frontend::message_dialog::MessageDialogButton::Cancel
            ),
            "Cancel"
        );

        for present_percent in [17, 63] {
            event_tx
                .send(NetworkEvent::ResourceProgress {
                    resource_id: 70,
                    present_percent,
                })
                .expect("advance slow scenario resource");
            app.update().expect("refresh scenario resource progress");
            assert_eq!(
                app.message_dialogs
                    .iter()
                    .find(|dialog| matches!(
                        dialog.continuation,
                        MessageDialogContinuation::BlockingResourceWait { .. }
                    ))
                    .and_then(|dialog| dialog.state.progress()),
                Some(present_percent)
            );
        }

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
            .expect("Cancel aborts the client resource wait");

        assert!(app.network.is_none());
        assert!(app.network_mode.is_none());
        assert!(app.pending_network_join_data.is_none());
        assert!(app.pending_client_start_status.is_none());
        assert!(app.blocking_resource_wait.is_none());
        assert!(app.admission_resources.resources.is_empty());
        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::NetworkGame);
        assert!(app.message_dialogs.iter().all(|dialog| !matches!(
            dialog.continuation,
            MessageDialogContinuation::BlockingResourceWait { .. }
        )));
        let [failure] = app.message_dialogs.as_slice() else {
            panic!("Cancel should report one startup-network failure");
        };
        assert_eq!(failure.state.caption(), "Error Log");
        assert_eq!(failure.state.message(), "Waiting for Scenario was aborted.");
    }

    #[test]
    fn player_resource_abort_releases_only_the_waiting_join() {
        let mut app = new_synthetic_running_sandbox_app();
        let (manager, _event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.engine.set_network_game(true);
        app.control_clients.register(0, true, false);
        let core = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 9,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            ..Default::default()
        };
        app.admission_resources.register_lobby_resource(&core);
        app.begin_blocking_resource_wait_at(
            BlockingResourceScope::PlayerJoin,
            core.id,
            Some(99),
            "player file for Ada".to_string(),
            Instant::now(),
        )
        .expect("open player progress dialog");

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
            .expect("Abort routes to the waiting JoinPlayer caller");

        assert!(app.blocking_resource_wait.is_none());
        assert_eq!(
            app.admission_resources.status(core.id),
            Some(&AdmissionResourceState::Loading { removed: false })
        );
        assert!(app
            .aborted_player_resource_joins
            .contains(&(core.id, 99)));
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
        assert!(pending_admission_resource(
            &mut app.admission_resources,
            &clients,
            &join(99),
            &app.aborted_player_resource_joins,
        )
        .is_none());
        assert_eq!(
            pending_admission_resource(
                &mut app.admission_resources,
                &clients,
                &join(100),
                &app.aborted_player_resource_joins,
            )
            .map(|pending| pending.info_id),
            Some(100),
            "a later caller still waits on the active backend transfer"
        );
        assert!(app.message_dialogs.is_empty());

        let player_path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../clonk-engine/tests/fixtures/embedded_player.c4p"
        ));
        app.admission_resources
            .mark_complete(core.id, player_path);
        app.apply_ready_controls(
            0,
            vec![
                NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        name: clonk_engine::LegacyCString::from_bytes(b"Ada".to_vec()).unwrap(),
                        id: 99,
                        ..Default::default()
                    }],
                    by_client: 1,
                    ..Default::default()
                }),
                join(99).pop().expect("resource join"),
            ],
        )
        .expect("consume the canceled join after transfer completion");
        assert!(!app
            .control_player_infos
            .get(99)
            .expect("player info was still applied")
            .is_joined());
        assert!(!app
            .engine
            .snapshot()
            .players
            .iter()
            .any(|player| player.player_info_id == 99));
        assert!(!app
            .aborted_player_resource_joins
            .contains(&(core.id, 99)));
    }

    #[test]
    fn failed_client_start_resource_aborts_instead_of_stalling_silently() {
        let mut app = new_menu_app(800, 600);
        let (manager, event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        let core = clonk_engine::NetworkResourceCore {
            id: 11,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
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
        .expect("open scenario progress dialog");
        event_tx
            .send(NetworkEvent::ResourceLoadFailed {
                resource_id: core.id,
            })
            .expect("fail scenario transfer");

        app.process_network_events()
            .expect("resource failure returns to startup");

        assert!(app.network.is_none());
        assert!(app.blocking_resource_wait.is_none());
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.message_dialogs[0].state.caption(), "Error Log");
        assert_eq!(
            app.message_dialogs[0].state.message(),
            "Unable to retrieve Scenario."
        );
    }

    #[test]
    fn l007_fresh_install_shutdown_persists_fullscreen_default() {
        let install = tempdir().expect("install root");
        let user_data = tempdir().expect("user data");
        fs::create_dir_all(install.path().join("planet")).expect("planet directory");
        fs::write(install.path().join("planet/System.c4g"), b"stub")
            .expect("system group stub");
        let config_file = user_data.path().join("custom/fresh.config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_CONFIG_FILE", None),
        ]);
        let paths = AppPaths::discover_with_config_file(Some(&config_file))
            .expect("discover fresh-install paths");
        paths.ensure_user_dirs().expect("prepare config parent");
        assert!(!config_file.exists());

        let mut display = DisplayOptions::load(Some(&paths));
        assert_eq!(display.mode, DisplayMode::Fullscreen);
        display.persist_if_dirty(&paths);

        let persisted = Config::load(&config_file).expect("load first-quit config");
        assert_eq!(
            persisted.get_in(Some("Graphics"), "DisplayMode"),
            Some("0")
        );

        fs::write(&config_file, "[General]\nSentinel=keep\n")
            .expect("write config without DisplayMode");
        let mut missing_key = DisplayOptions::load(Some(&paths));
        assert_eq!(missing_key.mode, DisplayMode::Fullscreen);
        missing_key.persist_if_dirty(&paths);
        let persisted = Config::load(&config_file).expect("reload missing-key config");
        assert_eq!(persisted.get_in(Some("General"), "Sentinel"), Some("keep"));
        assert_eq!(
            persisted.get_in(Some("Graphics"), "DisplayMode"),
            Some("0")
        );
    }

    #[test]
    fn fully_disabled_test_audio_skips_install_resource_discovery() {
        let audio = AudioContext::try_new(AudioOptions {
            sound_enabled: false,
            music_enabled: false,
            menu_music_enabled: false,
            menu_sound_enabled: false,
            ..AudioOptions::default()
        })
        .expect("silent audio context");

        assert!(audio.resolver.global.is_empty());
        assert!(audio.resolver.base_sample_loads.is_empty());
        assert!(audio.music_resolver.global.assets.is_empty());
        assert!(audio.music_resolver.extra.is_none());
    }

    #[test]
    fn install_walker_registers_defcoreless_c4d_sound_groups() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Objects.c4d");
        let pure_sounds = root.join("Potions.c4d");
        fs::create_dir_all(&pure_sounds).expect("create pure install sound group");
        fs::write(pure_sounds.join("Drink.wav"), silent_pcm_wav(20))
            .expect("write pure install sound");

        let group = Group::open(&root).expect("open install definition root");
        let mut engine = Engine::new();
        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.resolver = SoundResolver::empty();
        audio.refresh_sound_catalog();
        load_definitions_from_group(
            &mut engine,
            &group,
            Some(NonNull::from(&mut audio)),
            &mut HashSet::new(),
            &mut None,
        )
        .expect("walk install definition tree");

        assert_eq!(audio.available_sound_samples(), ["drink.wav"]);
        assert!(
            audio
                .ensure_sound_with_key("Drink")
                .expect("decode install pure-container sample")
                .is_some()
        );
    }

    #[test]
    fn message_dialog_buttons_use_active_language_resources() {
        let _lock = env_lock().lock();
        reset_cached_app_paths();
        let user_data = tempdir().expect("isolated localized-dialog user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "General", "LanguageEx", "DE")
            .expect("select German resources");
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
        .expect("open localized dialog");

        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        let dialog = &mut app.message_dialogs[0].state;
        assert_eq!(
            dialog.button_label(clonk_frontend::message_dialog::MessageDialogButton::Ok),
            "&OK"
        );
        assert_eq!(
            dialog.button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel),
            "&Abbrechen"
        );
        assert_eq!(
            dialog.handle_hotkey('A'),
            Some(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        );
        assert_eq!(dialog.handle_hotkey('C'), None);
        let layout = dialog.layout(640, 480, &fonts.text);
        let close = layout.close_button.expect("close button");
        let close_point = GuiPoint::new((close.x + 1) as f32, (close.y + 1) as f32);
        dialog.handle_pointer_move(close_point, &layout);
        assert_eq!(
            dialog
                .tooltip_state(Some(close_point), &layout)
                .expect("localized close tooltip")
                .text,
            "Schließen"
        );
        reset_cached_app_paths();
    }

    #[test]
    fn l016_plrclr_submits_full_owner_packet_and_authoritative_rows_recolor() {
        let mut app = new_menu_app(640, 480);
        let (_events, mut commands) = install_classic_host_network_stub(&mut app);
        let fred = clonk_engine::ControlPlayerInfoEntry {
            id: 4,
            name: clonk_engine::LegacyCString::from_bytes(b"Fred".to_vec()).unwrap(),
            color: 0x0000_ff00,
            original_color: 0x0000_ff00,
            ..Default::default()
        };
        app.control_clients
            .replace_snapshot([message_client(0, b"Exact Host")]);
        app.control_player_infos.replace_snapshot(
            4,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![fred.clone()],
                by_client: 0,
            }],
        );
        app.sync_classic_lobby_roster();

        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
            "/plrclr Fred FF0000".to_string(),
        ))
        .expect("submit lobby player-color request");

        let updates = commands.take_player_info_updates();
        assert_eq!(updates.len(), 1);
        let mut expected = fred.clone();
        expected.original_color = 0x00ff_0000;
        assert_eq!(
            updates[0],
            clonk_network::PlayerInfoUpdateRequest {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![expected.clone()],
            },
            "the complete owner packet is cloned and only OriginalColor changes"
        );

        expected.color = expected.original_color;
        let authoritative = clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![expected],
            by_client: 0,
        };
        app.control_player_infos
            .replace_snapshot(4, [authoritative.clone()]);
        app.sync_classic_lobby_roster();
        let expected_color = lobby_rgba(0x00ff_0000);
        assert!(app
            .classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .rows()
            .iter()
            .any(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 4 && player.color == expected_color)));

        let mut client = new_menu_app(640, 480);
        client.startup_view = StartupView::NetworkLobby;
        client.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
        client.control_clients.replace_snapshot([
            message_client(0, b"Exact Host"),
            message_client(7, b"Client"),
        ]);
        client.control_player_infos.replace_snapshot(4, [authoritative]);
        client.sync_classic_lobby_roster();
        assert!(client
            .network_lobby
            .as_ref()
            .unwrap()
            .roster_rows
            .iter()
            .any(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 4 && player.color == expected_color)));
    }

    #[test]
    fn generic_client_resource_save_hit_target_emits_the_resource_id() {
        let root = tempdir().expect("resource save root");
        let work = root.path().join("Network");
        fs::create_dir(&work).expect("network work directory");
        let source = work.join("Downloaded.c4s");
        fs::write(&source, b"payload").expect("downloaded resource");

        let mut app = new_menu_app(640, 480);
        app.startup_view = StartupView::NetworkLobby;
        let mut settings = ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        );
        settings.resource_directory = work.clone();
        app.network_mode = Some(NetworkMode::Client(settings));
        let (network, _events) = NetworkManager::test_stub_for_client_id(7);
        app.network = Some(network);
        app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
        let core = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Scenario as u8,
            id: 23,
            loadable: true,
            filename: LegacyCString::from_bytes(b"Remote/Downloaded.c4s".to_vec()).unwrap(),
            ..Default::default()
        };
        app.admission_resources.register_lobby_resource(&core);
        app.admission_resources
            .mark_complete_with_locality(core.id, source.clone(), false);
        app.register_classic_lobby_resource(&core, 100);
        app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Resources))
            .expect("open the client Resources sheet");
        assert!(app.network_lobby.as_ref().unwrap().resource_rows[&core.id].save_possible);

        {
            let lobby = app.network_lobby.as_mut().unwrap();
            let rect = lobby
                .update_layout(640.0, 480.0)
                .resource_save_buttons
                .first()
                .expect("save rect for the saveable resource")
                .1;
            lobby.handle_panel_pointer_move(GuiPoint::new(
                rect.origin.x + rect.size.width / 2.0,
                rect.origin.y + rect.size.height / 2.0,
            ));
        }
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press the resource save icon");
        app.handle_mouse_button(ElementState::Released)
            .expect("release the resource save icon through the retained controller");
        assert_eq!(
            fs::read(root.path().join("Downloaded.c4s")).expect("saved copy"),
            b"payload",
            "the routed SaveResourceRequested reaches request_lobby_resource_save"
        );
        assert_eq!(
            app.message_dialogs
                .last()
                .expect("save feedback dialog")
                .state
                .caption(),
            "Resource saved"
        );
    }

    #[test]
    fn l098_takeover_selection_submits_full_local_packet_with_savegame_association() {
        let mut app = new_menu_app(640, 480);
        install_test_free_savegame_player_row(&mut app, 50);
        let (network, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        let chosen = clonk_engine::ControlPlayerInfoEntry {
            id: 31,
            name: LegacyCString::from_bytes(b"Chooser".to_vec()).unwrap(),
            color: 0x0012_3456,
            original_color: 0x0065_4321,
            team: 3,
            extra_data: [1, 2, 3, 4],
            ..Default::default()
        };
        let sibling = clonk_engine::ControlPlayerInfoEntry {
            id: 32,
            name: LegacyCString::from_bytes(b"Sibling".to_vec()).unwrap(),
            flags: clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED,
            color: 0x0000_00aa,
            ..Default::default()
        };
        let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
        app.control_player_infos.replace_snapshot(
            99,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: packet_flags,
                players: vec![chosen.clone(), sibling.clone()],
                by_client: 7,
            }],
        );

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(50),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("free savegame player context opens");
        let root = app.context_menu.as_ref().unwrap().layout().panels[0].rows[0].rect;
        app.handle_context_menu_pointer_move(GuiPoint::new(
            (root.x + 1) as f32,
            (root.y + 1) as f32,
        ))
        .expect("open takeover submenu");

        let mut live_sibling = sibling.clone();
        live_sibling.color = 0x0000_00bb;
        app.control_player_infos.replace_snapshot(
            99,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: packet_flags,
                players: vec![chosen.clone(), live_sibling.clone()],
                by_client: 7,
            }],
        );
        let child = app.context_menu.as_ref().unwrap().layout().panels[1].rows[0].rect;
        app.handle_context_menu_pointer_move(GuiPoint::new(
            (child.x + 1) as f32,
            (child.y + 1) as f32,
        ))
        .expect("select takeover child");
        assert!(
            app.handle_context_menu_pointer_button(
                ElementState::Pressed,
                ContextMenuPointerButton::Left,
            )
            .expect("activate takeover child")
        );
        assert!(app.context_menu.is_none());

        let mut expected_chosen = chosen.clone();
        expected_chosen.savegame_player = 50;
        assert_eq!(
            commands.take_player_info_updates(),
            vec![clonk_network::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: packet_flags,
                players: vec![expected_chosen, live_sibling],
            }]
        );
        assert_eq!(
            app.control_player_infos
                .client_update_request(7)
                .unwrap()
                .players[0]
                .savegame_player,
            0,
            "takeover waits for the authoritative PlayerInfo echo"
        );
        assert!(
            app.handle_context_menu_pointer_button(
                ElementState::Released,
                ContextMenuPointerButton::Left,
            )
            .expect("consume takeover activation release")
        );
    }

    #[test]
    fn l081_new_color_resets_only_current_color_in_full_packet() {
        let mut app = new_menu_app(640, 480);
        let (mut chooser, companion) = install_test_classic_host_team_lobby(&mut app);
        chooser.color = 0x00ab_cdef;
        app.control_player_infos.replace_snapshot(
            9,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![chooser.clone(), companion.clone()],
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

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(chooser.id),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("player context opens");
        assert!(
            app.handle_context_menu_key(VirtualKeyCode::C, ElementState::Pressed)
                .expect("activate New Color hotkey")
        );

        let mut reset = chooser.clone();
        reset.color = reset.original_color;
        assert_eq!(
            commands.take_player_info_updates(),
            vec![clonk_network::PlayerInfoUpdateRequest {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![reset, companion],
            }]
        );
        assert_eq!(
            app.control_player_infos
                .client_update_request(0)
                .unwrap()
                .players[0]
                .color,
            chooser.color,
            "the roster waits for the authoritative echo"
        );
    }

    #[test]
    fn invisible_random_teams_sheet_uses_one_lazy_header_and_client_packet_order() {
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
        let player = |id, flags, player_type| clonk_engine::ControlPlayerInfoEntry {
            id,
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
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 7,
                    players: vec![player(20, 0, clonk_engine::PLAYER_INFO_TYPE_USER)],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 8,
                    players: vec![
                        player(30, 0, clonk_engine::PLAYER_INFO_TYPE_SCRIPT),
                        player(31, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                    ],
                    ..Default::default()
                },
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
        let rows = classic_lobby_roster_projection(
            &clients,
            &infos,
            Some(&metadata),
            0,
            LobbySheet::Teams,
        )
        .0;
        assert_eq!(
            rows.iter().map(LobbyRosterRow::id).collect::<Vec<_>>(),
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
        assert_eq!(header.label, "Random team");
        assert_eq!(header.icon, LobbyRosterIcon::Standard(19));

        infos.replace_snapshot(
            2,
            [
                clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    players: vec![player(
                        40,
                        clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                        clonk_engine::PLAYER_INFO_TYPE_USER,
                    )],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 7,
                    players: vec![player(41, 0, clonk_engine::PLAYER_INFO_TYPE_USER)],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 8,
                    players: vec![player(
                        42,
                        clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                        clonk_engine::PLAYER_INFO_TYPE_USER,
                    )],
                    ..Default::default()
                },
            ],
        );
        assert!(
            classic_lobby_roster_projection(
                &clients,
                &infos,
                Some(&metadata),
                0,
                LobbySheet::Teams,
            )
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
        assert_eq!(
            transfer_edit_selection(&mut edit, false, |selection| {
                copied = selection.to_string();
                Ok::<(), ()>(())
            }),
            Ok(true)
        );
        assert_eq!(copied, "alpha");
        assert_eq!(edit.text(), "alpha beta", "Copy does not mutate text");

        assert!(transfer_edit_selection(&mut edit, true, |_| Err("clipboard")).is_err());
        assert_eq!(
            edit.text(),
            "alpha beta",
            "failed Cut must retain the selection"
        );
        transfer_edit_selection(&mut edit, true, |_| Ok::<(), ()>(()))
            .expect("successful Cut");
        assert_eq!(edit.text(), " beta");

        edit.set_text("replace me");
        edit.select_all();
        assert!(apply_scensel_search_paste(
            &mut edit,
            "\r\nleft|right\r\nignored"
        ));
        assert_eq!(
            edit.text(),
            "left¦right",
            "leading blank lines are skipped and the first real newline submits/aborts"
        );

        edit.set_text("");
        assert!(!apply_scensel_search_paste(&mut edit, &"x".repeat(300)));
        assert_eq!(edit.text().len(), SEARCH_EDIT_MAX_BYTES);

        edit.set_text("selection");
        edit.select_all();
        assert!(!apply_scensel_search_paste(&mut edit, "\n"));
        assert_eq!(
            edit.selected_text(),
            Some("selection"),
            "blank-only paste does not delete the selection"
        );
    }

    #[test]
    fn scensel_rename_pointer_completion_cancels_target_focus_transfer() {
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = "Pointer.c4s".to_string();
        scenario.title = "Pointer".to_string();
        let scenarios = vec![scenario];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("pointer rename menu");
        let mut app = new_menu_app(800, 600);
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_scenario_browser();
        app.sync_scenario_game_option_bounds();

        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start empty mouse focus-loss rename");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("clear inline title");
        let record = app
            .scenario_game_options
            .layout()
            .rect(GameOptionButton::Record)
            .expect("Record option bounds");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(record.x + record.w / 2),
            f64::from(record.y + record.h / 2),
        ))
        .expect("point at Record option");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("outside mouse down finishes empty rename");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Search);
        app.handle_mouse_button(ElementState::Released)
            .expect("Record still receives the completing click");
        assert!(app.scenario_game_options.values().record);
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::Search,
            "empty FinishRename focus survives the complete mouse gesture"
        );

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start same-title touch focus-loss rename");
        let fonts = app.assets.clonk_fonts.as_ref().expect("classic fonts");
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, fonts);
        let search = GuiPoint::new(
            (layout.search_edit.x + layout.search_edit.w / 2) as f32,
            (layout.search_edit.y + layout.search_edit.h / 2) as f32,
        );
        app.handle_touch(TouchPhase::Started, search)
            .expect("outside touch start finishes same-title rename");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::List);
        app.handle_touch(TouchPhase::Ended, search)
            .expect("search edit still receives the completing touch");
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::List,
            "RR_Deleted list focus survives the complete touch gesture"
        );
    }

    #[test]
    fn startup_tooltip_app_uses_the_shared_cmouse_clock_and_runtime_resources() {
        let mut app = new_real_classic_menu_app(640, 480);
        let button = clonk_frontend::main_menu_layout(640, 480).buttons[0];
        let point = GuiPoint::new(
            (button.x + button.w / 2) as f32,
            (button.y + button.h / 2) as f32,
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("hover Start");

        let target = app
            .startup_element_tooltip_target_at(point)
            .expect("Start owns a native tooltip");
        assert_eq!(
            target,
            StartupTooltip::resource("IDS_DLGTIP_STARTGAME")
        );
        assert_eq!(
            app.resolve_startup_tooltip_text(target),
            "Start a local game without network support."
        );
        assert_eq!(
            app.resolve_startup_tooltip_text(StartupTooltip::resource(
                "IDS_L022_MISSING_RESOURCE"
            )),
            "[Undefined: IDS_L022_MISSING_RESOURCE]"
        );

        // Render the exact hovered base first with mouse input suppressed.
        // That frame is cacheable because no tooltip can become due.
        app.startup_tooltip.note_non_pointer_input();
        let mut base = vec![0; 640 * 480 * 4];
        assert!(app.render(&mut base).expect("render suppressed base"));
        assert!(app.menu_frame_cache.is_some());

        // Re-arm the one process-level clock far enough in the past to make
        // the inclusive 500ms boundary eligible. Pending hover bypasses the
        // cached base and the final overlay changes pixels.
        let started = Instant::now()
            .checked_sub(
                clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY
                    + Duration::from_millis(1),
            )
            .expect("monotonic clock supports a 501ms lookback");
        app.startup_tooltip = ClassicTooltipTracker::new_at(started);
        app.startup_tooltip.note_pointer_move_at(point, started);
        assert!(app.startup_element_tooltip_pending());
        assert_eq!(app.startup_tooltip.eligible_pointer(), Some(point));
        let mut tipped = vec![0; 640 * 480 * 4];
        assert!(app.render(&mut tipped).expect("render eligible tooltip"));
        assert_ne!(tipped, base);

        // A physical key clears active mouse input before any downstream key
        // owner. Same-pixel motion remains suppressed; a genuinely different
        // ceil-quantized pixel starts a fresh delay.
        app.handle_key(VirtualKeyCode::Z, ElementState::Pressed)
            .expect("route unbound key");
        assert!(!app.startup_element_tooltip_pending());
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x) - 0.25,
            f64::from(point.y) - 0.25,
        ))
        .expect("same native pixel motion");
        assert!(!app.startup_element_tooltip_pending());
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x) + 0.25,
            f64::from(point.y),
        ))
        .expect("next native pixel motion");
        assert!(app.startup_element_tooltip_pending());
        assert_eq!(app.startup_tooltip.eligible_pointer(), None);

        app.open_options_menu();
        assert_eq!(app.startup_tooltip.pointer_position(), None);
        assert!(!app.startup_element_tooltip_pending());

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(point.x),
            f64::from(point.y),
        ))
        .expect("re-arm hover before resize");
        assert!(app.startup_tooltip.pointer_position().is_some());
        app.resize(800, 600).expect("resize startup dialog");
        assert_eq!(app.startup_tooltip.pointer_position(), None);
    }

    #[test]
    fn l080_dialog_titles_use_the_process_global_tooltip_delay_and_close_resource() {
        use clonk_frontend::startup_options_advanced::{
            AdvancedConfigController, AdvancedConfigLabels,
        };
        use clonk_frontend::startup_options_dlg::OptionsSheet;

        fn assert_delayed_target(
            app: &mut GameApp,
            point: GuiPoint,
            expected: StartupTooltip,
        ) {
            let started = Instant::now();
            app.startup_tooltip = ClassicTooltipTracker::new_at(started);
            app.startup_tooltip.note_pointer_move_at(point, started);
            assert!(app
                .startup_tooltip
                .eligible_pointer_at(
                    started + clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY
                        - Duration::from_millis(1),
                )
                .and_then(|point| app.classic_dialog_title_tooltip_target_at(point))
                .is_none());
            assert_eq!(
                app.startup_tooltip
                    .eligible_pointer_at(
                        started + clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY,
                    )
                    .and_then(|point| app.classic_dialog_title_tooltip_target_at(point)),
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
        app.startup_options_advanced_dialog = Some(PendingOptionsAdvancedDialog {
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
            .startup_options_advanced_dialog
            .as_mut()
            .expect("advanced dialog")
            .controller
            .handle_pointer_move(close_point);
        assert_delayed_target(
            &mut app,
            close_point,
            StartupTooltip::resource("IDS_MNU_CLOSE"),
        );

        use clonk_frontend::runtime_client_list::{
            RuntimeClientListDialog, RuntimeClientListStatus, RuntimeClientRow,
            RuntimeClientStatusIcon,
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
        };
        let line_height = app
            .assets
            .clonk_fonts
            .as_deref()
            .expect("classic fonts")
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
            (runtime_layout.caption.x + 8) as f32,
            (runtime_layout.caption.y + runtime_layout.caption.h / 2) as f32,
        );
        assert!(runtime.handle_pointer_move(runtime_title, preferred, line_height));
        app.mode = AppMode::Running;
        app.runtime_client_list = Some(runtime);
        assert_delayed_target(
            &mut app,
            runtime_title,
            StartupTooltip::text("Network clients"),
        );
        let runtime_close = GuiPoint::new(
            (runtime_layout.close_button.x + 1) as f32,
            (runtime_layout.close_button.y + 1) as f32,
        );
        assert!(app
            .runtime_client_list
            .as_mut()
            .expect("runtime list")
            .handle_pointer_move(runtime_close, preferred, line_height));
        assert_delayed_target(
            &mut app,
            runtime_close,
            StartupTooltip::resource("IDS_MNU_CLOSE"),
        );

        let dragged_point = GuiPoint::new(runtime_title.x + 15.0, runtime_title.y - 4.0);
        assert!(app
            .runtime_client_list
            .as_mut()
            .expect("runtime list")
            .handle_pointer_down(runtime_title, preferred, line_height));
        let before_layered_move = app
            .runtime_client_list
            .as_ref()
            .expect("runtime list")
            .layout(preferred, line_height)
            .bounds;
        app.external_irc_dialog_visible = true;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(dragged_point.x),
            f64::from(dragged_point.y),
        ))
        .expect("move retained drag below higher layer");
        assert_ne!(
            app.runtime_client_list
                .as_ref()
                .expect("runtime list")
                .layout(preferred, line_height)
                .bounds,
            before_layered_move,
            "CMouse updates its retained drag element before z-order routing"
        );
        assert!(app
            .runtime_client_list
            .as_ref()
            .expect("runtime list")
            .has_positional_pointer_drag());
        app.handle_mouse_button(ElementState::Released)
            .expect("release retained drag below higher layer");
        assert!(!app
            .runtime_client_list
            .as_ref()
            .expect("runtime list")
            .has_positional_pointer_drag());
        app.external_irc_dialog_visible = false;

        let dragged_layout = app
            .runtime_client_list
            .as_ref()
            .expect("runtime list")
            .layout(preferred, line_height);
        let resize_drag_start = GuiPoint::new(
            (dragged_layout.caption.x + 8) as f32,
            (dragged_layout.caption.y + dragged_layout.caption.h / 2) as f32,
        );
        assert!(app
            .runtime_client_list
            .as_mut()
            .expect("runtime list")
            .handle_pointer_down(resize_drag_start, preferred, line_height));
        assert!(app
            .runtime_client_list
            .as_mut()
            .expect("runtime list")
            .handle_pointer_move(
                GuiPoint::new(resize_drag_start.x + 7.0, resize_drag_start.y + 3.0),
                preferred,
                line_height,
            ));
        app.resize(641, 481).expect("resize active runtime dialog");
        assert!(!app
            .runtime_client_list
            .as_ref()
            .expect("runtime list")
            .has_positional_pointer_drag());
        preferred = scoreboard_preferred_rect(
            app.graphics
                .preferred_dialog_rect(app.mouse_control.then_some(app.local_owner)),
        );
        let retained_after_resize = app
            .runtime_client_list
            .as_ref()
            .expect("runtime list")
            .layout(preferred, line_height)
            .bounds;
        let _ = app
            .runtime_client_list
            .as_mut()
            .expect("runtime list")
            .handle_pointer_move(GuiPoint::new(3.0, 3.0), preferred, line_height);
        assert_eq!(
            app.runtime_client_list
                .as_ref()
                .expect("runtime list")
                .layout(preferred, line_height)
                .bounds,
            retained_after_resize,
            "resize cancels capture without discarding the retained offset"
        );

        let mut info = RuntimeClientListDialog::new_info("Client information", row);
        let info_layout = info
            .info_layout(preferred, line_height)
            .expect("info layout");
        let info_title = GuiPoint::new(
            (info_layout.caption.x + 8) as f32,
            (info_layout.caption.y + info_layout.caption.h / 2) as f32,
        );
        assert!(info.handle_pointer_move(info_title, preferred, line_height));
        app.mode = AppMode::Menu;
        app.runtime_client_list = Some(info);
        assert_delayed_target(
            &mut app,
            info_title,
            StartupTooltip::text("Client information"),
        );
        let info_close = GuiPoint::new(
            (info_layout.close_button.x + 1) as f32,
            (info_layout.close_button.y + 1) as f32,
        );
        assert!(app
            .runtime_client_list
            .as_mut()
            .expect("client info")
            .handle_pointer_move(info_close, preferred, line_height));
        assert_delayed_target(
            &mut app,
            info_close,
            StartupTooltip::resource("IDS_MNU_CLOSE"),
        );

        app.runtime_client_list = None;
        app.startup_options_advanced_dialog = None;
        let mut definition = clonk_frontend::definition_sel::DefinitionSelController::new(
            "",
            Vec::new(),
            Vec::new(),
        );
        let (definition_width, definition_height) = {
            let surface = app.graphics.surface();
            (surface.width() as i32, surface.height() as i32)
        };
        let definition_layout = definition.layout(
            definition_width,
            definition_height,
            &app.assets.clonk_fonts.as_deref().expect("classic fonts").text,
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
            .as_mut()
            .expect("definition selector")
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
        let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
        assets.menu_background = None;
        assets.logo = None;
        assets.button_textures = None;
        let mut frame = vec![0_u8; 320 * 200 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("main menu must not use bitmap/solid fallbacks");
        assert!(matches!(
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
        let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
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
        assert_eq!(
            error,
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
        assert!(frame.iter().all(|byte| *byte == 0x91));
        let mut native = vec![0x57; 640 * 400 * 4];
        let error = app
            .render_native_loader_text(&mut native, 640, 400)
            .expect_err("global bundle precedes native loader errors");
        assert_global_gui_boundary(&error, expected);
        assert!(native.iter().all(|byte| *byte == 0x57));

        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated loading-refresh user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let mut app = new_menu_app_with_paths(320, 200, &paths);
        app.mode = AppMode::Loading;
        let loader_state_before = app
            .loader_screen
            .as_ref()
            .expect("real startup loader")
            .state()
            .clone();
        let loader_gui_before = app
            .loader_screen
            .as_ref()
            .expect("real startup loader")
            .resources()
            .gui_progress()
            .clone();
        let loader_fonts_before = app
            .loader_screen
            .as_ref()
            .expect("real startup loader")
            .resources()
            .fonts()
            .clone();
        let resources = app.assets.loader_resources().expect("loader resources");
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
            .expect("queue refresh");
        sender
            .send(ScenarioLoadingEvent::Finished(Err(
                "finished must remain queued".to_string(),
            )))
            .expect("queue finish");
        let cached = vec![0x31; 320 * 200 * 4];
        app.menu_frame_cache = Some(MenuFrameCache {
            view: StartupView::MainMenu,
            version: app.menu_render_version,
            width: 320,
            height: 200,
            native_text_deferred: false,
            frame: cached.clone(),
        });
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
        let state = app.loading_state.as_ref().expect("loading state retained");
        assert!(state.refresh_requested);
        assert!(state.refreshed_resources.is_some());
        assert_eq!(
            state.refreshed_global_gui_failures.as_ref(),
            Some(&failures)
        );
        assert!(app.active_global_gui_failures.is_empty());
        assert_eq!(app.mode, AppMode::Loading);
        let loader = app
            .loader_screen
            .as_ref()
            .expect("loader remains installed");
        assert_eq!(loader.state(), &loader_state_before);
        assert_eq!(loader.resources().gui_progress(), &loader_gui_before);
        assert!(Arc::ptr_eq(
            loader.resources().fonts(),
            &loader_fonts_before
        ));
        assert_eq!(
            app.menu_frame_cache.as_ref().expect("cache retained").frame,
            cached
        );

        let error = app
            .update()
            .expect_err("latched failure must guard the next update at ingress");
        assert_engine_parity_boundary(error, boundary);
        let mut frame = vec![0x62; 320 * 200 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("latched failure must guard logical loader render");
        assert!(matches!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(ClassicParityBoundary::GlobalGuiBootstrapResources { .. })
        ));
        assert!(frame.iter().all(|byte| *byte == 0x62));
    }

    #[test]
    fn l021_accepted_loading_reaches_100_only_after_successful_activation() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated accepted-refresh user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");

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
            .expect("valid refreshed loader resources")
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
        let expected_progress = refreshed
            .progress_bar()
            .expect("refreshed progress image")
            .pixels()
            .to_vec();
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
            .expect("queue accepted refresh");
        sender
            .send(ScenarioLoadingEvent::Finished(Ok(scenario)))
            .expect("queue successful finish");
        success
            .poll_loading()
            .expect("accept refresh and activation");
        assert_eq!(success.mode, AppMode::Running);
        assert!(success.loading_state.is_none());
        assert_eq!(
            success
                .loader_screen
                .as_ref()
                .expect("loader retained")
                .state()
                .progress(),
            100
        );
        assert!(success.active_global_gui_failures.is_empty());
        assert_eq!(
            success
                .loader_screen
                .as_ref()
                .expect("loader retained")
                .resources()
                .progress_bar()
                .expect("installed refreshed progress")
                .pixels(),
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
            .expect("queue accepted refresh before activation failure");
        sender
            .send(ScenarioLoadingEvent::Finished(Ok(scenario)))
            .expect("queue activation failure");
        failure
            .poll_loading()
            .expect("activation failure is restored to the menu");
        assert_eq!(failure.mode, AppMode::Menu);
        assert_eq!(failure.startup_view, StartupView::MainMenu);
        assert!(failure.loading_state.is_none());
        assert!(failure.loader_screen.is_none());
        assert!(failure.loader_error.is_none());
        assert!(failure.active_global_gui_failures.is_empty());
        assert_startup_error_log(
            &failure,
            "Scenario `Rust Sandbox` is missing a filesystem path",
        );
        assert_eq!(
            failure.startup_restart_diagnostics,
            StartupRestartDiagnostics::default()
        );
    }

    #[test]
    fn visible_ingame_menu_without_exact_resources_fails_before_rendering() {
        let mut app = new_menu_app(320, 200);
        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start explicit test sandbox");
        let assets = Arc::get_mut(&mut app.assets).expect("frontend assets are app-owned");
        for name in ["Menu.png", "Options.png", "Control.png", "Player.png"] {
            assets.startup_dialog_images.remove(name);
        }
        Arc::make_mut(&mut assets.hud_graphics).captain = None;
        app.ingame_menu
            .replace(app.local_owner, Some(IngameMenuState::surrender_menu()));
        app.scoreboard_initial_reconcile_pending = true;
        let before = runtime_global_ui_snapshot(&app);
        let mut frame = vec![0_u8; 320 * 200 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("text-only in-game menu fallback must not render");
        assert!(matches!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(ClassicParityBoundary::IngameMenuResources { missing })
                if missing.contains(&"Menu.png")
                    && missing.contains(&"Options.png")
                    && missing.contains(&"Control.png")
                    && missing.contains(&"Player.png")
                    && missing.contains(&"Captain.png")
        ));
        assert_eq!(runtime_global_ui_snapshot(&app), before);
    }

    #[test]
    fn l016_screenshot_folder_override_falls_back_to_install_root() {
        let install = tempdir().expect("screenshot install root");
        let user_data = tempdir().expect("screenshot user data");
        fs::create_dir_all(install.path().join("planet/System.c4g"))
            .expect("fixture System group");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("fixture app paths");
        paths.ensure_user_dirs().expect("fixture user directories");
        fs::write(
            paths.config_file(),
            b"[General]\nName=M\xe4ker\nScreenshotFolder=Configured Screenshots\n",
        )
        .expect("configure screenshot folder");
        let blocked = install.path().join("Configured Screenshots");
        fs::write(&blocked, b"not a directory").expect("block configured screenshot folder");

        let (path, result) = prepare_numbered_screenshot_path(Some(&paths));

        result.expect("install-root screenshot fallback");
        assert_eq!(path, install.path().join("Screenshot001.png"));
    }

    #[test]
    fn scenario_head_font_installs_the_pre_definition_size_twenty_loader_bundle() {
        let _lock = env_lock().lock();
        let root = tempdir().expect("scenario font fixture");
        install_global_gui_and_loader_test_root(root.path());
        let scenario_path = root.path().join("content/FontScenario.c4s");
        fs::create_dir_all(&scenario_path).expect("scenario group");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Font scenario\nLoader=LoaderFont.png\nFont=SomeFace,20\n",
        )
        .expect("scenario head font");
        write_preview_png(
            &scenario_path.join("LoaderFont.png"),
            [0x12, 0x34, 0x56, 0xff],
        );
        fs::copy(
            root.path().join("planet/System.c4g/Endeavour.ttf"),
            scenario_path.join("SomeFace.ttf"),
        )
        .expect("scenario vector face");
        let user = root.path().join("user");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(root.path())),
            ("LC_CONTENT_DIR", Some(root.path().join("content").as_path())),
            ("LC_USER_DATA_DIR", Some(user.as_path())),
        ]);
        let paths = AppPaths::discover().expect("scenario font paths");
        paths.ensure_user_dirs().expect("scenario font user dirs");
        fs::write(paths.config_file(), "[General]\nLanguageEx=US\n")
            .expect("deterministic loader language");
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

        let setup = build_scenario_loader(&scenario, &definition_load, &paths, &assets)
            .expect("Head.Font scenario loader resolves");
        let fonts = setup.screen.resources().fonts();
        for (name, line_height) in [
            ("Log", fonts.mini.line_height),
            ("MainSmall", fonts.main_small.line_height),
            ("Main", fonts.text.line_height),
            ("Caption", fonts.caption.line_height),
            ("Title", fonts.title.line_height),
        ] {
            assert_eq!(line_height, 31, "explicit ,20 must override {name} size");
        }
        let tooltip = setup
            .initial_tooltip_font
            .as_deref()
            .expect("pre-definition tooltip font");
        assert_eq!(tooltip.line_height, 31);
        assert_eq!(tooltip.h_space, 0);
        assert!(setup.initial_native_font_source.is_none());
        assert!(setup.refreshed_native_font_source.is_none());

        // A definition root is not registered until the later full resource
        // refresh. It cannot rescue a face missing during InitLoaderScreen.
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Font scenario\nLoader=LoaderFont.png\nFont=DefinitionOnly,20\n",
        )
        .expect("definition-only Head.Font");
        let definition_root = tempdir().expect("definition font root");
        let objects = definition_root.path().join("Objects.c4d");
        fs::create_dir_all(&objects).expect("definition module");
        fs::copy(
            root.path().join("planet/System.c4g/Endeavour.ttf"),
            objects.join("DefinitionOnly.ttf"),
        )
        .expect("definition-only vector face");
        let error = build_scenario_loader(
            &scenario,
            &ScenarioDefinitionLoad::Fixed {
                modules: vec!["Objects.c4d".to_string()],
                definition_root: Some(path_with_trailing_native_separator(
                    definition_root.path(),
                )),
            },
            &paths,
            &assets,
        )
        .err()
        .expect("pre-definition missing face must fail");
        assert!(
            error.to_string().contains("DefinitionOnly"),
            "unexpected pre-definition font error: {error:#}"
        );
    }

    #[test]
    fn installed_startup_loader_renders_before_boot_completion() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("installed paths");
        let mut app = GameApp::new(
            320,
            200,
            AudioOptions::default(),
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("app");
        assert_eq!(app.mode, AppMode::Loading);
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("classic startup loader");
        assert!(frame.chunks_exact(4).any(|pixel| pixel != [0, 0, 0, 0]));
        assert_eq!(
            app.loader_screen
                .as_ref()
                .expect("loader")
                .selection()
                .context(),
            clonk_frontend::loader_screen::LoaderContext::Startup
        );
        let state = app.loader_screen.as_ref().expect("loader").state();
        assert_eq!(state.title(), "Loading...");
        assert_eq!(state.progress(), 0);
        assert_eq!(state.log(), &clonk_frontend::loader_screen::LoaderLog::Hidden);
    }

    #[test]
    fn installed_scenario_loader_uses_recursive_folder_resource_tier() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("installed paths");
        paths.ensure_user_dirs().expect("user dirs");
        let mut config = Config::new();
        config.set_in(Some("General"), "LanguageEx", "US");
        config
            .save(paths.config_file())
            .expect("explicit loader language config");
        let mut app = GameApp::new(
            320,
            200,
            AudioOptions::default(),
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("app");
        let scenario =
            resolve_next_mission_scenario(&app.scenario_catalog, "Fantasy.c4f/Crystalvalley.c4s")
                .expect("shipped Crystal Valley scenario");
        let setup = build_scenario_loader(
            &scenario,
            &ScenarioDefinitionLoad::Seed {
                modules: vec!["Objects.c4d".to_string()],
                definition_root: None,
            },
            &paths,
            app.assets.as_ref(),
        )
        .expect("scenario loader");
        assert_eq!(
            setup.screen.selection().context(),
            clonk_frontend::loader_screen::LoaderContext::Scenario
        );
        assert_eq!(
            setup.screen.selection().effective_specification(),
            "LoaderFantasy*"
        );
        assert!(setup
            .screen
            .selection()
            .selected_filename()
            .starts_with("LoaderFantasy"));

        let initial_source = setup
            .initial_native_font_source
            .clone()
            .expect("shipped loader font has a validated native source");
        let refreshed_source = setup
            .refreshed_native_font_source
            .clone()
            .expect("shipped running font has a validated native source");
        app.configure_native_startup_fonts(1.5, false);
        app.install_active_classic_fonts(
            setup.screen.resources().fonts().clone(),
            setup.initial_tooltip_font.clone(),
            Some(initial_source),
        );
        assert!(app.can_defer_native_loader_text(1.5));

        app.install_active_classic_fonts(
            setup.refreshed_resources.fonts().clone(),
            setup.refreshed_tooltip_font.clone(),
            Some(refreshed_source),
        );
        app.mode = AppMode::Running;
        assert!(app.can_present_ordered_native_text(1.5));
    }

    #[test]
    fn client_network_settings_supply_the_local_system_resource_candidate() {
        // GameRes.InitNetwork resolves the host's non-loadable System core
        // against the client's installed System.c4g before DoLobby
        // (src/C4GameParameters.cpp:125-160;
        // src/C4Network2.cpp:329-344).
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
        let address = SocketAddr::from(([127, 0, 0, 1], 11_112));

        let settings = client_settings_for_paths(address, "Client".to_string(), Some(&paths));

        assert_eq!(
            settings.server_addresses,
            [
                clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Tcp, address),
                clonk_network::NetworkAddress::new(clonk_network::NetworkProtocol::Udp, address),
            ]
        );
        assert_eq!(
            settings.resource_directory,
            paths.cache_dir().join("Network")
        );
        assert_eq!(
            settings.local_system_path.as_deref(),
            Some(paths.system_group_path())
        );
        assert_eq!(
            settings.mesh_tcp_bind_address,
            Some(SocketAddr::from(([0_u16; 8], 11_112)))
        );
        assert_eq!(
            settings.mesh_udp_bind_address,
            Some(SocketAddr::from(([0_u16; 8], 11_113)))
        );
        assert!(settings
            .local_resource_roots
            .iter()
            .any(|root| Some(root.as_path()) == paths.content_dir()));
    }

    #[test]
    fn player_context_menu_missing_global_resources_fails_typed_without_selection_mutation() {
        let mut app = new_menu_app(640, 480);
        app.startup_player_models
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
        app.startup_player_dialog
            .as_mut()
            .expect("player controller")
            .set_pointer_position(Some(GuiPoint::new(
                (layout.list_client.x + 2) as f32,
                (layout.list_client.y + layout.item_height / 2) as f32,
            )));

        remove_global_gui_sheet(&mut app, "GUISpinBoxArrow.png");
        let selected_before = app
            .startup_player_dialog
            .as_ref()
            .expect("player controller")
            .selected_index();
        let version_before = app.menu_render_version;
        let error = app
            .open_startup_player_context_menu(false)
            .expect_err("missing process-global resource must fail typed");
        assert!(matches!(
            error,
            EngineError::ClassicMenuParityBoundary { ref detail }
                if detail.contains("GUISpinBoxArrow")
        ));
        assert!(app.context_menu.is_none());
        assert_eq!(
            app.startup_player_dialog
                .as_ref()
                .expect("player controller")
                .selected_index(),
            selected_before
        );
        assert_eq!(app.menu_render_version, version_before);
    }

    #[test]
    fn resource_join_record_copies_player_group_for_replay() {
        let directory = tempdir().expect("record directory");
        let player_path = directory.path().join("Alice.c4p");
        let mut player_group = MutableGroup::new("Alice.c4p");
        player_group
            .add_file(
                "Player.txt",
                b"[Player]\nName=Alice\n[Preferences]\nColorDw=255\n".to_vec(),
            )
            .expect("add player core");
        fs::write(&player_path, player_group.pack().expect("pack player"))
            .expect("write player group");
        let output_path = directory.path().join("001-Resource.c4s");
        let mut app = new_state_only_running_sandbox_app();
        install_test_recording_template(&mut app, output_path.clone());
        app.admission_resources.mark_complete(17, player_path);
        app.start_recording(true).unwrap();
        let core = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 17,
            loadable: true,
            filename: LegacyCString::from_bytes(b"Players/Alice.c4p".to_vec()).unwrap(),
            ..clonk_engine::NetworkResourceCore::default()
        };
        let packet = clonk_engine::ControlPacket::JoinPlayer(clonk_engine::JoinPlayerControlData {
            filename: LegacyCString::from_bytes(b"Alice.c4p".to_vec()).unwrap(),
            at_client: 0,
            info_id: 1,
            source: clonk_engine::JoinPlayerSource::Resource(core.clone()),
            by_client: 0,
        });

        app.record_control_packet(&packet);
        assert!(app.finish_recording().is_none());

        let record = Group::open(&output_path).expect("record group");
        let copied = record
            .open_child("17-Alice.c4p")
            .expect("recorded player child");
        assert!(copied.exists("Player.txt"));
        let mut scenario = FrontendScenario::fallback();
        scenario.path = Some(output_path);
        app.active_scenario = Some(scenario);
        app.control_playback = Some(
            ControlRecordPlayback::from_bytes(
                &record.read_file("CtrlRec.c4b").expect("record stream"),
            )
            .expect("open record stream"),
        );
        assert_eq!(
            app.replay_record_player_file(&core)
                .expect("reload copied player")
                .name,
            "Alice"
        );
    }

    #[test]
    fn synchronized_player_file_with_empty_filename_never_resolves_the_install_root() {
        // C4Player::Save on a filename-less player fails at its EraseItem/
        // C4Group_MoveItem calls without ever renaming the installation
        // (C4Player.cpp:406-462). The Rust fallback used to resolve the empty
        // filename to `install_root.join("")` — the install root itself — and
        // then swap the whole installation aside for the staged commit.
        let install = tempdir().expect("throwaway install root");
        let planet = install.path().join("planet");
        fs::create_dir_all(planet.join("System.c4g")).expect("create system group");
        fs::write(install.path().join("Sentinel.txt"), b"install root survives")
            .expect("write install sentinel");
        let user_data = tempdir().expect("player sync user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_CONTENT_DIR", None),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover throwaway paths");

        let mut app = new_state_only_synthetic_crew_running_sandbox_app();
        app.app_paths = Some(paths);
        let player_number = app.local_owner;
        let info_id = 603;
        let mut state = app.engine.capture_state();
        let player = state
            .players
            .iter_mut()
            .find(|player| player.id == player_number)
            .expect("sandbox player state");
        player.player_info_id = info_id;
        player.status = clonk_engine::PlayerStatus::Active;
        player.script_player = false;
        app.engine
            .restore_state(&state)
            .expect("install filename-less player state");
        app.control_player_infos.replace_snapshot(
            info_id,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: info_id,
                    game_number: player_number,
                    ..clonk_engine::ControlPlayerInfoEntry::default()
                }],
                by_client: 0,
                ..clonk_engine::PlayerInfoControlData::default()
            }],
        );

        let info = app
            .control_player_infos
            .get(info_id)
            .cloned()
            .expect("filename-less player info");
        assert_eq!(info.filename.as_bytes(), b"");
        assert_eq!(app.synchronized_player_profile_path(&info), None);

        assert!(!app.persist_synchronized_local_player_files());
        assert_eq!(
            fs::read(install.path().join("Sentinel.txt")).expect("install root intact"),
            b"install root survives"
        );
        let residue = fs::read_dir(install.path().parent().expect("install parent"))
            .expect("scan install siblings")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("lc-rewrite")
            })
            .count();
        assert_eq!(residue, 0, "no staged/backup rewrite residue may appear");
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
                    script: clonk_engine::LegacyCString::from_bytes(
                        b"SetPreSend(76, \"client a*\")".to_vec(),
                    )
                    .expect("script is NUL-free"),
                    by_client: 0,
                }),
                NetworkControl::ClientRemove(clonk_engine::ClientRemoveControlData {
                    client_id: 7,
                    reason: clonk_engine::LegacyCString::default(),
                    by_client: 0,
                }),
            ],
        )
        .expect("SetPreSend applies before the following local-client removal");

        assert!(app.network.is_none());
        assert!(app.network_control_clock.is_none());
        assert_eq!(runtime_flash_text(&app), Some("TargetFPS: 76"));
    }

    #[test]
    fn adaptive_presend_uses_live_target_and_emits_the_exact_classic_flash() {
        let mut app = new_running_sandbox_app();
        let mut clock = NetworkControlClock::new(0, 1);
        clock.set_target_fps(76);
        clock.observe_round_trip_ms(300);
        for _ in 0..6 {
            assert!(clock.calculate_performance().is_none());
            clock.complete_control_frame();
        }
        let change = clock
            .calculate_performance()
            .expect("the seventh sample changes presend to two");
        app.apply_control_presend_change(change)
            .expect("intermediate adaptive flash installs");
        assert_eq!(
            runtime_flash_text(&app),
            Some("PreSend: 2  - TargetFPS: 76")
        );
        clock.complete_control_frame();
        for _ in 0..6 {
            assert!(clock.calculate_performance().is_none());
            clock.complete_control_frame();
        }
        let change = clock
            .calculate_performance()
            .expect("live target changes the fourteenth sample to presend three");
        app.apply_control_presend_change(change)
            .expect("adaptive flash installs");
        assert_eq!(
            runtime_flash_text(&app),
            Some("PreSend: 3  - TargetFPS: 76")
        );
    }

    #[test]
    fn l036_console_script_strictness_matches_native_tokens_and_reaches_packets() {
        use clonk_engine::ScriptStrictness::{NonStrict, Strict1, Strict2, Strict3};

        for (config, expected) in [
            ("[Developer]\nConsoleScriptStrictness=NonStrict\n", NonStrict),
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
            ("[Developer]\nConsoleScriptStrictness=\"Strict2\"\n", Strict3),
            ("[Developer]\nConsoleScriptStrictness =Strict2\n", Strict3),
            ("[Developer]\nConsoleScriptStrictness= Strict2\n", Strict2),
            (
                "[Developer]\nConsoleScriptStrictness=Strict2 # comment\n",
                Strict2,
            ),
        ] {
            assert_eq!(
                configured_console_script_strictness(config.as_bytes()),
                expected,
                "config {config:?}"
            );
        }
        let wide_unsigned_long = std::mem::size_of::<std::os::raw::c_ulong>() > 4;
        for (value, expected) in [
            (
                "4294967296",
                if wide_unsigned_long { NonStrict } else { Strict3 },
            ),
            (
                "4294967298",
                if wide_unsigned_long { Strict2 } else { Strict3 },
            ),
        ] {
            let config = format!("[Developer]\nConsoleScriptStrictness={value}\n");
            assert_eq!(
                configured_console_script_strictness(config.as_bytes()),
                expected,
                "native unsigned-long conversion for {value}"
            );
        }

        let _lock = env_lock().lock();
        let fixture = tempdir().expect("console strictness configuration");
        let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
        fs::write(
            paths.config_file(),
            b"[Developer]\nConsoleScriptStrictness=Strict2\n",
        )
        .expect("write exact console strictness config");

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
        assert_eq!(script.strictness, Strict2);
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
        let mut snapshot = host_config
            .initial_join_snapshot
            .expect("default host publishes JoinData");
        snapshot.parameters.control_rate = 3;
        let join_data = clonk_network::JoinDataEnvelope {
            client_id: 3,
            start_control_tick: 23,
            status: host_config.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        event_tx
            .send(NetworkEvent::JoinData(join_data.clone()))
            .expect("queue JoinData");

        app.process_network_events().expect("retain JoinData");

        assert_eq!(app.pending_network_join_data, Some(join_data));
        assert_eq!(
            app.network_control_clock,
            Some(NetworkControlClock::new(23, 3))
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
            app.scenario_catalog.insert(identifier.to_string(), scenario);
        }
        let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
        lobby.select_scenario("Old.c4s", "Old");
        lobby.preload.record_result(true);
        app.network_lobby = Some(lobby);

        assert!(app.select_network_lobby_scenario("New.c4s", "New"));

        let preload = app.network_lobby.as_ref().unwrap().preload;
        assert!(!preload.spent);
        assert!(preload.manual_button_present);
        assert!(preload.eligible);
        assert_eq!(
            app.network_lobby
                .as_ref()
                .and_then(NetworkLobbyState::selected_identifier),
            Some("New.c4s")
        );
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
            let core = clonk_engine::NetworkResourceCore {
                resource_type: clonk_network::HostResourceType::Player as u8,
                id: resource_id,
                loadable: true,
                ..Default::default()
            };
            clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: player_id,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(core),
                    ..Default::default()
                }],
                by_client: 0,
                ..Default::default()
            }
        };

        event_tx
            .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
                player_info(1, 48),
            )))
            .expect("queue direct PlayerInfo");
        app.process_network_events().expect("apply direct PlayerInfo");
        assert_eq!(
            app.admission_resources.status(48),
            Some(&AdmissionResourceState::Loading { removed: false })
        );

        app.apply_ready_controls(0, vec![NetworkControl::PlayerInfo(player_info(2, 49))])
            .expect("apply synchronized PlayerInfo");
        assert_eq!(
            app.admission_resources.status(49),
            Some(&AdmissionResourceState::Loading { removed: false })
        );
    }

    #[test]
    fn missing_join_client_does_not_start_or_stall_a_resource_load() {
        let resource_id = 50;
        let controls = vec![NetworkControl::JoinPlayer(
            clonk_engine::JoinPlayerControlData {
                at_client: 9,
                info_id: 1,
                source: clonk_engine::JoinPlayerSource::Resource(
                    clonk_engine::NetworkResourceCore {
                        resource_type: clonk_network::HostResourceType::Player as u8,
                        id: resource_id,
                        loadable: true,
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
        )];
        let mut resources = AdmissionResourceStore::default();

        assert!(preflight_admission_resources(
            &mut resources,
            &ControlClientRegistry::default(),
            &controls,
            &HashSet::new(),
        ));
        assert_eq!(resources.status(resource_id), None);
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
        app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
            metadata,
        ));
        app.control_player_infos.replace_snapshot(
            30,
            [clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![
                    set_control_test_player(10, 1, 0),
                    set_control_test_player(20, 1, 0),
                ],
                ..Default::default()
            }],
        );

        event_tx
            .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
                clonk_engine::PlayerInfoControlData {
                    client_id: 3,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED,
                    players: vec![
                        set_control_test_player(10, 1, 0),
                        set_control_test_player(20, 1, 0),
                        set_control_test_player(30, 1, 0),
                    ],
                    by_client: 0,
                },
            )))
            .expect("queue host-authored PlayerInfo addition");
        app.process_network_events()
            .expect("apply host-authored PlayerInfo addition");

        let teams = app
            .network_team_assignment
            .as_ref()
            .expect("host team assignment remains installed")
            .teams();
        assert_eq!(teams.teams[0].player_ids, vec![20, 30]);
        assert_eq!(teams.teams[1].player_ids, vec![10]);
        assert_eq!(app.control_player_infos.get(10).unwrap().team, 2);
        assert_eq!(app.control_player_infos.get(20).unwrap().team, 1);
        assert_eq!(app.control_player_infos.get(30).unwrap().team, 1);

        let broadcasts = commands.take_broadcast_player_infos();
        let [updated] = broadcasts.as_slice() else {
            panic!("expected one rebalanced PlayerInfo packet, got {broadcasts:?}");
        };
        assert_eq!((updated.client_id, updated.by_client), (3, 0));
        assert_eq!(
            updated.flags
                & (clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
                    | clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED),
            0,
        );
        assert_eq!(
            updated
                .players
                .iter()
                .map(|player| (player.id, player.team))
                .collect::<Vec<_>>(),
            vec![(10, 2), (20, 1), (30, 1)],
        );
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
        let resource = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: 61,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"Remote.c4p".to_vec())
                .expect("valid resource filename"),
            ..Default::default()
        };

        event_tx
            .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
                clonk_engine::PlayerInfoControlData {
                    client_id: 3,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 41,
                        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                        resource: Some(resource.clone()),
                        ..Default::default()
                    }],
                    by_client: 0,
                    ..Default::default()
                },
            )))
            .expect("queue direct resource-backed PlayerInfo");

        app.process_network_events()
            .expect("process direct resource-backed PlayerInfo");

        assert_eq!(
            app.admission_resources.status(resource.id),
            Some(&AdmissionResourceState::Loading { removed: false })
        );
        assert_eq!(
            commands.take_submitted_join_players(),
            vec![(
                tick,
                clonk_engine::JoinPlayerControlData {
                    filename: resource.filename.clone(),
                    at_client: 3,
                    info_id: 41,
                    source: clonk_engine::JoinPlayerSource::Resource(resource),
                    by_client: 0,
                },
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
        app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
            metadata,
        ));
        let player = |id, team, color, original_color, projected_gain, name: &[u8], forced: &[u8]| {
            let mut player = set_control_test_player(id, team, 0);
            player.color = color;
            player.original_color = original_color;
            player.league_projected_gain = projected_gain;
            player.name = clonk_engine::LegacyCString::from_bytes(name.to_vec()).unwrap();
            player.forced_name = clonk_engine::LegacyCString::from_bytes(forced.to_vec()).unwrap();
            player
        };
        let mut gain_only = player(50, 0, 0x0000_00f4, 0x0000_00f4, 4, b"History", b"");
        gain_only.flags =
            clonk_engine::PLAYER_INFO_FLAG_JOINED | clonk_engine::PLAYER_INFO_FLAG_REMOVED;
        app.control_player_infos.replace_snapshot(
            50,
            [
                clonk_engine::PlayerInfoControlData {
                    client_id: 3,
                    players: vec![
                        player(
                            10,
                            1,
                            0x0000_f400,
                            0x00f4_0000,
                            6,
                            b"Alice",
                            b"Alice (2)",
                        ),
                        player(20, 1, 0x0000_00f4, 0x0000_00f4, -1, b"Bob", b""),
                        player(30, 1, 0x00f4_f400, 0x00f4_f400, 0, b"Cara", b""),
                    ],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 4,
                    players: vec![player(
                        40,
                        2,
                        0x00f4_0000,
                        0x00f4_0000,
                        9,
                        b"Alice",
                        b"",
                    )],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 5,
                    players: vec![gain_only],
                    ..Default::default()
                },
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
        .expect("execute synchronized client removal");

        assert!(!app.control_clients.contains(4));
        assert!(app.control_player_infos.get(40).is_none());
        let teams = app
            .network_team_assignment
            .as_ref()
            .expect("host team assignment remains installed")
            .teams();
        assert_eq!(teams.teams[0].player_ids, vec![20, 30]);
        assert_eq!(teams.teams[1].player_ids, vec![10]);
        assert_eq!(app.control_player_infos.get(10).unwrap().team, 2);
        assert_eq!(app.control_player_infos.get(10).unwrap().color, 0x00f4_0000);
        assert!(app
            .control_player_infos
            .get(10)
            .unwrap()
            .forced_name
            .is_empty());
        assert_eq!(
            app.control_player_infos
                .client_packet(3)
                .unwrap()
                .players
                .iter()
                .map(|player| player.league_projected_gain)
                .collect::<Vec<_>>(),
            vec![-1, -1, -1],
        );

        let broadcasts = commands.take_broadcast_player_infos();
        let [updated, gain_only] = broadcasts.as_slice() else {
            panic!("expected two final PlayerInfo packets, got {broadcasts:?}");
        };
        assert_eq!((updated.client_id, updated.by_client), (3, 0));
        assert_eq!(
            updated.flags
                & (clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
                    | clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED),
            0,
        );
        assert_eq!(
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
                .collect::<Vec<_>>(),
            vec![
                (10, 2, 0x00f4_0000, Vec::new(), -1),
                (20, 1, 0x0000_00f4, Vec::new(), -1),
                (30, 1, 0x00f4_f400, Vec::new(), -1),
            ],
        );
        assert_eq!((gain_only.client_id, gain_only.by_client), (5, 0));
        assert_eq!(gain_only.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED, 0);
        assert_eq!(gain_only.players.len(), 1);
        assert_eq!(gain_only.players[0].id, 50);
        assert_eq!(gain_only.players[0].league_projected_gain, -1);
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
        let core = clonk_engine::NetworkResourceCore {
            resource_type: 3,
            id: resource_id,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            ..Default::default()
        };
        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id,
                core,
                path: path.clone(),
                local: false,
            })
            .unwrap();

        app.process_network_events().unwrap();

        assert_eq!(
            app.admission_resources.complete_path(resource_id),
            Some(path.as_path())
        );
    }

    #[test]
    fn unknown_loadable_resource_join_stalls_until_resource_completion() {
        let mut app = new_synthetic_running_sandbox_app();
        let (manager, event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.engine.set_network_game(true);
        let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
        let initial_frame = app.engine.frame();
        let info_id = 18;
        let resource_id = 62;
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../clonk-engine/tests/fixtures/embedded_player.c4p"
        ));
        let core = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: resource_id,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"Player.c4p".to_vec()).unwrap(),
            ..Default::default()
        };
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: vec![
                    NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData {
                        client_id: 1,
                        players: vec![clonk_engine::ControlPlayerInfoEntry {
                            id: info_id,
                            name: clonk_engine::LegacyCString::from_bytes(
                                b"Delayed resource".to_vec(),
                            )
                            .unwrap(),
                            flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                            resource: Some(core.clone()),
                            ..Default::default()
                        }],
                        by_client: 1,
                        ..Default::default()
                    }),
                    NetworkControl::JoinPlayer(clonk_engine::JoinPlayerControlData {
                        at_client: 0,
                        info_id,
                        source: clonk_engine::JoinPlayerSource::Resource(core.clone()),
                        by_client: 1,
                        ..Default::default()
                    }),
                ],
            })
            .expect("queue resource-backed join before completion");

        app.update().expect("pending resource stalls the control tick");

        assert_eq!(app.engine.frame(), initial_frame);
        assert!(app.network_ticks.ready.contains_key(&tick));
        assert!(app.control_player_infos.get(info_id).is_none());
        assert_eq!(
            app.admission_resources.status(resource_id),
            Some(&AdmissionResourceState::Loading { removed: false })
        );
        let wait = app
            .blocking_resource_wait
            .as_ref()
            .expect("resource-backed JoinPlayer opens the progress wait");
        assert_eq!(wait.scope, BlockingResourceScope::PlayerJoin);
        assert_eq!(wait.resource_id, resource_id);
        assert_eq!(wait.display_name, "player file for Delayed resource");
        let progress = app
            .message_dialogs
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
            .expect("player resource progress dialog");
        assert_eq!(progress.state.progress(), Some(0));
        assert_eq!(progress.state.message(), "Waiting for player file for Delayed resource...");

        event_tx
            .send(NetworkEvent::ResourceProgress {
                resource_id,
                present_percent: 47,
            })
            .expect("advance delayed player resource");
        app.update().expect("updated progress still stalls the control tick");
        assert_eq!(app.engine.frame(), initial_frame);
        assert_eq!(
            app.blocking_resource_wait
                .as_ref()
                .expect("wait remains active")
                .present_percent(),
            47
        );
        assert_eq!(
            app.message_dialogs
                .iter()
                .find(|dialog| matches!(
                    dialog.continuation,
                    MessageDialogContinuation::BlockingResourceWait { .. }
                ))
                .and_then(|dialog| dialog.state.progress()),
            Some(47)
        );

        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id,
                core,
                path: path.clone(),
                local: false,
            })
            .expect("complete delayed player resource");
        app.update().expect("completed resource releases control tick");

        assert_eq!(app.engine.frame(), initial_frame + 1);
        assert!(!app.network_ticks.ready.contains_key(&tick));
        assert_eq!(
            app.admission_resources.complete_path(resource_id),
            Some(path.as_path())
        );
        assert!(app
            .snapshot
            .players
            .iter()
            .any(|player| player.player_info_id == info_id));
        assert!(app.blocking_resource_wait.is_none());
        assert!(!app.message_dialogs.iter().any(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::BlockingResourceWait { .. }
        )));
    }

    #[test]
    fn failed_loadable_resource_releases_the_stalled_tick_as_a_noop() {
        let mut app = new_running_sandbox_app();
        let (manager, event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
        let initial_frame = app.engine.frame();
        let resource_id = 63;
        let core = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: resource_id,
            loadable: true,
            ..Default::default()
        };
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: vec![NetworkControl::JoinPlayer(
                    clonk_engine::JoinPlayerControlData {
                        at_client: 0,
                        info_id: 99,
                        source: clonk_engine::JoinPlayerSource::Resource(core),
                        by_client: 1,
                        ..Default::default()
                    },
                )],
            })
            .expect("queue resource-backed join before failure");

        app.update().expect("pending resource stalls the control tick");
        assert_eq!(app.engine.frame(), initial_frame);
        assert_eq!(
            app.admission_resources.status(resource_id),
            Some(&AdmissionResourceState::Loading { removed: false })
        );

        event_tx
            .send(NetworkEvent::ResourceLoadFailed { resource_id })
            .expect("fail delayed player resource");
        app.update().expect("failed resource releases control tick");

        assert_eq!(app.engine.frame(), initial_frame + 1);
        assert_eq!(
            app.admission_resources.status(resource_id),
            Some(&AdmissionResourceState::Unavailable(
                AdmissionResourceUnavailable::TransferFailed
            ))
        );
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
        let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
        let initial_frame = app.engine.frame();
        let info_id = 17;
        let resource_id = 61;
        let local_owner = app.local_owner;
        let resource = clonk_engine::NetworkResourceCore {
            resource_type: 3,
            id: resource_id,
            loadable: false,
            filename: clonk_engine::LegacyCString::from_bytes(b"Missing.c4p".to_vec())
                .expect("valid resource filename"),
            ..Default::default()
        };
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: vec![
                    NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData {
                        client_id: 1,
                        players: vec![clonk_engine::ControlPlayerInfoEntry {
                            id: info_id,
                            ..Default::default()
                        }],
                        by_client: 1,
                        ..Default::default()
                    }),
                    NetworkControl::JoinPlayer(clonk_engine::JoinPlayerControlData {
                        at_client: 0,
                        info_id,
                        source: clonk_engine::JoinPlayerSource::Resource(resource),
                        by_client: 1,
                        ..Default::default()
                    }),
                    NetworkControl::Player {
                        owner: local_owner,
                        event: ControlEvent::Press(ControlButton::Right),
                    },
                ],
            })
            .expect("queue unloadable resource join");

        app.update().expect("execute nonblocking resource tick");

        assert_eq!(
            app.admission_resources.status(resource_id),
            Some(&AdmissionResourceState::Unavailable(
                AdmissionResourceUnavailable::Unloadable
            ))
        );
        assert!(
            app.snapshot
                .players
                .iter()
                .all(|player| player.player_info_id != info_id),
            "an unavailable resource cannot create a player"
        );
        assert_ne!(
            app.engine
                .player(local_owner)
                .expect("local player")
                .control
                .pressed_coms
                & (1 << clonk_engine::COM_RIGHT),
            0,
            "the later control still executes"
        );
        assert_eq!(app.engine.frame(), initial_frame + 1);
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
        let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
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
        let resource = clonk_engine::NetworkResourceCore {
            resource_type: 3,
            id: resource_id,
            loadable: true,
            filename: clonk_engine::LegacyCString::from_bytes(b"WrongCorePath.c4p".to_vec())
                .expect("valid resource filename"),
            ..Default::default()
        };
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: vec![
                    NetworkControl::PlayerInfo(clonk_engine::PlayerInfoControlData {
                        client_id: 1,
                        players: vec![clonk_engine::ControlPlayerInfoEntry {
                            name: clonk_engine::LegacyCString::from_bytes(b"Resource Tyler".to_vec())
                                .expect("valid player name"),
                            id: info_id,
                            ..Default::default()
                        }],
                        by_client: 1,
                        ..Default::default()
                    }),
                    NetworkControl::JoinPlayer(clonk_engine::JoinPlayerControlData {
                        filename: clonk_engine::LegacyCString::from_bytes(
                            b"WrongPacketPath.c4p".to_vec(),
                        )
                        .expect("valid packet filename"),
                        at_client: 0,
                        info_id,
                        source: clonk_engine::JoinPlayerSource::Resource(resource),
                        by_client: 0,
                    }),
                ],
            })
            .expect("queue complete resource join");

        app.update().expect("execute complete resource tick");

        let joined = app
            .snapshot
            .players
            .iter()
            .find(|player| player.player_info_id == info_id)
            .expect("completed resource player joined");
        assert_eq!(joined.name, "Resource Tyler");
        assert_eq!((joined.score, joined.total_playing_time), (42, 99));
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
            clonk_engine::NetworkControlTiming::new(9, 2).expect("valid network timing"),
        );

        event_tx
            .send(NetworkEvent::ReadyTick {
                tick: 9,
                controls: Vec::new(),
            })
            .expect("queue initial ready tick");
        app.update().expect("execute initial control frame");
        assert_eq!(app.engine.frame(), 1);
        assert_eq!(commands.take_finalized_ticks(), vec![9]);

        app.update().expect("presend from the intervening frame");
        assert_eq!(app.engine.frame(), 2);
        assert_eq!(
            commands.take_finalized_ticks(),
            vec![10],
            "tick 10 must leave one frame before its execution frame"
        );

        // Deliver the echoed aggregate after that one-frame delay. Frame 2
        // must execute immediately rather than first submitting tick 10 and
        // stalling for another round trip.
        event_tx
            .send(NetworkEvent::ReadyTick {
                tick: 10,
                controls: Vec::new(),
            })
            .expect("return the presend aggregate");
        app.update().expect("execute the already-ready tick");
        assert_eq!(app.engine.frame(), 3);
        assert_eq!(
            app.network_control_clock
                .map(network::NetworkControlClock::current_tick),
            Some(11)
        );
        assert!(commands.take_finalized_ticks().is_empty());
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
        assert!(matches!(
            app.construction_menu_drag.as_ref(),
            Some(ConstructionMenuDrag::Active {
                site_valid: true,
                ..
            })
        ));
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift");
        app.handle_mouse_button(ElementState::Released)
            .expect("release valid construction drag");

        let (controls, commands, selections) = network_commands.take_submitted_player_inputs();
        assert!(controls.is_empty());
        assert_eq!(
            commands,
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
        assert!(selections.is_empty());
        assert!(app.construction_menu_drag.is_none());
    }

    #[test]
    fn script_menu_pointer_resource_failure_never_clicks_through_to_the_world() {
        let mut app = new_classic_running_sandbox_app();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish viewport layout");
        let viewport = app
            .graphics
            .viewport_rect(app.local_owner)
            .expect("local viewport");
        let point = PhysicalPosition::new(
            f64::from(viewport.x) + f64::from(viewport.width) / 2.0,
            f64::from(viewport.y) + f64::from(viewport.height) / 2.0,
        );
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
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
            .expect("install unresolved script menu");

        let hover = app
            .handle_cursor_moved(point)
            .expect_err("hover must propagate the missing pointer resource");
        assert!(matches!(
            &hover,
            EngineError::ClassicMenuParityBoundary { .. }
        ));
        assert!(hover.to_string().contains("{{MISS}}"));
        assert!(app.ingame_pointer.is_some());

        assert!(app.mouse_state.is_none());
        let left = app
            .handle_mouse_button(ElementState::Pressed)
            .expect_err("left-down must fail before world drag handling");
        assert!(matches!(
            &left,
            EngineError::ClassicMenuParityBoundary { .. }
        ));
        assert!(left.to_string().contains("{{MISS}}"));
        assert!(app.mouse_state.is_none());

        let (manager, _events) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.status_text.clear();
        let right = app
            .handle_right_mouse_button(ElementState::Released)
            .expect_err("right-up must fail before network/world context handling");
        assert!(matches!(
            &right,
            EngineError::ClassicMenuParityBoundary { .. }
        ));
        assert!(right.to_string().contains("{{MISS}}"));
        assert!(
            app.status_text.is_empty(),
            "resource failure must not reach the network context-command fallback"
        );
    }

    #[test]
    fn script_menu_pointer_requires_global_resources_before_fallback_layout() {
        let mut app = new_running_sandbox_app();
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame).expect("establish viewport layout");
        let viewport = app
            .graphics
            .viewport_rect(app.local_owner)
            .expect("local viewport");
        let point = PhysicalPosition::new(
            f64::from(viewport.x) + f64::from(viewport.width) / 2.0,
            f64::from(viewport.y) + f64::from(viewport.height) / 2.0,
        );
        let cursor = app
            .engine
            .crew_cursor(app.local_owner)
            .expect("sandbox cursor");
        app.engine
            .apply_object_update(
                cursor,
                ObjectUpdate {
                    menu: Some(Some(two_item_script_menu(cursor))),
                    ..ObjectUpdate::default()
                },
            )
            .expect("install valid script menu");
        app.assets = Arc::new(FrontendAssets::load(None));

        let error = app
            .handle_cursor_moved(point)
            .expect_err("pointer layout must reject missing classic global resources");
        assert!(matches!(
            &error,
            EngineError::ClassicMenuParityBoundary { .. }
        ));
        assert!(error
            .to_string()
            .contains("classic process-global C4GUI bootstrap"));
        assert!(error.to_string().contains("FontRegular: missing"));
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
            assert_eq!(
                error.downcast_ref::<ClassicParityBoundary>(),
                Some(&expected)
            );
            assert!(
                error.to_string().contains("refusing generic Rust fallback"),
                "boundary must explain why the fallback is unreachable: {error:#}"
            );
            assert_eq!(frame, frame_before, "preflight must precede output writes");
            assert_eq!(
                app.graphics.surface().pixels(),
                surface_before.as_slice(),
                "preflight must precede logical-surface writes"
            );
        };

        let upper_board = Arc::make_mut(
            &mut Arc::get_mut(&mut app.assets)
                .expect("frontend assets are app-owned")
                .hud_graphics,
        )
        .upper_board
        .take()
        .expect("classic fixture upper board");
        assert_refusal(&mut app, vec!["UpperBoard.png"]);
        Arc::make_mut(
            &mut Arc::get_mut(&mut app.assets)
                .expect("frontend assets are app-owned")
                .hud_graphics,
        )
        .upper_board = Some(upper_board);

        let background = Arc::make_mut(
            &mut Arc::get_mut(&mut app.assets)
                .expect("frontend assets are app-owned")
                .hud_graphics,
        )
        .background
        .take()
        .expect("classic fixture message-board background");
        assert_refusal(&mut app, vec!["Background.png"]);
        Arc::make_mut(
            &mut Arc::get_mut(&mut app.assets)
                .expect("frontend assets are app-owned")
                .hud_graphics,
        )
        .background = Some(background);

        let (background, upper_board) = {
            let hud = Arc::make_mut(
                &mut Arc::get_mut(&mut app.assets)
                    .expect("frontend assets are app-owned")
                    .hud_graphics,
            );
            (
                hud.background.take().expect("restored background"),
                hud.upper_board.take().expect("restored upper board"),
            )
        };
        assert_refusal(
            &mut app,
            vec!["Background.png", "UpperBoard.png"],
        );
        let hud = Arc::make_mut(
            &mut Arc::get_mut(&mut app.assets)
                .expect("frontend assets are app-owned")
                .hud_graphics,
        );
        hud.background = Some(background);
        hud.upper_board = Some(upper_board);
    }

    #[test]
    fn runtime_f3_and_ingame_music_action_install_the_localized_flash() {
        let mut app = new_running_sandbox_app();
        let configured_music = app.audio.as_ref().map(|audio| audio.options.music_enabled);
        let expected_enabled = !app.audio.as_ref().expect("test audio").music_is_playing();
        let resources = app
            .runtime_flash_resources()
            .expect("process-start flash resources")
            .clone();
        let expected_text = resources.music_on_off(expected_enabled);
        app.handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("bare F3 toggles music");
        assert_eq!(app.runtime_music_enabled, expected_enabled);
        assert_eq!(
            app.audio.as_ref().map(|audio| audio.options.music_enabled),
            configured_music,
            "running global F3 must not change persisted RXMusic"
        );
        let message = app.runtime_flash_message.as_ref().expect("music flash");
        assert_eq!(message.text, expected_text);
        assert_eq!(
            usize::from(message.remaining_draws),
            runtime_flash_stored_bytes(&expected_text, resources.charset)
                .expect("encode expected music flash")
                .len()
                * 2
        );
        let after_down = app.runtime_flash_message.clone();
        app.handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("a repeated F3 down invokes MusicToggle again");
        assert_eq!(app.runtime_music_enabled, !expected_enabled);
        assert_ne!(app.runtime_flash_message, after_down);
        assert_eq!(
            app.audio.as_ref().map(|audio| audio.options.music_enabled),
            configured_music
        );
        let after_repeat = app.runtime_flash_message.clone();
        app.handle_key(VirtualKeyCode::F3, ElementState::Released)
            .expect("MusicToggle has no Up callback");
        assert_eq!(app.runtime_flash_message, after_repeat);

        let mut menu = new_running_sandbox_app();
        let configured_before = menu.audio.as_ref().map(|audio| audio.options.music_enabled);
        menu.ingame_menu.replace(
            menu.local_owner,
            Some(IngameMenuState::options_menu(
                &menu.option_flags(menu.local_owner),
                1,
            )),
        );
        menu.apply_ingame_menu_action(MenuAction::ToggleMusic)
            .expect("Options:Music uses the same live producer");
        assert!(menu.runtime_flash_message.is_some());
        assert_eq!(
            menu.ingame_menu.as_ref().map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::Options)
        );
        if let (Some(before), Some(audio)) = (configured_before, menu.audio.as_ref()) {
            assert_eq!(audio.options.music_enabled, !before);
            assert_eq!(menu.runtime_music_enabled, !before);
        }

        let mut startup = new_running_sandbox_app();
        startup.return_to_menu();
        startup
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("startup F3 does not use the running producer");
        assert!(startup.runtime_flash_message.is_none());
        startup.mode = AppMode::Loading;
        startup
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("loading excludes the running producer");
        assert!(startup.runtime_flash_message.is_none());
    }

    #[test]
    fn graphics_resources_validate_liquid_even_when_animation_disabled() {
        let directory = tempdir().expect("Liquid validation fixture");
        let base_path = directory.path().join("base.c4g");
        let override_path = directory.path().join("override.c4g");
        fs::create_dir(&base_path).expect("base graphics group");
        fs::create_dir(&override_path).expect("override graphics group");

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
        fs::write(base_path.join("C4.pal"), vec![0_u8; GamePalette::BYTE_LEN])
            .expect("base game palette");
        let cached_cursors = Arc::new(CursorAtlas::new(vec![
            Some(ImageData::new(1, 1, vec![1, 2, 3, 255]));
            8
        ]));

        let missing = resolve_game_graphics_resources(
            &[],
            &Group::open(&base_path).expect("open missing-Liquid base"),
            Some(Arc::clone(&cached_cursors)),
            false,
        )
        .err()
        .expect("disabled animation still rejects a missing Liquid resource");
        assert_eq!(
            missing.to_string(),
            "failed to load game graphics resource `Liquid`"
        );
        assert_eq!(
            FrontendAssets::liquid_animation_issue(&missing),
            Some(ClassicGuiBootstrapIssue::missing("Liquid"))
        );

        write_preview_image(
            &base_path.join("Liquid.bmp"),
            [10, 20, 30, 255],
            image::ImageFormat::Bmp,
        );
        fs::write(override_path.join("Liquid.png"), b"not a png")
            .expect("malformed winning Liquid.png");
        let malformed_registration = [LoaderGroupRegistration {
            priority: 200,
            registration_order: 0,
            group: Group::open(&override_path).expect("open malformed override"),
        }];
        let malformed = resolve_game_graphics_resources(
            &malformed_registration,
            &Group::open(&base_path).expect("open valid BMP fallback"),
            Some(Arc::clone(&cached_cursors)),
            false,
        )
        .err()
        .expect("disabled animation still decodes the winning Liquid resource");
        assert_eq!(
            malformed.to_string(),
            "failed to load game graphics resource `Liquid`"
        );
        assert!(format!("{malformed:#}").contains("Liquid.png"));
        let malformed_issue = FrontendAssets::liquid_animation_issue(&malformed)
            .expect("malformed selected Liquid reaches the startup boundary");
        assert!(matches!(
            &malformed_issue,
            ClassicGuiBootstrapIssue {
                resource: "Liquid",
                defect: ClassicGuiBootstrapDefect::Malformed { .. },
            }
        ));
        let mut startup_assets = synthetic_classic_test_assets();
        startup_assets.liquid_animation_issue = Some(malformed_issue.clone());
        assert_eq!(
            startup_assets
                .require_classic_global_gui_bootstrap_resources(&HashMap::new())
                .expect_err("startup must reject a malformed selected Liquid resource"),
            ClassicParityBoundary::GlobalGuiBootstrapResources {
                issues: vec![malformed_issue],
            }
        );

        write_preview_png(
            &override_path.join("Liquid.png"),
            [170, 180, 190, 255],
        );
        let valid_registration = [LoaderGroupRegistration {
            priority: 200,
            registration_order: 0,
            group: Group::open(&override_path).expect("open valid override"),
        }];
        let disabled = resolve_game_graphics_resources(
            &valid_registration,
            &Group::open(&base_path).expect("open valid disabled base"),
            Some(Arc::clone(&cached_cursors)),
            false,
        )
        .expect("valid Liquid is accepted while animation is disabled");
        assert!(disabled.liquid_animation.is_none());

        let enabled = resolve_game_graphics_resources(
            &valid_registration,
            &Group::open(&base_path).expect("open valid enabled base"),
            Some(cached_cursors),
            true,
        )
        .expect("valid Liquid is retained while animation is enabled");
        assert_eq!(
            enabled
                .liquid_animation
                .as_deref()
                .expect("enabled Liquid animation")
                .pixels(),
            [170, 180, 190, 255]
        );
    }

    #[test]
    fn runtime_client_list_renders_with_the_classic_gui_resource_set() {
        let mut app = new_classic_running_sandbox_app();
        let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
        app.control_clients
            .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
        app.handle_key(VirtualKeyCode::F4, ElementState::Pressed)
            .expect("open runtime client list");
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .expect("render runtime client list with exact resources");
        assert!(frame.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn load_frontend_scenarios_discovers_install_entries() {
        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let scenario_dir = install_dir.path().join("Scenarios");
        let alpha_dir = scenario_dir.join("Alpha.c4s");
        fs::create_dir_all(&alpha_dir).unwrap();
        fs::write(
            alpha_dir.join("Scenario.json"),
            br#"{"name":"Alpha Mission"}"#,
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(
            scenarios.len(),
            1,
            "expected discovered scenario without fallback"
        );
        let scenario = &scenarios[0];
        assert_eq!(scenario.identifier, "Alpha.c4s");
        assert_eq!(scenario.title, "Alpha Mission");
        assert!(scenario.is_playable);
        assert_eq!(
            scenario
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("Alpha.c4s")
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_discovers_repository_content() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let install_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .expect("repository root");

        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(install_root))]);
        let scenarios = load_frontend_scenarios();

        assert!(
            scenarios
                .iter()
                .any(|scenario| scenario.identifier != "rust_sandbox"),
            "expected repository content scenarios to be discoverable"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_prefers_user_over_install() {
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let install_scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&install_scenario_dir).unwrap();
        fs::write(
            install_scenario_dir.join("Scenario.json"),
            br#"{"name":"System Alpha"}"#,
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let user_scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&user_scenario_dir).unwrap();
        fs::write(
            user_scenario_dir.join("Scenario.json"),
            br#"{"name":"User Alpha"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(scenarios.len(), 1, "duplicate scenario should be merged");
        let scenario = &scenarios[0];
        assert_eq!(scenario.identifier, "Alpha.c4s");
        assert_eq!(
            scenario.title, "User Alpha",
            "user scenario should override install variant"
        );
        let path = scenario.path.as_ref().expect("scenario path");
        assert!(
            path.starts_with(&user_dir),
            "expected scenario path to point at user overrides"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_fills_missing_preview_from_install() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let install_scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&install_scenario_dir).unwrap();
        fs::write(
            install_scenario_dir.join("Scenario.json"),
            br#"{"name":"Install Alpha"}"#,
        )
        .unwrap();
        write_preview_png(
            &install_scenario_dir.join("Title.png"),
            [0x10, 0x20, 0x30, 0x40],
        );

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let user_scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&user_scenario_dir).unwrap();
        fs::write(
            user_scenario_dir.join("Scenario.json"),
            br#"{"name":"User Alpha"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(scenarios.len(), 1, "duplicate scenario should be merged");
        let scenario = &scenarios[0];
        assert_eq!(scenario.title, "User Alpha");
        let preview = scenario.preview.as_ref().expect("merged preview");
        assert_eq!(preview.width(), 1);
        assert_eq!(preview.height(), 1);
        assert_eq!(preview.pixels(), &[0x10, 0x20, 0x30, 0x40]);

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_merges_folder_children_across_roots() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let install_folder = install_dir.path().join("Scenarios").join("Worlds.c4f");
        fs::create_dir_all(&install_folder).unwrap();
        fs::write(install_folder.join("Folder.txt"), "Title=Worlds\n").unwrap();
        let install_scenario = install_folder.join("Alpha.c4s");
        fs::create_dir_all(&install_scenario).unwrap();
        fs::write(
            install_scenario.join("Scenario.json"),
            br#"{"name":"Alpha Install"}"#,
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let user_folder = user_dir.join("Scenarios").join("Worlds.c4f");
        fs::create_dir_all(&user_folder).unwrap();
        fs::write(user_folder.join("Folder.txt"), "Title=Worlds\n").unwrap();
        let user_scenario = user_folder.join("Beta.c4s");
        fs::create_dir_all(&user_scenario).unwrap();
        fs::write(
            user_scenario.join("Scenario.json"),
            br#"{"name":"Beta User"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(
            scenarios.len(),
            1,
            "duplicate folders should merge instead of duplicating entries"
        );
        let folder = &scenarios[0];
        assert_eq!(folder.identifier, "Worlds.c4f");
        assert!(
            matches!(folder.kind, ScenarioKind::Folder),
            "expected merged entry to remain a folder"
        );
        assert_eq!(
            folder.children.len(),
            2,
            "merged folder should expose children from all roots"
        );
        let identifiers: Vec<_> = folder
            .children
            .iter()
            .map(|child| child.identifier.as_str())
            .collect();
        assert_eq!(
            identifiers,
            vec!["Worlds.c4f/Alpha.c4s", "Worlds.c4f/Beta.c4s"],
            "children should be sorted deterministically"
        );
        let user_entry = folder
            .children
            .iter()
            .find(|child| child.identifier == "Worlds.c4f/Beta.c4s")
            .expect("user scenario present");
        assert_eq!(user_entry.title, "Beta User");
        assert!(
            user_entry
                .path
                .as_ref()
                .map(|path| path.starts_with(&user_dir))
                .unwrap_or(false),
            "user scenario should retain user path"
        );
        let install_entry = folder
            .children
            .iter()
            .find(|child| child.identifier == "Worlds.c4f/Alpha.c4s")
            .expect("install scenario present");
        assert_eq!(install_entry.title, "Alpha Install");
        assert!(
            install_entry
                .path
                .as_ref()
                .map(|path| path.starts_with(&install_dir))
                .unwrap_or(false),
            "install scenario should retain install path"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn scenario_roots_deduplicates_case_insensitive_variants() {
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let install_scenarios = install_dir.path().join("Scenarios");
        fs::create_dir_all(&install_scenarios).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let paths = AppPaths::discover().expect("discover app paths");
        let roots = scenario_roots(&paths);

        let expected_key = scenario_root_key(&install_scenarios);
        let duplicate_count = roots
            .iter()
            .map(|root| scenario_root_key(&root.path))
            .filter(|key| key == &expected_key)
            .count();

        assert_eq!(
            duplicate_count, 1,
            "install scenarios path should appear once despite case variants"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn start_real_scenario_loads_from_disk() {
        clonk_logging::init();

        let fixture = tempdir().unwrap();
        let user_dir = fixture.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
        let scripts_dir = scenario_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Alpha Mission\n",
        )
        .unwrap();
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
        .unwrap();
        fs::write(scripts_dir.join("mover.aul"), walker_script()).unwrap();

        let (_guard, paths) = exact_loader_test_paths(&user_dir, None);

        let mut app = GameApp::new(
            320,
            200,
            AudioOptions::default(),
            Some(&paths),
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app");

        let scenario = app
            .scenario_catalog
            .get("Alpha.c4s")
            .cloned()
            .expect("scenario discovered");
        assert_eq!(scenario.title, "Alpha Mission");

        let frontend_music = app
            .audio
            .as_ref()
            .expect("test audio")
            .system
            .load_music(&silent_pcm_wav(5_000))
            .expect("load frontend music fixture");
        app.audio
            .as_ref()
            .expect("test audio")
            .system
            .play_music(&frontend_music, true)
            .expect("start frontend music fixture");

        app.start_scenario(scenario).expect("start disk scenario");
        assert!(
            app.audio
                .as_ref()
                .expect("test audio")
                .system
                .music_is_playing(),
            "scenario initialization must fade rather than halt frontend music"
        );
        assert!(app.resume_frontend_music_after_fade);
        wait_for_running(&mut app);

        assert!(
            matches!(app.mode, AppMode::Running),
            "mode should be Running"
        );
        assert_eq!(app.scenario_label, "Alpha Mission");
        assert_eq!(app.fallback_ground, 72);
        assert!(
            app.snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "Mover"),
            "expected spawned Mover object"
        );
        assert!(
            app.focus_id.is_some(),
            "expected focus to be assigned for crew member"
        );
        assert_eq!(
            app.active_scenario
                .as_ref()
                .and_then(|active| active.path.as_ref())
                .map(|path| path.as_path()),
            Some(scenario_dir.as_path()),
            "active scenario should track disk path"
        );
    }

    #[test]
    fn install_definition_resolver_prefers_global_pack_before_folder_local_collision() {
        fn write_definition(root: &Path, directory: &str, id: &str, value: i32) {
            let definition = root.join(directory);
            fs::create_dir_all(&definition).expect("definition directory");
            fs::write(
                definition.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\nValue={value}\n"),
            )
            .expect("definition core");
            write_test_definition_graphics(&definition);
        }

        let dir = tempdir().expect("tempdir");
        let content = dir.path().join("content");
        let global = content.join("Objects.c4d");
        let family = content.join("Tutorial.c4f");
        let local = family.join("Objects.c4d");
        let scenario = family.join("Tutorial01.c4s");
        write_definition(&global, "Global.c4d", "GLOB", 1);
        write_definition(&global, "Shared.c4d", "SAME", 1);
        let global_graphics = global.join("Graphics.c4g");
        fs::create_dir_all(&global_graphics).expect("definition graphics group");
        write_preview_png(
            &global_graphics.join("DefinitionSky.png"),
            [0x12, 0x34, 0x56, 0xff],
        );
        write_definition(&local, "Local.c4d", "LOCL", 2);
        write_definition(&local, "Shared.c4d", "SAME", 2);
        fs::create_dir_all(&scenario).expect("scenario directory");
        fs::write(
            scenario.join("Scenario.txt"),
            "[Head]\nTitle=Collision\n\n[Definitions]\nDefinition1=Objects.c4d\n\n[Landscape]\nSky=DefinitionSky\n",
        )
        .expect("scenario core");

        let scenario_group = Group::open(&scenario).expect("scenario group");
        let resolver = InstallDefinitionResolver::new(None);
        let groups = resolver
            .resolve_definition_groups(&scenario_group, "Objects.c4d")
            .expect("colliding definition groups resolve");
        let roots = groups
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect::<Vec<_>>();

        assert_eq!(
            roots,
            [global.clone()],
            "the resolver returns the one explicit global resource; InitDefs adds folder-local resources separately"
        );

        let loaded = Scenario::load_from_path_with(&scenario, &resolver)
            .expect("collision scenario loads through app resolver");
        assert_eq!(
            loaded.definition_resource_paths(),
            [global.clone(), family.clone()]
        );
        assert_eq!(
            loaded
                .definition_root_groups()
                .iter()
                .map(|group| group.root().to_path_buf())
                .collect::<Vec<_>>(),
            [global, family],
            "folder-local definitions are appended to C++'s final NRT_Definitions vector"
        );
        assert_eq!(
            &loaded
                .sky()
                .and_then(|sky| sky.surface.as_ref())
                .expect("definition-pack SkyDef surface")
                .pixels()[..4],
            &[0x12, 0x34, 0x56, 0xff],
            "the retained definition root participates in the live graphics chain"
        );
        let mut engine = Engine::new();
        loaded
            .apply(&mut engine)
            .expect("collision scenario applies");
        assert!(engine.definition_ids().any(|id| id == "GLOB"));
        assert!(engine.definition_ids().any(|id| id == "LOCL"));
        assert_eq!(
            engine.definition_value("SAME"),
            Some(2),
            "the later folder-local pass overloads the explicit global pack"
        );
    }

    fn assert_parent_resource_order(scenario: &Group, inner: &Path, outer: &Path) {
        let resolver = InstallDefinitionResolver::new(None);
        let graphics = resolver
            .resolve_graphics_groups(scenario)
            .expect("graphics parent chain resolves");
        assert_eq!(
            graphics
                .iter()
                .map(|group| group.root().to_path_buf())
                .collect::<Vec<_>>(),
            [inner.join("Graphics.c4g"), outer.join("Graphics.c4g")]
        );
        assert_eq!(
            graphics[0].read_file("Source.txt").expect("inner graphic"),
            b"inner graphics"
        );
        assert_eq!(
            graphics[1].read_file("Source.txt").expect("outer graphic"),
            b"outer graphics"
        );

        let materials = resolver
            .resolve_material_groups(scenario)
            .expect("material parent chain resolves");
        assert_eq!(
            materials
                .iter()
                .map(|group| group.root().to_path_buf())
                .collect::<Vec<_>>(),
            [inner.join("Material.c4g"), outer.join("Material.c4g")]
        );
        assert_eq!(
            materials[0]
                .read_file("Source.txt")
                .expect("inner material"),
            b"inner materials"
        );
        assert_eq!(
            materials[1]
                .read_file("Source.txt")
                .expect("outer material"),
            b"outer materials"
        );
    }

    #[test]
    fn install_definition_resolver_opens_packed_parent_resource_chain() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();
        let dir = tempdir().expect("tempdir");
        let inner_png_path = dir.path().join("inner.png");
        let outer_png_path = dir.path().join("outer.png");
        write_preview_png(&inner_png_path, [1, 2, 3, 255]);
        write_preview_png(&outer_png_path, [9, 8, 7, 255]);
        let inner_png = fs::read(inner_png_path).expect("read inner PNG");
        let outer_png = fs::read(outer_png_path).expect("read outer PNG");
        let outer_graphics = packed_test_group(&[
            ("Source.txt", false, b"outer graphics"),
            ("Priority.png", false, outer_png.as_slice()),
        ]);
        let outer_materials = packed_test_group(&[
            ("Source.txt", false, b"outer materials"),
            ("TexMap.txt", false, b"1=Earth-Rough\n"),
            (
                "Earth.c4m",
                false,
                b"[Material]\nName=Earth\nDensity=50\n",
            ),
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
            (
                "Earth.c4m",
                false,
                b"[Material]\nName=Earth\nDensity=100\n",
            ),
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
        fs::write(&outer_path, outer).expect("write packed outer folder");
        let scenario_group = open_group_path_for_folder_map(
            &outer_path.join("Inner.c4f/Scen.c4s"),
        )
        .expect("open packed scenario through its parent chain");

        assert_parent_resource_order(
            &scenario_group,
            &outer_path.join("Inner.c4f"),
            &outer_path,
        );
        let scenario_path = outer_path.join("Inner.c4f/Scen.c4s");
        preflight_offline_startup(&scenario_path)
            .expect("packed nested scenario passes offline startup preflight");
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
        .expect("packed nested scenario starts with parent resources");
        assert_eq!(
            &loaded
                .sky()
                .and_then(|sky| sky.surface.as_ref())
                .expect("inner parent sky")
                .pixels()[..4],
            &[1, 2, 3, 255]
        );
        assert_eq!(
            load_material_render_info(&scenario_path, None).get("earth"),
            Some(
                &clonk_frontend::MaterialRenderInfo::new([0; 9], [0; 6], None, 0, 100)
                    .with_placement(70)
            )
        );
        assert_eq!(
            load_scenario_material_textures(&scenario_path, None)
                .get("rough")
                .expect("inner parent material texture")
                .surface32_image()
                .expect("rough texture is PNG-backed")
                .pixels(),
            &[1, 2, 3, 255]
        );
        reset_cached_app_paths();
    }

    #[test]
    fn install_definition_resolver_keeps_unpacked_parent_resource_chain() {
        let dir = tempdir().expect("tempdir");
        let outer = dir.path().join("Outer.c4f");
        let inner = outer.join("Inner.c4f");
        let scenario = inner.join("Scen.c4s");
        for (parent, graphic, material) in [
            (&outer, b"outer graphics".as_slice(), b"outer materials".as_slice()),
            (&inner, b"inner graphics".as_slice(), b"inner materials".as_slice()),
        ] {
            fs::create_dir_all(parent.join("Graphics.c4g")).expect("graphics group");
            fs::create_dir_all(parent.join("Material.c4g")).expect("material group");
            fs::write(parent.join("Graphics.c4g/Source.txt"), graphic)
                .expect("write graphic marker");
            fs::write(parent.join("Material.c4g/Source.txt"), material)
                .expect("write material marker");
        }
        fs::create_dir_all(&scenario).expect("scenario group");
        let scenario_group = Group::open(&scenario).expect("open unpacked scenario");

        assert_parent_resource_order(&scenario_group, &inner, &outer);
    }

    #[test]
    fn install_definition_resolver_prioritizes_scenario_graphics_over_folder() {
        let dir = tempdir().expect("tempdir");
        let family = dir.path().join("Tutorial.c4f");
        let scenario = family.join("Tutorial01.c4s");
        let scenario_graphics = scenario.join("Graphics.c4g");
        let folder_graphics = family.join("Graphics.c4g");
        fs::create_dir_all(&scenario_graphics).expect("scenario graphics");
        fs::create_dir_all(&folder_graphics).expect("folder graphics");
        fs::write(scenario_graphics.join("Shared.png"), b"scenario")
            .expect("scenario graphic");
        fs::write(folder_graphics.join("Shared.png"), b"folder").expect("folder graphic");

        let scenario_group = Group::open(&scenario).expect("scenario group");
        let graphics = InstallDefinitionResolver::new(None)
            .resolve_graphics_groups(&scenario_group)
            .expect("graphics chain resolves");

        assert_eq!(graphics.len(), 2);
        assert_eq!(graphics[0].root(), scenario_graphics.as_path());
        assert_eq!(graphics[1].root(), folder_graphics.as_path());
        assert_eq!(
            graphics[0].read_file("Shared.png").expect("local graphic"),
            b"scenario"
        );
    }

    #[test]
    fn definition_pack_gui_sheet_wins_the_active_override_selection() {
        let _env_lock = crate::tests::env_lock().lock();
        let root = tempdir().expect("definition GUI fixture");
        let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
        let scenario = content.join("Scenario.c4s");
        fs::create_dir_all(&scenario).expect("scenario group");
        fs::write(scenario.join("Scenario.txt"), "[Head]\nTitle=GUI Override\n")
            .expect("scenario core");

        let definition = content.join("Objects.c4d");
        let definition_graphics = definition.join("Graphics.c4g");
        fs::create_dir_all(&definition_graphics).expect("definition Graphics.c4g");
        write_preview_png(
            &definition_graphics.join("GUIBigArrows.png"),
            [0x12, 0x34, 0x56, 0xff],
        );
        let base_graphics = root.path().join("planet/Graphics.c4g");
        fs::create_dir_all(&base_graphics).expect("base Graphics.c4g");
        write_preview_png(
            &base_graphics.join("GUIBigArrows.png"),
            [0xaa, 0xbb, 0xcc, 0xff],
        );

        let scenario_group = Group::open(&scenario).expect("scenario group");
        let head = ScenarioLoaderHead::load_from_group(&scenario_group).expect("scenario head");
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
        .expect("definition root registration");
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].priority, 1);
        assert_eq!(registrations[0].group.root(), definition.as_path());

        let resolution = resolve_classic_global_gui_sheet_overrides(
            &registrations,
            &Group::open(&base_graphics).expect("base graphics group"),
        );
        assert!(
            resolution.failures.is_empty(),
            "a decodable definition-pack sheet must not fail: {:?}",
            resolution.failures
        );
        let sheet = resolution
            .overrides
            .iter()
            .find(|sheet| sheet.stem == "GUIBigArrows")
            .expect("the definition-pack GUI sheet wins over the priority-zero base sheet");
        assert_eq!(sheet.canonical_name, "GUIBigArrows.png");
        assert_eq!(
            sheet.source,
            format!("{}:GUIBigArrows.png", definition_graphics.display())
        );
        assert_eq!(
            &sheet.image.pixels()[..4],
            &[0x12, 0x34, 0x56, 0xff],
            "the applied override carries the winning group's decoded pixels"
        );
    }

    #[test]
    fn install_definition_resolver_handles_case_insensitive_paths() {
        clonk_logging::init();
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let objects_dir = planet_dir.join("objects.ocd").join("clonk.c4d");
        fs::create_dir_all(&objects_dir).unwrap();
        fs::write(
            objects_dir.join("DefCore.txt"),
            "[DefCore]\nid=CLNK\nName=Clonk\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
        )
        .unwrap();
        fs::write(objects_dir.join("Script.c"), walker_script()).unwrap();

        let scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&scenario_dir).unwrap();
        let local_shadow = scenario_dir.join("objects.ocd").join("clonk.c4d");
        fs::create_dir_all(&local_shadow).unwrap();
        fs::write(
            local_shadow.join("DefCore.txt"),
            "[DefCore]\nid=LOCL\nName=Local shadow\nCategory=1\n",
        )
        .unwrap();
        let scenario_group = Group::open(&scenario_dir).unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let paths = cached_app_paths().expect("discover app paths");
        let resolver = InstallDefinitionResolver::new(Some(paths.clone()));
        let groups = resolver
            .resolve_definition_groups(&scenario_group, "Objects.ocd\\Clonk.c4d")
            .expect("resolve definition groups");
        let first_root = groups.first().expect("one prioritized definition").root();
        assert!(
            first_root
                .to_string_lossy()
                .eq_ignore_ascii_case(&objects_dir.to_string_lossy()),
            "ExePath definitions precede scenario/folder-local collisions: {}",
            first_root.display()
        );
        assert!(!first_root.starts_with(&scenario_dir));
        let found_definition = groups.iter().any(|group| {
            group
                .root()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with("clonk.c4d")
        });
        assert!(found_definition, "expected to locate definition group");

        let absolute_groups = resolver
            .resolve_definition_groups(&scenario_group, &objects_dir.to_string_lossy())
            .expect("resolve retained absolute definition resource");
        assert_eq!(absolute_groups.len(), 1);
        assert_eq!(absolute_groups[0].root(), objects_dir.as_path());

        let local_only = scenario_dir.join("OnlyLocal.c4d");
        fs::create_dir_all(&local_only).expect("scenario-local definition fixture");
        assert!(matches!(
            resolver.resolve_definition_groups(&scenario_group, "OnlyLocal.c4d"),
            Err(ScenarioError::LegacyDefinitionNotFound { path }) if path == "OnlyLocal.c4d"
        ));

        reset_cached_app_paths();
    }

    #[test]
    fn load_install_definitions_discovers_mixed_case_objects_group() {
        clonk_logging::init();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let objects_dir = planet_dir.join("objects.c4d").join("clonk.c4d");
        fs::create_dir_all(&objects_dir).unwrap();
        fs::write(
            objects_dir.join("DefCore.txt"),
            "[DefCore]\nid=Clonk\nName=Invalid\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
        )
        .unwrap();
        fs::write(objects_dir.join("Script.c"), walker_script()).unwrap();
        let long_id = planet_dir.join("objects.c4d").join("clone.c4d");
        fs::create_dir_all(&long_id).unwrap();
        fs::write(
            long_id.join("DefCore.txt"),
            "[DefCore]\nid=WIPFEX\nName=Wipf\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
        )
        .unwrap();
        fs::write(long_id.join("Script.c"), walker_script()).unwrap();
        write_test_definition_graphics(&long_id);
        let zero_id = planet_dir.join("objects.c4d").join("zero.c4d");
        fs::create_dir_all(&zero_id).unwrap();
        fs::write(
            zero_id.join("DefCore.txt"),
            "[DefCore]\nid=0000\nName=Zero\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
        )
        .unwrap();
        fs::write(zero_id.join("ActMap.txt"), "not an action map").unwrap();
        let canonical = planet_dir.join("objects.c4d").join("canonical.c4d");
        fs::create_dir_all(&canonical).unwrap();
        fs::write(
            canonical.join("DefCore.txt"),
            "[DefCore]\nid=CLNK\nName=Clonk\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
        )
        .unwrap();
        fs::write(canonical.join("Script.c"), walker_script()).unwrap();
        write_test_definition_graphics(&canonical);

        let missing_graphics = planet_dir.join("objects.c4d").join("missing.c4d");
        fs::create_dir_all(&missing_graphics).unwrap();
        fs::write(
            missing_graphics.join("DefCore.txt"),
            "[DefCore]\nid=MISS\nName=Missing graphics\nCategory=1\n",
        )
        .unwrap();

        let old_gfx = planet_dir.join("objects.c4d").join("oldgfx.c4d");
        fs::create_dir_all(&old_gfx).unwrap();
        fs::write(
            old_gfx.join("DefCore.txt"),
            "[DefCore]\nid=OLDG\nName=Old graphics\nCategory=1\nNeededGfxMode=2\n",
        )
        .unwrap();
        write_test_definition_graphics(&old_gfx);

        let particle = planet_dir.join("objects.c4d").join("particle.c4d");
        fs::create_dir_all(&particle).unwrap();
        fs::write(
            particle.join("DefCore.txt"),
            "[DefCore]\nid=PART\nName=Particle\nCategory=1\n",
        )
        .unwrap();
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
        .unwrap();
        let particle_override = particle.join("Override.c4d");
        fs::create_dir_all(&particle_override).unwrap();
        fs::write(
            particle_override.join("Particle.txt"),
            "[Particle]\nName=InstallParticle\nInitFn=StdInit\nExecFn=StdExec\nDrawFn=Std\nFace=0,0,1,1,0,0\n",
        )
        .unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([9, 8, 7, 255]))
            .save(particle_override.join("Graphics.png"))
            .unwrap();
        let invalid_override = particle_override.join("Invalid.c4d");
        fs::create_dir_all(&invalid_override).unwrap();
        fs::write(
            invalid_override.join("Particle.txt"),
            "[Particle]\nName=InstallParticle\nInitFn=StdInit\nExecFn=StdExec\nDrawFn=MissingDrawProc\nFace=0,0,1,1,0,0\n",
        )
        .unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([99, 98, 97, 255]))
            .save(invalid_override.join("Graphics.png"))
            .unwrap();

        let bad_overlay = planet_dir.join("objects.c4d").join("overlay.c4d");
        fs::create_dir_all(&bad_overlay).unwrap();
        fs::write(
            bad_overlay.join("DefCore.txt"),
            "[DefCore]\nid=OVLY\nName=Bad overlay\nCategory=1\nColorByOwner=1\n",
        )
        .unwrap();
        write_test_definition_graphics(&bad_overlay);
        image::RgbaImage::from_pixel(2, 1, image::Rgba([32, 32, 32, 255]))
            .save(bad_overlay.join("Overlay.png"))
            .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let paths = cached_app_paths().expect("discover app paths");
        let mut engine = Engine::new();
        let spawn =
            load_install_definitions(&mut engine, &paths, None).expect("load install definitions");
        assert_eq!(spawn.as_deref(), Some("CLNK"));
        assert!(
            engine
                .definition_ids()
                .any(|id| id == "CLNK"),
            "expected Clonk definition to be registered"
        );
        assert!(engine.definition_ids().any(|id| id == "WIPF"));
        assert!(!engine.definition_ids().any(|id| id == "Clon"));
        for rejected in ["MISS", "OLDG", "OVLY", "PART"] {
            assert!(!engine.definition_ids().any(|id| id == rejected));
        }
        let particle = engine
            .particle_system()
            .get_def("InstallParticle")
            .expect("Particle.txt group registers through production traversal");
        assert_eq!(particle.length, 1);
        assert_eq!(
            particle.graphics.as_ref().unwrap().image.pixels(),
            [9, 8, 7, 255],
            "later valid overload wins and a later invalid overload preserves it"
        );
        let particle_sprites = particle_sprite_map(&engine);
        assert_eq!(
            particle_sprites["InstallParticle"].image.pixels(),
            [9, 8, 7, 255],
            "frontend registry receives the final post-overload image"
        );

        let objects_group = Group::open(planet_dir.join("objects.c4d")).unwrap();
        assert!(
            find_definition_in_group(&objects_group, "Clon")
                .expect("lowercase ID lookup skips")
                .is_none()
        );
        assert!(
            find_definition_in_group(&objects_group, "0000")
                .expect("invalid lookup skips")
                .is_none()
        );
        for rejected in ["MISS", "OLDG", "OVLY", "PART"] {
            assert!(
                find_definition_in_group(&objects_group, rejected)
                    .expect("load-ladder rejection remains nonfatal")
                    .is_none()
            );
        }
        assert_eq!(
            find_definition_in_group(&objects_group, "WIPF")
                .expect("truncated lookup succeeds")
                .expect("WIPF exists")
                .core
                .id,
            "WIPF"
        );

        reset_cached_app_paths();
    }
