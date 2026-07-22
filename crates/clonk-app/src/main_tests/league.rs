// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

    #[test]
    fn classic_command_line_join_urls_and_update_stub_are_disjoint() {
        // C++ initializes and serializes the host C4XVERBUILD in every game
        // reference (oracle-src-pinned src/C4Network2Reference.cpp:79,100-102;
        // src/C4GameVersion.h:35-37).
        let join = parse_classic_command_line(&[OsString::from("ClOnK:///host:11112///")]);
        assert_eq!(join.direct_join.as_deref(), Some("host:11112"));
        assert_eq!(join.network_active, Some(true));
        assert!(!join.update_requested);

        let update = parse_classic_command_line(&[OsString::from("CLONK:///UpDaTe///")]);
        assert_eq!(update.direct_join, None);
        assert_eq!(update.network_active, None);
        assert!(update.update_requested);
        let mut update_app = new_state_only_menu_app(320, 200);
        update_app
            .apply_classic_command_line(&update)
            .expect("queue classic update hand-off");
        assert!(update_app.auto_open_update_dialog);

        let direct = parse_classic_command_line(
            &[
                "/join:127.0.0.1:11112",
                "/observe",
                "/tcpport:2222",
                "/udpport:3333",
                "/pass:secret",
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>(),
        );
        let endpoint = classic_direct_reference_endpoint("127.0.0.1", None)
            .expect("classic direct reference endpoint");
        assert_eq!(
            endpoint,
            clonk_network::ReferenceEndpoint::Address(SocketAddr::from((
                [127, 0, 0, 1],
                clonk_network::DEFAULT_REFERENCE_PORT,
            )))
        );
        assert_eq!(
            classic_direct_reference_endpoint(
                direct.direct_join.as_deref().expect("parsed join address"),
                None,
            )
            .expect("explicit reference endpoint"),
            clonk_network::ReferenceEndpoint::Address(SocketAddr::from((
                [127, 0, 0, 1],
                11_112,
            )))
        );
        assert_eq!(
            classic_direct_reference_endpoint("games.example.test", None)
                .expect("hostname reference endpoint"),
            clonk_network::ReferenceEndpoint::Url(format!(
                "http://games.example.test:{}/",
                clonk_network::DEFAULT_REFERENCE_PORT,
            ))
        );
        let game_address = SocketAddr::from(([127, 0, 0, 1], 41_234));
        let reference = clonk_network::NetworkGameReference {
            build: clonk_network::CURRENT_GAME_BUILD + 2,
            source_address: SocketAddr::from((
                [127, 0, 0, 1],
                clonk_network::DEFAULT_REFERENCE_PORT,
            )),
            addresses: vec![clonk_network::NetworkAddress::new(
                clonk_network::NetworkProtocol::Tcp,
                game_address,
            )],
            ..clonk_network::NetworkGameReference::default()
        };
        let settings = classic_client_settings_for_reference(
            &reference,
            "Player".to_string(),
            None,
            None,
            &direct,
        )
        .expect("classic direct client settings");
        assert_eq!(
            settings.compatibility_build,
            clonk_network::CURRENT_GAME_BUILD + 2
        );
        assert_eq!(settings.server_addresses[0].endpoint, game_address);
        assert!(settings.observer);
        assert_eq!(
            settings.mesh_tcp_bind_address.map(|address| address.port()),
            Some(2222)
        );
        assert_eq!(
            settings.mesh_udp_bind_address.map(|address| address.port()),
            Some(3333)
        );
        assert_eq!(settings.password.as_bytes(), b"secret");
    }

    #[test]
    fn staged_host_prebind_accepts_league_signup_and_rejects_missing_resource() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated host preflight user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let mut app = new_menu_app_with_paths(640, 480, &paths);
        let league = GameOptionValues {
            league_server_signup: true,
            ..GameOptionValues::default()
        };
        app.scenario_game_options =
            GameOptionButtons::new(GameOptionContext::NetworkHostSelector, league);
        let staged = app
            .prepare_network_host_scenario(
                {
                    let mut scenario = FrontendScenario::fallback();
                    scenario.path = Some(repository.join("content/Tutorial.c4f/Tutorial01.c4s"));
                    scenario
                },
                ScenarioDefinitionLoad::Seed {
                    modules: vec!["Objects.c4d".to_string()],
                    definition_root: None,
                },
            )
            .expect("league signup is represented by the lifecycle driver");
        assert!(staged.options.league_server_signup);
        assert!(app.network.is_none());
        assert!(app.startup_network_connection.is_none());

        app.scenario_game_options = GameOptionButtons::new(
            GameOptionContext::NetworkHostSelector,
            GameOptionValues::default(),
        );
        Arc::get_mut(&mut app.assets)
            .expect("test owns frontend assets")
            .startup_dialog_images
            .remove("GUIContext.png");
        let error = app
            .prepare_network_host_scenario(
                {
                    let mut scenario = FrontendScenario::fallback();
                    scenario.path = Some(repository.join("content/Tutorial.c4f/Tutorial01.c4s"));
                    scenario
                },
                ScenarioDefinitionLoad::Seed {
                    modules: vec!["Objects.c4d".to_string()],
                    definition_root: None,
                },
            )
            .err()
            .expect("missing GUIContext is rejected before bind");
        assert!(matches!(
            error.downcast_ref::<ClassicParityBoundary>(),
            Some(ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources { detail }))
                if detail.contains("GUIContext.png")
        ));
        assert!(app.network.is_none());
        assert!(app.startup_network_connection.is_none());
    }

    #[test]
    fn classic_host_start_honors_the_league_split_screen_gate() {
        let mut app = new_menu_app(640, 480);
        install_test_classic_host_team_lobby(&mut app);
        app.network_is_league = true;
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
            player_name: "Exact Host".to_string(),
            prepared: None,
        }));
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
            countdown_seconds: 5,
            check_league_rules: true,
            confirm_unassociated_savegame_players: false,
        }])
        .expect("league rule violation opens the native warning");

        assert!(commands.take_submitted_lobby_countdowns().is_empty());
        assert!(app.host_lobby_countdown.is_none());
        let warning = app.message_dialogs.last().expect("league warning dialog");
        assert_eq!(warning.state.caption(), "League error");
        assert_eq!(
            warning.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::Standard(46)
        );
        assert!(warning.state.message().contains("Chooser"));
        assert!(warning.state.message().contains("Companion"));
    }

    #[test]
    fn classic_league_start_removes_a_known_remote_split_screen_client() {
        let mut app = new_menu_app(640, 480);
        let (chooser, companion) = install_test_classic_host_team_lobby(&mut app);
        app.control_player_infos.replace_snapshot(
            8,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![
                    chooser,
                    clonk_engine::ControlPlayerInfoEntry {
                        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                        team: 4,
                        ..Default::default()
                    },
                    companion,
                ],
                by_client: 0,
            }],
        );
        app.control_clients.replace_snapshot([
            message_client(0, b"Exact Host"),
            message_client(7, b"Remote"),
        ]);
        app.network_is_league = true;
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
            player_name: "Exact Host".to_string(),
            prepared: None,
        }));
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
            countdown_seconds: 5,
            check_league_rules: true,
            confirm_unassociated_savegame_players: false,
        }])
        .expect("remote league violation is removed before starting");

        assert_eq!(
            commands.take_submitted_client_removes(),
            vec![clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: LegacyCString::from_bytes(b"Players Chooser and Companion would be playing against each other in split-screen. This is disallowed in league games!".to_vec()).unwrap(),
                by_client: 0,
            }]
        );
        assert_eq!(app.host_lobby_countdown, Some(HostLobbyCountdown::new()));
        assert!(app.message_dialogs.is_empty());
    }

    #[test]
    fn l016_forwarded_help_clear_kick_and_observer_commands_stay_in_lobby() {
        let mut app = new_real_classic_menu_app(640, 480);
        let (_events, mut commands) = install_classic_host_network_stub(&mut app);
        app.control_clients.replace_snapshot([
            message_client(0, b"Exact Host"),
            message_client(7, b"Remote"),
        ]);

        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
            "/observer Remote".to_string(),
        ))
        .expect("submit observer command");
        assert_eq!(
            commands.take_submitted_client_updates(),
            [clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                client_id: 7,
                data: 0,
                by_client: 0,
            }]
        );

        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
            "/kick Remote".to_string(),
        ))
        .expect("submit kick command");
        let removals = commands.take_submitted_client_removes();
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].client_id, 7);
        assert_eq!(removals[0].by_client, 0);
        assert_eq!(removals[0].reason.as_bytes(), b"kicked from messageboard");

        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
            "/joinplr definitely-missing-l016.c4p".to_string(),
        ))
        .expect("missing join-player command stays in the lobby");
        assert_eq!(
            app.classic_host_lobby
                .as_ref()
                .unwrap()
                .controller
                .logs()
                .last()
                .map(|line| line.text.as_str()),
            Some("Cannot join player definitely-missing-l016.c4p: File not found!")
        );

        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/help".to_string()))
            .expect("render lobby help locally");
        assert!(app
            .classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .iter()
            .any(|line| line.text.contains("/set maxplayer")));
        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/clear".to_string()))
            .expect("clear lobby log locally");
        assert!(app
            .classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .is_empty());
        app.process_classic_lobby_chat_request(LobbyChatRequest::OpenExternalDialog)
            .expect("external-chat button opens the implemented classic dialog");
        assert!(app.classic_host_lobby.is_some());
        assert!(app.external_irc_dialog_visible);
    }

    #[test]
    fn l104_network_start_wait_kick_click_reuses_direct_and_league_paths() {
        let setup = |league: bool| {
            let mut app = new_menu_app(640, 480);
            let (network, _events, commands) =
                NetworkManager::test_stub_with_commands_for_client_id(0);
            app.network = Some(network);
            app.network_mode = Some(NetworkMode::Host(host_network_settings()));
            app.network_is_league = league;
            app.control_clients.replace_snapshot([
                message_client(0, b"Host"),
                message_client(7, b"Remote"),
            ]);
            if league {
                app.control_player_infos.replace_snapshot(
                    1,
                    [clonk_engine::PlayerInfoControlData {
                        client_id: 7,
                        players: vec![clonk_engine::ControlPlayerInfoEntry {
                            id: 1,
                            name: LegacyCString::from_bytes(b"League player".to_vec()).unwrap(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                );
            }
            app.mode = AppMode::Loading;
            app.begin_network_start_wait(clonk_network::NetworkStatus {
                state: clonk_network::NETWORK_STATE_GO,
                control_mode: 1,
                target_tick: 4,
            });
            app.show_reached_network_start_wait()
                .expect("show host start wait");
            let layout = app.network_start_wait_layout().expect("visible wait layout");
            let kick = layout
                .clients
                .iter()
                .find(|row| row.client_id == 7)
                .and_then(|row| row.kick_button)
                .expect("remote kick button");
            let point = GuiPoint::new(
                (kick.x + kick.w / 2) as f32,
                (kick.y + kick.h / 2) as f32,
            );
            physical_left_click_with_modifiers(
                &mut app,
                point,
                ModifiersState::empty(),
                ModifiersState::empty(),
            );
            assert!(app
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible));
            (app, commands)
        };

        let (_direct, mut direct_commands) = setup(false);
        assert_eq!(
            direct_commands.take_submitted_client_removes(),
            vec![clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: LegacyCString::from_bytes(
                    b"kicked from startup waiting dialog".to_vec()
                )
                .unwrap(),
                by_client: 0,
            }]
        );
        assert!(direct_commands.take_submitted_votes().is_empty());

        let (_league, mut league_commands) = setup(true);
        assert_eq!(
            league_commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 0,
            }]
        );
        assert!(league_commands.take_submitted_client_removes().is_empty());
    }

    #[test]
    fn l081_remove_aborts_countdown_before_swap_removed_update_and_clears_league_password() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated league configuration");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "Network", "LeaguePassword", "remembered secret")
            .expect("seed league password");

        let mut app = new_menu_app(640, 480);
        app.app_paths = Some(paths.clone());
        let (chooser, companion) = install_test_classic_host_team_lobby(&mut app);
        let (network, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(0);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        app.league_auth_session = Some(clonk_network::LeagueAuthRequestHead {
            password: LegacyCString::from_bytes(b"remembered secret".to_vec()).unwrap(),
            ..Default::default()
        });
        app.network_is_league = true;
        app.host_lobby_countdown = Some(HostLobbyCountdown::with_seconds(5));
        app.apply_lobby_countdown_presentation(clonk_network::LobbyCountdownPacket::new(5));

        app.process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Player(chooser.id),
            position: GuiPoint::new(200.0, 150.0),
        }])
        .expect("player context opens");
        assert!(
            app.handle_context_menu_key(VirtualKeyCode::R, ElementState::Pressed)
                .expect("activate Remove hotkey")
        );

        assert_eq!(
            commands.take_lobby_player_update_commands(),
            vec![
                crate::network::TestLobbyPlayerUpdateCommand::Countdown(
                    clonk_network::LobbyCountdownPacket::new(
                        clonk_network::LobbyCountdownPacket::ABORT,
                    ),
                ),
                crate::network::TestLobbyPlayerUpdateCommand::PlayerInfo(
                    clonk_network::PlayerInfoUpdateRequest {
                        client_id: 0,
                        flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                        players: vec![companion],
                    },
                ),
            ],
            "RemoveInfo swap-removes the target only after the host abort packet"
        );
        assert!(app.host_lobby_countdown.is_none());
        assert!(!app
            .classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .countdown()
            .is_locked());
        assert!(app
            .league_auth_session
            .as_ref()
            .expect("league session")
            .password
            .is_empty());
        assert!(
            load_league_auth_settings(Some(&paths)).password.is_empty(),
            "league removal rerequires authentication"
        );
    }

    #[test]
    fn classic_lobby_remote_context_kicks_directly_or_starts_league_vote() {
        let setup = |league: bool| {
            let mut app = new_menu_app(640, 480);
            install_test_classic_host_lobby(&mut app);
            let (manager, _events, commands) = NetworkManager::test_stub_with_commands();
            app.network = Some(manager);
            app.network_mode = Some(NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                player_name: "Host".to_string(),
                prepared: None,
            }));
            app.network_is_league = league;
            app.control_clients.replace_snapshot([
                clonk_engine::ClientCoreControlData {
                    client_id: 0,
                    activated: true,
                    name: LegacyCString::from_bytes(b"Host".to_vec()).unwrap(),
                    ..Default::default()
                },
                clonk_engine::ClientCoreControlData {
                    client_id: 7,
                    activated: true,
                    name: LegacyCString::from_bytes(b"Remote".to_vec()).unwrap(),
                    ..Default::default()
                },
            ]);
            if league {
                app.control_player_infos.replace_snapshot(
                    1,
                    [clonk_engine::PlayerInfoControlData {
                        client_id: 7,
                        players: vec![clonk_engine::ControlPlayerInfoEntry {
                            id: 1,
                            name: LegacyCString::from_bytes(b"League player".to_vec()).unwrap(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                );
            }
            app.sync_classic_lobby_roster();
            (app, commands)
        };

        let (mut direct, mut direct_commands) = setup(false);
        direct
            .process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
                row: LobbyRosterId::Client(7),
                position: GuiPoint::new(200.0, 150.0),
            }])
            .expect("remote client context opens");
        assert_eq!(direct.context_menu.as_ref().unwrap().layout().panels[0].rows.len(), 4);
        assert!(direct.select_classic_lobby_sheet(LobbySheet::Resources));
        assert!(direct.context_menu.is_none());
        assert_eq!(direct.context_menu_lobby_kick_client, None);
        assert!(direct.select_classic_lobby_sheet(LobbySheet::Players));
        direct
            .process_classic_lobby_actions(vec![ClassicLobbyAction::RosterContextRequested {
                row: LobbyRosterId::Client(7),
                position: GuiPoint::new(200.0, 150.0),
            }])
            .expect("remote client context reopens");
        direct.kick_classic_lobby_client(7);
        assert_eq!(
            direct_commands.take_submitted_client_removes(),
            vec![clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: LegacyCString::from_bytes(
                    b"kicked from startup waiting dialog".to_vec()
                )
                .unwrap(),
                by_client: 0,
            }]
        );
        assert!(direct_commands.take_submitted_votes().is_empty());
        direct
            .control_clients
            .apply_remove(&clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: LegacyCString::default(),
                by_client: 0,
            });
        direct.sync_classic_lobby_roster();
        assert!(direct.context_menu.is_none());
        assert_eq!(direct.context_menu_lobby_kick_client, None);

        let (mut league, mut league_commands) = setup(true);
        league.kick_classic_lobby_client(7);
        assert_eq!(
            league_commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 0,
            }]
        );
        assert!(league_commands.take_submitted_client_removes().is_empty());

        let (mut removed, mut removed_commands) = setup(true);
        removed.control_player_infos.replace_snapshot(
            1,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    flags: clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        removed.kick_classic_lobby_client(7);
        assert_eq!(removed_commands.take_submitted_client_removes().len(), 1);
        assert!(removed_commands.take_submitted_votes().is_empty());
    }

    #[test]
    fn host_disconnect_menu_lists_clients_and_dispatches_kick() {
        let mut direct = new_running_sandbox_app();
        let (_events, mut direct_commands) =
            install_running_network_stub(&mut direct, 0, 40, 4);
        let mut observer = message_client(9, b"Spectator");
        observer.activated = false;
        observer.observer = true;
        direct.control_clients.replace_snapshot([
            observer,
            message_client(7, b"Remote"),
            message_client(0, b"Host"),
        ]);

        direct
            .apply_ingame_menu_action(MenuAction::ActivateHostDisconnect)
            .expect("open host disconnect page");
        let owner = direct.local_owner;
        let menu = direct
            .ingame_menu
            .get(owner)
            .expect("host disconnect page is visible");
        assert_eq!(menu.page(), ingame_menu::MenuPage::HostDisconnect);
        assert_eq!(menu.caption(), "Disconnect client");
        assert!(menu.is_permanent());
        assert_eq!(menu.close_action(), Some(&MenuAction::ActivateMain));
        assert_eq!(
            menu.items()
                .iter()
                .map(|item| {
                    let icon = match &item.symbol {
                        ingame_menu::MenuSymbol::GuiIcon(icon) => *icon,
                        other => panic!("unexpected host-client row symbol: {other:?}"),
                    };
                    (item.caption.clone(), icon, item.action.clone())
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    "Host (Host)".to_string(),
                    ingame_menu::ICO_HOST,
                    MenuAction::KickClient(0),
                ),
                (
                    "Remote (Remote)".to_string(),
                    ingame_menu::ICO_CLIENT,
                    MenuAction::KickClient(7),
                ),
                (
                    "Spectator (Spectator)".to_string(),
                    ingame_menu::ICO_OBSERVER_CLIENT,
                    MenuAction::KickClient(9),
                ),
            ]
        );

        direct
            .handle_menu_command_failsafe(owner, ControlCommand::MenuEnter, CommandKind::Press)
            .expect("host row is a no-op");
        assert!(direct.ingame_menu.get(owner).is_some());
        assert!(direct_commands.take_submitted_votes().is_empty());
        assert!(direct_commands.take_submitted_client_removes().is_empty());

        direct
            .ingame_menu
            .get_mut(owner)
            .expect("page remains open")
            .set_selection(1);
        direct
            .handle_menu_command_failsafe(owner, ControlCommand::MenuEnter, CommandKind::Press)
            .expect("direct host-menu kick");
        assert_eq!(
            direct_commands.take_submitted_client_removes(),
            vec![clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: clonk_engine::LegacyCString::from_bytes(b"kicked from host menu".to_vec())
                    .expect("fixture reason"),
                by_client: 0,
            }]
        );
        assert!(direct_commands.take_submitted_votes().is_empty());
        assert!(direct.ingame_menu.get(owner).is_none());

        let mut league = new_running_sandbox_app();
        let (_events, mut league_commands) =
            install_running_network_stub(&mut league, 0, 40, 4);
        league.network_is_league = true;
        league
            .control_clients
            .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
        league
            .engine
            .register_player(PlayerConfig::new(17, "Remote Player"))
            .expect("register remote runtime player");
        league
            .engine
            .player_mut(17)
            .expect("remote player exists")
            .set_at_client(clonk_engine::PlayerAtClient::new(7));
        league
            .apply_ingame_menu_action(MenuAction::ActivateHostDisconnect)
            .expect("open league host disconnect page");
        let owner = league.local_owner;
        league
            .ingame_menu
            .get_mut(owner)
            .expect("league page is visible")
            .set_selection(1);
        league
            .handle_menu_command_failsafe(owner, ControlCommand::MenuEnter, CommandKind::Press)
            .expect("league host-menu kick vote");
        assert_eq!(
            league_commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 0,
            }]
        );
        assert!(league_commands.take_submitted_client_removes().is_empty());
        assert_eq!(
            league.ingame_menu.get(owner).map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::HostDisconnect)
        );
    }

    #[test]
    fn search_settings_recover_only_rust_truncated_masterserver_urls_before_query() {
        // C++ defaults both addresses to the complete official HTTPS URL, while
        // malformed non-default values remain inputs to ParseOldStyle
        // (C4Config.h:35-38; C4Config.cpp:253-259;
        // C4HTTPClient.cpp:105-118). Only bare schemes can be persisted damage
        // from the old Rust parser treating `//` as an inline comment.
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

        for (key, use_alternate, persisted, expected) in [
            (
                "ServerAddress",
                false,
                "https:",
                clonk_network::DEFAULT_MASTER_SERVER_URL,
            ),
            (
                "AlternateServerAddress",
                true,
                "http:",
                clonk_network::DEFAULT_MASTER_SERVER_URL,
            ),
            ("ServerAddress", false, "https://", "https://"),
        ] {
            let mut config = Config::new();
            config.set_in(Some("Network"), "MasterServerSignUp", "1");
            config.set_in(
                Some("Network"),
                "UseAlternateServer",
                i32::from(use_alternate).to_string(),
            );
            config.set_in(Some("Network"), key, persisted);
            config
                .save(paths.config_file())
                .expect("persist masterserver setting");

            let settings = load_network_search_settings(Some(&paths));
            assert_eq!(settings.use_alternate_server, use_alternate);
            assert_eq!(settings.master_server_url, expected);
            let mut search = clonk_network::NetworkGameSearch::new(settings);
            assert!(matches!(
                &search.refresh()[1],
                clonk_network::SearchCommand::QueryReferences {
                    endpoint: clonk_network::ReferenceEndpoint::Url(url),
                    source: clonk_network::ReferenceQuerySource::Masterserver,
                    ..
                } if url == expected
            ));
        }
    }

    #[test]
    fn l040_masterserver_row_projects_counts_motd_and_query_error_states() {
        use clonk_frontend::startup_netdlg::{NetDlgRowIcon, NetDlgTextLine};

        let reply = clonk_network::MasterserverReplyInfo {
            motd: "Welcome back".to_string(),
            motd_url: "https://news.example/motd".to_string(),
            game_count: 3,
            player_count: 17,
            ..Default::default()
        };
        let entry = GameApp::startup_masterserver_reply_entry(
            None,
            "https://master.example:8443/refs",
            &reply,
        );
        assert_eq!(entry.title, "Internet server on master.example");
        assert_eq!(entry.details, "3 game(s) found.");
        assert_eq!(
            entry.extra_lines,
            [
                NetDlgTextLine::Plain("Message of the day: Welcome back".to_string()),
                NetDlgTextLine::Hyperlink {
                    label: "https://news.example/motd".to_string(),
                    url: "https://news.example/motd".to_string(),
                },
            ]
        );
        assert_eq!(entry.row_icon, NetDlgRowIcon::QueryStatic);

        let zero = GameApp::startup_masterserver_reply_entry(
            None,
            "https://master.example/",
            &clonk_network::MasterserverReplyInfo::default(),
        );
        assert_eq!(zero.details, "No games found.");

        let mut app = new_classic_menu_app(800, 600);
        attach_l040_network_dialog(&mut app);
        app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::MasterserverReply(
            reply,
        ))
        .expect("project masterserver success");
        app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::Cleared)
            .expect("reset masterserver query generation");
        let master = app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .masterserver_entry();
        assert_eq!(master.details, "Querying game infos...");
        assert_eq!(master.row_icon, NetDlgRowIcon::Query);
        assert!(master.extra_lines.is_empty());

        app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::SearchError {
            source: Some(clonk_network::ReferenceQuerySource::Masterserver),
            message: "masterserver timed out".to_string(),
        })
        .expect("project permanent masterserver error row");
        let master = app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .masterserver_entry();
        assert_eq!(master.details, "masterserver timed out");
        assert_eq!(master.row_icon, NetDlgRowIcon::Error);
        assert!(master.extra_lines.is_empty());

        app.apply_startup_game_search_event(
            clonk_network::StartupGameSearchEvent::MasterserverReply(
                clonk_network::MasterserverReplyInfo {
                    game_count: 1,
                    player_count: 2,
                    ..Default::default()
                },
            ),
        )
        .expect("a successful retry recovers the masterserver row");
        let master = app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .masterserver_entry();
        assert_eq!(master.details, "1 game(s) found.");
        assert_eq!(master.row_icon, NetDlgRowIcon::QueryStatic);
        assert!(app.status_text.is_empty());
        assert!(app.message_dialogs.is_empty());

        app.set_startup_masterserver_error("masterserver timed out".to_string());

        let next_query_at = app
            .startup_masterserver_next_query_at
            .expect("terminal failure arms a response-relative retry");
        app.tick_startup_network_query_rows_at(
            next_query_at - Duration::from_millis(1),
        );
        assert_eq!(
            app.startup_network_dialog
                .as_ref()
                .unwrap()
                .masterserver_entry()
                .row_icon,
            NetDlgRowIcon::Error
        );
        app.tick_startup_network_query_rows_at(next_query_at);
        let master = app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .masterserver_entry();
        assert_eq!(master.details, "Querying game infos...");
        assert_eq!(master.row_icon, NetDlgRowIcon::Query);
        assert!(app.startup_masterserver_next_query_at.is_none());

        app.set_startup_masterserver_error("stale disabled error".to_string());
        if let Some(dialog) = app.startup_network_dialog.as_mut() {
            dialog.sync_masterserver_signup_from_config(false);
            // The controller performs this toggle before emitting the action.
            dialog.sync_masterserver_signup_from_config(true);
        }
        app.process_network_dialog_actions(vec![
            clonk_frontend::startup_netdlg::NetDlgAction::MasterserverSignupChanged(true),
        ])
        .expect("reenable Internet query row");
        let master = app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .masterserver_entry();
        assert_eq!(master.details, "Querying game infos...");
        assert_eq!(master.row_icon, NetDlgRowIcon::Query);

        let unchanged_refresh = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .unwrap();
        app.startup_network_last_refresh = Some(unchanged_refresh);
        app.refresh_retained_network_dialog_internet();
        assert_eq!(
            app.startup_network_last_refresh,
            Some(unchanged_refresh),
            "showing an already-enabled retained dialog starts no new query"
        );
        app.set_startup_masterserver_error("stale disabled error".to_string());
        app.startup_network_dialog
            .as_mut()
            .unwrap()
            .sync_masterserver_signup_from_config(false);
        app.refresh_retained_network_dialog_internet();
        assert!(app
            .startup_network_last_refresh
            .is_some_and(|refresh| refresh > unchanged_refresh));
        let master = app
            .startup_network_dialog
            .as_ref()
            .unwrap()
            .masterserver_entry();
        assert_eq!(master.details, "Querying game infos...");
        assert_eq!(master.row_icon, NetDlgRowIcon::Query);
    }

    #[test]
    fn masterserver_results_do_not_throttle_manual_reload() {
        for event in [
            clonk_network::StartupGameSearchEvent::MasterserverReply(
                clonk_network::MasterserverReplyInfo::default(),
            ),
            clonk_network::StartupGameSearchEvent::SearchError {
                source: Some(clonk_network::ReferenceQuerySource::Masterserver),
                message: "terminal failure".to_string(),
            },
        ] {
            let mut app = new_classic_menu_app(800, 600);
            attach_l040_network_dialog(&mut app);
            let reload_at = Instant::now();
            let prior_reload = reload_at
                .checked_sub(STARTUP_NETWORK_MIN_REFRESH_INTERVAL + Duration::from_secs(1))
                .unwrap();
            app.startup_network_last_refresh = Some(prior_reload);

            app.apply_startup_game_search_event(event)
                .expect("project a terminal masterserver result");
            assert_eq!(app.startup_network_last_refresh, Some(prior_reload));

            app.request_startup_network_refresh_at(reload_at)
                .expect("the independent manual reload remains eligible");
            assert_eq!(app.startup_network_last_refresh, Some(reload_at));
        }
    }

    #[test]
    fn l040_direct_empty_and_error_rows_expire_after_ten_seconds() {
        let mut app = new_classic_menu_app(800, 600);
        attach_l040_network_dialog(&mut app);
        let now = Instant::now();
        let expires_at = now + STARTUP_NETWORK_QUERY_ERROR_LIFETIME;
        app.startup_direct_reference_queries = vec![
            StartupDirectReferenceQuery {
                id: 1,
                address: "empty.example".to_string(),
                state: StartupDirectReferenceQueryState::Empty,
                expires_at: Some(expires_at),
            },
            StartupDirectReferenceQuery {
                id: 2,
                address: "failed.example".to_string(),
                state: StartupDirectReferenceQueryState::Failed("query failed".to_string()),
                expires_at: Some(expires_at),
            },
            StartupDirectReferenceQuery {
                id: 3,
                address: "pending.example".to_string(),
                state: StartupDirectReferenceQueryState::Pending,
                expires_at: None,
            },
        ];
        app.sync_startup_network_game_rows();
        assert_eq!(
            app.startup_network_dialog.as_ref().unwrap().games()[0].row_icon,
            clonk_frontend::startup_netdlg::NetDlgRowIcon::QueryStatic
        );
        assert_eq!(
            app.startup_network_dialog.as_ref().unwrap().games()[1].row_icon,
            clonk_frontend::startup_netdlg::NetDlgRowIcon::Error
        );
        app.startup_network_dialog.as_mut().unwrap().focus_game(0);

        app.tick_startup_network_query_rows_at(expires_at - Duration::from_millis(1));
        assert_eq!(app.startup_direct_reference_queries.len(), 3);
        app.tick_startup_network_query_rows_at(expires_at);
        assert_eq!(
            app.startup_direct_reference_queries,
            [StartupDirectReferenceQuery {
                id: 3,
                address: "pending.example".to_string(),
                state: StartupDirectReferenceQueryState::Pending,
                expires_at: None,
            }]
        );
        assert_eq!(
            app.startup_network_dialog
                .as_ref()
                .unwrap()
                .games()
                .len(),
            1
        );
        assert_eq!(
            app.startup_network_dialog.as_ref().unwrap().selected_game(),
            Some(0),
            "expiring a selected query row selects its next native sibling"
        );
    }

    #[test]
    fn l040_masterserver_redirect_decline_latches_and_accept_persists() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("L040 user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover L040 paths");
        paths.ensure_user_dirs().expect("create L040 user dirs");
        persist_config_value(&paths, "General", "LanguageEx", "US")
            .expect("configure L040 language");
        persist_config_value(&paths, "Network", "MasterServerSignUp", "0")
            .expect("disable live masterserver query");
        persist_config_value(
            &paths,
            "Network",
            "ServerAddress",
            "https://old.example",
        )
        .expect("configure old server");
        persist_config_value(
            &paths,
            "Network",
            "AlternateServerAddress",
            "https://old.example",
        )
        .expect("configure matching alternate server");
        persist_config_value(&paths, "Network", "UseAlternateServer", "0")
            .expect("configure official server mode");

        let mut app = new_classic_menu_app(800, 600);
        app.app_paths = Some(paths.clone());
        attach_l040_network_dialog(&mut app);
        let redirect = clonk_network::MasterserverReplyInfo {
            league_server_redirect: "https://new.example".to_string(),
            game_count: 1,
            ..Default::default()
        };

        app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::MasterserverReply(
            redirect.clone(),
        ))
        .expect("open redirect confirmation");
        assert_eq!(app.message_dialogs.len(), 1);
        let modal = &app.message_dialogs[0].state;
        assert_eq!(modal.caption(), "Server Redirection");
        assert!(modal.message().contains("https://new.example"));
        assert_eq!(
            modal.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
        );
        assert_eq!(
            modal.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::Standard(44)
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
            .expect("decline redirect");
        assert!(app.startup_network_ignore_redirect);
        app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::MasterserverReply(
            redirect.clone(),
        ))
        .expect("ignore repeated redirect");
        assert!(app.message_dialogs.is_empty());
        assert_eq!(
            Config::load(paths.config_file())
                .unwrap()
                .get_in(Some("Network"), "ServerAddress"),
            Some("https://old.example")
        );

        app.startup_network_ignore_redirect = true;
        app.open_network_game_dialog();
        app.startup_game_search = None;
        assert!(!app.startup_network_ignore_redirect);
        app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::MasterserverReply(
            redirect,
        ))
        .expect("reopened dialog offers redirect again");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
            .expect("accept and persist redirect");
        assert_eq!(
            Config::load(paths.config_file())
                .unwrap()
                .get_in(Some("Network"), "ServerAddress"),
            Some("https://new.example")
        );
        assert_eq!(app.message_dialogs.len(), 1);
        let applied = &app.message_dialogs[0].state;
        assert_eq!(applied.caption(), "Server Redirection");
        assert_eq!(applied.message(), "Server redirection has been applied.");
        assert_eq!(
            applied.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::OK
        );
        assert_eq!(
            applied.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::Standard(44)
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss applied confirmation");

        persist_config_value(&paths, "Network", "UseAlternateServer", "1")
            .expect("enable alternate server");
        persist_config_value(
            &paths,
            "Network",
            "AlternateServerAddress",
            "https://alternate.example",
        )
        .expect("configure independent alternate server");
        app.apply_startup_game_search_event(clonk_network::StartupGameSearchEvent::MasterserverReply(
            clonk_network::MasterserverReplyInfo {
                league_server_redirect: "https://ignored.example".to_string(),
                game_count: 1,
                ..Default::default()
            },
        ))
        .expect("independent alternate server cannot redirect");
        assert!(app.message_dialogs.is_empty());
    }

    fn serve_one_record_stream_upload() -> (String, thread::JoinHandle<Vec<u8>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let request = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let header = std::str::from_utf8(&request[..header_end]).unwrap();
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                    })
                    .unwrap();
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            request
        });
        (format!("http://{address}/record?game=7&"), request)
    }

    #[test]
    fn forced_recording_writes_replay_group_and_league_sha() {
        let directory = tempdir().expect("record directory");
        let output_path = directory.path().join("001-Scenario.c4s");
        let mut app = new_state_only_running_sandbox_app();
        app.network_is_league = true;
        let game_number = app.local_owner;
        app.control_player_infos.replace_snapshot(
            17,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 17,
                    game_number,
                    name: LegacyCString::from_bytes(b"Recorded player".to_vec())
                        .expect("recorded player name"),
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        install_test_recording_template(&mut app, output_path.clone());
        let mut nested = MutableGroup::new("Nested.c4g");
        nested
            .add_file("Nested.txt", b"nested".to_vec())
            .expect("nested component");
        app.recording_template
            .as_mut()
            .unwrap()
            .group
            .add_child("Nested.c4g", nested)
            .expect("nested group");

        app.start_recording(true).unwrap();
        assert!(output_path.is_dir(), "active record must be unpacked");
        assert!(
            output_path.join("Nested.c4g").is_file(),
            "top-level unpack keeps child groups as packed physical files"
        );
        assert_eq!(
            Group::open(output_path.join("Nested.c4g"))
                .expect("physical child group opens")
                .read_file("Nested.txt")
                .expect("nested file"),
            b"nested"
        );
        let initial = Group::open(&output_path).expect("initial record is durable at start");
        assert_eq!(
            initial.read_file("Sentinel.txt").expect("copied component"),
            b"preserved"
        );
        assert_eq!(
            initial.read_file("CtrlRec.c4b").expect("open CtrlRec"),
            Vec::<u8>::new()
        );
        let packet = recorded_right_control(app.local_owner);
        app.apply_ready_controls(
            0,
            vec![network::network_control_for_packet(packet.clone())
                .expect("supported control")],
        )
        .expect("execute and record control");
        assert_eq!(
            fs::read(output_path.join("CtrlRec.c4b")).expect("live CtrlRec is durable"),
            app.recording
                .as_ref()
                .expect("recording remains active")
                .writer
                .bytes(),
            "IMMEDIATEREC flushes each control chunk before Stop"
        );
        let metadata = app.finish_recording().expect("league record metadata");

        let packed = fs::read(&output_path).expect("packed record group");
        assert_eq!(metadata.sha1, <[u8; 20]>::from(Sha1::digest(&packed)));
        assert_eq!(
            metadata.name.as_bytes(),
            output_path.to_string_lossy().as_bytes()
        );
        let group = Group::open(&output_path).expect("C4Group record opens");
        assert_eq!(
            group.read_file("Sentinel.txt").expect("copied component"),
            b"preserved"
        );
        let scenario = String::from_utf8(group.read_file("Scenario.txt").unwrap()).unwrap();
        assert!(scenario.contains("Replay=1"));
        assert!(scenario.contains("Icon=29"));
        assert!(group.exists("DescUS.rtf"));
        let description = group.read_file("DescUS.rtf").expect("record description");
        assert!(description
            .windows(b"Engine version: 362".len())
            .any(|window| window == b"Engine version: 362"));
        let final_player_infos = clonk_network::decode_player_info_list_ini(
            &group
                .read_file("RecPlayerInfos.txt")
                .expect("final player infos"),
        )
        .expect("decode final player infos");
        assert_eq!(final_player_infos.clients[0].players[0].id, 17);
        let stream = group.read_file("CtrlRec.c4b").expect("binary CtrlRec");
        let mut playback = ControlRecordPlayback::from_bytes(&stream).expect("CtrlRec opens");
        assert_eq!(playback.take_controls(0), vec![packet]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn league_streamed_player_strip_and_record_name_match_cpp_bytes() {
        let directory = tempdir().expect("record directory");
        #[cfg(all(unix, not(target_os = "macos")))]
        let output_path = {
            use std::os::unix::ffi::OsStringExt;

            directory
                .path()
                .join(OsString::from_vec(b"001-Streamed-\xff.c4s".to_vec()))
        };
        // Darwin rejects non-UTF-8 path components with EILSEQ before the
        // recording code can observe them. Keep the full filesystem lifecycle
        // non-ASCII there; the raw invalid-byte conversion is asserted below.
        #[cfg(target_os = "macos")]
        let output_path = {
            use std::os::unix::ffi::OsStringExt;

            directory.path().join(OsString::from_vec(
                b"001-Streamed-e\xcc\x81.c4s".to_vec(),
            ))
        };
        #[cfg(not(unix))]
        let output_path = directory.path().join("001-Streamed.c4s");
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            let invalid_path = directory
                .path()
                .join(OsString::from_vec(b"001-Streamed-\xff.c4s".to_vec()));
            let invalid_name = league_record_name(&invalid_path).unwrap();
            assert_eq!(
                invalid_name.as_bytes(),
                path_to_legacy_bytes(&invalid_path)
            );
            assert!(invalid_name
                .as_bytes()
                .ends_with(b"001-Streamed-\xff.c4s"));
        }
        let (endpoint, request) = serve_one_record_stream_upload();
        let mut app = new_state_only_running_sandbox_app();
        app.process_group_maker =
            LegacyCString::from_bytes(b"League stream maker".to_vec()).unwrap();
        install_test_recording_template(&mut app, output_path.clone());
        let output_name_bytes = path_to_legacy_bytes(&output_path);
        let initial_name = league_record_name(&output_path).unwrap();
        let initial_chunk = clonk_network::encode_league_stream_file_chunk(
            &initial_name,
            b"packed initial record",
        )
        .unwrap();
        app.recording_template
            .as_mut()
            .unwrap()
            .initial_stream_chunk = initial_chunk.clone();
        let (network, _events) = NetworkManager::test_stub_with_league_record_stream(endpoint);
        app.network = Some(network);
        app.network_is_league = true;
        assert!(app.engine.definition_name("CLNK").is_some());
        let mut ranked = Definition::from_script("RANK", "Ranked crew", "")
            .expect("custom-rank definition");
        ranked.set_rank_system(
            Some(vec!["Cadet".to_string(), "Veteran".to_string()]),
            Some(1_200),
        );
        app.engine
            .register_definition(ranked)
            .expect("register custom-rank definition");

        let player_path = directory.path().join("Alice.c4p");
        let mut player_group = MutableGroup::new("Alice.c4p");
        let raw_player_core = b"[Player]\n\
Name=<i>Al</i>ice\n\
Comment=stream candidate\n\
Score=9\n\
VendorPlayerField=discard me\n\
[Preferences]\n\
Color=2\n\
ColorDw=0\n\
AutoStopControl=2\n\
[VendorPlayer]\n\
Retain=never\n";
        player_group
            .add_file("Player.txt", raw_player_core.to_vec())
            .unwrap();
        player_group
            .add_file("BigIcon.png", b"large player icon".to_vec())
            .unwrap();
        player_group
            .add_file("Private.bin", b"must stay local".to_vec())
            .unwrap();
        let mut valid_crew = MutableGroup::new("Alice.c4i");
        let raw_valid_crew = b"[ObjectInfo]\n\
id=RANK-extra\n\
Name=Alice\n\
DeathMessage=@fell\n\
RankName=Stale rank\n\
NextRankName=Stale next rank\n\
NextRankExp=999\n\
VendorCrewField=discard me\n\
[Physical]\n\
Energy=1\n\
[VendorCrew]\n\
Retain=never\n";
        valid_crew
            .add_file("ObjectInfo.txt", raw_valid_crew.to_vec())
            .unwrap();
        let raw_portrait = encode_rgba_png(1, 1, &[1, 2, 3, 255]).unwrap();
        valid_crew
            .add_file("Portrait.png", raw_portrait.clone())
            .unwrap();
        valid_crew
            .add_file("Private.bin", b"crew extra".to_vec())
            .unwrap();
        player_group.add_child("Alice.c4i", valid_crew).unwrap();
        let mut missing_crew = MutableGroup::new("Missing.c4i");
        let raw_missing_crew = b"[ObjectInfo]\nid=MISS\nName=Missing\n";
        missing_crew
            .add_file("ObjectInfo.txt", raw_missing_crew.to_vec())
            .unwrap();
        player_group
            .add_child("Missing.c4i", missing_crew)
            .unwrap();
        let raw_nested_crew = b"[ObjectInfo]\n\
id=CLNK\n\
Name=Nested\n\
VendorNestedField=discard me\n";
        let mut nested_crew = MutableGroup::new("Alice.c4i");
        nested_crew
            .add_file("ObjectInfo.txt", raw_nested_crew.to_vec())
            .unwrap();
        nested_crew
            .add_file("Private.bin", b"nested extra".to_vec())
            .unwrap();
        let mut roster = MutableGroup::new("Roster.c4f");
        // The repeated source filename makes the streamed names prove native
        // direct-first recursive discovery: direct Alice keeps Alice.c4i,
        // then nested Alice collides and falls back to its core name.
        roster.add_child("Alice.c4i", nested_crew).unwrap();
        player_group.add_child("Roster.c4f", roster).unwrap();
        fs::write(&player_path, player_group.pack().unwrap()).unwrap();
        app.admission_resources
            .mark_complete(17, player_path.clone());
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
            source: clonk_engine::JoinPlayerSource::Resource(core),
            by_client: 0,
        });
        app.start_recording(true).unwrap();
        let frame = u32::try_from(app.engine.frame()).unwrap();
        app.record_control_packet(&packet);
        let live_local_player_path = output_path.join("17-Alice.c4p");
        assert!(
            live_local_player_path.is_file(),
            "PreRec copies the local player resource before the control chunk"
        );
        let live_local_player = Group::open(&live_local_player_path)
            .expect("live unstripped player resource opens");
        assert!(live_local_player.exists("Private.bin"));
        assert!(live_local_player.exists("Missing.c4i"));

        let mut expected_writer = ControlRecordWriter::new();
        expected_writer.record_packet(frame, &packet).unwrap();
        let control_bytes = expected_writer.bytes().to_vec();
        let metadata = app.finish_recording().expect("streamed record metadata");
        assert_eq!(metadata.name.as_bytes(), output_name_bytes.as_slice());
        assert_eq!(metadata.name.as_bytes(), initial_name.as_bytes());

        let request = request.join().unwrap();
        let header_end = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let header = std::str::from_utf8(&request[..header_end]).unwrap();
        assert!(header.starts_with(
            "POST /record?game=7&pos=0&end=true HTTP/1."
        ));
        let mut decoded = Vec::new();
        ZlibDecoder::new(&request[header_end + 4..])
            .read_to_end(&mut decoded)
            .unwrap();
        assert!(decoded.starts_with(&initial_chunk));
        let mut cursor = initial_chunk.len();
        assert_eq!(
            &decoded[cursor..cursor + 2],
            &[0, clonk_network::LEAGUE_STREAM_FILE_CHUNK_TYPE]
        );
        cursor += 2;
        let name_end = decoded[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| cursor + offset)
            .unwrap();
        assert_eq!(&decoded[cursor..name_end], b"17-Alice.c4p");
        cursor = name_end + 1;
        let mut packed_size = 0_usize;
        let mut shift = 0_u32;
        loop {
            let byte = decoded[cursor];
            cursor += 1;
            packed_size |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let packed_end = cursor + packed_size;
        let streamed_player = Group::from_memory(
            PathBuf::from("17-Alice.c4p"),
            decoded[cursor..packed_end].to_vec(),
        )
        .expect("stripped streamed player group opens");
        assert_eq!(
            streamed_player.maker_bytes(),
            Some(b"League stream maker".as_slice())
        );
        assert!(streamed_player.exists("Player.txt"));
        assert!(streamed_player.exists("Alice.c4i"));
        assert!(streamed_player.exists("Nested.c4i"));
        assert!(!streamed_player.exists("Missing.c4i"));
        assert!(!streamed_player.exists("Roster.c4f"));
        assert!(!streamed_player.exists("BigIcon.png"));
        assert!(!streamed_player.exists("Private.bin"));
        let canonical_crlf = |bytes: &[u8]| {
            bytes.iter().enumerate().all(|(index, byte)| {
                *byte != b'\n' || index.checked_sub(1).is_some_and(|index| bytes[index] == b'\r')
            })
        };
        let streamed_player_core = streamed_player
            .read_file("Player.txt")
            .expect("canonical streamed player core");
        assert_ne!(streamed_player_core.as_slice(), raw_player_core);
        assert!(canonical_crlf(&streamed_player_core));
        let streamed_player_text = std::str::from_utf8(&streamed_player_core).unwrap();
        assert!(!streamed_player_text.contains("VendorPlayer"));
        assert!(!streamed_player_text.contains("<i>"));
        let parsed_streamed_player =
            PlayerFile::load_with_portraits(&streamed_player, false).unwrap();
        assert_eq!(parsed_streamed_player.name, "Alice");
        assert_eq!(parsed_streamed_player.score, 9);
        assert_eq!(parsed_streamed_player.pref_color_dw, 0x00c800);
        assert!(parsed_streamed_player.pref_control_style);
        assert!(parsed_streamed_player.pref_auto_context_menu);
        assert_eq!(
            parsed_streamed_player
                .exact_info_core()
                .pref_control_style_value,
            2
        );
        assert_eq!(
            parsed_streamed_player
                .exact_info_core()
                .pref_auto_context_menu_value,
            2
        );
        assert_eq!(
            parsed_streamed_player
                .crew
                .iter()
                .map(|crew| crew.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alice", "Nested"]
        );
        let streamed_crew = streamed_player
            .open_child("Alice.c4i")
            .expect("valid streamed crew opens");
        assert_eq!(
            streamed_crew.maker_bytes(),
            Some(b"League stream maker".as_slice())
        );
        assert!(streamed_crew.exists("ObjectInfo.txt"));
        assert!(!streamed_crew.exists("Portrait.png"));
        assert!(!streamed_crew.exists("Private.bin"));
        let streamed_crew_core = streamed_crew
            .read_file("ObjectInfo.txt")
            .expect("canonical streamed crew core");
        assert_ne!(streamed_crew_core.as_slice(), raw_valid_crew);
        assert!(canonical_crlf(&streamed_crew_core));
        let streamed_crew_text = std::str::from_utf8(&streamed_crew_core).unwrap();
        assert!(!streamed_crew_text.contains("VendorCrew"));
        assert!(!streamed_crew_text.contains("RANK-extra"));
        assert!(!streamed_crew_text.contains("PortraitFile=custom"));
        assert!(streamed_crew_core
            .windows(b"DeathMessage= fell\r\n".len())
            .any(|window| window == b"DeathMessage= fell\r\n"));
        let streamed_alice = &parsed_streamed_player.crew[0];
        assert_eq!(streamed_alice.id, "RANK");
        assert_eq!(streamed_alice.death_message, "fell");
        assert_eq!(streamed_alice.rank_name, "Cadet");
        assert_eq!(streamed_alice.core.next_rank_name, "Veteran");
        assert_eq!(streamed_alice.core.next_rank_exp, 1_200);
        assert!(streamed_alice.core.portrait_file.is_empty());
        assert_eq!(streamed_alice.physical.energy, 50_000);
        assert_eq!(streamed_alice.physical.can_dig, 1);
        assert_eq!(streamed_alice.physical.can_chop, 1);
        assert_eq!(streamed_alice.physical.can_construct, 1);
        assert_eq!(streamed_alice.physical.can_scale, 1);
        assert_eq!(streamed_alice.physical.can_hangle, 1);
        let streamed_nested = streamed_player
            .open_child("Nested.c4i")
            .expect("flattened streamed crew opens");
        assert_eq!(
            streamed_nested.maker_bytes(),
            Some(b"League stream maker".as_slice())
        );
        assert!(streamed_nested.exists("ObjectInfo.txt"));
        assert!(!streamed_nested.exists("Private.bin"));
        let streamed_nested_core = streamed_nested.read_file("ObjectInfo.txt").unwrap();
        assert_ne!(streamed_nested_core.as_slice(), raw_nested_crew);
        assert!(canonical_crlf(&streamed_nested_core));
        assert!(!std::str::from_utf8(&streamed_nested_core)
            .unwrap()
            .contains("VendorNestedField"));
        assert_eq!(&decoded[packed_end..], control_bytes);

        let record = Group::open(&output_path).expect("record group");
        let local_player = record
            .open_child("17-Alice.c4p")
            .expect("unstripped local player group");
        assert!(local_player.exists("BigIcon.png"));
        assert!(local_player.exists("Private.bin"));
        assert!(local_player.exists("Missing.c4i"));
        assert!(local_player.exists("Roster.c4f"));
        assert_eq!(
            local_player.read_file("BigIcon.png").unwrap().as_slice(),
            b"large player icon"
        );
        assert_eq!(
            local_player.read_file("Private.bin").unwrap().as_slice(),
            b"must stay local"
        );
        assert_eq!(
            local_player.read_file("Player.txt").unwrap().as_slice(),
            raw_player_core
        );
        assert_eq!(
            local_player
                .open_child("Missing.c4i")
                .unwrap()
                .read_file("ObjectInfo.txt")
                .unwrap()
                .as_slice(),
            raw_missing_crew
        );
        let local_crew = local_player
            .open_child("Alice.c4i")
            .expect("unstripped local crew group");
        assert!(local_crew.exists("Portrait.png"));
        assert!(local_crew.exists("Private.bin"));
        assert_eq!(local_crew.read_file("Portrait.png").unwrap(), raw_portrait);
        assert_eq!(
            local_crew.read_file("Private.bin").unwrap().as_slice(),
            b"crew extra"
        );
        assert_eq!(
            local_crew.read_file("ObjectInfo.txt").unwrap().as_slice(),
            raw_valid_crew
        );
        let local_nested = local_player
            .open_child("Roster.c4f")
            .unwrap()
            .open_child("Alice.c4i")
            .expect("nested local crew group");
        assert!(local_nested.exists("Private.bin"));
        assert_eq!(
            local_nested.read_file("Private.bin").unwrap().as_slice(),
            b"nested extra"
        );
        assert_eq!(
            local_nested
                .read_file("ObjectInfo.txt")
                .unwrap()
                .as_slice(),
            raw_nested_crew
        );
        let local = record.read_file("CtrlRec.c4b").expect("local CtrlRec");
        assert_eq!(&local[..control_bytes.len()], control_bytes);
        assert_eq!(local.len(), control_bytes.len() + 2);
        assert_eq!(local.last(), Some(&clonk_engine::RCT_END));
    }

    #[test]
    fn league_stream_player_strip_requires_direct_crew_and_flattens_nested_valid_crew() {
        let app = new_state_only_running_sandbox_app();
        assert!(app.engine.definition_name("CLNK").is_some());

        let mut no_crew = MutableGroup::new("Empty.c4p");
        no_crew
            .add_file("Player.txt", b"[Player]\nName=Empty\n".to_vec())
            .unwrap();
        let no_crew = Group::from_memory(PathBuf::from("Empty.c4p"), no_crew.pack().unwrap())
            .expect("empty player opens");
        assert!(
            app.pack_stripped_stream_player(&no_crew, b"1-Empty.c4p")
                .unwrap_err()
                .contains("no loadable direct crew")
        );

        let mut source = MutableGroup::new("Nested.c4p");
        source
            .add_file("Player.txt", b"[Player]\nName=Nested\n".to_vec())
            .unwrap();
        let mut missing = MutableGroup::new("Missing.c4i");
        missing
            .add_file(
                "ObjectInfo.txt",
                b"[ObjectInfo]\nid=MISS\nName=Missing\n".to_vec(),
            )
            .unwrap();
        source.add_child("Missing.c4i", missing).unwrap();
        let mut nested = MutableGroup::new("Nested.c4i");
        nested
            .add_file(
                "ObjectInfo.txt",
                b"[ObjectInfo]\nid=CLNK\nName=Nested\n".to_vec(),
            )
            .unwrap();
        nested
            .add_file("Portrait.png", b"not streamed".to_vec())
            .unwrap();
        let mut folder = MutableGroup::new("Roster.c4f");
        folder.add_child("Nested.c4i", nested).unwrap();
        source.add_child("Roster.c4f", folder).unwrap();
        let source = Group::from_memory(PathBuf::from("Nested.c4p"), source.pack().unwrap())
            .expect("nested player opens");

        let stripped = app
            .pack_stripped_stream_player(&source, b"2-Nested.c4p")
            .expect("direct invalid crew still lets native strip proceed");
        let stripped = Group::from_memory(PathBuf::from("2-Nested.c4p"), stripped)
            .expect("stripped player opens");
        assert!(stripped.exists("Player.txt"));
        assert!(stripped.exists("Nested.c4i"));
        assert!(!stripped.exists("Missing.c4i"));
        assert!(!stripped.exists("Roster.c4f"));
        assert!(!stripped.open_child("Nested.c4i").unwrap().exists("Portrait.png"));
    }

    #[test]
    fn league_record_resource_name_preserves_raw_legacy_basename() {
        let core = clonk_engine::NetworkResourceCore {
            id: 23,
            filename: LegacyCString::from_bytes(b"Players/Andr\xe9.c4p".to_vec()).unwrap(),
            ..clonk_engine::NetworkResourceCore::default()
        };

        assert_eq!(recorded_player_resource_name(&core), b"23-Andr\xe9.c4p");
    }

    #[test]
    fn l013_running_fast_slow_commands_bound_and_honor_league_gate() {
        let mut app = new_state_only_running_sandbox_app();
        assert!(!app.full_speed);
        assert_eq!(app.frame_skip, 1);

        app.process_running_chat_text("/fast 12tail");
        assert!(app.full_speed);
        assert_eq!(app.frame_skip, 12);
        assert!(app.runtime_flash_message.is_none());

        app.process_running_chat_text("/fast 999");
        assert_eq!(app.frame_skip, 500);
        app.process_running_chat_text("/fast -4");
        assert!(app.full_speed, "/fast 1 remains unpaced");
        assert_eq!(app.frame_skip, 1);

        app.process_running_chat_text("/fast 0");
        app.process_running_chat_text("/fast");
        assert!(app.full_speed);
        assert_eq!(app.frame_skip, 1, "zero input is a recognized no-op");
        assert!(message_board_logical_entries(&app)
            .iter()
            .all(|line| !line.contains("Unknown command")));

        app.full_speed = true;
        app.frame_skip = 37;
        app.network_is_league = true;
        app.process_running_chat_text("/fast 7");
        assert!(app.full_speed);
        assert_eq!(app.frame_skip, 37);
        assert!(latest_message_board_logical_entry(&app)
            .as_deref()
            .is_some_and(|line| line.contains("not allowed in league")));

        app.process_running_chat_text("/slow");
        assert!(!app.full_speed, "/slow is allowed in league games");
        assert_eq!(app.frame_skip, 1);
        assert!(app.runtime_flash_message.is_none());
    }

    #[test]
    fn l119_running_kick_uses_exact_name_and_live_player_league_gate() {
        let mut app = new_state_only_running_sandbox_app();
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);
        app.control_clients
            .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);

        app.process_running_chat_text("/kick Remote");
        let removals = commands.take_submitted_client_removes();
        assert_eq!(removals.len(), 1);
        assert_eq!(removals[0].client_id, 7);
        assert_eq!(removals[0].by_client, 0);
        assert_eq!(removals[0].reason.as_bytes(), b"kicked from messageboard");
        assert!(commands.take_submitted_votes().is_empty());

        app.network_is_league = true;
        app.engine
            .register_player(PlayerConfig::new(17, "Remote Player"))
            .expect("register remote league player");
        app.engine
            .player_mut(17)
            .expect("remote league player exists")
            .set_at_client(clonk_engine::PlayerAtClient::new(7));
        app.process_running_chat_text("/kick Remote");
        assert_eq!(
            commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 0,
            }]
        );
        assert!(commands.take_submitted_client_removes().is_empty());

        app.process_running_chat_text("/kick remote");
        assert!(commands.take_submitted_votes().is_empty());
        assert!(latest_message_board_logical_entry(&app)
            .as_deref()
            .is_some_and(|line| line.contains("remote") && line.contains("not found")));
        assert!(message_board_logical_entries(&app)
            .iter()
            .all(|line| !line.contains("Unknown command")));
    }

    #[test]
    fn l143_exclusive_vote_outside_hit_still_reaches_exposed_chart() {
        let mut app = new_running_sandbox_app();
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                "Vote?",
                "Voting",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Small,
                true,
            ),
            MessageDialogContinuation::LeagueSurrender,
        )
        .expect("show exclusive vote above chart");
        app.toggle_network_chart();
        assert!(!app.network_chart_elevated);
        let resources = app
            .assets
            .network_chart_resources()
            .expect("synthetic chart resources");
        let preferred = scoreboard_preferred_rect(
            app.graphics
                .preferred_dialog_rect(app.mouse_control.then_some(app.local_owner)),
        );
        let chart_layout = app
            .network_chart_dialog
            .as_ref()
            .expect("open chart")
            .layout(preferred, resources);
        let exposed = (chart_layout.chart.y
            ..chart_layout.chart.y.saturating_add(chart_layout.chart.h))
            .step_by(4)
            .find_map(|y| {
                (chart_layout.chart.x
                    ..chart_layout.chart.x.saturating_add(chart_layout.chart.w))
                    .step_by(4)
                    .map(|x| GuiPoint::new(x as f32, y as f32))
                    .find(|point| app.top_message_dialog_hit_index(*point).is_none())
            })
            .expect("chart has an exposed point outside the smaller vote");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(exposed.x),
            f64::from(exposed.y),
        ))
        .expect("shared Screen scans past the non-hit vote");
        app.handle_mouse_button_classified(ElementState::Pressed, false)
            .expect("exposed chart receives left press");
        assert!(!app.network_chart_pointer_capture);
        assert!(app.network_chart_elevated);
        assert_eq!(app.message_dialog_active_index, None);
        app.handle_mouse_button_classified(ElementState::Released, false)
            .expect("release exposed chart gesture");
        assert_eq!(app.message_dialogs.len(), 1);
        assert!(app.network_chart_owns_stronger_escape());
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("elevated chart owns Escape above the vote");
        assert!(app.network_chart_dialog.is_none());
        assert_eq!(app.message_dialog_active_index, Some(0));
    }

    #[test]
    fn eliminated_and_surrendered_viewports_draw_localized_notice_instead_of_menus() {
        let mut app = new_classic_running_sandbox_app();
        let owner = app.local_owner;
        let cursor = app.engine.crew_cursor(owner).expect("sandbox cursor");
        app.display_flags.show_commands = false;
        app.engine
            .player_mut(owner)
            .expect("sandbox player")
            .set_name("Ada");
        app.snapshot = app.engine.snapshot();
        app.snapshot
            .players
            .iter_mut()
            .find(|player| player.id == owner)
            .expect("sandbox snapshot player")
            .status = PlayerStatus::Eliminated;

        let mut notice_only = vec![0_u8; app.graphics.surface().pixels().len()];
        app.render(&mut notice_only)
            .expect("render eliminated viewport notice");

        let mut invalid_hidden_menu = two_item_script_menu(cursor);
        invalid_hidden_menu.style = 99;
        install_test_cursor_menu(&mut app, cursor, invalid_hidden_menu);
        app.ingame_menu
            .replace(owner, Some(IngameMenuState::surrender_menu()));
        let mut with_hidden_menus = vec![0_u8; app.graphics.surface().pixels().len()];
        app.render(&mut with_hidden_menus)
            .expect("eliminated viewport skips menu preflight and drawing");
        assert_eq!(
            with_hidden_menus, notice_only,
            "script and player menus contribute no eliminated-viewport pixels"
        );

        app.ingame_menu.clear();
        app.save_browser = Some(SaveBrowserState::new(SaveBrowserMode::Load, Vec::new()));
        let mut with_hidden_save_menu = vec![0_u8; app.graphics.surface().pixels().len()];
        app.render(&mut with_hidden_save_menu)
            .expect("eliminated viewport skips the legacy save-menu boundary");
        assert_eq!(
            with_hidden_save_menu, notice_only,
            "the legacy save-menu fallback contributes no eliminated-viewport pixels"
        );
        app.save_browser = None;

        let mut retargeted = new_classic_running_sandbox_app();
        let local_owner = retargeted.local_owner;
        let eliminated_target = local_owner + 1;
        let retargeted_cursor = retargeted
            .engine
            .crew_cursor(local_owner)
            .expect("retargeted sandbox cursor");
        retargeted
            .engine
            .register_player(PlayerConfig::new(eliminated_target, "Retargeted"))
            .expect("register eliminated film target");
        retargeted.snapshot = retargeted.engine.snapshot();
        retargeted
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.id == eliminated_target)
            .expect("retargeted snapshot player")
            .status = PlayerStatus::Eliminated;
        assert!(retargeted.set_physical_film_view(eliminated_target));
        let mut retargeted_notice = vec![0_u8; retargeted.graphics.surface().pixels().len()];
        retargeted
            .render(&mut retargeted_notice)
            .expect("render eliminated target through an owned physical viewport");
        retargeted.ingame_menu.replace(
            local_owner,
            IngameMenuState::main_menu(&MainMenuConditions::default()),
        );
        let mut retargeted_hidden_script_menu = two_item_script_menu(retargeted_cursor);
        retargeted_hidden_script_menu.style = 99;
        install_test_cursor_menu(
            &mut retargeted,
            retargeted_cursor,
            retargeted_hidden_script_menu,
        );
        retargeted.save_browser =
            Some(SaveBrowserState::new(SaveBrowserMode::Load, Vec::new()));
        let mut with_retargeted_menus =
            vec![0_u8; retargeted.graphics.surface().pixels().len()];
        retargeted
            .render(&mut with_retargeted_menus)
            .expect("retargeted eliminated viewport suppresses local-owner menus");
        assert_eq!(
            with_retargeted_menus, retargeted_notice,
            "SetFilmView suppression follows the displayed player, not the physical owner"
        );
        let retargeted_viewport = retargeted
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == eliminated_target)
            .expect("retargeted eliminated viewport remains active")
            .rect;
        assert_eq!(
            retargeted.ingame_menu_pointer_target(GuiPoint::new(
                retargeted_viewport.x as f32 + retargeted_viewport.width as f32 / 2.0,
                retargeted_viewport.y as f32 + retargeted_viewport.height as f32 / 2.0,
            )),
            None
        );
        assert_eq!(
            retargeted
                .script_menu_pointer_target_for_owner(
                    local_owner,
                    GuiPoint::new(
                        retargeted_viewport.x as f32
                            + retargeted_viewport.width as f32 / 2.0,
                        retargeted_viewport.y as f32
                            + retargeted_viewport.height as f32 / 2.0,
                    ),
                )
                .expect("retargeted hidden script-menu routing is inert"),
            None
        );

        app.clear_physical_viewport_states();
        let observer = app.ownerless_physical_viewport_state();
        app.physical_viewports.push(observer);
        app.physical_viewports_authoritative = true;
        assert!(app.set_physical_film_view(owner));
        let mut ownerless_notice_only = vec![0_u8; app.graphics.surface().pixels().len()];
        app.render(&mut ownerless_notice_only)
            .expect("render eliminated player through physical observer viewport");
        app.ingame_menu
            .replace(OWNER_NONE, Some(IngameMenuState::surrender_menu()));
        let mut with_hidden_fullscreen_menu =
            vec![0_u8; app.graphics.surface().pixels().len()];
        app.render(&mut with_hidden_fullscreen_menu)
            .expect("eliminated observer target suppresses the fullscreen menu");
        assert_eq!(
            with_hidden_fullscreen_menu, ownerless_notice_only,
            "the fullscreen menu contributes no eliminated-viewport pixels"
        );

        let viewport = app
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.owner == owner)
            .expect("eliminated player retains a viewport")
            .rect;
        let menu_point = GuiPoint::new(
            viewport.x as f32 + viewport.width as f32 / 2.0,
            viewport.y as f32 + viewport.height as f32 / 2.0,
        );
        assert_eq!(
            app.script_menu_pointer_target_for_owner(owner, menu_point)
                .expect("hidden script menu pointer routing is inert"),
            None
        );
        app.local_controls = LocalControlRegistry::default();
        app.mouse_control = true;
        assert_eq!(app.ingame_menu_pointer_target(menu_point), None);

        app.startup_tooltip_resources.insert(
            "IDS_PLR_ELIMINATED".to_string(),
            "Spieler %s|eliminiert!".to_string(),
        );
        app.startup_tooltip_resources.insert(
            "IDS_PLR_SURRENDERED".to_string(),
            "Spieler %s|hat aufgegeben.".to_string(),
        );
        install_native_test_fonts(&mut app, 3.0);
        let (_, _, eliminated_plan) = render_ordered_test_frame(&mut app, 3.0, 960, 600);
        let eliminated_commands = eliminated_plan
            .batches
            .iter()
            .flat_map(|batch| &batch.text)
            .collect::<Vec<_>>();
        let eliminated = eliminated_commands
            .iter()
            .copied()
            .find(|command| command.text == "Spieler Ada|eliminiert!")
            .expect("localized eliminated notice reaches FontRegular");
        assert_eq!(
            eliminated.role,
            clonk_graphics::clonk_font::ClonkFontRole::GuiText
        );
        assert_eq!(eliminated.color, [255, 0, 0, 250]);
        assert_eq!(
            eliminated.align,
            clonk_graphics::clonk_font::TextAlign::Center
        );
        assert!(eliminated.markup, "the resource pipe splits two lines");
        assert_eq!(eliminated.clip, Some(viewport));
        assert_eq!(
            (eliminated.x, eliminated.y),
            (
                viewport.x + viewport.width as i32 / 2,
                viewport.y + 2 * viewport.height as i32 / 3,
            )
        );
        assert!(eliminated_commands.iter().all(|command| !matches!(
            command.text.as_str(),
            "Choose" | "First" | "Surrender" | "Yes"
        )));

        let surrendered_player = app
            .snapshot
            .players
            .iter_mut()
            .find(|player| player.id == owner)
            .expect("sandbox snapshot player");
        surrendered_player.status = PlayerStatus::Surrendered;
        surrendered_player.surrendered = true;
        let (_, _, surrendered_plan) = render_ordered_test_frame(&mut app, 3.0, 960, 600);
        let surrendered_commands = surrendered_plan
            .batches
            .iter()
            .flat_map(|batch| &batch.text)
            .collect::<Vec<_>>();
        let surrendered = surrendered_commands
            .iter()
            .copied()
            .find(|command| command.text == "Spieler Ada|hat aufgegeben.")
            .expect("localized surrendered notice reaches FontRegular");
        assert_eq!(surrendered.color, [255, 0, 0, 250]);
        assert_eq!(
            surrendered.align,
            clonk_graphics::clonk_font::TextAlign::Center
        );
        assert!(surrendered_commands.iter().all(|command| !matches!(
            command.text.as_str(),
            "Choose" | "First" | "Surrender" | "Yes"
        )));
    }

    #[test]
    fn change_to_local_preserves_synchronized_league_state() {
        // ActivateMain suppresses New Player while Game.Parameters.isLeague()
        // sees a nonempty synchronized LeagueAddress. JoinData replaces those
        // parameters for clients. C4GameControl::ChangeToLocal clears network
        // control without clearing Game.Parameters, so the league gate remains
        // part of the running round (pristine 9ffa0a5d
        // src/C4MainMenu.cpp:643-686; src/C4GameParameters.h:126-173;
        // src/C4GameControl.cpp:93-127; src/C4Network2.cpp:1595-1602).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, event_tx, _commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        let host_config = clonk_network::HostConfig::default();
        let mut snapshot = host_config
            .initial_join_snapshot
            .expect("default host publishes JoinData");
        snapshot.parameters.league =
            clonk_engine::LegacyCString::from_bytes(b"League".to_vec()).unwrap();
        snapshot.parameters.league_address =
            clonk_engine::LegacyCString::from_bytes(b"https://league.invalid/".to_vec()).unwrap();
        snapshot.parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
            last_player_id: 41,
            clients: vec![clonk_network::ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 41,
                    flags: clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                    league_progress_data: clonk_engine::LegacyCString::from_bytes(
                        b"retained-progress".to_vec(),
                    )
                    .unwrap(),
                    ..Default::default()
                }],
            }],
        };
        snapshot
            .parameters
            .clients
            .clients
            .push(clonk_engine::ClientCoreControlData {
                client_id: 7,
                activated: false,
                observer: true,
                ..Default::default()
            });
        snapshot.parameters.clients.local_client_id = Some(7);
        let synchronized_max_players = snapshot.parameters.max_players as usize;
        event_tx
            .send(NetworkEvent::JoinData(clonk_network::JoinDataEnvelope {
                client_id: 7,
                start_control_tick: snapshot.dynamic_tick,
                status: host_config.initial_status,
                dynamic: snapshot.dynamic,
                parameters: snapshot.parameters,
            }))
            .expect("queue league JoinData");

        app.process_network_events().expect("apply league JoinData");
        app.pending_network_join_data = None;

        assert_eq!(app.network_league_name, b"League");
        assert_eq!(app.engine.snapshot().league_name, b"League");
        assert_eq!(
            app.engine
                .snapshot()
                .player_info_league_progress_data
                .get(&41),
            Some(&Some(b"retained-progress".to_vec()))
        );

        let conditions = app.main_menu_conditions();
        assert!(conditions.is_league);
        let menu = IngameMenuState::main_menu(&conditions).expect("main menu has entries");
        assert!(
            !menu
                .items()
                .iter()
                .any(|item| item.action == MenuAction::ActivateNewPlayer)
        );

        app.change_network_control_to_local(7);
        assert!(app.main_menu_conditions().is_league);
        assert_eq!(app.network_max_players, synchronized_max_players);
        assert_eq!(app.engine.snapshot().league_name, b"League");
        assert_eq!(
            app.engine
                .snapshot()
                .player_info_league_progress_data
                .get(&41),
            Some(&Some(b"retained-progress".to_vec()))
        );
    }

    #[test]
    fn league_update_applies_projected_gains_and_directly_rebroadcasts_owners() {
        let mut app = new_state_only_running_sandbox_app();
        let (manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
        app.control_player_infos.replace_snapshot(
            20,
            [
                clonk_engine::PlayerInfoControlData {
                    client_id: 3,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 10,
                        league_projected_gain: 1,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 4,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 20,
                        league_projected_gain: 2,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        );
        let response = clonk_network::LeagueUpdateResponse {
            player_infos: clonk_network::ClientPlayerInfosSnapshot {
                client_id: -1,
                flags: 0,
                players: vec![
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 20,
                        league_projected_gain: 7,
                        ..Default::default()
                    },
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 10,
                        league_projected_gain: 1,
                        ..Default::default()
                    },
                ],
            },
            ..clonk_network::LeagueUpdateResponse::default()
        };
        event_tx
            .send(NetworkEvent::LeagueUpdate(response.clone()))
            .expect("queue league Update reply");

        app.process_network_events().expect("apply league Update");

        assert_eq!(
            app.control_player_infos
                .get(20)
                .unwrap()
                .league_projected_gain,
            7
        );
        let (broadcasts, invalidations) = commands.take_league_update_effects();
        assert_eq!(
            broadcasts
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![4]
        );
        assert_eq!(invalidations, 1);

        event_tx
            .send(NetworkEvent::LeagueUpdate(response))
            .expect("queue unchanged league Update reply");
        app.process_network_events()
            .expect("ignore unchanged league Update");
        assert_eq!(commands.take_league_update_effects(), (Vec::new(), 0));
    }

    #[test]
    fn league_host_and_client_report_the_correct_connection_failure_players() {
        let joined = |id| clonk_engine::ControlPlayerInfoEntry {
            id,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            ..Default::default()
        };

        let mut host = new_state_only_running_sandbox_app();
        let (manager, event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        host.network = Some(manager);
        host.network_mode = Some(NetworkMode::Host(host_network_settings()));
        host.network_is_league = true;
        host.control_player_infos.replace_snapshot(
            41,
            [clonk_engine::PlayerInfoControlData {
                client_id: 8,
                players: vec![joined(41)],
                ..Default::default()
            }],
        );
        let host_report = std::thread::spawn(move || commands.complete_league_disconnect_report());
        event_tx
            .send(NetworkEvent::PeerConnectionFailed { client_id: 8 })
            .expect("queue host route loss");
        host.process_network_events().expect("report host route loss");
        let (reason, players) = host_report
            .join()
            .expect("join host report worker")
            .expect("host sent report");
        assert_eq!(reason, clonk_network::LeagueDisconnectReason::ConnectionFailed);
        assert_eq!(players.client_id, 8);
        assert_eq!(players.players[0].id, 41);

        let mut client = new_state_only_running_sandbox_app();
        let (manager, event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        client.network = Some(manager);
        client.network_mode = Some(NetworkMode::Client(client_network_settings()));
        client.network_is_league = true;
        client.control_player_infos.replace_snapshot(
            55,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                players: vec![joined(55)],
                ..Default::default()
            }],
        );
        let client_report =
            std::thread::spawn(move || commands.complete_league_disconnect_report());
        event_tx
            .send(NetworkEvent::PeerDisconnected {
                client_id: 0,
                reason: Some("lost".to_string()),
            })
            .expect("queue client host loss");
        client
            .process_network_events()
            .expect("report client host loss");
        let (reason, players) = client_report
            .join()
            .expect("join client report worker")
            .expect("client sent report");
        assert_eq!(reason, clonk_network::LeagueDisconnectReason::ConnectionFailed);
        assert_eq!(players.client_id, 7);
        assert_eq!(players.players[0].id, 55);
    }

    #[test]
    fn league_player_rejection_uses_cpp_swap_with_last_iteration_order() {
        let mut players = [1, 2, 3, 4]
            .into_iter()
            .map(|id| clonk_engine::ControlPlayerInfoEntry {
                id,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let mut visited = Vec::new();

        retain_player_infos_with_cpp_swap_remove(&mut players, |player| {
            visited.push(player.id);
            player.id != 1 && player.id != 3
        });

        assert_eq!(visited, vec![1, 4, 2, 3]);
        assert_eq!(
            players.iter().map(|player| player.id).collect::<Vec<_>>(),
            vec![4, 2]
        );
    }

    fn poll_league_auth_until(
        app: &mut GameApp,
        context: &str,
        mut complete: impl FnMut(&GameApp) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !complete(app) {
            assert!(Instant::now() < deadline, "timed out waiting for {context}");
            app.poll_league_player_auth().expect("poll Auth exchange");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn abort_open_league_signup(app: &mut GameApp) {
        let abort = app
            .league_signup_dialog
            .as_mut()
            .expect("league signup dialog")
            .controller
            .abort();
        app.process_league_signup_actions(vec![abort])
            .expect("abort league signup");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss league signup cancellation notice");
    }

    #[test]
    fn league_client_authenticates_each_published_player_and_submits_only_auid_survivors() {
        // JoinLocalPlayer publishes player resources first, authenticates each
        // loaded row in packet order, removes failures in place, and sends the
        // successful AUIDs in one CIF_Initial packet
        // (src/C4Network2Players.cpp:78-137;
        // src/C4Network2.cpp:2596-2738).
        let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("user data");
        let players = tempdir().expect("configured players");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_root)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        paths.ensure_user_dirs().expect("create user directories");
        let write_player = |filename: &str, name: &str, color: u32| {
            let path = players.path().join(filename);
            let mut group = clonk_resources::MutableGroup::new(filename);
            group
                .add_file_with_metadata(
                    "Player.txt",
                    format!("[Player]\nName={name}\n[Preferences]\nColorDw={color}\n").into_bytes(),
                    1,
                    false,
                )
                .expect("add player core");
            fs::write(&path, group.pack().expect("pack player")).expect("write player group");
            path
        };
        let accepted_path = write_player("Accepted.c4p", "Accepted", 0x11_22_33);
        let rejected_path = write_player("Rejected.c4p", "Rejected", 0x44_55_66);
        let mut config = b"[General]\nName=Maker\nParticipants=\"".to_vec();
        config.extend_from_slice(accepted_path.as_os_str().as_encoded_bytes());
        config.push(b';');
        config.extend_from_slice(rejected_path.as_os_str().as_encoded_bytes());
        config.extend_from_slice(b"\"\n");
        fs::write(paths.config_file(), config).expect("write configured participants");

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
        .expect("initialize app");
        wait_for_menu(&mut app);
        app.freeze_configured_client_players_for_game()
            .expect("freeze configured players");
        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        let auth = clonk_network::LeagueAuthRequestHead {
            account: clonk_engine::LegacyCString::from_bytes(b"account".to_vec()).unwrap(),
            password: clonk_engine::LegacyCString::from_bytes(b"password".to_vec()).unwrap(),
            ..Default::default()
        };
        app.network_mode = Some(NetworkMode::Client(
            ClientSettings::new(
                SocketAddr::from(([127, 0, 0, 1], 11_112)),
                "Client",
            )
            .with_league_auth(auth.clone()),
        ));
        app.network_is_league = true;
        let configured_paths = [accepted_path, rejected_path];
        let cores = configured_paths
            .iter()
            .enumerate()
            .map(|(index, path)| clonk_engine::NetworkResourceCore {
                resource_type: clonk_network::HostResourceType::Player as u8,
                id: (7 << 16) + index as i32,
                loadable: true,
                filename: clonk_engine::LegacyCString::from_bytes(
                    path.as_os_str().as_encoded_bytes().to_vec(),
                )
                .unwrap(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let all_rejected_cores = cores.clone();
        let responses = vec![
            clonk_network::decode_league_auth_response(
                b"[Response]\r\nStatus=Success\r\nAUID=accepted-token\r\n",
            ),
        ];
        let observer = thread::spawn(move || {
            commands.complete_initial_league_client_join(cores, responses)
        });

        assert_eq!(
            app.submit_initial_client_player_info(7, "league.example".to_string()),
            LeaguePlayerAuthStatus::Pending
        );
        poll_league_auth_until(&mut app, "Auth error", |app| {
            matches!(
                app.message_dialogs.last().map(|dialog| &dialog.continuation),
                Some(MessageDialogContinuation::LeaguePlayerAuthError)
            )
        });
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("acknowledge rejected Auth");
        let retry = {
            let retry = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("rejected Auth reopens login")
                .controller;
            retry.set_password("replacement");
            retry.submit()
        };
        app.process_league_signup_actions(vec![retry])
            .expect("submit rejected Auth retry");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
            .expect("Abort drops the rejected player");
        let (order, auth_heads, auth_players, requests) =
            observer.join().expect("league command observer");

        assert_eq!(
            order,
            vec!["publish", "publish", "auth", "auth", "auth", "player-info"]
        );
        assert_eq!(&auth_heads[..2], &[auth.clone(), auth.clone()]);
        assert_eq!(
            auth_players
                .iter()
                .map(|player| player.name.as_bytes())
                .collect::<Vec<_>>(),
            vec![
                b"Accepted".as_slice(),
                b"Rejected".as_slice(),
                b"Rejected".as_slice(),
            ]
        );
        let [request] = requests.as_slice() else {
            panic!("expected one initial PlayerInfo request");
        };
        assert_eq!(request.flags, clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL);
        let [player] = request.players.as_slice() else {
            panic!("only the authenticated player should survive");
        };
        assert_eq!(player.name.as_bytes(), b"Accepted");
        assert_eq!(player.auth_id.as_bytes(), b"accepted-token");

        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        if let Some(NetworkMode::Client(settings)) = app.network_mode.as_mut() {
            settings.league_auth = auth;
        }
        let observer = thread::spawn(move || {
            commands.complete_initial_league_client_join(
                all_rejected_cores,
                Vec::new(),
            )
        });
        assert_eq!(
            app.submit_initial_client_player_info(7, "league.example".to_string()),
            LeaguePlayerAuthStatus::Pending
        );
        poll_league_auth_until(&mut app, "Auth error", |app| {
            matches!(
                app.message_dialogs.last().map(|dialog| &dialog.continuation),
                Some(MessageDialogContinuation::LeaguePlayerAuthError)
            )
        });
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("acknowledge first rejected Auth");
        abort_open_league_signup(&mut app);
        abort_open_league_signup(&mut app);
        let (_, _, _, requests) = observer.join().expect("all-rejected observer");
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].players.is_empty(),
            "an initial all-failed auth still sends the empty observer packet"
        );
    }

    #[test]
    fn league_auth_wait_is_abortable_and_success_uses_exact_welcome_confirmation() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated league Auth configuration");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "Network", "LeagueAutoLogin", "0")
            .expect("disable league auto-login");
        let mut app = new_menu_app_with_paths(640, 480, &paths);
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        let auth = clonk_network::LeagueAuthRequestHead {
            account: LegacyCString::from_bytes(b"account".to_vec()).unwrap(),
            password: LegacyCString::from_bytes(b"password".to_vec()).unwrap(),
            ..Default::default()
        };
        app.network_mode = Some(NetworkMode::Client(
            ClientSettings::new(
                SocketAddr::from(([127, 0, 0, 1], 11_112)),
                "Client",
            )
            .with_league_auth(auth.clone()),
        ));
        let player = clonk_engine::ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Exact Player".to_vec()).unwrap(),
            ..Default::default()
        };
        let request = || clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![player.clone()],
        };

        assert_eq!(
            app.begin_league_player_auth_exchange(
                LeaguePlayerAuthContinuation::InitialClient {
                    request: request(),
                    index: 0,
                    server_name: "league.example".to_string(),
                },
                auth.clone(),
                clonk_frontend::league_signup::LeagueSignupMode::Login,
            )
            .expect("begin league Auth"),
            LeaguePlayerAuthStatus::Pending
        );
        let replacement = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                name: LegacyCString::from_bytes(b"Replacement".to_vec()).unwrap(),
                ..Default::default()
            }],
        };
        assert_eq!(
            app.continue_league_player_auth(LeaguePlayerAuthContinuation::InitialClient {
                request: replacement,
                index: 0,
                server_name: "other.example".to_string(),
            })
            .expect("reject overlapping Auth without replacing it"),
            LeaguePlayerAuthStatus::Completed(false)
        );
        assert_eq!(
            app.pending_league_player_auth
                .as_ref()
                .map(|pending| GameApp::league_auth_continuation_player_name(
                    &pending.continuation
                )),
            Some("Exact Player".to_string())
        );
        let wait = app.message_dialogs.last().expect("Auth wait dialog");
        assert_eq!(
            wait.state.message(),
            "League login for player Exact Player on league.example..."
        );
        assert_eq!(wait.state.caption(), "League Login");
        assert_eq!(
            wait.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::Standard(3)
        );
        assert_eq!(
            wait.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::CANCEL
        );
        assert_eq!(
            wait.state.button_label(
                clonk_frontend::message_dialog::MessageDialogButton::Cancel
            ),
            "Abort"
        );
        let command = commands.receive_league_player_auth();
        assert!(command.complete(Ok(clonk_network::decode_league_auth_response(
            b"[Response]\r\nStatus=Success\r\nAUID=one-use-token\r\nMessage=Server welcome\r\n",
        ))));
        app.poll_league_player_auth()
            .expect("resolve successful Auth");

        let welcome = app.message_dialogs.last().expect("welcome confirmation");
        assert_eq!(welcome.state.message(), "Server welcome");
        assert_eq!(welcome.state.caption(), "Confirm League Login");
        assert_eq!(
            welcome.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::Extended(8)
        );
        assert_eq!(
            welcome.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL
        );
        assert_eq!(
            welcome.state.button_label(
                clonk_frontend::message_dialog::MessageDialogButton::Cancel
            ),
            "Abort"
        );
        assert!(commands.take_player_info_updates().is_empty());
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("approve welcome");
        let updates = commands.take_player_info_updates();
        let [submitted] = updates.as_slice() else {
            panic!("welcome approval submits one initial PlayerInfo");
        };
        assert_eq!(submitted.players[0].auth_id.as_bytes(), b"one-use-token");

        assert_eq!(
            app.begin_league_player_auth_exchange(
                LeaguePlayerAuthContinuation::InitialClient {
                    request: request(),
                    index: 0,
                    server_name: "league.example".to_string(),
                },
                auth,
                clonk_frontend::league_signup::LeagueSignupMode::Login,
            )
            .expect("begin abandoned league Auth"),
            LeaguePlayerAuthStatus::Pending
        );
        let abandoned = commands.receive_league_player_auth();
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
            .expect("Abort wait drops current player without hanging");
        assert!(!abandoned.complete(Ok(
            clonk_network::LeagueAuthResponse::default()
        )));
        let updates = commands.take_player_info_updates();
        let [submitted] = updates.as_slice() else {
            panic!("aborted Auth submits the empty observer packet");
        };
        assert!(submitted.players.is_empty());
    }

    #[test]
    fn league_auth_error_dialog_retries_with_cleared_password() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated league Auth configuration");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "Network", "LeagueAutoLogin", "0")
            .expect("disable league auto-login");
        persist_config_value(&paths, "Network", "LeaguePassword", "password")
            .expect("seed remembered password");
        let mut app = new_menu_app_with_paths(640, 480, &paths);
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        let auth = clonk_network::LeagueAuthRequestHead {
            account: LegacyCString::from_bytes(b"account".to_vec()).unwrap(),
            password: LegacyCString::from_bytes(b"password".to_vec()).unwrap(),
            ..Default::default()
        };
        app.network_mode = Some(NetworkMode::Client(
            ClientSettings::new(
                SocketAddr::from(([127, 0, 0, 1], 11_112)),
                "Client",
            )
            .with_league_auth(auth.clone()),
        ));
        let player = clonk_engine::ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Exact Player".to_vec()).unwrap(),
            ..Default::default()
        };
        assert_eq!(
            app.begin_league_player_auth_exchange(
                LeaguePlayerAuthContinuation::InitialClient {
                    request: clonk_network::PlayerInfoUpdateRequest {
                        client_id: 7,
                        flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                        players: vec![player.clone()],
                    },
                    index: 0,
                    server_name: "league.example".to_string(),
                },
                auth,
                clonk_frontend::league_signup::LeagueSignupMode::Login,
            )
            .expect("begin league Auth"),
            LeaguePlayerAuthStatus::Pending
        );
        let rejected = commands.receive_league_player_auth();
        assert_eq!(rejected.auth.password.as_bytes(), b"password");
        assert!(rejected.complete(Ok(clonk_network::decode_league_auth_response(
            b"[Response]\r\nStatus=Failure\r\nMessage=Wrong password\r\n",
        ))));
        app.poll_league_player_auth()
            .expect("resolve rejected Auth");
        let error = app.message_dialogs.last().expect("Auth error dialog");
        assert_eq!(error.state.message(), "League server reply: Wrong password");
        assert_eq!(error.state.caption(), "League Login Failed");
        assert_eq!(
            error.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::ERROR
        );

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("acknowledge server error and retry");
        let retry_submission = {
            let retry = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("login retry")
                .controller;
            assert!(retry.password().is_empty());
            retry.set_password("replacement");
            retry.submit()
        };
        app.process_league_signup_actions(vec![retry_submission])
            .expect("submit login retry");
        let retry = commands.receive_league_player_auth();
        assert_eq!(retry.player, player);
        assert_eq!(retry.auth.password.as_bytes(), b"replacement");
        assert!(retry.complete(Ok(clonk_network::decode_league_auth_response(
            b"[Response]\r\nStatus=Success\r\nAUID=retry-token\r\nAccount=Master\r\n",
        ))));
        app.poll_league_player_auth()
            .expect("resolve retried Auth");
        let welcome = app.message_dialogs.last().expect("derived welcome dialog");
        assert_eq!(
            welcome.state.message(),
            "Player: Exact Player|League user name: Master|Server: league.example"
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("approve retried Auth");
        let updates = commands.take_player_info_updates();
        let [submitted] = updates.as_slice() else {
            panic!("retry success submits initial PlayerInfo");
        };
        assert_eq!(submitted.players[0].auth_id.as_bytes(), b"retry-token");
        assert!(load_league_auth_settings(Some(&paths)).password.is_empty());

        // A declined registration welcome loops in registration mode. Native
        // retains the local old Password fallback and the server's canonical
        // AccountMaster while clearing only the process-global password.
        let registration_auth = clonk_network::LeagueAuthRequestHead {
            account: LegacyCString::from_bytes(b"old-master".to_vec()).unwrap(),
            password: LegacyCString::from_bytes(b"old-password".to_vec()).unwrap(),
            new_account: LegacyCString::from_bytes(b"requested".to_vec()).unwrap(),
            new_password: LegacyCString::from_bytes(b"old-password".to_vec()).unwrap(),
        };
        assert_eq!(
            app.begin_league_player_auth_exchange(
                LeaguePlayerAuthContinuation::InitialClient {
                    request: clonk_network::PlayerInfoUpdateRequest {
                        client_id: 7,
                        flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                        players: vec![player],
                    },
                    index: 0,
                    server_name: "league.example".to_string(),
                },
                registration_auth,
                clonk_frontend::league_signup::LeagueSignupMode::Registration,
            )
            .expect("begin registration Auth"),
            LeaguePlayerAuthStatus::Pending
        );
        let registration = commands.receive_league_player_auth();
        assert!(registration.complete(Ok(clonk_network::decode_league_auth_response(
            b"[Response]\r\nStatus=Success\r\nAUID=registration-token\r\nAccount=canonical-master\r\n",
        ))));
        app.poll_league_player_auth()
            .expect("resolve registration Auth");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
            .expect("decline registration welcome");
        assert!(matches!(
            app.message_dialogs.last().map(|dialog| &dialog.continuation),
            Some(MessageDialogContinuation::LeaguePlayerAuthCancelled)
        ));
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss registration cancellation notice");
        let registration_retry = app
            .league_signup_dialog
            .as_ref()
            .expect("registration welcome cancellation reopens registration");
        assert_eq!(
            registration_retry.controller.mode(),
            clonk_frontend::league_signup::LeagueSignupMode::Registration
        );
        assert_eq!(registration_retry.auth.account.as_bytes(), b"canonical-master");
        assert_eq!(registration_retry.auth.password.as_bytes(), b"old-password");
        assert!(registration_retry.auth.new_account.is_empty());
        assert!(registration_retry.auth.new_password.is_empty());
    }

    #[test]
    fn league_runtime_player_auth_defers_add_until_welcome_approval() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated runtime Auth configuration");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "Network", "LeagueAutoLogin", "0")
            .expect("disable league auto-login");
        let mut app = new_menu_app_with_paths(640, 480, &paths);
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        let auth = clonk_network::LeagueAuthRequestHead {
            account: LegacyCString::from_bytes(b"account".to_vec()).unwrap(),
            password: LegacyCString::from_bytes(b"password".to_vec()).unwrap(),
            ..Default::default()
        };
        app.network_mode = Some(NetworkMode::Client(
            ClientSettings::new(
                SocketAddr::from(([127, 0, 0, 1], 11_112)),
                "Client",
            )
            .with_league_auth(auth.clone()),
        ));
        let player = clonk_engine::ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Runtime Player".to_vec()).unwrap(),
            ..Default::default()
        };
        assert_eq!(
            app.begin_league_player_auth_exchange(
                LeaguePlayerAuthContinuation::RuntimePlayer {
                    request: clonk_network::PlayerInfoUpdateRequest {
                        client_id: 7,
                        flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                        players: vec![player.clone()],
                    },
                    index: 0,
                    server_name: "league.example".to_string(),
                    host: false,
                    alternate_resource_id: 0,
                    alternate_color: 0,
                },
                auth,
                clonk_frontend::league_signup::LeagueSignupMode::Login,
            )
            .expect("begin runtime league Auth"),
            LeaguePlayerAuthStatus::Pending
        );
        let command = commands.receive_league_player_auth();
        assert_eq!(command.player, player);
        assert!(command.complete(Ok(clonk_network::decode_league_auth_response(
            b"[Response]\r\nStatus=Success\r\nAUID=runtime-token\r\nMessage=Welcome runtime player\r\n",
        ))));
        app.poll_league_player_auth()
            .expect("resolve runtime league Auth");
        assert!(commands.take_player_info_updates().is_empty());

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("approve runtime welcome");
        let updates = commands.take_player_info_updates();
        let [submitted] = updates.as_slice() else {
            panic!("runtime approval submits exactly one add request");
        };
        assert_eq!(submitted.flags, clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS);
        assert_eq!(submitted.players[0].auth_id.as_bytes(), b"runtime-token");
    }

    #[test]
    fn league_signup_retains_client_server_caption_after_join_envelope_release() {
        let mut mode = NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        ));
        {
            let joined_league_address =
                LegacyCString::from_bytes(b"https://league.example:443/action".to_vec()).unwrap();
            assert_eq!(
                retain_client_league_server_name(Some(&mut mode), &joined_league_address),
                "league.example"
            );
        }

        assert_eq!(
            retained_client_league_server_name(Some(&mode)),
            "league.example",
            "the caption host must survive release of the JoinData envelope"
        );
    }

    #[test]
    fn league_signup_persists_account_but_not_session_password() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated league configuration");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let account = LegacyCString::from_bytes(b"Andr\xe9".to_vec()).unwrap();

        persist_league_account_preference(&paths, &account).expect("persist native league account");
        let config = fs::read(paths.config_file()).expect("read native league configuration");
        assert_eq!(
            clonk_app_netplay::configured_native_value(&config, "Network", "LeagueNick")
                .expect("persisted LeagueNick")
                .as_bytes(),
            b"Andr\xe9"
        );
        assert!(
            clonk_app_netplay::configured_native_value(&config, "Network", "LeaguePassword").is_none(),
            "C++ keeps the entered password in process memory only"
        );

        // C4Config never compiles LeaguePassword. Even a stale hand-written
        // key cannot become an auto-login credential after process startup.
        persist_native_config_values(
            &paths,
            "Network",
            &[
                (
                    "LeaguePassword",
                    clonk_app_netplay::NativeConfigValue::CppEscapedString(b"secret"),
                ),
                ("LeagueAutoLogin", clonk_app_netplay::NativeConfigValue::RawAscii("1")),
            ],
        )
        .expect("seed ignored disk password");
        let mut app = new_menu_app_with_paths(320, 200, &paths);
        assert!(load_league_auth_settings(Some(&paths)).password.is_empty());
        assert!(app.league_login_prompt_required());

        app.league_auth_session = Some(clonk_network::LeagueAuthRequestHead {
            account,
            password: LegacyCString::from_bytes(b"session secret".to_vec()).unwrap(),
            ..Default::default()
        });
        assert!(!app.league_login_prompt_required());
        persist_native_config_values(
            &paths,
            "Network",
            &[("LeagueAutoLogin", clonk_app_netplay::NativeConfigValue::RawAscii("0"))],
        )
        .expect("disable league auto-login");
        assert!(app.league_login_prompt_required());
    }

    #[test]
    fn league_signup_edit_interactions_match_classic_edit_behavior() {
        use clonk_frontend::league_signup::{
            LeagueSignupAction, LeagueSignupConfig, LeagueSignupControl,
            LeagueSignupEditClipboardShortcut, LeagueSignupEditContextCommand,
            LeagueSignupEditKey, LeagueSignupField, LeagueSignupKeyModifiers, LeagueSignupMode,
            LeagueSignupStrings,
        };

        let app = new_classic_running_sandbox_app();
        let font = &app.assets.clonk_fonts.as_deref().expect("classic fonts").text;
        let mut login = clonk_frontend::league_signup::LeagueSignupController::new(
            LeagueSignupConfig::new("Player", "league.example", LeagueSignupMode::Login)
                .with_preferences("account", "secret"),
            LeagueSignupStrings::default(),
        );
        let layout = login.layout(1280, 720, font);

        login.set_account(&"wide".repeat(80));
        login.set_focus(LeagueSignupControl::Account);
        login.handle_edit_key_with_layout(
            LeagueSignupEditKey::End,
            LeagueSignupKeyModifiers::default(),
            &layout,
            font,
        );
        assert!(login.field_horizontal_scroll(LeagueSignupField::Account) > 0);

        login.set_account("alpha beta");
        login.set_focus(LeagueSignupControl::Password);
        let beta_x = layout.account.edit.x
            + 4
            + font.measure("alpha be", false).0;
        let edit_y = layout.account.edit.y + layout.account.edit.h / 2;
        login.handle_pointer_double_click(
            GuiPoint::new(beta_x as f32, edit_y as f32),
            &layout,
            font,
        );
        let (start, end) = login
            .field_selection(LeagueSignupField::Account)
            .expect("double-click word selection");
        assert_eq!(&login.account()[start..end], "beta");
        assert_eq!(
            login.focused_control(),
            Some(LeagueSignupControl::Password),
            "LeftDouble does not synthesize Control's LeftDown focus transfer"
        );

        let left = GuiPoint::new((layout.account.edit.x + 5) as f32, edit_y as f32);
        let right = GuiPoint::new(
            (layout.account.edit.x + layout.account.edit.w - 5) as f32,
            edit_y as f32,
        );
        login.handle_pointer_down(left, &layout, font);
        login.handle_pointer_up(right, &layout, font);
        assert!(login.field_selection(LeagueSignupField::Account).is_some());

        login.set_focus(LeagueSignupControl::Password);
        login.handle_clipboard_shortcut(
            LeagueSignupEditClipboardShortcut::SelectAll,
            None,
            &layout,
            font,
        );
        assert!(matches!(
            login
                .handle_clipboard_shortcut(
                    LeagueSignupEditClipboardShortcut::Copy,
                    None,
                    &layout,
                    font,
                )
                .as_slice(),
            [LeagueSignupAction::ClipboardTransfer { text, cut: false, .. }]
                if text == "secret"
        ));
        assert!(matches!(
            login
                .handle_clipboard_shortcut(
                    LeagueSignupEditClipboardShortcut::Cut,
                    None,
                    &layout,
                    font,
                )
                .as_slice(),
            [LeagueSignupAction::ClipboardTransfer { cut: true, .. }]
        ));
        assert!(matches!(
            login
                .confirm_clipboard_cut(LeagueSignupField::Password, &layout, font)
                .as_slice(),
            [LeagueSignupAction::TextChanged { .. }]
        ));
        assert!(login.password().is_empty());

        login.set_focus(LeagueSignupControl::Account);
        login.handle_clipboard_shortcut(
            LeagueSignupEditClipboardShortcut::SelectAll,
            None,
            &layout,
            font,
        );
        login.handle_clipboard_shortcut(
            LeagueSignupEditClipboardShortcut::Paste,
            Some("raw|paste"),
            &layout,
            font,
        );
        assert_eq!(login.account(), "raw\u{a6}paste");
        login.set_focus(LeagueSignupControl::Password);
        login.handle_pointer_middle_down(left, Some("|primary"), &layout, font);
        assert!(login.account().contains("|primary"));
        assert_eq!(
            login.focused_control(),
            Some(LeagueSignupControl::Password),
            "native MiddleDown edits without transferring focus"
        );

        login.set_focus(LeagueSignupControl::Account);
        login.handle_clipboard_shortcut(
            LeagueSignupEditClipboardShortcut::SelectAll,
            None,
            &layout,
            font,
        );
        let context = login.request_context_menu_at(left, true, &layout);
        assert!(matches!(
            context.as_slice(),
            [LeagueSignupAction::OpenEditContextMenu(request)]
                if request.field == LeagueSignupField::Account
                    && request.items.iter().any(|item| item.command == LeagueSignupEditContextCommand::Cut)
                    && request.items.iter().any(|item| item.command == LeagueSignupEditContextCommand::Paste)
        ));

        login.set_account("account");
        login.set_password("old");
        login.set_focus(LeagueSignupControl::Password);
        let pasted = login.handle_clipboard_shortcut(
            LeagueSignupEditClipboardShortcut::Paste,
            Some("new-password\nignored"),
            &layout,
            font,
        );
        assert!(pasted
            .iter()
            .any(|action| matches!(action, LeagueSignupAction::Submitted(_))));

        let mut movable = clonk_frontend::league_signup::LeagueSignupController::new(
            LeagueSignupConfig::new("Player", "league.example", LeagueSignupMode::Login),
            LeagueSignupStrings::default(),
        );
        let movable_layout = movable.layout(1280, 720, font);
        let caption_point = GuiPoint::new(
            (movable_layout.caption.x + 10) as f32,
            (movable_layout.caption.y + 10) as f32,
        );
        let moved_point = GuiPoint::new(caption_point.x + 17.0, caption_point.y - 9.0);
        movable.handle_pointer_down(caption_point, &movable_layout, font);
        movable.handle_pointer_move(moved_point, &movable_layout, font);
        movable.handle_pointer_up(moved_point, &movable_layout, font);
        assert_eq!(movable.dialog_offset(), (17, -9));
        movable.reset_location();
        assert_eq!(movable.dialog_offset(), (0, 0));

        let mut registration = clonk_frontend::league_signup::LeagueSignupController::new(
            LeagueSignupConfig::new(
                "Player",
                "league.example",
                LeagueSignupMode::Registration,
            ),
            LeagueSignupStrings::default(),
        );
        registration.set_password_enabled(true);
        registration.set_focus(LeagueSignupControl::Password);
        registration.set_password_enabled(false);
        assert_eq!(registration.focused_control(), None);
        assert!(matches!(
            registration.handle_key_down(KeyCode::Tab, false).as_slice(),
            [LeagueSignupAction::FocusChanged(LeagueSignupControl::Close)]
        ));

        let collapsed = registration.layout(1280, 720, font);
        let checkbox = collapsed.password_checkbox.as_ref().expect("checkbox");
        let checkbox_point = GuiPoint::new(
            (checkbox.square.x + checkbox.square.w / 2) as f32,
            (checkbox.square.y + checkbox.square.h / 2) as f32,
        );
        let ok_point = GuiPoint::new(
            (collapsed.ok_button.x + 2) as f32,
            (collapsed.ok_button.y + 2) as f32,
        );
        registration.handle_pointer_down(ok_point, &collapsed, font);
        assert!(matches!(
            registration
                .handle_pointer_up(checkbox_point, &collapsed, font)
                .as_slice(),
            [LeagueSignupAction::PasswordEnabledChanged(true)]
        ));
        assert_eq!(
            registration.take_sound_events(),
            vec![
                clonk_frontend::league_signup::LeagueSignupSound::ArrowHit,
                clonk_frontend::league_signup::LeagueSignupSound::ArrowHit,
                clonk_frontend::league_signup::LeagueSignupSound::ArrowHit,
            ],
            "pressed-button release and checkbox toggle each keep their native sound"
        );

        for key in [
            VirtualKeyCode::Return,
            VirtualKeyCode::NumpadEnter,
            VirtualKeyCode::Escape,
            VirtualKeyCode::Space,
        ] {
            assert!(league_signup_dialog_key_code(key, ModifiersState::CTRL).is_none());
            assert!(league_signup_dialog_key_code(key, ModifiersState::SHIFT).is_none());
        }
        assert_eq!(
            league_signup_dialog_key_code(VirtualKeyCode::Tab, ModifiersState::SHIFT),
            Some(KeyCode::Tab)
        );
        assert!(
            league_signup_dialog_key_code(VirtualKeyCode::Tab, ModifiersState::CTRL).is_none()
        );

        let first_press = Instant::now();
        let mut last_press = None;
        assert!(!classic_press_is_double_click(
            &mut last_press,
            first_press
        ));
        assert!(classic_press_is_double_click(
            &mut last_press,
            first_press + Duration::from_millis(399)
        ));
        assert!(last_press.is_none(), "native clears the double-click timer");
        assert!(!classic_press_is_double_click(
            &mut last_press,
            first_press + Duration::from_millis(800)
        ));
    }

    #[test]
    fn league_signup_headless_login_registration_and_abort_match_cpp_auth_flow() {
        use clonk_frontend::league_signup::{
            LeagueSignupControl, LeagueSignupField, LeagueSignupMode,
        };

        let mut app = new_classic_running_sandbox_app();
        // Keep the already-loaded exact GUI bundle while making credentials,
        // auto-login and the registration Nick preference deterministic.
        app.app_paths = None;
        app.network_is_league = true;
        let pending_player = || clonk_engine::ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Exact Player".to_vec()).unwrap(),
            forced_name: LegacyCString::from_bytes(b"Forced Player".to_vec()).unwrap(),
            ..Default::default()
        };
        let pending_request = || clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![pending_player()],
        };
        let continuation = |request| LeaguePlayerAuthContinuation::InitialClient {
            request,
            index: 0,
            server_name: "league.example".to_string(),
        };

        // Empty LeaguePassword returns to the event loop with the login form
        // installed. Since no observer is servicing the command channel yet,
        // reaching this assertion also proves no Auth request was submitted.
        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.league_auth_session = Some(clonk_network::LeagueAuthRequestHead::default());
        assert_eq!(
            app.continue_league_player_auth(continuation(pending_request()))
                .expect("open missing-password login"),
            LeaguePlayerAuthStatus::Pending
        );
        let login = &app
            .league_signup_dialog
            .as_ref()
            .expect("login dialog")
            .controller;
        assert_eq!(login.mode(), LeagueSignupMode::Login);
        assert_eq!(login.focused_control(), Some(LeagueSignupControl::Password));
        assert!(login.field_visible(LeagueSignupField::Account));
        assert!(login.field_visible(LeagueSignupField::Password));
        assert!(!login.field_visible(LeagueSignupField::PasswordConfirmation));

        let invalid = app
            .league_signup_dialog
            .as_mut()
            .expect("login dialog")
            .controller
            .submit();
        app.process_league_signup_actions(vec![invalid])
            .expect("show validation modal");
        let validation = app.message_dialogs.last().expect("validation modal");
        assert_eq!(validation.state.caption(), "Invalid Entry");
        assert_eq!(
            validation.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::ERROR
        );
        assert_eq!(
            app.league_signup_dialog
                .as_ref()
                .expect("login remains open")
                .controller
                .focused_control(),
            Some(LeagueSignupControl::Account)
        );
        app.finish_message_dialog_at(
            app.message_dialogs.len() - 1,
            clonk_frontend::message_dialog::MessageDialogResult::Ok,
        )
        .expect("dismiss validation modal");

        let observer = thread::spawn(move || {
            commands.complete_initial_league_client_join(
                Vec::new(),
                vec![clonk_network::decode_league_auth_response(
                    b"[Response]\r\nStatus=Success\r\nAUID=login-token\r\n",
                )],
            )
        });
        let submission = {
            let login = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("login dialog")
                .controller;
            login.set_account("account");
            login.set_password("password");
            login.submit()
        };
        app.process_league_signup_actions(vec![submission])
            .expect("submit login form");
        poll_league_auth_until(&mut app, "login completion", |app| {
            app.pending_league_player_auth.is_none()
        });
        let (order, auth_heads, _, requests) = observer.join().expect("login observer");
        assert_eq!(order, vec!["auth", "player-info"]);
        assert_eq!(auth_heads.len(), 1);
        assert_eq!(auth_heads[0].account.as_bytes(), b"account");
        assert_eq!(auth_heads[0].password.as_bytes(), b"password");
        assert!(auth_heads[0].new_account.is_empty());
        assert!(auth_heads[0].new_password.is_empty());
        assert_eq!(requests[0].players[0].auth_id.as_bytes(), b"login-token");

        // A server refusal is not a failed player join. Native shows the
        // league error first, clears only the process password when that
        // modal closes, and retries the same player with an empty login edit.
        let failed_auth = clonk_network::LeagueAuthRequestHead {
            account: LegacyCString::from_bytes(b"account".to_vec()).unwrap(),
            password: LegacyCString::from_bytes(b"outdated".to_vec()).unwrap(),
            ..Default::default()
        };
        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        app.league_auth_session = Some(failed_auth.clone());
        let observer = thread::spawn(move || {
            commands.complete_initial_league_client_join(
                Vec::new(),
                vec![
                    clonk_network::decode_league_auth_response(
                        b"[Response]\r\nStatus=Failure\r\nMessage=Invalid password\r\n",
                    ),
                    clonk_network::decode_league_auth_response(
                        b"[Response]\r\nStatus=Success\r\nAUID=retry-token\r\n",
                    ),
                ],
            )
        });
        assert_eq!(
            app.continue_league_player_auth(continuation(pending_request()))
                .expect("show failed-auth message"),
            LeaguePlayerAuthStatus::Pending
        );
        poll_league_auth_until(&mut app, "failed-auth message", |app| {
            matches!(
                app.message_dialogs.last().map(|dialog| &dialog.continuation),
                Some(MessageDialogContinuation::LeaguePlayerAuthError)
            )
        });
        assert!(app.league_signup_dialog.is_none());
        let failure = app.message_dialogs.last().expect("failed-auth message");
        assert_eq!(failure.state.caption(), "League Login Failed");
        assert_eq!(failure.state.message(), "League server reply: Invalid password");
        assert_eq!(
            app.league_auth_session
                .as_ref()
                .expect("credentials remain while message is modal")
                .password
                .as_bytes(),
            b"outdated"
        );
        app.finish_message_dialog_at(
            app.message_dialogs.len() - 1,
            clonk_frontend::message_dialog::MessageDialogResult::Ok,
        )
        .expect("dismiss failed-auth message");
        let retry = &app
            .league_signup_dialog
            .as_ref()
            .expect("login retry")
            .controller;
        assert_eq!(retry.mode(), LeagueSignupMode::Login);
        assert_eq!(retry.account(), "account");
        assert!(retry.password().is_empty());
        assert!(app
            .league_auth_session
            .as_ref()
            .expect("session credentials")
            .password
            .is_empty());
        let retry_submission = {
            let retry = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("login retry")
                .controller;
            retry.set_password("replacement");
            retry.submit()
        };
        app.process_league_signup_actions(vec![retry_submission])
            .expect("submit login retry");
        poll_league_auth_until(&mut app, "login retry completion", |app| {
            app.pending_league_player_auth.is_none()
        });
        let (order, auth_heads, _, requests) = observer.join().expect("retry observer");
        assert_eq!(order, vec!["auth", "auth", "player-info"]);
        assert_eq!(auth_heads[0], failed_auth);
        assert_eq!(auth_heads[1].password.as_bytes(), b"replacement");
        assert_eq!(requests[0].players[0].auth_id.as_bytes(), b"retry-token");

        // Abort always closes without validation. Native shows its Notify
        // modal before returning failure to the outer swap-remove loop; only
        // after that modal closes is the empty initial PlayerInfo submitted.
        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        app.league_auth_session = Some(clonk_network::LeagueAuthRequestHead::default());
        assert_eq!(
            app.continue_league_player_auth(continuation(pending_request()))
                .expect("open cancellable login"),
            LeaguePlayerAuthStatus::Pending
        );
        let abort = app
            .league_signup_dialog
            .as_mut()
            .expect("cancellable login")
            .controller
            .abort();
        app.process_league_signup_actions(vec![abort])
            .expect("abort login form");
        assert!(app.league_signup_dialog.is_none());
        assert_eq!(
            app.message_dialogs.last().map(|dialog| dialog.state.icon()),
            Some(clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY)
        );
        let observer = thread::spawn(move || {
            commands.complete_initial_league_client_join(Vec::new(), Vec::new())
        });
        app.finish_message_dialog_at(
            app.message_dialogs.len() - 1,
            clonk_frontend::message_dialog::MessageDialogResult::Ok,
        )
        .expect("dismiss cancellation notice");
        let (order, auth_heads, _, requests) = observer.join().expect("abort observer");
        assert_eq!(order, vec!["player-info"]);
        assert!(auth_heads.is_empty());
        assert!(requests[0].players.is_empty());

        // A Register response, not an empty stored account, selects the
        // registration form. With its optional password unchecked, native
        // sends the old login password as NewPassword.
        let old_auth = clonk_network::LeagueAuthRequestHead {
            account: LegacyCString::from_bytes(b"old-account".to_vec()).unwrap(),
            password: LegacyCString::from_bytes(b"old-password".to_vec()).unwrap(),
            ..Default::default()
        };
        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        app.league_auth_session = Some(old_auth.clone());
        let observer = thread::spawn(move || {
            commands.complete_initial_league_client_join(
                Vec::new(),
                vec![
                    clonk_network::decode_league_auth_response(
                        b"[Response]\r\nStatus=Register\r\nAccount=master-account\r\n",
                    ),
                    clonk_network::decode_league_auth_response(
                        b"[Response]\r\nStatus=Success\r\nAUID=\r\nAccount=retry-master\r\n",
                    ),
                    clonk_network::decode_league_auth_response(
                        b"[Response]\r\nStatus=Success\r\nAUID=registered-token\r\n",
                    ),
                ],
            )
        });
        assert_eq!(
            app.continue_league_player_auth(continuation(pending_request()))
                .expect("receive Register response"),
            LeaguePlayerAuthStatus::Pending
        );
        poll_league_auth_until(&mut app, "registration form", |app| {
            app.league_signup_dialog.as_ref().is_some_and(|dialog| {
                dialog.controller.mode() == LeagueSignupMode::Registration
            })
        });
        let registration = &app
            .league_signup_dialog
            .as_ref()
            .expect("registration dialog")
            .controller;
        assert_eq!(registration.mode(), LeagueSignupMode::Registration);
        assert_eq!(registration.account(), "Forced Player");
        assert_eq!(
            registration.focused_control(),
            Some(LeagueSignupControl::Account)
        );
        assert!(!registration.password_enabled());
        assert!(!registration.field_visible(LeagueSignupField::Password));
        let submission = {
            let registration = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("registration dialog")
                .controller;
            registration.set_account("New User");
            registration.submit()
        };
        app.process_league_signup_actions(vec![submission])
            .expect("submit registration form");
        poll_league_auth_until(&mut app, "registration failure", |app| {
            matches!(
                app.message_dialogs.last().map(|dialog| &dialog.continuation),
                Some(MessageDialogContinuation::LeaguePlayerAuthError)
            )
        });
        assert!(app.league_signup_dialog.is_none());
        let failure = app
            .message_dialogs
            .last()
            .expect("missing-AUID registration failure");
        assert_eq!(failure.state.caption(), "League Login Failed");
        assert_eq!(
            failure.state.message(),
            "League server reply: League server reply without authentication-id!"
        );
        app.finish_message_dialog_at(
            app.message_dialogs.len() - 1,
            clonk_frontend::message_dialog::MessageDialogResult::Ok,
        )
        .expect("dismiss registration failure");
        assert!(app
            .league_auth_session
            .as_ref()
            .expect("session credentials")
            .password
            .is_empty());
        let retry_submission = {
            let registration = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("registration retry")
                .controller;
            assert_eq!(registration.mode(), LeagueSignupMode::Registration);
            assert_eq!(registration.account(), "Forced Player");
            assert!(!registration.password_enabled());
            registration.set_account("Retry User");
            registration.submit()
        };
        app.process_league_signup_actions(vec![retry_submission])
            .expect("submit registration retry");
        poll_league_auth_until(&mut app, "registration retry completion", |app| {
            app.pending_league_player_auth.is_none()
        });
        let (order, auth_heads, _, requests) = observer.join().expect("registration observer");
        assert_eq!(order, vec!["auth", "auth", "auth", "player-info"]);
        assert_eq!(auth_heads[0], old_auth);
        assert_eq!(auth_heads[1].account.as_bytes(), b"master-account");
        assert_eq!(auth_heads[1].password.as_bytes(), b"old-password");
        assert_eq!(auth_heads[1].new_account.as_bytes(), b"New User");
        assert_eq!(auth_heads[1].new_password.as_bytes(), b"old-password");
        assert_eq!(auth_heads[2].account.as_bytes(), b"retry-master");
        assert_eq!(auth_heads[2].password.as_bytes(), b"old-password");
        assert_eq!(auth_heads[2].new_account.as_bytes(), b"Retry User");
        assert_eq!(auth_heads[2].new_password.as_bytes(), b"old-password");
        assert_eq!(
            requests[0].players[0].auth_id.as_bytes(),
            b"registered-token"
        );

        // C4Network2Players::JoinLocalPlayer routes later lobby additions
        // through the same modal before its CIF_AddPlayers request. Resuming
        // must not republish the already-materialized player resource.
        let local_add = || LeaguePlayerAuthContinuation::RuntimePlayer {
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![pending_player()],
            },
            index: 0,
            server_name: "league.example".to_string(),
            host: false,
            alternate_resource_id: 41,
            alternate_color: 0x0012_3456,
        };
        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        app.league_auth_session = Some(clonk_network::LeagueAuthRequestHead::default());
        assert_eq!(
            app.continue_league_player_auth(local_add())
                .expect("open local-add login"),
            LeaguePlayerAuthStatus::Pending
        );
        assert_eq!(
            app.league_signup_dialog
                .as_ref()
                .expect("local-add login")
                .controller
                .mode(),
            LeagueSignupMode::Login
        );
        let observer = thread::spawn(move || {
            commands.complete_initial_league_client_join(
                Vec::new(),
                vec![clonk_network::decode_league_auth_response(
                    b"[Response]\r\nStatus=Success\r\nAUID=local-token\r\n",
                )],
            )
        });
        let submission = {
            let login = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("local-add login")
                .controller;
            login.set_account("account");
            login.set_password("password");
            login.submit()
        };
        app.process_league_signup_actions(vec![submission])
            .expect("submit local-add login");
        poll_league_auth_until(&mut app, "local-add completion", |app| {
            app.pending_league_player_auth.is_none()
        });
        let (order, _, _, requests) = observer.join().expect("local-add observer");
        assert_eq!(order, vec!["auth", "player-info"]);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].flags,
            clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
        );
        assert_eq!(requests[0].players[0].auth_id.as_bytes(), b"local-token");

        let (manager, _event_tx, mut commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        app.network = Some(manager);
        app.league_auth_session = Some(clonk_network::LeagueAuthRequestHead::default());
        assert_eq!(
            app.continue_league_player_auth(local_add())
                .expect("open cancellable local-add login"),
            LeaguePlayerAuthStatus::Pending
        );
        let abort = app
            .league_signup_dialog
            .as_mut()
            .expect("cancellable local-add login")
            .controller
            .abort();
        app.process_league_signup_actions(vec![abort])
            .expect("abort local-add login");
        app.finish_message_dialog_at(
            app.message_dialogs.len() - 1,
            clonk_frontend::message_dialog::MessageDialogResult::Ok,
        )
        .expect("dismiss local-add cancellation notice");
        assert!(
            commands.take_player_info_updates().is_empty(),
            "a cancelled local add must not submit an empty PlayerInfo packet"
        );

        // The staged host owns its manager inside the continuation while the
        // modal returns to the event loop. Accepted players are handed back to
        // startup without rerunning Auth during host finalization.
        let (manager, _event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = None;
        app.network_mode = None;
        app.league_auth_session = Some(clonk_network::LeagueAuthRequestHead::default());
        let host_continuation = LeaguePlayerAuthContinuation::StartupHost {
            mode: NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                player_name: "Host".to_string(),
                prepared: None,
            }),
            manager,
            selected_scenario: None,
            purpose: StartupNetworkPurpose::StagedHost,
            players: vec![pending_player()],
            index: 0,
            server_name: "league.example".to_string(),
        };
        assert_eq!(
            app.continue_league_player_auth(host_continuation)
                .expect("open staged-host login"),
            LeaguePlayerAuthStatus::Pending
        );
        let observer = thread::spawn(move || {
            commands.complete_league_player_auths(vec![clonk_network::decode_league_auth_response(
                b"[Response]\r\nStatus=Success\r\nAUID=host-token\r\n",
            )])
        });
        let submission = {
            let login = &mut app
                .league_signup_dialog
                .as_mut()
                .expect("staged-host login")
                .controller;
            login.set_account("host-account");
            login.set_password("host-password");
            login.submit()
        };
        app.process_league_signup_actions(vec![submission])
            .expect("submit staged-host login");
        poll_league_auth_until(&mut app, "staged-host completion", |app| {
            app.startup_network_connection.is_some()
        });
        let (auth_heads, auth_players) = observer.join().expect("staged-host observer");
        assert_eq!(auth_heads.len(), 1);
        assert_eq!(auth_players.len(), 1);
        assert_eq!(auth_heads[0].account.as_bytes(), b"host-account");
        assert_eq!(auth_players[0].name.as_bytes(), b"Exact Player");
        assert_eq!(
            app.startup_network_connection
                .as_ref()
                .and_then(|connection| connection.authenticated_league_players.as_ref())
                .expect("authenticated staged-host players")[0]
                .auth_id
                .as_bytes(),
            b"host-token"
        );
        assert!(app.startup_network_connection.is_some());
    }

    #[test]
    fn league_lobby_checks_only_new_ids_removes_failures_and_consumes_successful_auid() {
        // HandlePlayerInfoUpdRequest resets gains after normalization, checks
        // only IDs absent from the retained list, removes rejected rows without
        // skipping the shifted successor, and clears a successful AUID before
        // SendUpdatedPlayers/CID_PlrInfo
        // (src/C4Network2Players.cpp:160-239).
        let legacy = |bytes: &[u8]| {
            clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).expect("NUL-free fixture")
        };
        let mut app = new_menu_app(320, 200);
        app.network_is_league = true;
        app.network_league_name = b"Cup".to_vec();
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
        app.control_player_infos.replace_snapshot(
            1,
            [clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    name: legacy(b"Existing"),
                    color: 0x0011_2233,
                    original_color: 0x0011_2233,
                    league_projected_gain: 6,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let (manager, event_tx, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = Some(manager);
        let responses = vec![
            clonk_network::decode_league_join_response(
                b"[Response]\r\nStatus=Failure\r\nMessage=Rejected\r\n",
            ),
            clonk_network::decode_league_join_response(
                b"[Response]\r\nStatus=Success\r\nAccount=Alice\r\nLeague=Cup\r\nScore=42\r\nRank=7\r\nRankSymbol=9\r\nProgressData=level3\r\nClanTag=TAG\r\n",
            ),
        ];
        let observer = thread::spawn(move || {
            commands.complete_host_league_player_checks(responses, 2)
        });
        event_tx
            .send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: 3,
                request: clonk_network::PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![
                        clonk_engine::ControlPlayerInfoEntry {
                            id: 1,
                            name: legacy(b"Existing"),
                            color: 0x0011_2233,
                            original_color: 0x0011_2233,
                            league_projected_gain: 8,
                            ..Default::default()
                        },
                        clonk_engine::ControlPlayerInfoEntry {
                            name: legacy(b"Rejected"),
                            auth_id: legacy(b"reject-token"),
                            color: 0x0044_5566,
                            original_color: 0x0044_5566,
                            league_projected_gain: 9,
                            ..Default::default()
                        },
                        clonk_engine::ControlPlayerInfoEntry {
                            name: legacy(b"Accepted"),
                            auth_id: legacy(b"accept-token"),
                            color: 0x0077_8899,
                            original_color: 0x0077_8899,
                            league_projected_gain: 10,
                            ..Default::default()
                        },
                    ],
                },
                by_host: false,
            })
            .expect("queue league PlayerInfo request");

        app.process_network_events()
            .expect("process league PlayerInfo request");
        let (checked, broadcasts) = observer.join().expect("league check observer");

        let refusal = app
            .message_dialogs
            .last()
            .expect("league check refusal is shown");
        assert_eq!(
            refusal.state.message(),
            "League server has refused the join of player Rejected: Rejected"
        );
        assert_eq!(refusal.state.caption(), "Error");
        assert_eq!(
            refusal.state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::ERROR
        );

        assert_eq!(
            checked
                .iter()
                .map(|player| (player.id, player.auth_id.as_bytes()))
                .collect::<Vec<_>>(),
            vec![(2, b"reject-token".as_slice()), (3, b"accept-token".as_slice())]
        );
        let [gain_reset, admitted, ..] = broadcasts.as_slice() else {
            panic!("expected gain reset before admitted PlayerInfo");
        };
        assert_eq!(gain_reset.client_id, 3);
        assert_eq!(gain_reset.players[0].league_projected_gain, -1);
        assert_eq!(
            admitted
                .players
                .iter()
                .map(|player| player.id)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert!(admitted.players[0].auth_id.is_empty());
        assert_eq!(admitted.players[0].league_projected_gain, -1);
        let accepted = &admitted.players[1];
        assert!(accepted.auth_id.is_empty());
        assert_eq!(accepted.league_account.as_bytes(), b"Alice");
        assert_eq!(accepted.clan_tag.as_bytes(), b"TAG");
        assert_eq!(
            (
                accepted.league_score,
                accepted.league_rank,
                accepted.league_rank_symbol,
                accepted.league_progress_data.as_bytes(),
                accepted.league_projected_gain,
            ),
            (42, 7, 9, b"level3".as_slice(), -1)
        );
    }

    #[test]
    fn league_player_info_request_after_lobby_resets_stored_gains_but_is_not_admitted() {
        // The league branch returns when network state is no longer GS_Lobby.
        // ID allocation/normalization and the internal gain reset precede that
        // return, but neither auth checks nor direct broadcasts occur
        // (src/C4Network2Players.cpp:160-239).
        let mut app = new_state_only_running_sandbox_app();
        app.network_is_league = true;
        app.network_league_name = b"Cup".to_vec();
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.control_player_infos.replace_snapshot(
            1,
            [clonk_engine::PlayerInfoControlData {
                client_id: 2,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    league_projected_gain: 5,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let (manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = Some(manager);
        event_tx
            .send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: 3,
                request: clonk_network::PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        auth_id: clonk_engine::LegacyCString::from_bytes(b"unchecked".to_vec())
                            .unwrap(),
                        ..Default::default()
                    }],
                },
                by_host: false,
            })
            .expect("queue post-lobby PlayerInfo request");

        app.process_network_events()
            .expect("reject post-lobby league request");

        assert_eq!(
            app.control_player_infos
                .get(1)
                .expect("retained player")
                .league_projected_gain,
            -1
        );
        assert!(app.control_player_infos.get(2).is_none());
        assert!(commands.take_broadcast_player_infos().is_empty());
    }

    #[test]
    fn script_league_progress_writes_mirror_null_and_empty_into_player_infos() {
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
        app.network_league_name = b"League".to_vec();
        app.control_player_infos.replace_snapshot(
            41,
            [clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 41,
                    league_score: 321,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        seed_engine_player_info_parameters(
            &mut app.engine,
            &app.network_league_name,
            &app.control_player_infos,
        );
        app.engine
            .install_scenario_script_with_convention(
                "League progress fixture",
                r#"#strict 3
                global func WriteProgress() { return SetLeagueProgressData("data", 41); }
                global func EmptyProgress() { return SetLeagueProgressData("", 41); }
                global func ClearProgress() { return SetLeagueProgressData(nil, 41); }
                global func ReadLeagueScore() { return SetMaxPlayer(GetLeagueScore(41)); }
                global func SetLimit() { return SetMaxPlayer(5); }
                "#,
                true,
            )
            .expect("fixture script installs");
        app.engine
            .call_scenario_script_function("ReadLeagueScore", Vec::new())
            .expect("retained PlayerInfo league score is visible");
        assert_eq!(app.engine.max_players(), Some(321));

        app.engine
            .call_scenario_script_function("WriteProgress", Vec::new())
            .expect("nonempty progress writes");
        app.handle_script_player_info_updates()
            .expect("nonempty progress mirrors");
        let info = app
            .control_player_infos
            .get(41)
            .expect("info remains retained");
        assert!(!info.league_progress_data_is_null);
        assert_eq!(info.league_progress_data.as_bytes(), b"data");
        let published = commands.take_published_join_snapshots();
        let info = &published
            .last()
            .expect("nonempty progress publishes JoinData")
            .parameters
            .player_infos
            .clients[0]
            .players[0];
        assert!(!info.league_progress_data_is_null);
        assert_eq!(info.league_progress_data.as_bytes(), b"data");

        app.engine
            .call_scenario_script_function("EmptyProgress", Vec::new())
            .expect("empty progress writes");
        app.handle_script_player_info_updates()
            .expect("empty progress mirrors");
        let info = app
            .control_player_infos
            .get(41)
            .expect("info remains retained");
        assert!(!info.league_progress_data_is_null);
        assert!(info.league_progress_data.is_empty());
        let published = commands.take_published_join_snapshots();
        let info = &published
            .last()
            .expect("empty progress publishes JoinData")
            .parameters
            .player_infos
            .clients[0]
            .players[0];
        assert!(!info.league_progress_data_is_null);
        assert!(info.league_progress_data.is_empty());

        app.engine
            .call_scenario_script_function("ClearProgress", Vec::new())
            .expect("nil progress clears");
        app.handle_script_player_info_updates()
            .expect("null progress mirrors");
        let info = app
            .control_player_infos
            .get(41)
            .expect("info remains retained");
        assert!(info.league_progress_data_is_null);
        assert!(info.league_progress_data.is_empty());
        let published = commands.take_published_join_snapshots();
        let info = &published
            .last()
            .expect("cleared progress publishes JoinData")
            .parameters
            .player_infos
            .clients[0]
            .players[0];
        assert!(info.league_progress_data_is_null);
        assert!(info.league_progress_data.is_empty());

        app.engine
            .call_scenario_script_function("SetLimit", Vec::new())
            .expect("SetMaxPlayer executes without a player-info update");
        app.handle_script_player_info_updates()
            .expect("SetMaxPlayer mirrors into host parameters");
        let published = commands.take_published_join_snapshots();
        assert_eq!(
            published
                .last()
                .expect("SetMaxPlayer publishes JoinData")
                .parameters
                .max_players,
            5
        );
        assert_eq!(
            app.host_join_snapshot
                .as_ref()
                .expect("host parameters remain retained")
                .parameters
                .max_players,
            5
        );
    }

    #[test]
    fn league_client_desync_reports_joined_local_players_before_change_to_local() {
        let mut app = new_state_only_running_sandbox_app();
        let local_client = 7;
        let local_info = 55;
        app.engine
            .player_mut(app.local_owner)
            .expect("local runtime player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        app.control_clients = ControlClientRegistry::default();
        app.control_clients.register(0, true, false);
        app.control_clients.register(local_client, true, false);
        app.control_player_infos.replace_snapshot(
            local_info,
            [clonk_engine::PlayerInfoControlData {
                client_id: local_client,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: local_info,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let (manager, events, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.network_is_league = true;

        let local = app.engine.sync_check(local_client);
        let mut remote = local.clone();
        remote.random_count = remote.random_count.wrapping_add(1);
        remote.by_client = 0;
        app.sync_checks.record_local(local);
        events
            .send(NetworkEvent::DirectControl(NetworkControl::SyncCheck(
                remote,
            )))
            .expect("queue mismatching host sync check");
        let report = std::thread::spawn(move || commands.complete_league_disconnect_report());

        app.process_network_events()
            .expect("process league desync");

        let (reason, players) = report
            .join()
            .expect("join league report worker")
            .expect("desync report was sent before network clear");
        assert_eq!(reason, clonk_network::LeagueDisconnectReason::Desync);
        assert_eq!(players.client_id, local_client);
        assert_eq!(players.players.len(), 1);
        assert_eq!(players.players[0].id, local_info);
        assert!(app.network.is_none());
        assert!(app.network_mode.is_none());
        assert_eq!(
            app.snapshot.round_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
        );
        assert_eq!(
            app.snapshot.round_results.network_result_message.as_slice(),
            &b"Network: Synchronization loss!"[..]
        );
    }

    #[test]
    fn observer_soft_kicks_players_but_plain_deactivation_does_not() {
        // CUT_Activate(false) preserves existing players. CUT_SetObserver is
        // the soft-kick path: deactivate, keep the client, remove its runtime
        // players, and mark history joined, removed, and disconnected
        // (src/C4Control.cpp:588-620; src/C4PlayerList.cpp:219-239).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _event_tx) = NetworkManager::test_stub();
        app.network = Some(manager);
        app.control_clients.register(3, true, false);
        app.engine
            .register_player(PlayerConfig::new(17, "Remote").with_player_info_id(7))
            .expect("register remote runtime player");
        app.control_player_infos
            .apply(clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 7,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                }],
                ..Default::default()
            });

        app.apply_ready_controls(
            0,
            vec![NetworkControl::ClientUpdate(
                clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    client_id: 3,
                    data: 0,
                    by_client: 0,
                },
            )],
        )
        .expect("execute deactivation");
        assert!(app
            .engine
            .snapshot()
            .players
            .iter()
            .any(|player| player.player_info_id == 7));

        app.apply_ready_controls(
            0,
            vec![NetworkControl::ClientUpdate(
                clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                    client_id: 3,
                    data: 0,
                    by_client: 0,
                },
            )],
        )
        .expect("execute observer soft kick");

        assert!(app.control_clients.contains(3));
        assert!(app.control_clients.is_observer(3));
        assert!(!app.control_clients.is_activated(3));
        assert!(app
            .engine
            .snapshot()
            .players
            .iter()
            .all(|player| player.player_info_id != 7));
        let retained = app.control_player_infos.get(7).expect("history remains");
        let expected_flags = clonk_engine::PLAYER_INFO_FLAG_JOINED
            | clonk_engine::PLAYER_INFO_FLAG_REMOVED
            | clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED;
        assert_eq!(retained.flags & expected_flags, expected_flags);
    }

    #[test]
    fn observer_soft_kick_releases_local_control_assignment_for_reuse() {
        // C4PlayerList::GetLocalByKbdSet and MouseControlTaken scan only the
        // live player list. CUT_SetObserver removes the client's players, so a
        // removed local player immediately stops owning its keyboard/mouse set
        // while unrelated assignments remain intact (pristine 9ffa0a5d
        // src/C4Control.cpp:607-619; src/C4PlayerList.cpp:122-128,156-162,
        // 219-268,466-477,556-562).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _event_tx) = NetworkManager::test_stub_for_client_id(3);
        app.network = Some(manager);
        app.control_clients.register(3, true, false);
        app.engine
            .register_player(PlayerConfig::new(17, "Local").with_player_info_id(7))
            .expect("register locally controlled runtime player");
        app.engine
            .register_player(PlayerConfig::new(18, "Remote").with_player_info_id(8))
            .expect("register unassigned runtime player");
        app.control_player_infos
            .apply(clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 7,
                        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                        ..Default::default()
                    },
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 8,
                        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            });
        app.local_controls = LocalControlRegistry::default();
        let removed = app.local_controls.initialize(LocalControlInit {
            owner: 17,
            preferred_set: 2,
            prefers_mouse: true,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        let retained = app.local_controls.initialize(LocalControlInit {
            owner: 99,
            preferred_set: 3,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        assert_eq!((removed.set, removed.mouse), (2, true));
        assert_eq!((retained.set, retained.mouse), (3, false));

        app.apply_ready_controls(
            0,
            vec![NetworkControl::ClientUpdate(
                clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                    client_id: 3,
                    data: 0,
                    by_client: 0,
                },
            )],
        )
        .expect("execute observer soft kick");

        assert_eq!(app.local_controls.owner_for_set(2), None);
        assert_eq!(app.local_controls.owner_for_set(3), Some(99));
        assert_eq!(app.local_controls.mouse_owner(), None);
        let replacement = app.local_controls.initialize(LocalControlInit {
            owner: 20,
            preferred_set: 2,
            prefers_mouse: true,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        assert_eq!((replacement.set, replacement.mouse), (2, true));
    }

    #[test]
    fn league_end_transport_retry_reissues_and_broadcasts_the_successful_result() {
        let mut app = new_classic_running_sandbox_app();
        let (_, reference) = default_exact_host_reference();
        let (network, _events, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.network_is_league = true;
        app.pending_league_end = Some(PendingLeagueEnd {
            reference,
            record: None,
            attempts: 0,
            last_failure: None,
            terminal_packet: None,
        });
        let success = clonk_network::LeagueRoundResultsPacket {
            success: true,
            result_string: LegacyCString::from_bytes(b"runtime placeholder".to_vec()).unwrap(),
            players: Vec::new(),
        };
        let expected_success = success.clone();
        let observer = thread::spawn(move || {
            commands.complete_league_end_flow(vec![
                LeagueEndAttempt::Retryable {
                    phase: LeagueEndFailurePhase::Send,
                    error: "temporary outage".to_string(),
                },
                LeagueEndAttempt::Finished(Some(success)),
            ])
        });

        app.run_pending_league_end_attempt()
            .expect("first End attempt opens retry dialog");
        let retry = app.message_dialogs.last().expect("Retry/Abort dialog");
        assert_eq!(retry.state.caption(), "League error");
        assert_eq!(
            retry.state.message(),
            "Could not send game result: temporary outage"
        );
        assert_eq!(
            retry.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::RETRY_CANCEL
        );
        assert_eq!(
            retry.state.button_label(
                clonk_frontend::message_dialog::MessageDialogButton::Cancel
            ),
            "Abort"
        );
        assert!(app.game_over_dialog.is_none());

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Retry)
            .expect("Retry reissues End and opens evaluation");
        let observed = observer.join().expect("join End command observer");
        assert_eq!(observed.attempts, 2);
        assert!(observed.finalizations.is_empty());
        assert_eq!(observed.broadcasts.len(), 1);
        assert_eq!(observed.broadcasts[0].success, expected_success.success);
        assert_eq!(
            observed.broadcasts[0].result_string.as_bytes(),
            b"League: evaluation successful."
        );
        assert!(app.game_over_dialog.is_some());
        assert_eq!(
            app.snapshot.round_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::LeagueOk)
        );
    }

    #[test]
    fn league_end_retry_is_capped_at_ten_attempts_before_failed_broadcast() {
        let mut app = new_classic_running_sandbox_app();
        let (_, reference) = default_exact_host_reference();
        let (network, _events, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.network_is_league = true;
        app.pending_league_end = Some(PendingLeagueEnd {
            reference,
            record: None,
            attempts: 0,
            last_failure: None,
            terminal_packet: None,
        });
        let outcomes = (0..LEAGUE_END_MAX_ATTEMPTS)
            .map(|_| LeagueEndAttempt::Retryable {
                phase: LeagueEndFailurePhase::Send,
                error: "offline".to_string(),
            })
            .collect();
        let observer =
            thread::spawn(move || commands.complete_league_end_flow(outcomes));

        app.run_pending_league_end_attempt()
            .expect("first End attempt opens retry dialog");
        for attempt in 1..=LEAGUE_END_MAX_ATTEMPTS {
            assert_eq!(
                app.pending_league_end
                    .as_ref()
                    .expect("End remains pending")
                    .attempts,
                attempt
            );
            app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Retry)
                .expect("Retry advances or finalizes the capped End loop");
        }

        let observed = observer.join().expect("join capped End observer");
        assert_eq!(observed.attempts, usize::from(LEAGUE_END_MAX_ATTEMPTS));
        assert_eq!(
            observed.finalizations,
            vec![b"Could not send game result: offline".to_vec()]
        );
        assert_eq!(observed.broadcasts.len(), 1);
        assert!(!observed.broadcasts[0].success);
        assert!(app.pending_league_end.is_none());
        assert!(app.game_over_dialog.is_some());
        assert_eq!(
            app.snapshot.round_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::LeagueError)
        );
    }

    #[test]
    fn league_end_server_rejection_is_abort_only_and_preserves_legacy_text() {
        let mut app = new_classic_running_sandbox_app();
        let (_, reference) = default_exact_host_reference();
        let (network, _events, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.network_is_league = true;
        app.pending_league_end = Some(PendingLeagueEnd {
            reference,
            record: None,
            attempts: 0,
            last_failure: None,
            terminal_packet: None,
        });
        let rejection = clonk_network::LeagueRoundResultsPacket {
            success: false,
            result_string: LegacyCString::from_bytes(b"Server says Andr\xe9".to_vec()).unwrap(),
            players: Vec::new(),
        };
        let observer = thread::spawn(move || {
            commands.complete_league_end_flow(vec![LeagueEndAttempt::Rejected(rejection)])
        });

        app.run_pending_league_end_attempt()
            .expect("server rejection opens Abort dialog");
        let rejected = app.message_dialogs.last().expect("Abort-only dialog");
        assert_eq!(
            rejected.state.message(),
            "Could not send game result: Server says André"
        );
        assert_eq!(
            rejected.state.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::CANCEL
        );
        assert_eq!(
            rejected.state.button_label(
                clonk_frontend::message_dialog::MessageDialogButton::Cancel
            ),
            "Abort"
        );

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
            .expect("Abort records terminal rejection");
        let observed = observer.join().expect("join End rejection observer");
        assert_eq!(observed.attempts, 1);
        assert_eq!(
            observed.finalizations,
            vec![b"Could not send game result: Server says Andr\xe9".to_vec()]
        );
        assert_eq!(observed.broadcasts.len(), 1);
        assert_eq!(
            observed.broadcasts[0].result_string.as_bytes(),
            b"Could not send game result: Server says Andr\xe9"
        );
        assert_eq!(
            app.snapshot.round_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::LeagueError)
        );
    }

    #[test]
    fn league_end_network_teardown_finalizes_and_broadcasts_an_open_retry() {
        let mut app = new_classic_running_sandbox_app();
        let (_, reference) = default_exact_host_reference();
        let (network, _events, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(0);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.network_is_league = true;
        app.pending_league_end = Some(PendingLeagueEnd {
            reference,
            record: None,
            attempts: 0,
            last_failure: None,
            terminal_packet: None,
        });
        let observer = thread::spawn(move || {
            commands.complete_league_end_flow(vec![LeagueEndAttempt::Retryable {
                phase: LeagueEndFailurePhase::Send,
                error: "closing outage".to_string(),
            }])
        });

        app.run_pending_league_end_attempt()
            .expect("open Retry/Abort dialog");
        app.change_network_control_to_local(0);

        let observed = observer.join().expect("join teardown observer");
        assert_eq!(observed.attempts, 1);
        assert_eq!(
            observed.finalizations,
            vec![b"Could not send game result: closing outage".to_vec()]
        );
        assert_eq!(observed.broadcasts.len(), 1);
        assert!(!observed.broadcasts[0].success);
        assert!(app.pending_league_end.is_none());
        assert!(app.network.is_none());
    }

    #[test]
    fn network_restore_projects_resumed_ids_into_league_teams_and_host_snapshot() {
        let mut app = new_menu_app(320, 200);
        let (network, _events) = NetworkManager::test_stub();
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Host(host_network_settings()));
        app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
        let team_metadata = set_control_test_metadata(
            false,
            vec![
                set_control_test_team(1, vec![91], 0),
                set_control_test_team(2, Vec::new(), 0),
            ],
        );
        app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
            team_metadata.clone(),
        ));
        app.engine
            .set_teams(runtime_teams_from_initial_metadata(&team_metadata));
        app.control_player_infos.replace_snapshot(
            91,
            [clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 91,
                    savegame_player: 7,
                    team: 1,
                    league_score: 55,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let restore = clonk_engine::ControlPlayerInfoEntry {
            id: 7,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            team: 2,
            ..Default::default()
        };
        let (_sender, receiver) = mpsc::channel();
        app.loading_state = Some(ScenarioLoadingState {
            scenario: FrontendScenario::fallback(),
            refreshed_resources: None,
            refreshed_tooltip_font: None,
            refreshed_native_font_source: None,
            refreshed_global_gui_failures: None,
            refreshed_gui_sheet_overrides: None,
            refresh_requested: false,
            receiver,
            finished: false,
            last_progress: 0,
            log: Vec::new(),
            prepared_go: Some(PreparedGoLoadingState {
                status: clonk_network::NetworkStatus {
                    state: clonk_network::NETWORK_STATE_GO,
                    control_mode: 0,
                    target_tick: 0,
                },
                local_reached: true,
                save_game: false,
                network_runtime_join: false,
                restore_player_infos: vec![restore],
                runtime_join_players: Vec::new(),
                initial_game_data: None,
                random_seed: 0,
                use_fair_crew: false,
                fair_crew_strength: 0,
                fair_crew_forced: false,
                allow_debug: true,
                auto_frame_skip: true,
                team_configuration: TeamConfiguration::default(),
                team_registry: Vec::new(),
                definition_modules: None,
            }),
            offline_startup_players: None,
            offline_savegame: None,
            offline_random_seed: None,
        });

        app.prepare_network_savegame_recreation();

        assert_eq!(app.deferred_network_savegame_recreation, vec![(3, 7)]);
        assert_eq!(
            app.engine.snapshot().player_info_league_scores.get(&7),
            Some(&55)
        );
        assert!(!app
            .engine
            .snapshot()
            .player_info_league_scores
            .contains_key(&91));
        assert!(app.engine.teams()[0].player_ids.is_empty());
        assert_eq!(app.engine.teams()[1].player_ids, vec![7]);
        let host = app.host_join_snapshot.as_ref().expect("host JoinData");
        assert_eq!(host.parameters.player_infos.clients[0].players[0].id, 7);
        assert!(host.parameters.teams.teams[0].player_ids.is_empty());
        assert_eq!(host.parameters.teams.teams[1].player_ids, vec![7]);
    }

    #[test]
    fn runtime_client_list_league_actions_gate_activate_and_vote_to_kick() {
        use clonk_frontend::runtime_client_list::RuntimeClientListAction;

        let mut app = new_running_sandbox_app();
        let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
        app.network_is_league = true;
        app.control_clients
            .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
        app.engine
            .register_player(PlayerConfig::new(17, "Remote Player"))
            .expect("register remote runtime player");
        app.engine
            .player_mut(17)
            .expect("remote player exists")
            .set_at_client(clonk_engine::PlayerAtClient::new(7));
        app.snapshot = app.engine.snapshot();

        app.handle_runtime_client_list_action(RuntimeClientListAction::ToggleActivate(7))
            .expect("league refusal is nonfatal");
        assert!(commands.take_submitted_client_updates().is_empty());
        assert_eq!(
            latest_message_board_logical_entry(&app).as_deref(),
            Some("Command not allowed in league games!"),
        );

        app.handle_runtime_client_list_action(RuntimeClientListAction::Kick(7))
            .expect("league kick submits a vote");
        assert_eq!(
            commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 0,
            }]
        );
        assert!(commands.take_submitted_client_removes().is_empty());
    }

    #[test]
    fn runtime_pause_routes_host_league_client_and_unknown_roles_nonfatally() {
        let mut host = new_running_sandbox_app();
        let (host_events, mut host_commands) = install_running_network_stub(&mut host, 0, 0, 2);
        queue_empty_ready_tick(&host, &host_events);
        host.update().expect("execute host control tick zero");
        assert_eq!((host.engine.frame(), host.next_network_control_tick()), (1, 1));
        let pause_target = host.next_network_control_tick();
        host.handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("nonleague host Pause requests synchronized halt");
        let pause_changes = host_commands
            .take_runtime_status_commands()
            .into_iter()
            .filter_map(|command| match command {
                network::TestRuntimeStatusCommand::Change(status) => Some(status),
                network::TestRuntimeStatusCommand::Reached { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(pause_changes.len(), 1);
        assert_eq!(
            (pause_changes[0].state, pause_changes[0].target_tick),
            (clonk_network::NETWORK_STATE_PAUSE, pause_target)
        );
        assert!(host.host_reference_paused);

        let go_target = host.displayed_network_control_tick();
        host.handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("a second host Pause press requests synchronized Go");
        let go_changes = host_commands
            .take_runtime_status_commands()
            .into_iter()
            .filter_map(|command| match command {
                network::TestRuntimeStatusCommand::Change(status) => Some(status),
                network::TestRuntimeStatusCommand::Reached { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(go_changes.len(), 1);
        assert_eq!(
            (go_changes[0].state, go_changes[0].target_tick),
            (clonk_network::NETWORK_STATE_GO, go_target)
        );
        assert_eq!(go_target, 0, "Start uses native's current ControlTick");
        assert_eq!(
            host.runtime_network_status_barrier
                .expect("Go replaces the pending Pause barrier")
                .status,
            go_changes[0]
        );
        assert!(!host.host_reference_paused);
        assert!(!host.take_exit_request());

        for (local_client_id, paused, expected_data) in
            [(0, false, 1), (0, true, 0), (7, false, 1), (7, true, 0)]
        {
            let mut league = new_running_sandbox_app();
            let (_events, mut commands) =
                install_running_network_stub(&mut league, local_client_id, 0, 1);
            league.network_is_league = true;
            league.network_control_running = !paused;
            let pause_target = league.next_network_control_tick();
            league
                .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
                .expect("league Pause submits a nonfatal vote");
            if local_client_id == 0 && !paused {
                let barrier = league
                    .runtime_network_status_barrier
                    .expect("a running league host pauses before its vote echo");
                assert_eq!(
                    (barrier.status.state, barrier.status.target_tick),
                    (clonk_network::NETWORK_STATE_PAUSE, pause_target)
                );
                assert!(league.league_votes.paused_for_vote);
                assert!(league.host_reference_paused);
            }
            assert_eq!(
                commands.take_submitted_votes(),
                vec![clonk_engine::VoteControlData {
                    vote_type: clonk_engine::VOTE_TYPE_PAUSE,
                    approve: true,
                    data: expected_data,
                    by_client: i32::try_from(local_client_id).unwrap(),
                }]
            );
            assert!(!league.take_exit_request());
        }

        let mut evaluated_league_host = new_running_sandbox_app();
        let (_events, mut evaluated_commands) =
            install_running_network_stub(&mut evaluated_league_host, 0, 0, 1);
        evaluated_league_host.network_is_league = true;
        evaluated_league_host.game_over_handled = true;
        evaluated_league_host
            .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("evaluated league host uses direct network Pause");
        assert!(evaluated_commands
            .take_runtime_status_commands()
            .iter()
            .any(|command| matches!(
                command,
                network::TestRuntimeStatusCommand::Change(status)
                    if status.state == clonk_network::NETWORK_STATE_PAUSE
            )));
        assert!(!evaluated_league_host.take_exit_request());

        let mut client = new_running_sandbox_app();
        let (_events, mut client_commands) = install_running_network_stub(&mut client, 7, 0, 1);
        client
            .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("nonleague client Pause is a consumed no-op");
        assert!(client_commands.take_runtime_status_commands().is_empty());
        assert!(client_commands.take_submitted_votes().is_empty());
        assert!(!client.take_exit_request());

        let mut ambiguous = new_running_sandbox_app();
        let (ambiguous_manager, _events, mut ambiguous_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(3);
        ambiguous.network = Some(ambiguous_manager);
        ambiguous.network_mode = Some(NetworkMode::Host(host_network_settings()));
        assert_eq!(ambiguous.runtime_network_role(), RuntimeNetworkRole::Ambiguous);
        ambiguous
            .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("an inconsistent runtime role is safely consumed");
        assert!(ambiguous_commands.take_runtime_status_commands().is_empty());
        assert!(ambiguous_commands.take_submitted_votes().is_empty());
        assert!(!ambiguous.take_exit_request());

        let mut disconnected_host = new_running_sandbox_app();
        let (manager, _events, commands) = NetworkManager::test_stub_with_commands();
        disconnected_host.network = Some(manager);
        disconnected_host.network_mode = Some(NetworkMode::Host(host_network_settings()));
        drop(commands);
        disconnected_host
            .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("a failed host status send never exits through Pause");
        assert!(!disconnected_host.take_exit_request());

        let mut unavailable_key_config = new_running_sandbox_app();
        unavailable_key_config.runtime_key_config_cache = OnceLock::new();
        unavailable_key_config
            .runtime_key_config_cache
            .set(Err("unsupported Pause override".to_string()))
            .expect("install unavailable KeyConfig result");
        unavailable_key_config
            .handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
            .expect("an unavailable Pause mapping is suppressed, never fatal");
        assert_eq!(unavailable_key_config.offline_halt_count, 0);
        assert!(!unavailable_key_config.take_exit_request());
    }

    #[test]
    fn league_abort_confirmation_routes_cancel_and_self_kick_votes() {
        let mut host = new_running_sandbox_app();
        let (_host_events, mut host_commands) =
            install_running_network_stub(&mut host, 0, 0, 1);
        host.network_is_league = true;
        assert!(host.show_abort_dialog(host.local_owner));
        finish_abort_dialog(
            &mut host,
            clonk_frontend::message_dialog::MessageDialogResult::Yes,
        );
        assert_eq!(
            host_commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_CANCEL,
                approve: true,
                data: 0,
                by_client: 0,
            }]
        );
        assert!(matches!(host.mode, AppMode::Running));

        let mut restart_host = new_running_sandbox_app();
        let (_restart_events, mut restart_commands) =
            install_running_network_stub(&mut restart_host, 0, 0, 1);
        restart_host.network_is_league = true;
        assert!(restart_host.show_abort_dialog(restart_host.local_owner));
        finish_abort_dialog(
            &mut restart_host,
            clonk_frontend::message_dialog::MessageDialogResult::Restart,
        );
        let restart_vote = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_CANCEL,
            approve: true,
            data: 0,
            by_client: 0,
        };
        assert_eq!(
            restart_commands.take_submitted_votes(),
            vec![restart_vote]
        );
        assert!(restart_host.abort_restart_pending);
        restart_host.league_votes.add(restart_vote);
        restart_host.execute_league_vote_end(clonk_engine::VoteControlData {
            approve: false,
            ..restart_vote
        });
        assert!(matches!(restart_host.mode, AppMode::Running));
        assert!(
            restart_host.abort_restart_pending,
            "a rejected vote leaves Application.NextMission scheduled"
        );
        assert!(restart_host.message_dialogs.iter().any(|dialog| matches!(
            dialog.continuation,
            MessageDialogContinuation::LeagueSurrender
        )));
        restart_host.loader_render_error = Some("test restart blocker".to_string());
        restart_host
            .hard_abort_running_game()
            .expect("a later hard quit consumes the scheduled restart");
        assert!(!restart_host.abort_restart_pending);
        assert_eq!(
            restart_host.scenario_selector_mode,
            ScenarioSelectorMode::NetworkHost
        );

        let mut client = new_running_sandbox_app();
        let (_client_events, mut client_commands) =
            install_running_network_stub(&mut client, 7, 0, 1);
        client.engine.set_control_host(false);
        client.network_is_league = true;
        assert!(client.show_abort_dialog(client.local_owner));
        finish_abort_dialog(
            &mut client,
            clonk_frontend::message_dialog::MessageDialogResult::Yes,
        );
        assert_eq!(
            client_commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 7,
            }]
        );
        assert!(matches!(client.mode, AppMode::Running));

        let mut observer = new_running_sandbox_app();
        let (_observer_events, mut observer_commands) =
            install_running_network_stub(&mut observer, 9, 0, 1);
        observer.engine.set_control_host(false);
        observer.engine.set_local_players([]);
        observer.local_controls = LocalControlRegistry::default();
        observer.network_is_league = true;
        assert!(observer.show_abort_dialog(OWNER_NONE));
        finish_abort_dialog(
            &mut observer,
            clonk_frontend::message_dialog::MessageDialogResult::Yes,
        );
        assert!(observer_commands.take_submitted_votes().is_empty());
        assert!(matches!(observer.mode, AppMode::Menu));
        assert!(observer.active_scenario.is_none());
    }

    // Surrender ends a local round with evaluation (C4MainMenu.cpp:791-795:
    // the surrendered player counts as inactive for the game-over check).
    #[test]
    fn surrender_from_menu_ends_local_round() {
        clonk_logging::init();
        let mut app = new_classic_running_sandbox_app();
        app.apply_ingame_menu_action(MenuAction::Surrender)
            .expect("surrender");
        for _ in 0..30 {
            app.update().expect("tick after surrender");
            if app.snapshot.game_over {
                break;
            }
        }
        assert!(app.snapshot.game_over, "round should end after surrender");
    }

    #[test]
    fn network_surrender_menu_queues_the_next_authenticated_control_tick() {
        // C4MainMenu queues CID_SurrenderPlayer through CDT_Queue; the
        // control packet captures the local client as iByClient
        // (src/C4MainMenu.cpp:790-795; src/C4Control.cpp:38-56).
        let mut app = new_running_sandbox_app();
        let player = app.local_owner;
        app.engine
            .player_mut(player)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(3));
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(3);
        app.network = Some(manager);
        let tick = app.local_control_submission_tick();

        app.apply_ingame_menu_action(MenuAction::Surrender)
            .expect("queue surrender");

        assert_eq!(
            commands.take_submitted_surrender_players(),
            vec![(
                tick,
                clonk_engine::SurrenderPlayerControlData {
                    player,
                    by_client: 3,
                },
            )]
        );
        assert!(!app
            .engine
            .player(player)
            .expect("local player")
            .surrendered());
    }

    #[test]
    fn non_league_network_part_continues_the_running_round_locally() {
        // Part clears C4Network2, whose C4GameControl::ChangeToLocal path
        // removes remote clients (and their players), clears queued network
        // control, and changes ControlRate to one without resetting the game
        // or Game.Parameters (pristine 9ffa0a5d src/C4MainMenu.cpp:820-831;
        // src/C4GameControl.cpp:93-127; src/C4Client.cpp:124-128,306-317;
        // src/C4PlayerList.cpp:466-476).
        let mut app = new_running_sandbox_app();
        let local_player = app.local_owner;
        let local_client = 3;
        let remote_player = 17;
        let remote_info = 73;
        app.engine
            .player_mut(local_player)
            .expect("local runtime player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        app.engine
            .register_player(
                PlayerConfig::new(remote_player, "Remote").with_player_info_id(remote_info),
            )
            .expect("register remote runtime player");
        app.engine
            .player_mut(remote_player)
            .expect("remote runtime player")
            .set_at_client(clonk_engine::PlayerAtClient::HOST);
        app.engine.set_network_game(true);
        app.engine.initialize_network_control_timing(
            clonk_engine::NetworkControlTiming::new(23, 3).expect("valid network timing"),
        );
        app.snapshot = app.engine.snapshot();

        let (manager, _events, commands) =
            NetworkManager::test_stub_with_commands_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.network_control_clock = Some(NetworkControlClock::new(23, 3));
        app.network_control_running = true;
        app.network_max_players = 8;
        app.network_is_league = false;
        app.control_clients = ControlClientRegistry::default();
        app.control_clients.register(0, true, false);
        app.control_clients.register(local_client, false, false);
        app.control_player_infos
            .apply(clonk_engine::PlayerInfoControlData {
                client_id: 0,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: remote_info,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                }],
                ..Default::default()
            });
        let queued_check = app.engine.sync_check(local_client);
        app.network_ticks.queue(
            23,
            23,
            vec![NetworkControl::SyncCheck(queued_check.clone())],
        );
        app.network_sync.queue(
            23,
            23,
            vec![NetworkControl::SyncCheck(queued_check.clone())],
        );
        app.sync_checks.record_local(queued_check);
        app.apply_ingame_menu_action(MenuAction::ActivateOptions)
            .expect("open options menu");
        assert!(
            app.engine
                .snapshot()
                .round_results
                .network_result
                .is_none(),
            "fresh Part fixture has no earlier, more-specific result"
        );

        let frame_before = app.engine.frame();
        let control_tick_before = app.engine.sync_check(local_client).control_tick;
        let scenario_before = app
            .active_scenario
            .as_ref()
            .map(|scenario| scenario.identifier.clone());
        let graceful_write = thread::spawn(move || commands.complete_graceful_part());

        app.apply_ingame_menu_action(MenuAction::Part)
            .expect("part from network game");

        assert!(
            graceful_write.join().expect("graceful writer exits"),
            "negative ConnRe must be written before local transition"
        );
        assert!(matches!(app.mode, AppMode::Running));
        assert_eq!(app.engine.frame(), frame_before);
        assert_eq!(
            app.engine.sync_check(local_client).control_tick,
            control_tick_before
        );
        assert_eq!(app.engine.control_rate, 1);
        assert_eq!(
            app.active_scenario
                .as_ref()
                .map(|scenario| scenario.identifier.clone()),
            scenario_before
        );
        assert_eq!(
            app.ingame_menu.as_ref().map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::Options)
        );
        assert!(app.network.is_none());
        assert!(app.network_mode.is_none());
        assert!(app.network_control_clock.is_none());
        assert_eq!(app.network_max_players, 8);
        assert!(!app.network_is_league);
        assert!(app.network_ticks.ready.is_empty());
        assert!(app.network_sync.scheduled.is_empty());
        assert!(app.sync_checks.local.is_empty());
        assert!(app.sync_checks.remote.is_empty());
        assert!(app.control_clients.contains(local_client));
        assert!(app.control_clients.is_activated(local_client));
        assert!(!app.control_clients.contains(0));
        assert!(app.engine.player(local_player).is_some());
        assert!(app.engine.player(remote_player).is_none());
        let engine_results = app.engine.snapshot().round_results;
        assert_eq!(
            engine_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
        );
        assert_eq!(
            engine_results.network_result_message.as_slice(),
            b"Game left via player menu."
        );
        assert_eq!(
            app.snapshot.round_results, engine_results,
            "the eventual evaluation screen sees the Part verdict immediately"
        );
        let removed = app
            .control_player_infos
            .get(remote_info)
            .expect("remote player history remains");
        assert_ne!(removed.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED, 0);
        assert_ne!(removed.flags & clonk_engine::PLAYER_INFO_FLAG_DISCONNECTED, 0);
        app.engine
            .install_scenario_script_with_convention(
                "NetworkParameterProbe.c",
                r#"
                    #strict
                    func Initialize() {
                        if (IsNetwork()) SetGravity(77);
                        else SetGravity(23);
                    }
                "#,
                true,
            )
            .expect("probe synchronized IsNetwork parameter");
        assert_eq!(
            app.engine.physics().gravity,
            77,
            "ChangeToLocal preserves Game.Parameters.IsNetworkGame"
        );
        let view_mode_script = format!("GetPlrViewMode({local_player})");
        let view_mode = app
            .engine
            .execute_script_control(
                &clonk_engine::ScriptControlData {
                    target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
                    strictness: clonk_engine::ScriptStrictness::Strict3,
                    script: clonk_engine::LegacyCString::from_bytes(view_mode_script.into_bytes())
                        .expect("view mode probe has no NUL"),
                    by_client: 0,
                },
                ScriptControlPolicy::live(false),
            )
            .expect("local view mode probe executes");
        assert_eq!(
            view_mode,
            Some(Value::Int(clonk_engine::PLAYER_VIEW_MODE_CURSOR)),
            "ChangeToLocal clears SyncMode while preserving IsNetworkGame"
        );

        app.update().expect("continue local simulation");
        assert_eq!(app.engine.frame(), frame_before + 1);
    }

    #[test]
    fn league_network_part_submits_authenticated_self_kick_vote() {
        // League Part starts a self-kick vote when a local player exists; it
        // must not execute the non-league Game.Network.Clear path
        // (pristine 9ffa0a5d src/C4MainMenu.cpp:820-831).
        let mut app = new_running_sandbox_app();
        let local_client = 7;
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client as i32));
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(local_client);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.network_is_league = true;
        app.engine.initialize_network_control_timing(
            clonk_engine::NetworkControlTiming::new(31, 3).expect("valid network timing"),
        );

        app.apply_ingame_menu_action(MenuAction::Part)
            .expect("request league part");

        assert!(app.network.is_some());
        assert!(matches!(app.network_mode, Some(NetworkMode::Client(_))));
        assert_eq!(app.engine.control_rate, 3);
        assert!(matches!(app.mode, AppMode::Running));
        assert_eq!(
            commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: local_client as i32,
                by_client: local_client as i32,
            }]
        );
        assert!(
            app.engine
                .snapshot()
                .round_results
                .network_result
                .is_none(),
            "league self-kick does not execute the ordinary Part verdict"
        );
    }

    #[test]
    fn rate_limited_own_vote_opens_surrender_but_active_duplicate_does_not() {
        // C4Network2::Vote applies one global 120-second limiter only to an
        // inactive (Type,Data) subject. A blocked Cancel or local-self Kick
        // opens the same singleton surrender prompt as a rejected own vote;
        // an already-active local ballot takes the earlier duplicate path
        // without opening it (src/C4Network2.cpp:2842-2868,2974-2982).
        let local_client = 7;
        let setup = || {
            let mut app = new_running_sandbox_app();
            let (manager, _events, commands) =
                NetworkManager::test_stub_with_commands_for_client_id(local_client);
            app.network = Some(manager);
            (app, commands)
        };

        for subject in [
            LeagueVoteSubject {
                vote_type: clonk_engine::VOTE_TYPE_CANCEL,
                data: 0,
            },
            LeagueVoteSubject {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                data: local_client as i32,
            },
        ] {
            let (mut app, mut commands) = setup();
            assert!(app.submit_own_league_vote_at(subject, true, 100));
            assert_eq!(commands.take_submitted_votes().len(), 1);

            assert!(!app.submit_own_league_vote_at(subject, true, 219));
            assert!(commands.take_submitted_votes().is_empty());
            assert_eq!(
                latest_message_board_logical_entry(&app).as_deref(),
                Some(
                    "Voting-Timeout: you have to wait two minutes until you can request a new vote."
                )
            );
            assert_eq!(app.message_dialogs.len(), 1);
            let prompt = &app.message_dialogs[0];
            assert!(matches!(
                prompt.continuation,
                MessageDialogContinuation::LeagueSurrender
            ));
            assert_eq!(prompt.state.caption(), "Voting");
            assert_eq!(
                prompt.state.message(),
                "It was decided that you cannot leave the game. However, you can forfeit the game instead.||Do you want to surrender?"
            );
            assert_eq!(
                prompt.state.buttons(),
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
            );
            assert_eq!(
                prompt.state.icon(),
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM
            );
            assert_eq!(
                prompt.state.focused_button(),
                Some(clonk_frontend::message_dialog::MessageDialogButton::No)
            );
        }

        let subject = LeagueVoteSubject {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            data: local_client as i32,
        };
        let (mut app, mut commands) = setup();
        let log_before = message_board_logical_entries(&app);
        assert!(app.submit_own_league_vote_at(subject, true, 100));
        let own_ballot = commands
            .take_submitted_votes()
            .into_iter()
            .next()
            .expect("initial own ballot");
        app.league_votes.add_at(own_ballot, 100);

        assert!(!app.submit_own_league_vote_at(subject, true, 101));
        assert!(commands.take_submitted_votes().is_empty());
        assert!(app.message_dialogs.is_empty());
        assert_eq!(message_board_logical_entries(&app), log_before);

        let mut app = new_running_sandbox_app();
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client as i32));
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(local_client);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.network_is_league = true;

        app.apply_ingame_menu_action(MenuAction::Part)
            .expect("submit first league Part vote");
        app.apply_ingame_menu_action(MenuAction::Part)
            .expect("rate-limit repeated league Part vote");
        assert_eq!(
            commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: local_client as i32,
                by_client: local_client as i32,
            }]
        );
        assert_eq!(app.message_dialogs.len(), 1);
        assert!(matches!(
            app.message_dialogs[0].continuation,
            MessageDialogContinuation::LeagueSurrender
        ));
        assert_eq!(
            app.message_dialogs[0].state.focused_button(),
            Some(clonk_frontend::message_dialog::MessageDialogButton::No)
        );

        app.apply_ingame_menu_action(MenuAction::Part)
            .expect("keep one surrender prompt on another repeated Part");
        assert!(commands.take_submitted_votes().is_empty());
        assert_eq!(app.message_dialogs.len(), 1);
    }

    #[test]
    fn own_league_vote_cooldown_matches_cpp_subject_rules() {
        // A new subject is blocked while now < iLastOwnVoting + 120, an
        // existing subject bypasses that check, equality is allowed, and an
        // approved own-origin EndVote clears the block
        // (src/C4Network2.cpp:2842-2868,2900-2914;
        // src/C4Network2.h:69-71).
        let local_client = 7;
        let own_kick = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: local_client,
            by_client: local_client,
        };
        let remote_pause = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_PAUSE,
            approve: true,
            data: 1,
            by_client: 2,
        };
        let cancel = LeagueVoteSubject {
            vote_type: clonk_engine::VOTE_TYPE_CANCEL,
            data: 0,
        };
        let kick_other = LeagueVoteSubject {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            data: 3,
        };
        let mut votes = LeagueVoteState::default();

        assert!(votes.try_submit_own_vote_at(LeagueVoteSubject::from(own_kick), 100));
        votes.add_at(own_kick, 100);
        votes.add_at(remote_pause, 101);
        assert!(votes.try_submit_own_vote_at(LeagueVoteSubject::from(remote_pause), 101));
        assert!(!votes.try_submit_own_vote_at(cancel, 219));
        assert!(votes.try_submit_own_vote_at(cancel, 220));

        assert_eq!(
            votes.end_at(
                LeagueVoteSubject::from(own_kick),
                true,
                Some(local_client),
                221,
            ),
            Some(local_client)
        );
        assert!(votes.try_submit_own_vote_at(kick_other, 221));
    }

    #[test]
    fn host_sec1_vote_timeout_queues_negative_vote_end() {
        // C4Network2::OnSec1Timer executes the host-only timeout and queues a
        // synchronized negative VoteEnd for the oldest subject
        // (src/C4Network2.cpp:675-731).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        let vote = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 2,
        };
        app.league_votes.add_at(vote, 100);

        assert!(!app.tick_host_league_vote_timeout_at(110));
        assert!(commands.take_submitted_vote_ends().is_empty());
        assert!(app.tick_host_league_vote_timeout_at(111));
        assert_eq!(
            commands.take_submitted_vote_ends(),
            vec![clonk_engine::VoteControlData {
                approve: false,
                by_client: 0,
                ..vote
            }]
        );
    }

    #[test]
    fn host_single_joined_player_approves_first_vote() {
        // With no C4Team list, all joined player infos form one pseudo-team.
        // A sole connected joined player voting Yes is a strict majority, so
        // the control host queues one synchronized affirmative VoteEnd
        // (src/C4Control.cpp:1366-1442).
        let mut app = new_state_only_running_sandbox_app();
        app.engine
            .player_mut(app.local_owner)
            .expect("host player")
            .set_at_client(clonk_engine::PlayerAtClient::HOST);
        app.control_clients = ControlClientRegistry::default();
        app.control_clients.register(0, true, false);
        let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        let vote = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 0,
        };
        event_tx
            .send(NetworkEvent::DirectControl(NetworkControl::Vote(vote)))
            .expect("queue host ballot");

        app.process_network_events().expect("execute host ballot");

        assert_eq!(
            commands.take_submitted_vote_ends(),
            vec![clonk_engine::VoteControlData {
                by_client: 0,
                ..vote
            }]
        );
    }

    #[test]
    fn only_host_vote_end_clears_its_exact_subject() {
        // C4ControlVoteEnd first requires HostControl, then EndVote removes
        // every ballot with the exact (Type,Data) key while leaving other
        // simultaneous subjects active (src/C4Control.cpp:1456-1461;
        // src/C4Network2.cpp:2888-2911).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _events) = NetworkManager::test_stub_for_client_id(7);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.control_clients = ControlClientRegistry::default();
        app.control_clients.register(0, true, false);
        app.control_clients.register(7, true, false);
        let kick = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 7,
        };
        let cancel = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_CANCEL,
            approve: true,
            data: 0,
            by_client: 0,
        };
        app.execute_league_vote(kick).expect("store kick vote");
        app.execute_league_vote(cancel).expect("store cancel vote");

        app.apply_ready_controls(
            23,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                approve: false,
                by_client: 7,
                ..kick
            })],
        )
        .expect("ignore nonhost VoteEnd");
        assert_eq!(app.league_votes.ballots, vec![kick, cancel]);

        app.apply_ready_controls(
            24,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                approve: false,
                by_client: 0,
                ..kick
            })],
        )
        .expect("execute host VoteEnd");

        assert_eq!(app.league_votes.ballots, vec![cancel]);
    }

    #[test]
    fn approved_kick_vote_end_queues_host_client_removal() {
        // Approved VT_Kick flags the target and the control host queues
        // C4ClientList::CtrlRemove with the exact "voted out" reason
        // (src/C4Control.cpp:1482-1496; LanguageUS.txt:1399).
        let mut app = new_state_only_running_sandbox_app();
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        app.control_clients.register(7, true, false);
        app.control_player_infos.replace_snapshot(
            72,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                players: vec![
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 71,
                        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                        ..Default::default()
                    },
                    clonk_engine::ControlPlayerInfoEntry {
                        id: 72,
                        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED
                            | clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
        );

        app.apply_ready_controls(
            23,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 0,
            })],
        )
        .expect("execute approved kick");

        assert_eq!(
            commands.take_submitted_client_removes(),
            vec![clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: clonk_engine::LegacyCString::from_bytes(b"voted out".to_vec())
                    .expect("valid C++ vote reason"),
                by_client: 0,
            }]
        );
        assert_ne!(
            app.control_player_infos
                .get(71)
                .expect("active target player info")
                .flags
                & clonk_engine::PLAYER_INFO_FLAG_VOTED_OUT,
            0,
        );
        assert_eq!(
            app.control_player_infos
                .get(72)
                .expect("removed target player info")
                .flags
                & clonk_engine::PLAYER_INFO_FLAG_VOTED_OUT,
            0,
            "removed history rows remain unchanged",
        );

        let mut game_over = new_state_only_running_sandbox_app();
        assert!(
            game_over
                .engine
                .request_game_over_from_control()
                .expect("mark the round game over")
        );
        let (manager, _events, _commands) = NetworkManager::test_stub_with_commands();
        game_over.network = Some(manager);
        game_over.network_mode = Some(NetworkMode::Host(host_network_settings()));
        game_over.control_clients.register(7, true, false);
        game_over.control_player_infos.replace_snapshot(
            81,
            [clonk_engine::PlayerInfoControlData {
                client_id: 7,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 81,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        game_over.apply_ready_controls(
            24,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: 7,
                by_client: 0,
            })],
        )
        .expect("execute a post-game-over approved kick");
        assert_eq!(
            game_over
                .control_player_infos
                .get(81)
                .expect("completed-round target row")
                .flags
                & clonk_engine::PLAYER_INFO_FLAG_VOTED_OUT,
            0,
            "C4ControlVoteEnd suppresses the history mutation after GameOver",
        );
    }

    #[test]
    fn approved_self_kick_clears_network_and_ends_local_round() {
        // When approved VT_Kick targets the local client, C++ records the
        // voted-out result, clears the network into local control, and calls
        // DoGameOver immediately so the removed client cannot continue alone
        // (src/C4Control.cpp:1497-1506; src/C4Network2.cpp:746-789).
        let mut app = new_classic_running_sandbox_app();
        let local_client = 7;
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        let (manager, _events) = NetworkManager::test_stub_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.control_clients = ControlClientRegistry::default();
        app.control_clients.register(0, true, false);
        app.control_clients.register(local_client, true, false);

        app.apply_ready_controls(
            23,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: true,
                data: local_client,
                by_client: 0,
            })],
        )
        .expect("execute approved self-kick");

        assert!(app.network.is_none());
        assert!(app.network_mode.is_none());
        assert!(app
            .engine
            .player(app.local_owner)
            .expect("local player remains for evaluation")
            .surrendered());
        app.update().expect("finish voted-out round");
        assert!(app.snapshot.game_over);
    }

    #[test]
    fn eligible_client_vote_prompt_defaults_no_and_yes_submits_ballot() {
        // A connected client with a currently joined local player gets one
        // exclusive Yes/No C4VoteDialog for a subject it has not voted on.
        // The dialog uses Ico_Confirm and fDefaultNo=true; closing Yes calls
        // Vote with the local authenticated client ID
        // (src/C4Network2.cpp:2941-2972,2992-3033).
        let mut app = new_running_sandbox_app();
        let local_client = 7;
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        let (manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.control_clients.replace_snapshot([
            clonk_engine::ClientCoreControlData {
                client_id: 0,
                name: clonk_engine::LegacyCString::from_bytes(b"Host".to_vec()).expect("host name"),
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: local_client,
                name: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec())
                    .expect("client name"),
                ..Default::default()
            },
        ]);
        let subject = LeagueVoteSubject {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            data: local_client,
        };
        event_tx
            .send(NetworkEvent::DirectControl(NetworkControl::Vote(
                clonk_engine::VoteControlData {
                    vote_type: subject.vote_type,
                    approve: true,
                    data: subject.data,
                    by_client: 0,
                },
            )))
            .expect("queue host vote");

        app.process_network_events().expect("open vote prompt");

        assert_eq!(app.message_dialogs.len(), 1);
        let prompt = &app.message_dialogs[0].state;
        assert_eq!(prompt.caption(), "Voting");
        assert_eq!(
            prompt.message(),
            "Host wants to kick client Client. Allow?|Notice: if a player leaves without being defeated, the opposing players will gain less league score in case of a win."
        );
        assert_eq!(
            prompt.buttons(),
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
        );
        assert_eq!(
            prompt.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM
        );
        assert_eq!(
            prompt.focused_button(),
            Some(clonk_frontend::message_dialog::MessageDialogButton::No)
        );

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
            .expect("approve vote");

        assert_eq!(
            commands.take_submitted_votes(),
            vec![clonk_engine::VoteControlData {
                vote_type: subject.vote_type,
                approve: true,
                data: subject.data,
                by_client: local_client,
            }]
        );
    }

    #[test]
    fn rejected_own_self_kick_opens_default_no_surrender_prompt() {
        // If an own-origin self-kick is rejected, EndVote offers the separate
        // league surrender dialog. It is Yes/No with default No; declining
        // leaves the network round running (src/C4Network2.cpp:2900-2928,
        // 2974-3033).
        let mut app = new_running_sandbox_app();
        let local_client = 7;
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        let (manager, _events) = NetworkManager::test_stub_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.control_clients.register(0, true, false);
        app.control_clients.register(local_client, true, false);
        app.execute_league_vote(clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: local_client,
            by_client: local_client,
        })
        .expect("store own self-kick");

        app.apply_ready_controls(
            23,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: false,
                data: local_client,
                by_client: 0,
            })],
        )
        .expect("reject own self-kick");

        assert_eq!(app.message_dialogs.len(), 1);
        let prompt = &app.message_dialogs[0].state;
        assert_eq!(prompt.caption(), "Voting");
        assert_eq!(
            prompt.message(),
            "It was decided that you cannot leave the game. However, you can forfeit the game instead.||Do you want to surrender?"
        );
        assert_eq!(
            prompt.focused_button(),
            Some(clonk_frontend::message_dialog::MessageDialogButton::No)
        );

        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
            .expect("decline surrender");

        assert!(app.message_dialogs.is_empty());
        assert!(app.network.is_some());
        assert!(matches!(app.mode, AppMode::Running));
    }

    #[test]
    fn accepting_league_surrender_clears_network_and_aborts_round() {
        // Accepting the fallback surrender records a league forfeit, clears
        // C4Network2 without a normal Part notification, then Game.Abort(true)
        // exits the round (src/C4Network2.cpp:2974-3033,2823-2828).
        let mut app = new_running_sandbox_app();
        let local_client = 7;
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        let (manager, _events, commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.network_is_league = true;
        app.control_clients.register(0, true, false);
        app.control_clients.register(local_client, true, false);
        let local_info = 55;
        app.control_player_infos.replace_snapshot(
            local_info,
            [clonk_engine::PlayerInfoControlData {
                client_id: local_client,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: local_info,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        app.execute_league_vote(clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: local_client,
            by_client: local_client,
        })
        .expect("store own self-kick");
        app.apply_ready_controls(
            23,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_KICK,
                approve: false,
                data: local_client,
                by_client: 0,
            })],
        )
        .expect("reject own self-kick");

        let no_report = thread::spawn(move || commands.complete_league_disconnect_report());
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
            .expect("accept league surrender");

        let (engine_results, presentation_results, network_was_live) = app
            .league_surrender_pre_abort_results
            .take()
            .expect("the accepted confirmation records its pre-abort state");
        assert!(network_was_live, "the result precedes Network.Clear");
        assert_eq!(
            engine_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
        );
        assert_eq!(
            engine_results.network_result_message.as_slice(),
            b"You have surrendered the league game."
        );
        assert_eq!(
            presentation_results, engine_results,
            "the presentation snapshot exposes the verdict before teardown"
        );
        assert!(app.network.is_none());
        assert!(app.network_mode.is_none());
        assert!(matches!(app.mode, AppMode::Menu));
        assert_eq!(
            no_report.join().expect("league report listener exits"),
            None,
            "the surrendering client leaves reporting to the other clients"
        );
    }

    #[test]
    fn approved_cancel_vote_end_aborts_network_round() {
        // Approved VT_Cancel marks the round's players voted out and calls
        // Game.Abort(true), leaving the active network round
        // (src/C4Control.cpp:1472-1481).
        let mut app = new_state_only_running_sandbox_app();
        let local_client = 7;
        app.engine
            .player_mut(app.local_owner)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(local_client));
        let (manager, _events) = NetworkManager::test_stub_for_client_id(local_client as u32);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));

        app.apply_ready_controls(
            23,
            vec![NetworkControl::VoteEnd(clonk_engine::VoteControlData {
                vote_type: clonk_engine::VOTE_TYPE_CANCEL,
                approve: true,
                data: 0,
                by_client: 0,
            })],
        )
        .expect("execute approved cancel");

        assert!(app.network.is_none());
        assert!(app.network_mode.is_none());
        assert!(matches!(app.mode, AppMode::Menu));
    }

    #[test]
    fn host_vote_pause_lifecycle_matches_pause_vote_result() {
        // The running host pauses for any active vote. Approved VT_Pause(1)
        // leaves that pause in place; an approved VT_Pause(0) begun while
        // already paused restores GS_Go once no ballots remain
        // (src/C4Network2.cpp:2861-2883,2929-2938;
        // src/C4Game.cpp:1024-1054).
        let mut pause_app = new_state_only_running_sandbox_app();
        pause_app
            .engine
            .player_mut(pause_app.local_owner)
            .expect("host player")
            .set_at_client(clonk_engine::PlayerAtClient::HOST);
        let (pause_snapshot, pause_reference) = default_exact_host_reference();
        pause_app.control_clients = ControlClientRegistry::default();
        pause_app
            .control_clients
            .replace_snapshot(pause_snapshot.parameters.clients.clients.clone());
        pause_app.host_join_snapshot = Some(pause_snapshot);
        pause_app.advertised_game_reference = Some(pause_reference);
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        pause_app.network = Some(manager);
        pause_app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        let pause = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_PAUSE,
            approve: true,
            data: 1,
            by_client: 0,
        };

        pause_app
            .execute_league_vote(pause)
            .expect("start pause vote");

        let pause_changes = commands.take_status_changes();
        assert_eq!(pause_changes.len(), 1);
        assert_eq!(pause_changes[0].state, clonk_network::NETWORK_STATE_PAUSE);
        assert_eq!(
            pause_app
                .advertised_game_reference
                .as_ref()
                .expect("pause refreshes the retained reference")
                .summary()
                .state,
            "Paused"
        );
        pause_app.execute_league_vote_end(pause);
        assert!(commands.take_status_changes().is_empty());
        assert_eq!(
            pause_app
                .advertised_game_reference
                .as_ref()
                .expect("approved pause remains advertised")
                .summary()
                .state,
            "Paused"
        );

        let mut unpause_app = new_state_only_running_sandbox_app();
        unpause_app
            .engine
            .player_mut(unpause_app.local_owner)
            .expect("host player")
            .set_at_client(clonk_engine::PlayerAtClient::HOST);
        let (unpause_snapshot, unpause_reference) = default_exact_host_reference();
        unpause_app.control_clients = ControlClientRegistry::default();
        unpause_app
            .control_clients
            .replace_snapshot(unpause_snapshot.parameters.clients.clients.clone());
        unpause_app.host_join_snapshot = Some(unpause_snapshot);
        unpause_app.advertised_game_reference = Some(unpause_reference);
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        unpause_app.network = Some(manager);
        unpause_app.network_mode = Some(NetworkMode::Host(HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        }));
        unpause_app.network_control_running = false;
        unpause_app.league_votes.paused_for_vote = true;
        unpause_app.host_reference_paused = true;
        unpause_app.publish_running_host_reference();
        let unpause = clonk_engine::VoteControlData { data: 0, ..pause };

        unpause_app
            .execute_league_vote(unpause)
            .expect("start unpause vote");
        assert!(commands.take_status_changes().is_empty());
        unpause_app.execute_league_vote_end(unpause);

        let go_changes = commands.take_status_changes();
        assert_eq!(go_changes.len(), 1);
        assert_eq!(go_changes[0].state, clonk_network::NETWORK_STATE_GO);
        assert_eq!(
            unpause_app
                .advertised_game_reference
                .as_ref()
                .expect("GO request refreshes the retained reference")
                .summary()
                .state,
            "Running"
        );
    }

    #[test]
    fn league_observer_part_uses_ordinary_network_clear_path() {
        // League changes Part into a self-kick vote only when
        // Game.Players.GetLocalByIndex(0) exists. An observer with no local
        // player follows the ordinary result+Network.Clear path
        // (src/C4MainMenu.cpp:820-831).
        let mut app = new_running_sandbox_app();
        let local_client = 7;
        app.engine
            .remove_player(app.local_owner)
            .expect("remove observer's synthetic local player");
        let (manager, _events, commands) =
            NetworkManager::test_stub_with_commands_for_client_id(local_client);
        app.network = Some(manager);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Observer",
        )));
        app.network_is_league = true;
        app.control_clients = ControlClientRegistry::default();
        app.control_clients
            .register(local_client as i32, false, true);
        let graceful_write = thread::spawn(move || commands.complete_graceful_part());

        app.apply_ingame_menu_action(MenuAction::Part)
            .expect("observer parts from league game");

        assert!(graceful_write.join().expect("graceful writer exits"));
        assert!(app.network.is_none());
        assert!(app.network_mode.is_none());
        assert!(matches!(app.mode, AppMode::Running));
        assert_eq!(
            app.snapshot.round_results.network_result,
            Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
        );
        assert_eq!(
            app.snapshot.round_results.network_result_message.as_slice(),
            b"Game left via player menu.",
            "a league observer uses the ordinary localized Part verdict"
        );
    }

    #[test]
    fn synchronized_surrender_executes_only_for_the_runtime_player_owner() {
        // C4Control executes CID_SurrenderPlayer through
        // C4ControlInternalPlayerScriptBase::Allowed, which requires the
        // runtime C4Player::AtClient to equal iByClient
        // (src/C4Control.cpp:93-109,1546-1578).
        let mut app = new_state_only_running_sandbox_app();
        let player = app.local_owner;
        app.engine
            .player_mut(player)
            .expect("local player")
            .set_at_client(clonk_engine::PlayerAtClient::new(3));
        assert_eq!(
            app.engine.player(player).expect("local player").at_client(),
            clonk_engine::PlayerAtClient::new(3)
        );

        app.apply_ready_controls(
            0,
            vec![NetworkControl::SurrenderPlayer(
                clonk_engine::SurrenderPlayerControlData {
                    player,
                    by_client: 7,
                },
            )],
        )
        .expect("execute spoofed surrender control");
        assert!(!app
            .engine
            .player(player)
            .expect("local player")
            .surrendered());

        app.apply_ready_controls(
            1,
            vec![NetworkControl::SurrenderPlayer(
                clonk_engine::SurrenderPlayerControlData {
                    player,
                    by_client: 3,
                },
            )],
        )
        .expect("execute owner surrender control");
        assert!(app
            .engine
            .player(player)
            .expect("local player")
            .surrendered());
    }
