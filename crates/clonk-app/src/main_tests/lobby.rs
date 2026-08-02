// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

#[test]
fn startup_player_count_uses_roster_only_at_frame_zero() {
    assert_eq!(
        startup_player_count_for_init(0, Some(7), Some(2)),
        Some(2),
        "fresh games overwrite a stale serialized parameter"
    );
    assert_eq!(
        startup_player_count_for_init(37, Some(7), Some(1)),
        Some(7),
        "runtime restores retain their original startup scalar"
    );
    assert_eq!(
        startup_player_count_for_init(-1, Some(0), Some(3)),
        Some(0),
        "the native branch tests exact zero rather than positive frames"
    );
    assert_eq!(startup_player_count_for_init(0, Some(4), None), None);
}

#[test]
fn classic_command_line_lobby_timeout_starts_the_host_countdown() {
    assert_eq!(
        parse_classic_command_line(&[OsString::from("/network")]).lobby_timeout,
        None,
    );
    assert_eq!(
        parse_classic_command_line(&[OsString::from("/lobby")]).lobby_timeout,
        Some(None),
    );
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.apply_classic_command_line(&ClassicCommandLine {
        scenario: Some(PathBuf::from("Fixture.c4s")),
        network_active: Some(true),
        lobby_timeout: Some(Some(120)),
        ..ClassicCommandLine::default()
    })
    .expect("apply classic lobby timeout");

    app.finish_classic_command_line_host_entry()
        .expect("finish classic lobby entry");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(120)]
    );
    assert_eq!(
        app.host_lobby_countdown,
        Some(HostLobbyCountdown::with_seconds(120))
    );
}

#[test]
fn classic_command_line_network_scenario_skips_unrequested_lobby() {
    let mut app = new_state_only_menu_app(320, 200);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (_sender, receiver) = mpsc::channel();
    app.lobby_preload_task = Some(LobbyPreloadTask {
        state: LobbyPreloadTaskState::Loading(receiver),
        start_host_when_ready: false,
        worker: LobbyPreloadWorker::new(thread::spawn(|| {})),
    });
    app.classic_command_line = ClassicCommandLine {
        scenario: Some(PathBuf::from("Fixture.c4s")),
        network_active: Some(true),
        lobby_timeout: None,
        ..ClassicCommandLine::default()
    };

    app.finish_classic_command_line_host_entry()
        .expect("request immediate host start");

    assert!(
        app.lobby_preload_task
            .as_ref()
            .expect("preload remains pending")
            .start_host_when_ready
    );
    assert_eq!(app.mode, AppMode::Loading);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
}

#[test]
fn already_failed_client_start_resource_never_opens_a_stale_progress_wait() {
    let mut app = new_menu_app(800, 600);
    let (manager, _event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    let core = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Definitions as u8,
        id: 12,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"Network\\Objects.c4d".to_vec())
            .unwrap(),
        ..Default::default()
    };
    app.admission_resources.register_lobby_resource(&core);
    app.admission_resources.mark_failed(core.id);

    app.wait_for_client_start_resource(PendingClientStartResource {
        role: ClientStartResourceRole::GameResource { index: 0 },
        core,
    })
    .expect("an already failed resource returns to startup");

    assert!(app.network.is_none());
    assert!(app.blocking_resource_wait.is_none());
    assert_eq!(app.message_dialogs.len(), 1);
    assert_eq!(app.message_dialogs[0].state.caption(), "Error Log");
    assert_eq!(
        app.message_dialogs[0].state.message(),
        "Unable to retrieve Object Definition: Objects.c4d."
    );
}

#[test]
fn network_control_catch_up_drains_ten_ready_ticks_in_one_pass() {
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(11, 1);
    let frame_before = app.engine.frame();

    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
        .expect("catch up ready control backlog");

    assert_eq!(outcome.executed_frames, 8);
    assert_eq!(app.engine.frame(), frame_before + 8);
    assert_eq!(app.network_control_pacing().behind, 3);
    assert!(!app.network_control_pacing().overflow);
    assert_eq!(accumulator, Duration::ZERO);
}

#[test]
fn automatic_frame_skip_is_consumed_by_an_already_suppressed_pass() {
    let mut frame_skip = AutomaticFrameSkip::default();
    frame_skip.finish_graphics_pass(true, Duration::from_millis(29), Duration::from_millis(28));

    frame_skip.consume_suppressed_graphics_pass();
    assert!(!frame_skip.begin_graphics_pass(true));
}

#[test]
fn network_control_catch_up_stops_at_a_ready_tick_gap() {
    let mut gate = NetworkTickGate::default();
    gate.queue(7, 7, Vec::new());
    gate.queue(7, 11, Vec::new());

    assert_eq!(gate.contiguous_ready_behind(7), 1);

    gate.queue(7, 8, Vec::new());
    gate.queue(7, 9, Vec::new());
    let mut inspected = 0;
    assert_eq!(
        gate.contiguous_ready_behind_if(7, |_| {
            inspected += 1;
            inspected < 2
        }),
        1,
        "a future control that fails PreExecute ends the ready prefix"
    );
}

#[test]
fn scenario_join_places_ready_crew_and_selects_cursor() {
    // C4Player::Join runs ScenarioInit -> PlaceReadyCrew for the
    // scenario's [PlayerN] Crew= spec (C4Player.cpp:670-777, 528-570)
    // and FinalInit's AdjustCursorCommand puts the cursor on the
    // hi-rank crew member (C4Player.cpp:794, 1235-1258). The app join
    // must go through that path instead of a bare player registration.
    let mut app = GameApp::new(
        320,
        200,
        AudioOptions::default(),
        None,
        RuntimeConfig {
            player_owner: 1,
            player_name: "Twonky".to_string(),
            network: None,
            record_enabled: false,
        },
    )
    .expect("initialise app");

    let mut definition =
        Definition::from_script("WLKR", "Walker", walker_script()).expect("crew definition");
    definition.set_crew_member(true);
    app.engine
        .register_definition(definition)
        .expect("register crew definition");
    let start = clonk_engine::scenario::PlayerStart {
        ready_crew: vec![("WLKR".to_string(), 2)],
        ..Default::default()
    };
    app.engine.set_player_starts(vec![start]);

    app.join_local_player().expect("join local player");

    // C4PlayerList::GetFreeNumber (C4PlayerList.cpp:189-201): the
    // first joining player takes number 0.
    assert_eq!(app.local_owner, 0, "local owner adopts the joined number");
    let snapshot = app.engine.snapshot();
    let crew: Vec<_> = snapshot
        .objects
        .iter()
        .filter(|object| object.crew_member && object.owner == app.local_owner)
        .collect();
    assert_eq!(crew.len(), 2, "Crew=WLKR=2 places two ready crew members");
    let selection = snapshot
        .crew_selection
        .get(&app.local_owner)
        .expect("crew selection exists after join");
    assert!(
        selection.cursor.is_some(),
        "cursor lands on a crew member at join"
    );

    // A second call must not join (or place crew) twice.
    app.join_local_player().expect("idempotent rejoin");
    assert_eq!(
        app.engine.players().count(),
        1,
        "rejoining does not add a second player"
    );
}

#[test]
fn local_join_publishes_control_before_initialize_player_audio() {
    // InitControl establishes C4Player::LocalControl before ScenarioInit calls
    // InitializePlayer (C4Player.cpp:323-349,769-775). Player-targeted Sound
    // in that callback must therefore pass this process's local gate
    // (C4Script.cpp:2297-2309).
    let mut app = GameApp::new(
        320,
        200,
        AudioOptions::default(),
        None,
        RuntimeConfig {
            player_owner: 1,
            player_name: "Callback listener".to_string(),
            network: None,
            record_enabled: false,
        },
    )
    .expect("initialise app");
    app.engine
        .load_scenario_script_with_convention(
            "local join audio fixture",
            "#strict 3\nglobal func InitializePlayer(int plr) { Sound(\"LocalJoin\", true, nil, 100, plr + 1); }",
            true,
        )
        .expect("local join audio fixture links");
    app.engine.set_local_players([]);

    app.join_local_player().expect("join local player");

    assert!(app.engine.pending_audio.iter().any(|command| matches!(
        command,
        clonk_engine::AudioCommand::PlaySound { name, .. } if name == "LocalJoin"
    )));
}

#[test]
fn network_too_few_warning_ok_stages_and_enters_exact_lobby() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir().expect("isolated warning-accept user data");
    let scenario_group = tempdir().expect("warning-accept scenario");
    fs::write(
                scenario_group.path().join("Scenario.txt"),
                "[Head]\nTitle=Needs players\nMinPlayer=2\nMaxPlayer=4\n\n[Definitions]\nAllowUserChange=true\n",
            )
            .expect("write warning-accept core");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "Participants", "")
        .expect("clear startup participants");
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "NeedsPlayers.c4s".to_string();
    scenario.title = "Needs players".to_string();
    scenario.path = Some(scenario_group.path().to_path_buf());
    scenario.allow_user_change = Some(true);
    let scenarios = vec![scenario.clone()];
    let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
        .expect("network warning-accept menu");
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scenario_catalog = build_scenario_catalog(&scenarios);
    app.open_network_host_scenario_browser();
    app.menu_state.definition_checkbox_checked = false;

    app.handle_menu_input(|_| {
        vec![StartupMenuAction::StartScenario(
            clonk_frontend::ScenarioSummary {
                identifier: scenario.identifier.clone(),
                title: scenario.title.clone(),
                kind: ScenarioKind::Scenario,
            },
        )]
    })
    .expect("too-few warning opens before staging");
    assert_eq!(app.message_dialogs.len(), 1);
    assert_eq!(
        app.message_dialogs[0].state.buttons(),
        clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL
    );

    // Keep activate_prepared_network_host from spawning or binding while
    // the OK continuation still executes the complete synchronous staging
    // path. The blocker is replaced with the existing socketless manager
    // stub immediately afterwards.
    let (blocker_sender, blocker_receiver) = mpsc::channel();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        blocker_receiver,
        None,
        StartupNetworkPurpose::StagedHost,
    ));
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .expect("warning OK stages the selected network scenario");

    assert!(app.message_dialogs.is_empty());
    assert!(app.definition_selector.is_none());
    let staged = app
        .staged_network_host_scenario
        .as_ref()
        .expect("warning OK reaches exact host staging");
    assert_eq!(staged.frontend.identifier, scenario.identifier);
    assert_eq!(staged.frontend.title, scenario.title);

    let blocker = app
        .startup_network_connection
        .take()
        .expect("blocking startup connection remains installed");
    drop(blocker);
    drop(blocker_sender);

    let (manager, _events) = NetworkManager::test_stub();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                player_name: "Exact Host".to_string(),
                prepared: None,
            }),
            manager,
        )))
        .expect("queue socketless exact-host completion");
    app.begin_startup_network_connection(
        receiver,
        StartupNetworkPurpose::StagedHost,
        Some((scenario.identifier.clone(), scenario.title.clone())),
        None,
    )
    .expect("begin exact-host transition");
    app.poll_startup_network_connection()
        .expect("poll exact-host transition");

    assert_eq!(app.mode, AppMode::Menu);
    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    assert!(app.network.is_some());
    assert!(app.network_lobby.is_none());
    assert!(app.status_text.is_empty());
    let lobby = &app
        .classic_host_lobby
        .as_ref()
        .expect("warning OK reaches the exact classic lobby")
        .controller;
    assert_eq!(lobby.role(), LobbyRole::Host);
    assert_eq!(lobby.title(), "Needs players - Lobby");
    assert!(matches!(
        lobby.rows(),
        [LobbyRosterRow::Client(LobbyClientRow {
            id: 0,
            name,
            status: LobbyClientStatus::Host,
            local: true,
            ..
        })] if name == "Exact Host"
    ));
    reset_cached_app_paths();
}

/// The client's `C4GameLobby::MainDlg` is as long-lived as the host's, and
/// tooltip timing belongs to the one `CMouse` on the screen, not to the
/// dialog: motion and clicks call `ResetToolTipTime`, and the tip appears
/// only after `C4GUI_ToolTipShowTime` of stillness
/// (src/C4Gui.cpp:502-536; src/C4Gui.h:148,1712-1713).
#[test]
fn joined_lobby_tooltips_survive_frames_and_use_shared_delay() {
    let mut app = new_menu_app(640, 480);
    app.network_lobby = Some(
        NetworkLobbyState::new(7, "Client".to_string(), false)
            .with_preloading(false, LobbyLabels::default()),
    );
    app.startup_view = StartupView::NetworkLobby;
    app.sync_network_lobby_game_option_state();
    let assets = Arc::clone(&app.assets);
    let layout = app
        .network_lobby
        .as_mut()
        .expect("joined lobby")
        .with_classic_controller_input(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
            |_, layout, _| layout.clone(),
        )
        .expect("layout retained joined lobby");
    let mut point = GuiPoint::new(
        (layout.chat_edit.x + 2) as f32,
        (layout.chat_edit.y + 2) as f32,
    );
    let tooltip_at = |app: &GameApp, now: Instant| {
        app.network_lobby
            .as_ref()
            .and_then(|lobby| lobby.controller.tooltip_state_at(now))
            .is_some()
    };
    let tooltip_text = |app: &GameApp| {
        app.network_lobby.as_ref().and_then(|lobby| {
            lobby
                .controller
                .tooltip_state_at(Instant::now() + Duration::from_secs(1))
                .map(|tooltip| tooltip.text)
        })
    };

    app.handle_network_lobby_pointer_move(point)
        .expect("prime joined hover");
    // Below C4GUI_ToolTipShowTime the retained clock keeps the tip hidden.
    assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));
    assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));

    // The tracker survives the per-frame projection rather than being
    // rebuilt: a reconstructed controller would restart the clock at draw
    // time and lose the hover owner.
    let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .render_classic(
            &mut surface,
            assets.as_ref(),
            &app.scenario_game_options,
            false,
            true,
            &startup_identity_gamma().clone(),
        )
        .expect("project one joined lobby frame");
    assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));
    assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));

    // Non-pointer input suppresses until real motion, exactly as on the host.
    app.handle_key(VirtualKeyCode::KeyA, ElementState::Pressed)
        .expect("route physical key");
    assert!(!tooltip_at(&app, Instant::now() + Duration::from_secs(1)));
    app.handle_network_lobby_pointer_move(point)
        .expect("route synthesized same-pixel motion");
    assert!(
        !tooltip_at(&app, Instant::now() + Duration::from_secs(1)),
        "same-pixel motion must not reactivate the tooltip"
    );
    point.x += 1.0;
    app.handle_network_lobby_pointer_move(point)
        .expect("reactivate after physical key");
    assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));

    // A new hover target takes ownership and restarts the clock, so the
    // retained tracker reports the newly owned element's text.
    let chat_tip = tooltip_text(&app).expect("chat tooltip");
    let exit = GuiPoint::new(
        (layout.exit_button.x + layout.exit_button.w / 2) as f32,
        (layout.exit_button.y + layout.exit_button.h / 2) as f32,
    );
    app.handle_network_lobby_pointer_move(exit)
        .expect("hover the Exit button");
    assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));
    let exit_tip = tooltip_text(&app).expect("exit tooltip");
    assert_ne!(chat_tip, exit_tip);
    app.handle_network_lobby_pointer_move(point)
        .expect("return to the chat edit");

    // A wheel is mouse input, so `ResetToolTipTime` restarts the stillness
    // clock rather than disabling the tip outright.
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
        .expect("route wheel");
    assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));
    assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));

    // A covering modal owns the pointer, so the lobby beneath draws no tip.
    point.x += 1.0;
    app.handle_network_lobby_pointer_move(point)
        .expect("re-arm before the modal");
    assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));
    assert!(app
        .render_startup_tooltips()
        .expect("uncovered lobby tooltip pass"));
    app.open_network_join_password_dialog()
        .expect("cover the lobby with a modal");
    assert!(!app
        .render_startup_tooltips()
        .expect("covered lobby tooltip pass"));
}

#[test]
fn persistent_classic_lobby_non_pointer_input_suppresses_tooltip_until_motion() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (layout, _) = app
        .classic_host_lobby_layouts()
        .expect("classic lobby layout");
    let mut point = GuiPoint::new(
        (layout.chat_edit.x + 2) as f32,
        (layout.chat_edit.y + 2) as f32,
    );
    let tooltip_visible = |app: &GameApp| {
        app.classic_host_lobby
            .as_ref()
            .and_then(|lobby| {
                lobby
                    .controller
                    .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            })
            .is_some()
    };

    app.handle_classic_lobby_pointer_move(point)
        .expect("prime lobby hover");
    assert!(tooltip_visible(&app));

    app.handle_key(VirtualKeyCode::KeyA, ElementState::Pressed)
        .expect("route physical key");
    assert!(!tooltip_visible(&app));
    app.handle_classic_lobby_pointer_move(point)
        .expect("route synthesized same-pixel motion");
    assert!(
        !tooltip_visible(&app),
        "same-pixel motion must not reactivate the tooltip"
    );

    point.x += 1.0;
    app.handle_classic_lobby_pointer_move(point)
        .expect("reactivate after physical key");
    assert!(tooltip_visible(&app));
    app.handle_text_input('x').expect("route text input");
    assert!(!tooltip_visible(&app));

    point.x += 1.0;
    app.handle_classic_lobby_pointer_move(point)
        .expect("reactivate after text input");
    assert!(tooltip_visible(&app));
    app.handle_gamepad_event(GamepadEvent::Direction {
        slot: GamepadSlot::new(0),
        button: ControlButton::Right,
        state: ElementState::Pressed,
    })
    .expect("route gamepad input");
    assert!(!tooltip_visible(&app));

    point.x += 1.0;
    app.handle_classic_lobby_pointer_move(point)
        .expect("reactivate after gamepad input");
    assert!(tooltip_visible(&app));

    let option_rect = app
        .scenario_game_options
        .layout()
        .rect(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
        .expect("host fair-crew option");
    let option_point = GuiPoint::new(
        (option_rect.x + option_rect.w / 2) as f32,
        (option_rect.y + option_rect.h / 2) as f32,
    );
    app.handle_classic_lobby_pointer_move(option_point)
        .expect("prime embedded option hover");
    assert!(app
        .scenario_game_options
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    app.handle_key(VirtualKeyCode::KeyA, ElementState::Pressed)
        .expect("suppress embedded option tooltip");
    assert!(app
        .scenario_game_options
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_none());
    app.handle_classic_lobby_pointer_move(option_point)
        .expect("route same-pixel option motion");
    assert!(
        app.scenario_game_options
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_none(),
        "the embedded option strip shares CMouse same-pixel suppression"
    );
    app.handle_classic_lobby_pointer_move(GuiPoint::new(option_point.x + 1.0, option_point.y))
        .expect("reactivate embedded option hover");
    assert!(app
        .scenario_game_options
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    app.handle_classic_lobby_secondary_button(ElementState::Released)
        .expect("route right-button release clock edge");
    assert!(
        app.scenario_game_options
            .tooltip_state_at(Instant::now() + Duration::from_millis(400))
            .is_none(),
        "right-button release resets the embedded option tooltip clock"
    );
    app.handle_classic_lobby_middle_button(ElementState::Released)
        .expect("route middle-button release clock edge");
    assert!(
        app.scenario_game_options
            .tooltip_state_at(Instant::now() + Duration::from_millis(400))
            .is_none(),
        "middle-button release resets the embedded option tooltip clock"
    );
}

#[test]
fn captured_classic_lobby_wheel_releases_tooltip_hover_ownership() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let seed = app
        .classic_host_lobby
        .as_ref()
        .expect("test lobby")
        .controller
        .rows()[0]
        .clone();
    let rows = (0..32)
        .map(|id| match seed.clone() {
            LobbyRosterRow::Client(mut client) => {
                client.id = id;
                client.name = format!("Client {id}");
                client.nick = client.name.clone();
                LobbyRosterRow::Client(client)
            }
            _ => unreachable!("test lobby starts with a client row"),
        })
        .collect();
    app.classic_host_lobby
        .as_mut()
        .expect("test lobby")
        .controller
        .set_rows(rows);

    let (layout, roster) = app
        .classic_host_lobby_layouts()
        .expect("scrollable lobby layout");
    assert!(roster.max_scroll > 0);
    let first_row = roster.rows.first().expect("visible roster row");
    let point = GuiPoint::new((first_row.rect.x + 2) as f32, (first_row.rect.y + 2) as f32);
    app.handle_classic_lobby_pointer_move(point)
        .expect("prime roster hover");
    assert!(app
        .classic_host_lobby
        .as_ref()
        .expect("test lobby")
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());

    app.handle_classic_lobby_wheel(-60).expect("scroll roster");
    assert!(
        app.classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_none(),
        "a captured ScrollWindow wheel must release pMouseOver tooltip ownership"
    );

    let exit_point = GuiPoint::new(
        (layout.exit_button.x + 2) as f32,
        (layout.exit_button.y + 2) as f32,
    );
    app.handle_classic_lobby_pointer_move(exit_point)
        .expect("move onto non-scroll control");
    app.handle_key(VirtualKeyCode::KeyA, ElementState::Pressed)
        .expect("suppress exit tooltip");
    app.handle_classic_lobby_wheel(60)
        .expect("route unconsumed wheel");
    assert!(
        app.classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_some(),
        "an unconsumed wheel re-establishes hover with a fresh delay"
    );

    let mut short_lobby = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut short_lobby);
    let (_, short_roster) = short_lobby
        .classic_host_lobby_layouts()
        .expect("unscrollable lobby layout");
    assert_eq!(short_roster.max_scroll, 0);
    let row = short_roster.rows.first().expect("single roster row");
    let row_point = GuiPoint::new((row.rect.x + 2) as f32, (row.rect.y + 2) as f32);
    short_lobby
        .handle_classic_lobby_pointer_move(row_point)
        .expect("prime unscrollable roster hover");
    assert!(short_lobby
        .classic_host_lobby
        .as_ref()
        .expect("test lobby")
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    short_lobby
        .handle_classic_lobby_wheel(60)
        .expect("route wheel captured by unscrollable roster");
    assert!(
        short_lobby
            .classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_none(),
        "ScrollWindow clears hover ownership even when its offset cannot change"
    );
}

fn test_lobby_team_rect(app: &mut GameApp) -> clonk_frontend::classic_gui::IntRect {
    let (_, roster) = app.classic_host_lobby_layouts().expect("team lobby layout");
    roster
        .rows
        .iter()
        .find(|row| row.index == 1)
        .and_then(|row| row.team)
        .expect("expanded team control")
}

#[test]
fn staged_host_completion_enters_exact_lobby_over_loader_background() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated host lobby user data");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "Network", "Nick", "Exact Nick")
        .expect("configure exact client nick");
    persist_config_value(&paths, "Lobby", "CountdownTime", "7").expect("configure exact countdown");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    // The production launcher applies C4Config.cpp:1671-1674 before the
    // app starts, enabling shader gamma for every migrated installation.
    persist_config_value(&paths, "Graphics", "Shader", "1")
        .expect("configure post-migration shader renderer");
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    assert!(
        app.graphics.fragment_gamma_enabled(),
        "the exact background comparison requires the post-migration shader-gamma path"
    );
    let accepted = GameOptionValues {
        master_server_signup: true,
        password: "round secret".to_string(),
        last_password: "round secret".to_string(),
        comment: "exact host".to_string(),
        fair_crew: true,
        record: true,
        ..GameOptionValues::default()
    };
    app.scenario_game_options =
        GameOptionButtons::new(GameOptionContext::NetworkHostSelector, accepted.clone());
    let staged = prepare_tutorial_host_lobby(&app, repository);
    assert!(
        staged.loader_screen.is_some(),
        "loader is selected before bind"
    );
    assert_eq!(staged.lobby.local_name, "Exact Host");
    assert_ne!(staged.lobby.local_name, app.player_name);
    let expected_title = staged
        .loader_screen
        .as_ref()
        .expect("staged loader")
        .state()
        .title()
        .to_string();
    assert_eq!(
        expected_title, staged.frontend.title,
        "the selected loader's pack-aware title is retained for JoinData"
    );
    app.staged_network_host_scenario = Some(staged);
    app.network_client_activity.mark_activated(99, 123);

    let (manager, _events) = NetworkManager::test_stub();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
                player_name: "Exact Host".to_string(),
                prepared: None,
            }),
            manager,
        )))
        .expect("queue exact host connection");
    app.begin_startup_network_connection(receiver, StartupNetworkPurpose::StagedHost, None, None)
        .expect("begin prepared host transition");
    assert_eq!(app.mode, AppMode::Loading);
    assert!(app.status_text.is_empty());
    assert_eq!(
        app.loader_screen
            .as_ref()
            .expect("live loader")
            .state()
            .title(),
        expected_title
    );
    app.poll_startup_network_connection()
        .expect("poll prepared host transition");

    assert_eq!(app.mode, AppMode::Menu);
    assert!(app.network_client_activity.last_frame.is_empty());
    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    assert!(
        app.network.is_some(),
        "host listener remains owned by lobby"
    );
    assert!(matches!(app.network_mode, Some(NetworkMode::Host(_))));
    assert!(app.network_lobby.is_none());
    assert!(app.classic_host_lobby.is_some());
    assert!(app.status_text.is_empty());
    assert_eq!(
        app.scenario_game_options.context(),
        GameOptionContext::LobbyHost
    );
    assert_eq!(
        app.scenario_game_options.values().password,
        accepted.password
    );
    assert_eq!(app.scenario_game_options.values().comment, accepted.comment);
    assert_eq!(
        app.scenario_game_options.values().fair_crew_strength,
        accepted.fair_crew_strength,
        "C++ fills zero scenario strength from accepted config when fair crew is active"
    );
    assert!(!app.scenario_game_options.values().countdown);

    let lobby = &app
        .classic_host_lobby
        .as_ref()
        .expect("exact lobby")
        .controller;
    assert_eq!(lobby.role(), LobbyRole::Host);
    assert_eq!(lobby.title(), format!("{expected_title} - Lobby"));
    assert_eq!(lobby.focus(), LobbyControl::ChatInput);
    assert!(!lobby.ready());
    assert_eq!(
        lobby.countdown(),
        clonk_frontend::game_lobby::LobbyCountdownState::None
    );
    assert!(matches!(
        lobby.rows().iter().find(|row| {
            matches!(row, LobbyRosterRow::Client(client) if client.id == 0)
        }),
        Some(LobbyRosterRow::Client(LobbyClientRow {
            id: 0,
            name,
            nick,
            status: LobbyClientStatus::Host,
            local: true,
            connected: false,
            resource_progress: None,
            ping_ms: None,
            ..
        })) if name == "Exact Host" && nick == "Exact Nick"
    ));
    let projected_player_ids = lobby
        .rows()
        .iter()
        .filter_map(|row| match row {
            LobbyRosterRow::Player(player) => Some(player.id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let authoritative_player_ids = app
        .control_player_infos
        .retained_rows_snapshot()
        .1
        .into_iter()
        .flat_map(|(_, _, players)| players)
        .filter(|player| {
            player.flags
                & (clonk_engine::PLAYER_INFO_FLAG_REMOVED
                    | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE)
                == 0
        })
        .map(|player| player.id)
        .collect::<Vec<_>>();
    assert_eq!(projected_player_ids, authoritative_player_ids);
    let fonts = app.assets.clonk_fonts.as_deref().expect("lobby fonts");
    let surface = app.graphics.surface();
    let layout = lobby.layout(surface.width() as i32, surface.height() as i32, fonts);
    assert_eq!(
        app.scenario_game_options.layout().bounds,
        layout.game_option_strip
    );

    let config = app.loader_render_config.expect("loader config");
    let mut background = Surface::new(800, 600, PixelFormat::Rgba8888);
    app.loader_screen
        .as_ref()
        .expect("scenario loader")
        .render_background(&mut background, config, app.loader_gamma.as_ref());
    let expected_corner = background.pixels()[..4].to_vec();
    let mut frame = vec![0_u8; 800 * 600 * 4];
    assert!(app.render(&mut frame).expect("render exact host lobby"));
    assert_eq!(&frame[..4], expected_corner.as_slice());
    assert!(
        app.menu_frame_cache.is_none(),
        "lobby frames are never cached"
    );
    assert!(app.render(&mut frame).expect("render live lobby again"));
    assert!(app.menu_frame_cache.is_none());

    app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("route lobby focus traversal");
    assert_ne!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby after tab")
            .controller
            .focus(),
        LobbyControl::ChatInput
    );
    app.handle_cursor_moved(PhysicalPosition::new(0.0, 0.0))
        .expect("route lobby pointer");
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0)
        .expect("route lobby wheel");
    app.handle_touch(TouchPhase::Started, GuiPoint::new(0.0, 0.0))
        .expect("route lobby touch down");
    app.handle_touch(TouchPhase::Ended, GuiPoint::new(0.0, 0.0))
        .expect("route lobby touch up");
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .expect("route lobby gamepad focus");

    app.show_main_menu();
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_control_running);
    assert!(app.control_clients.contains(0));
    assert!(app.control_clients.is_activated(0));
}

#[test]
fn staged_host_installs_activated_participant_before_building_lobby_roster() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated participant lobby user data");
    let content = tempdir().expect("minimal participant lobby content");
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    configure_test_startup_participant(&paths, user_data.path());
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::NetworkHostSelector,
        GameOptionValues::default(),
    );
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let prepared = build_network_host_preparation(
        &app,
        &staged.frontend,
        &staged.definition_load,
        &staged.effective_definition_modules,
        &staged.definition_resources,
        Some((&staged.definition_executable_path, &staged.definition_path)),
        Some((&staged.lobby.local_name, &staged.lobby.nick)),
    )
    .expect("build participant host preparation")
    .prepare()
    .expect("prepare participant host");
    let expected_alternate_colors = prepared.local_player_alternate_colors_by_resource().clone();
    assert_eq!(expected_alternate_colors.len(), 1);
    app.staged_network_host_scenario = Some(staged);

    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    let admission = thread::spawn(move || {
        let (allowed, completion) = commands.receive_join_allowed();
        assert!(allowed, "prepared participant opens lobby admission");
        completion
            .send(Ok(()))
            .expect("confirm prepared lobby admission");
    });
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
                player_name: "Exact Host".to_string(),
                prepared: Some(prepared),
            }),
            manager,
        )))
        .expect("queue prepared participant host connection");
    app.begin_startup_network_connection(receiver, StartupNetworkPurpose::StagedHost, None, None)
        .expect("begin prepared participant host transition");
    app.poll_startup_network_connection()
        .expect("poll prepared participant host transition");
    admission
        .join()
        .expect("join prepared lobby admission responder");

    assert!(app.status_text.is_empty(), "{}", app.status_text);
    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    assert!(app.network_lobby.is_none());
    assert_eq!(
        app.host_local_alternate_colors_by_resource, expected_alternate_colors,
        "the prepared host retains its process-local AlternateColorDw sidecar"
    );
    assert_eq!(app.host_local_player_info_ids.len(), 1);
    let lobby = &app
        .classic_host_lobby
        .as_ref()
        .expect("participant enters exact lobby")
        .controller;
    assert_eq!(lobby.players_title(), "&Players (1/1)");
    let [LobbyRosterRow::Client(client), LobbyRosterRow::Player(player)] = lobby.rows() else {
        panic!(
            "expected local client followed by activated player: {:?}",
            lobby.rows()
        );
    };
    assert_eq!(client.id, 0);
    assert_eq!(client.name, "Exact Host");
    assert_eq!(client.color, [0x3b, 0x3b, 0xff, 0xff]);
    assert_eq!(player.client_id, 0);
    assert_eq!(player.name, "Exact Player");
    assert_eq!(player.color, [0x3b, 0x3b, 0xff, 0xff]);
    assert!(matches!(
        &player.icon,
        LobbyRosterIcon::Raster(icon)
            if icon.width() == 1
                && icon.height() == 1
                && icon.pixels() == [12, 34, 56, 255]
    ));

    let (_, retained) = app.control_player_infos.retained_rows_snapshot();
    let retained_player = &retained[0].2[0];
    assert_eq!(retained_player.id, player.id);
    let resource = retained_player
        .resource
        .as_ref()
        .expect("activated player retains its resource core");
    assert!(
        app.admission_resources.complete_path(resource.id).is_some(),
        "activated player resource is installed before lobby admission"
    );
}

#[test]
fn l093_classic_lobby_identity_sanitizes_native_bytes() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated native identity user data");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let config_with_identity = |name: &[u8], nick: &[u8]| {
        let mut config = b"[Network]\nLocalName=\"".to_vec();
        config.extend_from_slice(name);
        config.extend_from_slice(b"\"\nNick=\"");
        config.extend_from_slice(nick);
        config.extend_from_slice(b"\"\n\n[Lobby]\nCountdownTime=7\n");
        config
    };
    let maximum_name = b"\xc3\xa4".repeat(15);
    fs::write(
        paths.config_file(),
        config_with_identity(&maximum_name, b"N\xc3\xa4ck"),
    )
    .expect("write 30-byte native identity");

    let (local_name, nick, countdown) =
        load_classic_lobby_identity_with_hostname_provider(&paths, || {
            panic!("explicit LocalName must not query the system hostname")
        })
        .expect("load native lobby identity");

    assert_eq!(
        clonk_resources::encode_legacy_script_text(&local_name),
        Some(maximum_name.clone()),
        "valid UTF-8-shaped bytes must not collapse during CP1252 encoding"
    );
    assert_eq!(
        clonk_resources::encode_legacy_script_text(&nick),
        Some(b"N\xc3\xa4ck".to_vec())
    );
    assert_eq!(countdown, 7);

    let overlong_name = b"\xc3\xa4".repeat(16);
    fs::write(
        paths.config_file(),
        config_with_identity(&overlong_name, b"Nick"),
    )
    .expect("write 32-byte native identity");
    let (local_name, _, _) =
        load_classic_lobby_identity(&paths).expect("C4MaxName validation truncates native bytes");
    assert_eq!(
        clonk_resources::encode_legacy_script_text(&local_name),
        Some(maximum_name),
        "C4MaxName truncation counts native bytes"
    );

    let mut removable_prefix = vec![b'{'; 1_025];
    removable_prefix.extend_from_slice(b"Alice");
    fs::write(
        paths.config_file(),
        config_with_identity(&removable_prefix, b"Nick"),
    )
    .expect("write dynamic name beyond CFG_MaxString");
    let (local_name, _, _) = load_classic_lobby_identity(&paths)
        .expect("dynamic name is sanitized before its final name cap");
    assert_eq!(local_name, "Alice");

    let dirty_name = b"  {<i>Guessed</i><c G> Host</c>}}<future>  ";
    let dirty_nick = b"  <i>Guessed Nick</i>}}  ";
    fs::write(
        paths.config_file(),
        config_with_identity(dirty_name, dirty_nick),
    )
    .expect("write noncanonical native identity");
    let (local_name, nick, _) = load_classic_lobby_identity(&paths)
        .expect("C++ name validation silently sanitizes identity");
    assert_eq!(local_name, "Guessed Host<future>");
    assert_eq!(nick, "Guessed Nick");

    fs::write(
        paths.config_file(),
        config_with_identity(b"Exact Host", b" {<i></i>}} "),
    )
    .expect("write Nick that sanitizes empty");
    let (local_name, nick, _) =
        load_classic_lobby_identity(&paths).expect("empty Nick falls back to LocalName");
    assert_eq!(
        (local_name.as_str(), nick.as_str()),
        ("Exact Host", "Exact Host")
    );

    fs::write(
        paths.config_file(),
        config_with_identity(b"Exact Host", b"<i<i>>"),
    )
    .expect("write Nick requiring its final client validation pass");
    let (_, nick, _) = load_classic_lobby_identity(&paths)
        .expect("configured Nick receives AllowEmpty then NoEmpty validation");
    assert_eq!(nick, "Unknown");

    fs::write(
        paths.config_file(),
        config_with_identity(b"<i<i<i<i>>>>", b""),
    )
    .expect("write nested LocalName with empty Nick");
    let (local_name, nick, _) = load_classic_lobby_identity(&paths)
        .expect("client validation uses the native finite pass count");
    assert_eq!(local_name, "<i<i>>");
    assert_eq!(nick, "Unknown");

    fs::write(
        paths.config_file(),
        b"[Network]\nNick=\"\"\n\n[Lobby]\nCountdownTime=7\n",
    )
    .expect("write identity with missing LocalName");
    let (local_name, nick, _) = load_classic_lobby_identity_with_hostname(&paths, b"H\xc3\xa4st")
        .expect("raw hostname bytes are preserved");
    assert_eq!(
        clonk_resources::encode_legacy_script_text(&local_name),
        Some(b"H\xc3\xa4st".to_vec())
    );
    assert_eq!(nick, local_name);

    fs::write(
        paths.config_file(),
        config_with_identity(b"<i>Unknown</i>", b""),
    )
    .expect("write LocalName that sanitizes to the hostname sentinel");
    let (local_name, nick, _) =
        load_classic_lobby_identity_with_hostname(&paths, b"Tylers-MacBook-Pro-M4-Max.local")
            .expect("bounded hostname fallback is hostable");
    assert_eq!(local_name, "Tylers-MacBook-Pro-M4-Ma");
    assert_eq!(nick, local_name);

    assert_eq!(
        sanitize_classic_lobby_name("", "test name", false).unwrap(),
        "empty",
        "an initially empty VAL_NameNoEmpty input uses the generic guard literal"
    );
    assert_eq!(
        sanitize_classic_lobby_name("<i></i>", "test name", false).unwrap(),
        "Unknown",
        "a nonempty input cleaned to empty uses the name-validator fallback"
    );
    assert_eq!(
        sanitize_classic_lobby_name("", "test nick", true).unwrap(),
        ""
    );
    assert!(sanitize_classic_lobby_name("☃", "test name", false).is_err());
}

#[test]
fn unsupported_classic_host_lobby_children_are_typed_fail_fast() {
    let cases = vec![(
        ClassicLobbyAction::RosterContextRequested {
            row: LobbyRosterId::Header(LobbyRosterHeader::ScriptPlayers),
            position: GuiPoint::new(1.0, 1.0),
        },
        "RosterContext",
    )];
    for (action, expected) in cases {
        let mut app = new_menu_app(640, 480);
        install_test_classic_host_lobby(&mut app);
        let error = app
            .process_classic_lobby_actions(vec![action])
            .expect_err("unimplemented lobby child must fail typed");
        assert!(
            error.to_string().contains(expected),
            "{expected} boundary missing from {error}"
        );
    }

    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::SheetRequested(
        LobbySheet::Players,
    )])
    .expect("already-visible Players sheet is a safe no-op");
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::SheetRequested(
        LobbySheet::Scenario,
    )])
    .expect("Scenario description sheet is implemented");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .active_sheet(),
        LobbySheet::Scenario
    );
}

#[test]
fn l085_lobby_options_refresh_only_while_the_sheet_is_active() {
    let mut app = new_menu_app(640, 480);
    let (_events, _commands) = install_classic_host_network_stub(&mut app);
    app.engine.set_control_rate(4);
    app.network_control_clock = Some(NetworkControlClock::new(0, 4));

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::SheetRequested(
        LobbySheet::Options,
    )])
    .expect("activate the Options list");
    let option_value = |app: &GameApp, kind| {
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .option_rows()
            .iter()
            .find(|row| row.kind == kind)
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(option_value(&app, LobbyOptionKind::ControlRate), "4");

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::SheetRequested(
        LobbySheet::Players,
    )])
    .expect("deactivate the Options list");
    app.network_control_clock
        .as_mut()
        .unwrap()
        .set_control_rate(7);
    app.sec1_timer().expect("pulse the inactive second timer");
    assert_eq!(
        option_value(&app, LobbyOptionKind::ControlRate),
        "4",
        "inactive options retain their last snapshot"
    );

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::SheetRequested(
        LobbySheet::Options,
    )])
    .expect("reactivation forces an immediate update");
    assert_eq!(option_value(&app, LobbyOptionKind::ControlRate), "7");

    app.network_control_clock
        .as_mut()
        .unwrap()
        .set_control_rate(8);
    app.sec1_timer().expect("pulse the active second timer");
    assert_eq!(option_value(&app, LobbyOptionKind::ControlRate), "8");
}

#[test]
fn l082_classic_lobby_internet_signup_is_pollable_and_rolls_back_failure() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live lobby registration without blocking");
    assert!(!app.scenario_game_options.values().master_server_signup);
    let enable = commands.receive_masterserver_signup();
    assert!(enable.enabled);
    assert!(!enable.config.league_server_signup);
    assert_eq!(enable.reference.summary().state, "Lobby");
    enable.complete(Ok(Some(clonk_network::LeagueStartResponse::default())));
    app.poll_live_masterserver_signup()
        .expect("apply live signup completion");
    assert!(app.scenario_game_options.values().master_server_signup);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live lobby deregistration without blocking");
    assert!(app.scenario_game_options.values().master_server_signup);
    let disable = commands.receive_masterserver_signup();
    assert!(!disable.enabled);
    disable.complete(Ok(None));
    app.poll_live_masterserver_signup()
        .expect("apply live deregistration completion");
    assert!(!app.scenario_game_options.values().master_server_signup);
    assert!(!app.scenario_game_options.values().league_server_signup);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue rejected registration without blocking");
    let rejected = commands.receive_masterserver_signup();
    assert!(rejected.enabled);
    rejected.complete(Err("masterserver rejected the game".to_string()));
    app.poll_live_masterserver_signup()
        .expect("apply rejected live registration");
    assert!(!app.scenario_game_options.values().master_server_signup);
    assert!(app.status_text.contains("masterserver rejected the game"));
}

#[test]
fn l082_aborting_live_internet_signup_keeps_the_prior_off_state() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live signup without waiting for its HTTP response");
    assert!(!app.scenario_game_options.values().master_server_signup);
    assert!(app.pending_lobby_internet_signup.is_some());
    assert!(
        !app.network_game_start_guard_passes(),
        "a host cannot launch while a Start or compensating End is unresolved"
    );
    let wait = app.message_dialogs.last().expect("cancellable wait dialog");
    assert!(matches!(
        wait.continuation,
        MessageDialogContinuation::LiveMasterserverSignup
    ));
    assert_eq!(
        wait.state.icon(),
        clonk_frontend::message_dialog::MessageDialogIcon::Standard(3)
    );
    assert_eq!(
        wait.state
            .button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel),
        "Abort"
    );

    let pending_command = commands.receive_masterserver_signup();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .expect("Abort closes the signup wait");
    pending_command.wait_for_cancellation();
    assert!(app.pending_lobby_internet_signup.is_none());
    assert!(!app.scenario_game_options.values().master_server_signup);
    assert_eq!(app.status_text, "Internet game signup cancelled.");
}

#[test]
fn l082_live_signup_applies_every_start_response_field() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (snapshot, reference) = default_exact_host_reference();
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);
    let league = LegacyCString::from_bytes(b"Cup".to_vec()).unwrap();
    let stream_to = LegacyCString::from_bytes(b"https://stream.example/upload?".to_vec()).unwrap();
    let response = clonk_network::LeagueStartResponse {
        league: league.clone(),
        stream_to: stream_to.clone(),
        seed: Some(0x1234_5678),
        max_players: 4,
        ..clonk_network::LeagueStartResponse::default()
    };

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live signup");
    commands
        .receive_masterserver_signup()
        .complete(Ok(Some(response)));
    app.poll_live_masterserver_signup()
        .expect("apply live Start response");

    let parameters = &app
        .host_join_snapshot
        .as_ref()
        .expect("live host JoinData")
        .parameters;
    assert_eq!(parameters.league, league);
    assert_eq!(parameters.random_seed, 0x1234_5678);
    assert_eq!(parameters.max_players, 4);
    assert_eq!(app.network_stream_address, stream_to);
    assert_eq!(app.network_league_name, b"Cup");
    assert_eq!(app.network_max_players, 4);
    assert_eq!(app.engine.max_players(), Some(4));
    let reference = app
        .advertised_game_reference
        .as_ref()
        .expect("updated exact host reference");
    assert_eq!(reference.parameters().league.as_bytes(), b"Cup");
    assert_eq!(reference.parameters().random_seed, 0x1234_5678);
    assert_eq!(reference.parameters().max_players, 4);
    let published = commands.take_published_join_snapshots();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].parameters.league.as_bytes(), b"Cup");
    assert_eq!(published[0].parameters.random_seed, 0x1234_5678);
    assert_eq!(published[0].parameters.max_players, 4);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live signup disable");
    let disable = commands.receive_masterserver_signup();
    assert!(!disable.enabled);
    disable.complete(Ok(None));
    app.poll_live_masterserver_signup()
        .expect("apply live signup disable");

    let parameters = &app
        .host_join_snapshot
        .as_ref()
        .expect("cleared live host JoinData")
        .parameters;
    assert!(parameters.league.is_empty());
    assert!(parameters.league_address.is_empty());
    assert_eq!(parameters.random_seed, 0x1234_5678);
    assert_eq!(parameters.max_players, 4);
    assert_eq!(app.network_stream_address, stream_to);
    assert!(app.network_league_name.is_empty());
    let reference = app
        .advertised_game_reference
        .as_ref()
        .expect("updated non-league host reference");
    assert!(reference.parameters().league.is_empty());
    assert!(reference.parameters().league_address.is_empty());
    assert_eq!(reference.parameters().random_seed, 0x1234_5678);
    assert_eq!(reference.parameters().max_players, 4);
    let published = commands.take_published_join_snapshots();
    assert_eq!(published.len(), 1);
    assert!(published[0].parameters.league.is_empty());
    assert!(published[0].parameters.league_address.is_empty());
    assert_eq!(published[0].parameters.random_seed, 0x1234_5678);
    assert_eq!(published[0].parameters.max_players, 4);
}

#[test]
fn l082_committed_start_apply_failure_tears_down_when_cleanup_cannot_start() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live signup");
    let signup = commands.receive_masterserver_signup();
    app.advertised_game_reference = None;
    signup.complete(Ok(Some(clonk_network::LeagueStartResponse::default())));
    app.poll_live_masterserver_signup()
        .expect("tear down the rejected live registration");

    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.message_dialogs.last().is_some_and(|dialog| dialog
        .state
        .message()
        .contains("could not begin compensating Internet signup cleanup")));
}

#[test]
fn l082_leaving_lobby_during_compensating_end_preserves_worker_cleanup() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (snapshot, reference) = default_exact_host_reference();
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live signup");
    commands
        .receive_masterserver_signup()
        .complete(Ok(Some(clonk_network::LeagueStartResponse {
            league: LegacyCString::from_bytes(b"Cup".to_vec()).expect("league name"),
            seed: Some(0x1234_5678),
            max_players: -1,
            ..clonk_network::LeagueStartResponse::default()
        })));
    app.poll_live_masterserver_signup()
        .expect("reject Start and queue compensating End");
    assert!(
        app.pending_lobby_internet_signup.is_some(),
        "the committed Start must remain visible until End"
    );
    let cleanup = commands.receive_masterserver_signup();
    assert!(!cleanup.enabled);

    app.show_main_menu();

    assert!(app.pending_lobby_internet_signup.is_none());
    assert!(app.network.is_none());
    cleanup.wait_for_cleanup_preservation();
}

#[test]
fn l082_failed_live_end_tears_the_host_down() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (snapshot, reference) = default_exact_host_reference();
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live signup");
    commands
        .receive_masterserver_signup()
        .complete(Ok(Some(clonk_network::LeagueStartResponse::default())));
    app.poll_live_masterserver_signup()
        .expect("commit live signup");
    commands.take_published_join_snapshots();

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live End");
    commands
        .receive_masterserver_signup()
        .complete(Err("End transport failed".to_owned()));
    app.poll_live_masterserver_signup()
        .expect("tear down after an unconfirmed End");

    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.message_dialogs.last().is_some_and(|dialog| dialog
        .state
        .message()
        .contains("Unable to confirm cleanup of the live Internet registration")));
}

#[test]
fn harpoonrace_invalid_live_seed_is_ended_before_the_host_can_launch() {
    let (prepared, _network_files) = prepare_harpoonrace_host_with_seed(1_784_903_471);
    let initial_snapshot = prepared
        .host_config()
        .initial_join_snapshot
        .clone()
        .expect("prepared HarpoonRace JoinData");
    let reference = prepared
        .initial_host_game_reference(true, &[])
        .expect("prepared HarpoonRace reference");
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    app.host_join_snapshot = Some(initial_snapshot);
    app.advertised_game_reference = Some(reference);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Host".to_owned(),
        prepared: Some(prepared),
    }));
    assert!(!app.scenario_game_options.values().master_server_signup);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('I'),
    )])
    .expect("queue live HarpoonRace signup");
    let signup = commands.receive_masterserver_signup();
    assert!(signup.enabled);
    signup.complete(Ok(Some(clonk_network::LeagueStartResponse {
        league: LegacyCString::from_bytes(b"Cup".to_vec()).expect("league name"),
        seed: Some(1_784_903_470),
        ..clonk_network::LeagueStartResponse::default()
    })));
    app.poll_live_masterserver_signup()
        .expect("reject and begin cleanup for invalid Start seed");

    assert!(
        app.pending_lobby_internet_signup.is_some(),
        "the committed Start must retain a compensating End transaction"
    );
    assert!(app.scenario_game_options.values().master_server_signup);
    assert!(
        !app.network_game_start_guard_passes(),
        "the host remains blocked while the compensating End is unresolved"
    );
    app.start_network_game_now()
        .expect("direct start remains blocked during compensating End");
    assert!(matches!(app.mode, AppMode::Menu));
    assert!(app.status_text.contains("1784903470"));
    assert_eq!(
        app.host_join_snapshot
            .as_ref()
            .expect("unchanged local JoinData")
            .parameters
            .random_seed,
        1_784_903_471,
        "the rejected response must not partially replace the local seed"
    );

    let rollback = commands.receive_masterserver_signup();
    assert!(!rollback.enabled);
    rollback.complete(Ok(None));
    app.poll_live_masterserver_signup()
        .expect("confirm compensating End");

    assert!(app.pending_lobby_internet_signup.is_none());
    assert!(!app.scenario_game_options.values().master_server_signup);
    assert_eq!(
        app.host_join_snapshot
            .as_ref()
            .expect("retained local JoinData")
            .parameters
            .random_seed,
        1_784_903_471
    );
    assert!(
        app.status_text.contains("1784903470"),
        "successful cleanup preserves the actionable rejection"
    );
    let scenario = match app.network_mode.as_ref() {
        Some(NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        })) => prepared
            .claim_scenario()
            .expect("claim retained valid HarpoonRace scenario"),
        _ => panic!("cleanup retains the prepared host"),
    };
    assert!(!scenario.generated_landscape_requires_seed_retry());
}

#[test]
fn classic_lobby_password_button_clears_then_presets_and_sets_live_password() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::LobbyHost,
        GameOptionValues {
            password: "old password".to_string(),
            last_password: "remembered password".to_string(),
            ..GameOptionValues::default()
        },
    );
    app.sync_scenario_game_option_bounds();
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(
        reference
            .replacing_lobby_options(true, LegacyCString::default())
            .expect("password-protected reference"),
    );

    let observer = thread::spawn(move || {
        let mut passwords = Vec::new();
        for result in [
            Err("host admission rejected the change".to_string()),
            Ok(()),
            Ok(()),
        ] {
            let (password, completion) = commands.receive_host_password();
            passwords.push(password.as_bytes().to_vec());
            completion.send(result).expect("return password result");
        }
        passwords
    });

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('P'),
    )])
    .expect("failed clear does not exit the lobby");
    assert_eq!(app.scenario_game_options.values().password, "old password");
    assert!(
        app.advertised_game_reference
            .as_ref()
            .expect("retained reference")
            .summary()
            .password_needed
    );

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('P'),
    )])
    .expect("one click clears the existing password");
    assert!(app.scenario_game_options.values().password.is_empty());
    assert!(
        !app.advertised_game_reference
            .as_ref()
            .expect("retained reference")
            .summary()
            .password_needed
    );

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('P'),
    )])
    .expect("open password input after clearing");
    assert_eq!(
        app.game_option_input_dialog
            .as_ref()
            .expect("password input")
            .controller
            .text(),
        "remembered password"
    );
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "unsupported 🔒".to_string(),
    )])
    .expect("unsupported password is rejected without mutating lobby state");
    assert!(app.scenario_game_options.values().password.is_empty());
    assert_eq!(
        app.scenario_game_options.values().last_password,
        "remembered password"
    );
    assert!(
        !app.advertised_game_reference
            .as_ref()
            .expect("retained reference")
            .summary()
            .password_needed
    );

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('P'),
    )])
    .expect("reopen password input after rejected text");
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "new password".to_string(),
    )])
    .expect("apply password input to the live host");

    assert_eq!(app.scenario_game_options.values().password, "new password");
    assert_eq!(
        app.scenario_game_options.values().last_password,
        "new password"
    );
    assert!(
        app.advertised_game_reference
            .as_ref()
            .expect("retained reference")
            .summary()
            .password_needed
    );
    assert_eq!(
        observer.join().expect("password observer"),
        vec![Vec::<u8>::new(), Vec::<u8>::new(), b"new password".to_vec(),]
    );
}

#[test]
fn classic_lobby_comment_updates_and_invalidates_the_advertised_reference() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Exact Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, mut commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    app.network = Some(manager);
    app.scenario_game_options.set_comment("old comment");
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(
        reference
            .replacing_lobby_options(
                false,
                LegacyCString::from_bytes(b"old comment".to_vec()).unwrap(),
            )
            .expect("commented reference"),
    );

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('M'),
    )])
    .expect("open lobby comment input");
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "old comment".to_string(),
    )])
    .expect("unchanged comment is a no-op");
    assert_eq!(commands.take_league_update_effects().1, 0);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('M'),
    )])
    .expect("reopen lobby comment input for invalid text");
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "unsupported 💬".to_string(),
    )])
    .expect("unsupported comment is rejected without mutating lobby state");
    assert_eq!(app.scenario_game_options.values().comment, "old comment");
    assert_eq!(commands.take_league_update_effects().1, 0);
    assert!(app
        .classic_host_lobby
        .as_ref()
        .expect("classic lobby")
        .controller
        .logs()
        .is_empty());
    assert_eq!(
        app.advertised_game_reference
            .as_ref()
            .expect("retained reference")
            .metadata()
            .comment
            .as_bytes(),
        b"old comment"
    );

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('M'),
    )])
    .expect("reopen lobby comment input");
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "new comment".to_string(),
    )])
    .expect("apply changed lobby comment");

    assert_eq!(commands.take_league_update_effects().1, 1);
    assert_eq!(
        app.advertised_game_reference
            .as_ref()
            .expect("retained reference")
            .metadata()
            .comment
            .as_bytes(),
        b"new comment"
    );
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("classic lobby")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some(clonk_frontend::game_option_buttons::COMMENT_CHANGED_LOG)
    );
}

#[test]
fn classic_lobby_fair_crew_control_echoes_and_countdown_or_force_gate_it() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::LobbyHost,
        GameOptionValues {
            fair_crew_strength: 75,
            ..GameOptionValues::default()
        },
    );
    app.sync_scenario_game_option_bounds();

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('F'),
    )])
    .expect("submit fair-crew enable control");
    let sets = commands.take_submitted_control_sets();
    assert_eq!(
        sets,
        vec![clonk_network::LegacyControlSet {
            value_type: 5,
            data: 75,
            by_client: 0,
        }]
    );
    assert!(!app.scenario_game_options.values().fair_crew);
    app.execute_control_set(sets[0]);
    assert!(app.scenario_game_options.values().fair_crew);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('F'),
    )])
    .expect("submit fair-crew disable control");
    assert_eq!(commands.take_submitted_control_sets()[0].data, -1);

    app.scenario_game_options.set_countdown(true);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('F'),
    )])
    .expect("countdown-disabled Fair Crew passes without a side effect");
    assert!(commands.take_submitted_control_sets().is_empty());

    app.scenario_game_options.set_countdown(false);
    app.scenario_game_options.set_lobby_fair_crew(false, true);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('F'),
    )])
    .expect("forced Fair Crew passes without a side effect");
    assert!(commands.take_submitted_control_sets().is_empty());

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::GameOptions(
        LobbyGameOptionInput::Hotkey('R'),
    )])
    .expect("record remains a local lobby preference");
    assert!(app.scenario_game_options.values().record);
    assert!(app.startup_view_flags.record);
}

#[test]
fn classic_host_configured_countdown_uses_sparse_packets_and_abort_unlocks_options() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: 12,
        check_league_rules: true,
        confirm_unassociated_savegame_players: false,
    }])
    .expect("start configured classic countdown");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(12)]
    );
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .countdown(),
        clonk_frontend::game_lobby::LobbyCountdownState::Long { seconds: 12 }
    );
    assert!(!app.scenario_game_options.values().countdown);

    assert!(!app.tick_network_lobby_countdown());
    assert!(commands.take_submitted_lobby_countdowns().is_empty());
    assert!(app.tick_network_lobby_countdown());
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(10)]
    );
    assert!(app.scenario_game_options.values().countdown);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::AbortCountdownRequested])
        .expect("abort configured classic countdown");
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(-1)]
    );
    assert!(app.host_lobby_countdown.is_none());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .countdown(),
        clonk_frontend::game_lobby::LobbyCountdownState::None
    );
    assert!(!app.scenario_game_options.values().countdown);
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some("Game start aborted.")
    );
}

#[test]
fn classic_host_zero_countdown_enters_go_without_a_countdown_packet() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated zero-countdown user data");
    let content = tempdir().expect("minimal zero-countdown content");
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let prepared = prepare_staged_network_host(&app, &staged);
    let expected_go = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: prepared.host_config().initial_status.control_mode,
        target_tick: 0,
    };
    app.host_join_snapshot = prepared.host_config().initial_join_snapshot.clone();
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Exact Host".to_string(),
        prepared: Some(prepared),
    }));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let mut generic_lobby = NetworkLobbyState::new(0, "Exact Host".to_string(), true);
    generic_lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(generic_lobby);
    install_test_classic_host_lobby(&mut app);
    let go_observer = thread::spawn(move || commands.complete_lobby_start(Ok(())));

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: 0,
        check_league_rules: true,
        confirm_unassociated_savegame_players: false,
    }])
    .expect("start prepared host immediately");

    assert_eq!(
        go_observer.join().expect("atomic Go observer"),
        vec![network::TestLobbyStartCommand::BeginGo {
            status: expected_go,
            join_allowed: false,
        }]
    );
    assert!(app.host_lobby_countdown.is_none());
    assert!(matches!(app.mode, AppMode::Loading));
    assert!(app.classic_host_lobby.is_none());
    assert!(app
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| !wait.visible));
}

#[test]
fn l085_atomic_go_worker_failure_is_reported_before_lobby_teardown() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated atomic Go user data");
    let content = tempdir().expect("minimal atomic Go content");
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let prepared = prepare_staged_network_host(&app, &staged);
    let expected_go = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: prepared.host_config().initial_status.control_mode,
        target_tick: 0,
    };
    app.host_join_snapshot = prepared.host_config().initial_join_snapshot.clone();
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Exact Host".to_string(),
        prepared: Some(prepared),
    }));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let mut generic_lobby = NetworkLobbyState::new(0, "Exact Host".to_string(), true);
    generic_lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(generic_lobby);
    install_test_classic_host_lobby(&mut app);
    let go_observer = thread::spawn(move || {
        commands.complete_lobby_start(Err("host loop rejected Go".to_string()))
    });

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: 0,
        check_league_rules: true,
        confirm_unassociated_savegame_players: false,
    }])
    .expect("report rejected atomic Go transition");

    assert_eq!(
        go_observer.join().expect("atomic Go observer"),
        vec![network::TestLobbyStartCommand::BeginGo {
            status: expected_go,
            join_allowed: false,
        }]
    );
    assert!(!matches!(app.mode, AppMode::Loading));
    assert!(app.loading_state.is_none());
    assert!(app.classic_host_lobby.is_some());
    assert!(app.network_lobby.is_some());
    assert!(app.status_text.contains("host loop rejected Go"));
}

#[test]
fn classic_ready_packets_update_roster_and_start_when_relevant_clients_are_ready() {
    let mut app = new_menu_app(640, 480);
    let (events, mut commands) = install_classic_host_network_stub(&mut app);
    app.control_clients.replace_snapshot([
        message_client(0, b"Exact Host"),
        message_client(7, b"Remote"),
    ]);
    app.control_player_infos.replace_snapshot(
        1,
        [clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        }],
    );
    app.sync_classic_lobby_roster();

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::ReadyChanged(true)])
        .expect("submit local host ready state");
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Ready,
        }]
    );
    assert!(app.classic_host_lobby.as_ref().unwrap().controller.ready());
    assert!(app.host_lobby_countdown.is_none());

    events
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue remote ready state");
    app.process_network_events()
        .expect("apply remote ready state");

    assert!(app
                .classic_host_lobby
                .as_ref()
                .unwrap()
                .controller
                .rows()
                .iter()
                .any(|row| matches!(row, LobbyRosterRow::Client(client) if client.id == 7 && client.status == LobbyClientStatus::Ready)));
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );
    assert_eq!(app.host_lobby_countdown, Some(HostLobbyCountdown::new()));
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Client Remote ready.", "The game will start in 5 seconds."]
    );

    events
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue duplicate remote ready state");
    app.process_network_events()
        .expect("log duplicate remote ready state");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some("Client Remote ready.")
    );
    assert!(commands.take_submitted_lobby_countdowns().is_empty());
}

#[test]
fn classic_lobby_system_logs_honor_timestamps() {
    let mut app = new_menu_app(640, 480);
    app.show_log_timestamps = true;
    let (events, _commands) = install_classic_host_network_stub(&mut app);
    app.control_clients.replace_snapshot([
        message_client(0, b"Exact Host"),
        message_client(7, b"Remote"),
    ]);

    events
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue timestamped ready state");
    app.process_network_events()
        .expect("append timestamped ready state");
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: 12,
        check_league_rules: true,
        confirm_unassociated_savegame_players: false,
    }])
    .expect("append timestamped countdown");

    let logs = app.classic_host_lobby.as_ref().unwrap().controller.logs();
    assert_eq!(logs.len(), 2);
    assert!(logs[0].text.ends_with(" Client Remote ready."));
    assert_ne!(logs[0].text, "Client Remote ready.");
    assert!(logs[1]
        .text
        .ends_with(" The game will start in 12 seconds."));
    assert_ne!(logs[1].text, "The game will start in 12 seconds.");
}

#[test]
fn classic_host_chat_start_abort_and_readycheck_use_live_lobby_actions() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);

    if let Some(lobby) = app.classic_host_lobby.as_mut() {
        lobby.chat_history_index = 3;
        lobby.controller.set_chat_draft("stale");
    }
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(String::new()))
        .expect("empty chat remains a local GUI error");
    assert_eq!(app.ui_sound_log, ["Error"]);
    assert!(commands.take_submitted_messages().is_empty());
    let lobby = app.classic_host_lobby.as_ref().unwrap();
    assert_eq!(lobby.chat_history_index, -1);
    assert!(lobby.controller.chat_edit_view().text.is_empty());

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/start 12".to_string()))
        .expect("start countdown from chat");
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(12)]
    );

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/start 3".to_string()))
        .expect("replace countdown from chat");
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![
            clonk_network::LobbyCountdownPacket::new(-1),
            clonk_network::LobbyCountdownPacket::new(3),
        ]
    );

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/abort".to_string()))
        .expect("abort countdown from chat");
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(-1)]
    );
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/abort".to_string()))
        .expect("report missing countdown in chat");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .last()
            .map(|line| (&*line.text, line.color)),
        Some(("Not in countdown!", [255, 32, 32, 255]))
    );

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/readycheck".to_string()))
        .expect("request ready check from chat");
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }]
    );
}

#[test]
fn l016_unknown_lobby_command_is_a_local_nonfatal_cpp_error() {
    let mut app = new_menu_app(640, 480);
    install_classic_host_network_stub(&mut app);
    app.show_log_timestamps = true;

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/xyz".to_string()))
        .expect("unknown lobby commands stay inside the lobby");

    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .last(),
        Some(&LobbyLogLine {
            text: "Unknown command: \"xyz\" - type /help to get a list of valid commands"
                .to_string(),
            color: [255, 32, 32, 255],
        }),
        "OnError bypasses timestamps, then AddTextLine makes red readable \
             (src/C4GameLobby.cpp:755-762; src/C4GuiLabels.cpp:293-299)"
    );
}

#[test]
fn l016_set_maxplayer_submits_sync_control_and_refreshes_lobby_count() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
        "/set maxplayer 4".to_string(),
    ))
    .expect("submit maximum-player setting");
    let sets = commands.take_submitted_control_sets();
    assert_eq!(
        sets,
        [clonk_network::LegacyControlSet {
            value_type: 2,
            data: 4,
            by_client: 0,
        }]
    );

    app.execute_control_set(sets[0]);
    assert_eq!(app.engine.max_players(), Some(4));
    assert_eq!(app.network_max_players, 4);
    let lobby = &app.classic_host_lobby.as_ref().unwrap().controller;
    assert!(lobby.players_title().contains("/4"));
    assert_eq!(
        lobby.logs().last().map(|line| line.text.as_str()),
        Some("MaxPlayer = 4")
    );
}

#[test]
fn l103_set_faircrew_submits_native_values_and_obeys_lobby_gates() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated fair-crew configuration");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "DefCrewStrength", "75")
        .expect("seed configured fair-crew strength");
    let mut app = new_menu_app(640, 480);
    app.app_paths = Some(paths);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::LobbyHost,
        GameOptionValues {
            fair_crew_strength: 999,
            ..GameOptionValues::default()
        },
    );

    for command in [
        "/set faircrew on",
        "/set faircrew off",
        "/set faircrew 42tail",
    ] {
        app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(command.to_string()))
            .expect("submit fair-crew command");
    }
    assert_eq!(
        commands.take_submitted_control_sets(),
        [
            clonk_network::LegacyControlSet {
                value_type: 5,
                data: 75,
                by_client: 0,
            },
            clonk_network::LegacyControlSet {
                value_type: 5,
                data: -1,
                by_client: 0,
            },
            clonk_network::LegacyControlSet {
                value_type: 5,
                data: 42,
                by_client: 0,
            },
        ]
    );

    app.network_is_league = true;
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
        "/set faircrew on".to_string(),
    ))
    .expect("league gate is a silent no-op");
    assert!(commands.take_submitted_control_sets().is_empty());

    app.network_is_league = false;
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
        "/set faircrew on".to_string(),
    ))
    .expect("non-host gate is a silent no-op");
    assert!(commands.take_submitted_control_sets().is_empty());

    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
        "/set faircrew -1".to_string(),
    ))
    .expect("malformed value is a silent no-op");
    assert!(commands.take_submitted_control_sets().is_empty());
}

#[test]
fn l083_reached_network_start_wait_uses_host_roster_and_client_abort_dialog() {
    let status = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: 1,
        target_tick: 4,
    };

    let mut host = new_menu_app(640, 480);
    host.network_mode = Some(NetworkMode::Host(host_network_settings()));
    host.begin_network_start_wait(status);
    host.show_reached_network_start_wait()
        .expect("show reached host wait");
    assert!(host
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| wait.visible && wait.expected_status == status));
    assert!(host.message_dialogs.is_empty());

    let mut client = new_menu_app(640, 480);
    client.mode = AppMode::Loading;
    client.network_mode = Some(NetworkMode::Client(client_network_settings()));
    client
        .show_reached_network_start_wait()
        .expect("show reached client wait");
    assert!(client.network_start_wait.is_none());
    let [dialog] = client.message_dialogs.as_slice() else {
        panic!("client should have exactly one start-wait dialog");
    };
    assert!(matches!(
        dialog.continuation,
        MessageDialogContinuation::NetworkClientStartWait
    ));
    assert_eq!(dialog.state.message(), "Waiting for start...");
    assert_eq!(dialog.state.caption(), "Network");
    assert_eq!(
        dialog.state.buttons(),
        clonk_frontend::message_dialog::MessageDialogButtons::CANCEL
    );
    assert_eq!(
        dialog.state.icon(),
        clonk_frontend::message_dialog::MessageDialogIcon::Standard(3)
    );
    assert_eq!(
        dialog.state.size(),
        clonk_frontend::message_dialog::MessageDialogSize::Small
    );
    assert_eq!(dialog.state.focused_button(), None);
    assert_eq!(
        dialog
            .state
            .button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel),
        "Cancel"
    );

    let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
    let blank = surface.pixels().to_vec();
    let gamma = client.startup_fragment_gamma();
    client
        .render_loading_message_dialogs(&mut surface, &gamma)
        .expect("render client wait over the loading surface");
    assert_ne!(surface.pixels(), blank.as_slice());
}

#[test]
fn classic_lobby_resource_sheet_refreshes_only_while_active() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app.classic_host_lobby
        .as_mut()
        .unwrap()
        .resource_rows
        .insert(
            7,
            LobbyResourceRow {
                id: 7,
                filename: "Network/Packs/Scenario.c4s".to_string(),
                present_percent: 10,
                save_possible: false,
            },
        );

    for sheet in [LobbySheet::Resources, LobbySheet::Players] {
        app.process_classic_lobby_actions(vec![ClassicLobbyAction::SheetRequested(sheet)])
            .expect("implemented classic right-hand sheets open without a boundary");
        assert_eq!(
            app.classic_host_lobby
                .as_ref()
                .unwrap()
                .controller
                .active_sheet(),
            sheet
        );
    }

    app.select_classic_lobby_sheet(LobbySheet::Resources);
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .resource_rows()[0]
            .present_percent,
        10
    );
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    events
        .send(NetworkEvent::ResourceProgress {
            resource_id: 7,
            present_percent: 33,
        })
        .unwrap();
    app.process_network_events()
        .expect("live resource progress is valid in the classic lobby");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .resource_rows()[0]
            .present_percent,
        33
    );

    app.select_classic_lobby_sheet(LobbySheet::Players);
    events
        .send(NetworkEvent::ResourceProgress {
            resource_id: 7,
            present_percent: 66,
        })
        .unwrap();
    app.process_network_events()
        .expect("hidden resource progress remains nonfatal");
    let lobby = app.classic_host_lobby.as_ref().unwrap();
    assert_eq!(lobby.resource_rows[&7].present_percent, 66);
    assert_eq!(
        lobby.controller.resource_rows()[0].present_percent,
        33,
        "inactive C4Network2ResDlg does not reconcile its visible rows"
    );
    events
        .send(NetworkEvent::ResourceComplete {
            resource_id: 7,
            core: clonk_engine::NetworkResourceCore {
                id: 7,
                loadable: true,
                filename: LegacyCString::from_bytes(b"Network/Scenario.c4s".to_vec()).unwrap(),
                ..Default::default()
            },
            path: PathBuf::from("Network/Scenario.c4s"),
            local: false,
        })
        .unwrap();
    app.process_network_events()
        .expect("hidden completion remains nonfatal");
    assert_eq!(
        app.classic_host_lobby.as_ref().unwrap().resource_rows[&7].present_percent,
        100
    );
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .resource_rows()[0]
            .present_percent,
        33
    );
    app.select_classic_lobby_sheet(LobbySheet::Resources);
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .resource_rows()[0]
            .present_percent,
        100,
        "activation forces an immediate resource-row refresh"
    );
    events
        .send(NetworkEvent::ResourceLoadFailed { resource_id: 7 })
        .unwrap();
    app.process_network_events()
        .expect("failed resource removal remains nonfatal");
    assert!(app
        .classic_host_lobby
        .as_ref()
        .unwrap()
        .resource_rows
        .is_empty());
    assert!(app
        .classic_host_lobby
        .as_ref()
        .unwrap()
        .controller
        .resource_rows()
        .is_empty());
    app.register_classic_lobby_player_resources(&[clonk_engine::ControlPlayerInfoEntry {
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        resource: Some(clonk_engine::NetworkResourceCore {
            id: 7,
            loadable: true,
            filename: LegacyCString::from_bytes(b"Network/Scenario.c4s".to_vec()).unwrap(),
            ..Default::default()
        }),
        ..Default::default()
    }]);
    assert!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .resource_rows
            .is_empty(),
        "an authoritative PlayerInfo replay cannot resurrect a failed resource"
    );
}

#[test]
fn generic_client_lobby_external_irc_button_is_retained_and_emits_typed_action() {
    let mut lobby = NetworkLobbyState::new(7, "Client".to_string(), false).with_external_chat(true);

    let pure_layout = lobby.update_layout(640.0, 480.0).clone();
    assert!(pure_layout.external_chat_button.is_some());

    let app = new_menu_app(640, 480);
    let (controller, _) = lobby
        .classic_render_state(
            app.graphics.surface(),
            app.assets.as_ref(),
            &app.scenario_game_options,
        )
        .expect("build client lobby with the retained external IRC snapshot");
    let fonts = app.assets.clonk_fonts.as_deref().unwrap();
    assert!(controller
        .layout(640, 480, fonts)
        .tab_buttons
        .iter()
        .any(|button| button.control == LobbyControl::ChatDialog));

    let rect = lobby
        .layout
        .as_ref()
        .and_then(|layout| layout.external_chat_button)
        .expect("rendered client lobby keeps the external IRC hit region");
    let point = GuiPoint::new(
        rect.origin.x + rect.size.width / 2.0,
        rect.origin.y + rect.size.height / 2.0,
    );
    let _ = lobby.controller.take_sounds();
    let down = lobby
        .classic_pointer_down(
            point,
            false,
            app.graphics.surface(),
            app.assets.as_ref(),
            &app.scenario_game_options,
        )
        .expect("press the retained external chat tab button");
    assert_eq!(down, Vec::new());
    assert_eq!(lobby.controller.take_sounds(), [LobbySound::ArrowHit]);
    let up = lobby
        .with_classic_controller_input(
            app.graphics.surface(),
            app.assets.as_ref(),
            &app.scenario_game_options,
            |controller, layout, roster| {
                controller.pointer_up(point, layout, roster, Instant::now())
            },
        )
        .expect("release the retained external chat tab button");
    assert_eq!(
        up,
        [ClassicLobbyAction::Chat(
            LobbyChatRequest::OpenExternalDialog
        )],
        "the single controller emission reaches the routed Chat arm"
    );
    assert_eq!(lobby.controller.take_sounds(), [LobbySound::Click]);

    let mut inactive = NetworkLobbyState::new(7, "Client".to_string(), false);
    assert!(inactive
        .update_layout(640.0, 480.0)
        .external_chat_button
        .is_none());
}

#[test]
fn joined_lobby_chat_routes_pointer_context_and_log_scroll() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    {
        let lobby = app.network_lobby.as_mut().expect("joined lobby");
        lobby.chat_edit = LobbyChatEditView {
            text: "alpha beta".to_string(),
            caret: 10,
            cursor_visible: true,
            ..LobbyChatEditView::default()
        };
        for index in 0..80 {
            lobby.push_log(LobbyLogLine {
                text: format!("joined lobby chat line {index}"),
                color: [255, 255, 255, 255],
            });
        }
    }

    let assets = Arc::clone(&app.assets);
    let (layout, max_scroll) = app
        .network_lobby
        .as_mut()
        .expect("joined lobby")
        .with_classic_controller_input(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
            |controller, layout, _| {
                let fonts = assets.clonk_fonts.as_deref().expect("classic fonts");
                let metrics = controller.chat_scroll_metrics(layout, &fonts.text);
                (layout.clone(), metrics.max_scroll)
            },
        )
        .expect("layout retained joined lobby");
    assert!(max_scroll > 0);
    let font = &assets.clonk_fonts.as_deref().expect("classic fonts").text;
    let edit_y = (layout.chat_edit.y + layout.chat_edit.h / 2) as f32;
    let beta = GuiPoint::new(
        (layout.chat_edit.x + 4 + font.measure("alpha b", false).0) as f32,
        edit_y,
    );

    app.handle_network_lobby_pointer_move(beta)
        .expect("move into joined edit");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, true)
        .expect("double click joined edit");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .selection,
        Some((6, 10)),
    );
    let label_point = GuiPoint::new(
        (layout.chat_label.x + layout.chat_label.w / 2) as f32,
        (layout.chat_label.y + layout.chat_label.h / 2) as f32,
    );
    app.handle_network_lobby_pointer_move(label_point)
        .expect("move over the already-focused joined chat label");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press the already-focused joined chat label");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release the already-focused joined chat label");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .selection,
        Some((6, 10)),
        "C4GUI::Dialog::SetFocus preserves the edit selection when focus is unchanged",
    );
    app.handle_network_lobby_pointer_move(beta)
        .expect("return to joined edit");
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .expect("open joined edit context");
    assert!(app.context_menu.is_some());
    app.handle_network_lobby_secondary_button(ElementState::Released)
        .expect("route joined secondary release through CMouse input timing");
    app.handle_network_lobby_middle_button(ElementState::Released)
        .expect("route joined middle release through CMouse input timing");
    app.context_menu = None;
    app.network_lobby.as_mut().expect("joined lobby").pointer = None;
    app.handle_network_lobby_context_key()
        .expect("Apps opens focused joined edit context without a pointer");
    assert!(app.context_menu.is_some());
    app.process_classic_lobby_chat_request(LobbyChatRequest::ContextCommand(
        LobbyChatContextCommand::Clear,
    ))
    .expect("clear joined selection");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .text,
        "alpha ",
    );
    app.context_menu = None;

    let roster_point = GuiPoint::new(
        (layout.roster_client.x + 2) as f32,
        (layout.roster_client.y + 2) as f32,
    );
    app.handle_network_lobby_pointer_move(roster_point)
        .expect("move into joined roster");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("focus joined roster");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release joined roster");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .focus(),
        LobbyControl::Roster,
    );
    app.keyboard_modifiers = ModifiersState::ALT;
    app.handle_key(VirtualKeyCode::KeyT, ElementState::Pressed)
        .expect("Alt+T routes through the retained joined lobby label");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .focus(),
        LobbyControl::ChatInput,
    );
    app.keyboard_modifiers = ModifiersState::empty();
    app.handle_network_lobby_pointer_move(roster_point)
        .expect("return to the joined roster after mnemonic focus");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press joined roster after mnemonic focus");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release joined roster after mnemonic focus");
    app.keyboard_modifiers = ModifiersState::CONTROL;
    assert!(!app
        .handle_network_lobby_chat_key(VirtualKeyCode::KeyA, ElementState::Pressed)
        .expect("unfocused joined edit rejects Ctrl+A"));
    app.keyboard_modifiers = ModifiersState::empty();
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .text,
        "alpha ",
    );
    {
        let lobby = app.network_lobby.as_mut().expect("joined lobby");
        let caret = lobby.chat_edit.caret;
        assert_eq!(
            lobby.handle_key(KeyCode::Enter, ElementState::Pressed),
            None,
            "an unfocused joined edit must not submit through the reduced adapter",
        );
        assert_eq!(
            lobby.handle_key(KeyCode::Left, ElementState::Pressed),
            None,
            "an unfocused joined edit must not consume cursor keys",
        );
        assert_eq!(lobby.chat_edit.caret, caret);
        assert_eq!(lobby.chat_edit.text, "alpha ");
    }
    let unfocused_view = app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .chat_edit
        .clone();
    app.handle_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed)
        .expect("full key dispatch leaves the unfocused joined edit alone");
    assert_eq!(
        app.network_lobby.as_ref().expect("joined lobby").chat_edit,
        unfocused_view,
    );
    app.handle_network_lobby_pointer_move(beta)
        .expect("move from the focused roster into joined chat");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, true)
        .expect("route a global cross-control double click");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .focus(),
        LobbyControl::Roster,
        "C4GUI::Edit::LeftDouble does not run the ordinary focus-changing LeftDown path",
    );
    app.handle_text_input('z')
        .expect("ordinary text refocuses joined chat");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .text,
        "z",
    );
    app.handle_text_input('\u{80}')
        .expect("C++ CharIn accepts a UTF-8 C1 code point");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .text,
        "z\u{80}",
    );
    let exact_modifier_view = app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .chat_edit
        .clone();
    for (modifiers, key) in [
        (ModifiersState::SHIFT, VirtualKeyCode::Enter),
        (ModifiersState::ALT, VirtualKeyCode::ArrowLeft),
        (
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            VirtualKeyCode::KeyA,
        ),
        (ModifiersState::CONTROL, VirtualKeyCode::ArrowUp),
    ] {
        app.keyboard_modifiers = modifiers;
        app.handle_key(key, ElementState::Pressed)
            .expect("reject an over-modified joined chat binding");
        assert_eq!(
            app.network_lobby.as_ref().expect("joined lobby").chat_edit,
            exact_modifier_view,
        );
    }
    app.keyboard_modifiers = ModifiersState::empty();

    let log_point = GuiPoint::new(
        (layout.chat_log_client.x + 2) as f32,
        (layout.chat_log_client.y + 2) as f32,
    );
    app.handle_network_lobby_pointer_move(log_point)
        .expect("move over joined chat log");
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0)
        .expect("wheel joined chat log");
    let wheel_scroll = app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .controller
        .chat_scroll();
    assert!(wheel_scroll < max_scroll);

    let assets = Arc::clone(&app.assets);
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .classic_render_state(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
        )
        .expect("refresh joined projection");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .chat_scroll(),
        wheel_scroll,
        "a frame projection must not repin the retained TextWindow",
    );

    let scrollbar_start = GuiPoint::new(
        (layout.chat_log_scrollbar.x + layout.chat_log_scrollbar.w / 2) as f32,
        (layout.chat_log_scrollbar.y + layout.chat_log_scrollbar.h / 2) as f32,
    );
    let scrollbar_end = GuiPoint::new(scrollbar_start.x, (layout.chat_log_scrollbar.y + 17) as f32);
    app.handle_network_lobby_touch(TouchPhase::Started, scrollbar_start, false)
        .expect("start joined scrollbar drag");
    app.handle_network_lobby_touch(TouchPhase::Moved, scrollbar_end, false)
        .expect("drag joined scrollbar thumb");
    app.handle_network_lobby_touch(TouchPhase::Ended, scrollbar_end, false)
        .expect("release joined scrollbar drag");
    let scrollbar_scroll = app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .controller
        .chat_scroll();
    assert!(scrollbar_scroll < wheel_scroll);
    let assets = Arc::clone(&app.assets);
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .classic_render_state(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
        )
        .expect("refresh joined projection after scrollbar drag");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .chat_scroll(),
        scrollbar_scroll,
    );

    app.handle_network_lobby_touch(TouchPhase::Started, scrollbar_start, false)
        .expect("start another joined scrollbar capture");
    app.cancel_underlying_interaction();
    let cancelled_scroll = app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .controller
        .chat_scroll();
    app.handle_network_lobby_touch(TouchPhase::Moved, scrollbar_end, false)
        .expect("motion after joined capture cancellation");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .chat_scroll(),
        cancelled_scroll,
        "an elevated dialog cancellation must not strand the joined TextWindow drag",
    );

    let drag_start = GuiPoint::new((layout.chat_edit.x + 4) as f32, edit_y);
    let drag_end = GuiPoint::new(
        (layout.chat_edit.x + layout.chat_edit.w + 40) as f32,
        edit_y,
    );
    app.process_classic_lobby_chat_request(LobbyChatRequest::InsertText(
        "touch selection".to_string(),
    ))
    .expect("seed joined touch selection");
    app.handle_network_lobby_touch(TouchPhase::Started, drag_start, false)
        .expect("start joined touch selection");
    app.handle_network_lobby_touch(TouchPhase::Moved, drag_end, false)
        .expect("drag joined touch selection outside edit");
    app.handle_network_lobby_touch(TouchPhase::Cancelled, drag_end, false)
        .expect("cancel joined touch capture");
    assert!(
        lobby_chat_selection(&app.network_lobby.as_ref().expect("joined lobby").chat_edit,)
            .is_some(),
        "touch cancel releases capture but retains the last selection",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView::default());
    app.process_classic_lobby_chat_request(LobbyChatRequest::InsertText("W".repeat(200)))
        .expect("type a joined draft wider than the C4GUI edit client");
    assert!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .horizontal_scroll
            > 0,
        "user text insertion keeps the C++ caret visible",
    );
    app.process_classic_lobby_chat_request(LobbyChatRequest::EditKey {
        key: LobbyChatEditKey::Home,
        modifiers: LobbyChatKeyModifiers::default(),
    })
    .expect("move the long joined draft caret home");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .horizontal_scroll,
        (font.measure("\u{a6}", false).0 / 2).saturating_sub(2),
        "ordinary cursor operations run C4GUI::Edit::ScrollCursorInView",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView::default());
    app.paste_classic_lobby_chat_text(&"W".repeat(200))
        .expect("paste a joined draft wider than the C4GUI edit client");
    assert!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .horizontal_scroll
            > 0,
        "clipboard insertion keeps the C++ caret visible",
    );
    let paste_scroll = app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .chat_edit
        .horizontal_scroll;
    app.paste_classic_lobby_chat_text("")
        .expect("route an empty joined paste");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .horizontal_scroll,
        paste_scroll,
        "C4GUI::Edit::InsertText returns before scrolling an empty paste",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "W".repeat(254),
        caret: 254,
        selection: None,
        horizontal_scroll: 123,
        cursor_visible: true,
    });
    app.process_classic_lobby_chat_request(LobbyChatRequest::InsertText("X".to_string()))
        .expect("reject joined text beyond the C++ edit capacity");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .horizontal_scroll,
        123,
        "a zero-byte C++ insertion preserves retained horizontal scroll",
    );

    app.message_input_history.clear();
    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "seed".to_string(),
        caret: 4,
        selection: None,
        horizontal_scroll: 37,
        cursor_visible: true,
    });
    app.process_classic_lobby_chat_request(LobbyChatRequest::History { older: true })
        .expect("browse beyond the joined chat history");
    let history_view = &app.network_lobby.as_ref().expect("joined lobby").chat_edit;
    assert!(history_view.text.is_empty());
    assert_eq!(
        history_view.horizontal_scroll, 37,
        "C++ history miss clears through DeleteSelection without scrolling",
    );
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerDown(drag_start))
        .expect("press the unchanged empty-history caret");
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerUp(drag_end))
        .expect("release without an intervening C++ drag update");
    let unchanged_pointer_view = &app.network_lobby.as_ref().expect("joined lobby").chat_edit;
    assert_eq!(unchanged_pointer_view.caret, 0);
    assert_eq!(
        unchanged_pointer_view.horizontal_scroll, 37,
        "same-caret down/up preserve C++ iXScroll",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "alpha".to_string(),
        caret: 0,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    });
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerDown(drag_start))
        .expect("start a joined selection without moving");
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerUp(drag_end))
        .expect("finish the joined selection at the release coordinate");
    assert_eq!(
        lobby_chat_selection(&app.network_lobby.as_ref().expect("joined lobby").chat_edit,),
        Some(0..5),
        "C4GUI::Screen::StopDragging applies the final release coordinate",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "alpha".to_string(),
        caret: 0,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    });
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerDown(drag_start))
        .expect("retain an equal C++ drag anchor");
    app.process_classic_lobby_chat_request(LobbyChatRequest::InsertText("Z".to_string()))
        .expect("edit while the joined pointer capture remains held");
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerMove(drag_end))
        .expect("continue dragging from the retained C++ anchor");
    assert_eq!(
        lobby_chat_selection(&app.network_lobby.as_ref().expect("joined lobby").chat_edit,),
        Some(0..6),
    );
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerUp(drag_end))
        .expect("release the retained joined drag anchor");

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "alpha".to_string(),
        caret: 5,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    });
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerDown(drag_end))
        .expect("start a reverse joined selection");
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerMove(drag_start))
        .expect("drag the joined selection back to the start");
    app.process_classic_lobby_chat_request(LobbyChatRequest::InsertText("Z".to_string()))
        .expect("replace the reverse selection while capture remains held");
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerMove(drag_end))
        .expect("continue from DeleteSelection's new native anchor");
    assert_eq!(
        lobby_chat_selection(&app.network_lobby.as_ref().expect("joined lobby").chat_edit,),
        Some(0..1),
    );
    app.process_classic_lobby_chat_request(LobbyChatRequest::PointerUp(drag_end))
        .expect("release the reverse joined selection");

    app.install_active_lobby_chat_view(LobbyChatEditView::default());
    app.paste_classic_lobby_chat_text(&("W".repeat(200) + "\n"))
        .expect("paste and submit one long joined line");
    let completed_line_view = &app.network_lobby.as_ref().expect("joined lobby").chat_edit;
    assert!(completed_line_view.text.is_empty());
    assert!(
        completed_line_view.horizontal_scroll > 0,
        "C++ scrolls each pasted line before clearing it for submission",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "submitted".to_string(),
        caret: 9,
        selection: None,
        horizontal_scroll: 61,
        cursor_visible: false,
    });
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("submitted".to_string()))
        .expect("submit a joined lobby draft");
    let submitted_view = &app.network_lobby.as_ref().expect("joined lobby").chat_edit;
    assert!(submitted_view.text.is_empty());
    assert_eq!(submitted_view.horizontal_scroll, 61);
    assert!(
        submitted_view.cursor_visible,
        "DeleteSelection refreshes the focused caret after nonempty submission",
    );
}

#[test]
fn l108_completed_scenario_description_uses_exact_desc_or_title() {
    let app = new_state_only_menu_app(640, 480);
    let directory = tempdir().expect("scenario description fixture");
    let scenario = directory.path().join("Remote.c4s");
    fs::create_dir_all(&scenario).expect("create unpacked scenario group");
    fs::write(
        scenario.join("DescUS.rtf"),
        br"{\rtf1 Gold Mine\par Mine some gold.\par}",
    )
    .expect("write scenario RTF description");

    assert_eq!(
        app.completed_lobby_scenario_description(&scenario, "Remote title".to_string()),
        LobbyScenarioText::Description("Gold Mine\nMine some gold.\n".to_string())
    );

    fs::remove_file(scenario.join("DescUS.rtf")).expect("remove exact description");
    fs::write(scenario.join("Scenario.txt"), b"Unrelated fallback")
        .expect("write unrelated scenario component");
    assert_eq!(
        app.completed_lobby_scenario_description(&scenario, "Remote title".to_string()),
        LobbyScenarioText::Title("Remote title".to_string())
    );
    assert_eq!(
        app.completed_lobby_scenario_description(
            &directory.path().join("Missing.c4s"),
            "Remote title".to_string(),
        ),
        LobbyScenarioText::Message("scenario file load error".to_string())
    );
}

#[test]
fn l163_lobby_scenario_description_ignores_bytes_after_native_nul() {
    let app = new_state_only_menu_app(640, 480);
    let directory = tempdir().expect("scenario description fixture");
    let scenario = directory.path().join("Remote.c4s");
    fs::create_dir_all(&scenario).expect("create unpacked scenario group");
    fs::write(
        scenario.join("DescUS.rtf"),
        b"{\\rtf1 Visible lobby description.\\par}\0}",
    )
    .expect("write native-NUL scenario description");

    assert_eq!(
        app.completed_lobby_scenario_description(&scenario, "Remote title".to_string()),
        LobbyScenarioText::Description("Visible lobby description.\n".to_string())
    );

    fs::write(scenario.join("DescUS.rtf"), b"\0ignored suffix")
        .expect("replace description with native-NUL-first data");
    fs::write(
        scenario.join("DescDE.rtf"),
        br"{\rtf1 Later language must not win.\par}",
    )
    .expect("write later-language scenario description");
    assert_eq!(
        load_lobby_scenario_description(
            &scenario,
            &["US".to_string(), "DE".to_string()],
            &LanguagePacks::default(),
        )
        .expect("load native-NUL-first lobby description"),
        None
    );
}

// C4GameLobby::MainDlg adds the Options sheet for every participant and
// fills it with one C4GameOptionsList (src/C4GameLobby.cpp:223,247). For a
// joined client every row is read-only: control mode is read-only in the
// lobby regardless of role, control rate and the team rows are read-only
// off the control host, and RuntimeJoin/RandomTeamCount are never added
// (src/C4GameOptions.cpp:80,126,154,186,211,234,269-280).
#[test]
fn joined_lobby_options_sheet_projects_read_only_rows() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (manager, events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    let mut status = host_config.initial_status;
    status.control_mode = 1;
    snapshot.parameters.control_rate = 3;
    snapshot.parameters.teams.active = 1;
    snapshot.parameters.teams.team_colors = 1;
    snapshot.parameters.teams.team_distribution = 3;
    snapshot.parameters.teams.random_team_count = 4;
    events
        .send(NetworkEvent::JoinData(clonk_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: 0,
            status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        }))
        .unwrap();
    app.process_network_events()
        .expect("client JoinData seeds lobby state");

    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Options))
        .expect("joined Options opens without a parity boundary");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().active_sheet,
        LobbySheet::Options
    );

    let rows = app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .option_rows()
        .to_vec();
    assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        [
            LobbyOptionKind::ControlMode,
            LobbyOptionKind::ControlRate,
            LobbyOptionKind::TeamDistribution,
            LobbyOptionKind::TeamColors,
        ],
        "no RuntimeJoin and no RandomTeamCount for a joined client"
    );
    assert!(
        rows.iter().all(|row| !row.editable),
        "every joined client row is a read-only ComboBox"
    );
    assert_eq!(rows[1].value, "3", "control rate follows the joined status");

    // C4GameOptionsList::Activate forces one Update, and its Sec1 timer
    // keeps the visible sheet current (src/C4GameOptions.cpp:302-308).
    {
        let lobby = app.network_lobby.as_mut().unwrap();
        lobby.controller.set_option_rows(Vec::new());
    }
    app.sec1_timer()
        .expect("second timer refreshes joined options");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .option_rows()
            .len(),
        4,
        "the one-second callback reprojects the visible sheet"
    );

    // An inactive sheet does no periodic work.
    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Players))
        .expect("leave the joined Options sheet");
    {
        let lobby = app.network_lobby.as_mut().unwrap();
        lobby.controller.set_option_rows(Vec::new());
    }
    app.sec1_timer().expect("second timer with Options hidden");
    assert!(
        app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .option_rows()
            .is_empty(),
        "a hidden Options sheet retains its last projection"
    );
}

// The Options tab is added for every participant, so a joined client can
// click the cog itself (src/C4GameLobby.cpp:223). Exercise the pointer route
// the report used rather than the action it lowers to.
#[test]
fn joined_lobby_options_tab_click_opens_the_read_only_sheet() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (manager, events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let host_config = clonk_network::HostConfig::default();
    let snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    events
        .send(NetworkEvent::JoinData(clonk_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: 0,
            status: host_config.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        }))
        .unwrap();
    app.process_network_events()
        .expect("client JoinData seeds lobby state");

    let assets = Arc::clone(&app.assets);
    let options_tab = app
        .network_lobby
        .as_mut()
        .expect("joined lobby")
        .with_classic_controller_input(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
            |_, layout, _| {
                layout
                    .tab_buttons
                    .iter()
                    .find(|tab| tab.sheet == Some(LobbySheet::Options))
                    .map(|tab| tab.rect)
            },
        )
        .expect("layout the joined lobby")
        .expect("the joined lobby offers the Options tab");
    let cog = GuiPoint::new(
        (options_tab.x + options_tab.w / 2) as f32,
        (options_tab.y + options_tab.h / 2) as f32,
    );

    app.handle_network_lobby_pointer_move(cog)
        .expect("move onto the joined Options tab");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press the joined Options tab");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("clicking the joined Options tab must not raise a parity boundary");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().active_sheet,
        LobbySheet::Options
    );
    assert!(
        !app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .option_rows()
            .is_empty(),
        "the activated sheet projects its read-only rows"
    );

    let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .render_classic(
            &mut surface,
            assets.as_ref(),
            &app.scenario_game_options,
            false,
            true,
            &startup_identity_gamma().clone(),
        )
        .expect("project one joined Options frame");

    // Every joined row is a read-only ComboBox, so clicking its value opens
    // no selection popup (src/C4GameOptions.cpp:80,126).
    let combo = app
        .network_lobby
        .as_mut()
        .expect("joined lobby")
        .with_classic_controller_input(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
            |_, _, roster| roster.rows.first().and_then(|row| row.option_value),
        )
        .expect("layout the joined Options sheet")
        .expect("the joined Options sheet stacks a ComboBox on its first row");
    let value = GuiPoint::new(
        (combo.x + combo.w / 2) as f32,
        (combo.y + combo.h / 2) as f32,
    );
    app.handle_network_lobby_pointer_move(value)
        .expect("move onto a read-only joined option value");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press a read-only joined option value");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release a read-only joined option value");
    assert!(
        app.context_menu.is_none(),
        "a read-only joined ComboBox opens no selection popup"
    );
}

#[test]
fn client_lobby_resource_sheet_tracks_hidden_transfer_progress() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (manager, events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    let scenario = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 9,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Scenarios/Remote.c4s".to_vec()).unwrap(),
        ..Default::default()
    };
    snapshot.parameters.scenario = scenario.clone();
    events
        .send(NetworkEvent::JoinData(clonk_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: 0,
            status: host_config.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        }))
        .unwrap();
    app.process_network_events()
        .expect("client JoinData seeds resource rows");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().resource_rows[&9].present_percent,
        0
    );

    events
        .send(NetworkEvent::ResourceProgress {
            resource_id: 9,
            present_percent: 37,
        })
        .unwrap();
    app.process_network_events()
        .expect("hidden client resource progress applies");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().active_sheet,
        LobbySheet::Players
    );
    assert_eq!(
        app.network_lobby.as_ref().unwrap().resource_rows[&9].present_percent,
        37
    );

    {
        let lobby = app.network_lobby.as_mut().unwrap();
        let layout = lobby.update_layout(640.0, 480.0).clone();
        let rect = layout
            .sheet_buttons
            .iter()
            .find(|(sheet, _)| *sheet == LobbySheet::Resources)
            .unwrap()
            .1;
        lobby.handle_panel_pointer_move(GuiPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        ));
    }
    app.handle_mouse_button(ElementState::Pressed)
        .expect("press the client Resources tab");
    app.handle_mouse_button(ElementState::Released)
        .expect("release the client Resources tab through the retained controller");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().active_sheet,
        LobbySheet::Resources
    );
    let (controller, _) = {
        let surface = app.graphics.surface();
        app.network_lobby.as_mut().unwrap().classic_render_state(
            surface,
            app.assets.as_ref(),
            &app.scenario_game_options,
        )
    }
    .expect("build client Resources presentation");
    assert_eq!(controller.active_sheet(), LobbySheet::Resources);
    assert_eq!(
        controller
            .resource_rows()
            .iter()
            .find(|row| row.id == 9)
            .unwrap()
            .present_percent,
        37
    );

    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Players))
        .expect("hide client Resources sheet");
    events
        .send(NetworkEvent::ResourceProgress {
            resource_id: 9,
            present_percent: 73,
        })
        .unwrap();
    app.process_network_events()
        .expect("hidden client resource progress remains retained");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().resource_rows[&9].present_percent,
        73
    );
    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Resources))
        .expect("reopen client Resources sheet");
    let (controller, _) = {
        let surface = app.graphics.surface();
        app.network_lobby.as_mut().unwrap().classic_render_state(
            surface,
            app.assets.as_ref(),
            &app.scenario_game_options,
        )
    }
    .expect("rebuild client Resources presentation");
    assert_eq!(
        controller
            .resource_rows()
            .iter()
            .find(|row| row.id == 9)
            .unwrap()
            .present_percent,
        73
    );

    {
        let lobby = app.network_lobby.as_mut().unwrap();
        for id in 100..150 {
            lobby.resource_rows.insert(
                id,
                LobbyResourceRow {
                    id,
                    filename: format!("Resource{id}.c4g"),
                    present_percent: 50,
                    save_possible: false,
                },
            );
        }
        let fonts = app.assets.clonk_fonts.as_deref().unwrap();
        let layout = clonk_frontend::game_lobby::game_lobby_layout(
            640,
            480,
            fonts.title.line_height,
            fonts.text.line_height,
            LobbyRole::Client,
            lobby.has_teams,
            false,
        );
        lobby.handle_panel_pointer_move(GuiPoint::new(
            (layout.roster.x + 5) as f32,
            (layout.roster.y + 5) as f32,
        ));
    }
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
        .expect("client Resources wheel scrolls overflow rows");
    assert!(app.network_lobby.as_ref().unwrap().resource_scroll > 0);
}

#[test]
fn prepared_lobby_resource_rows_seed_complete_eligible_cores_in_id_order() {
    let core = |id, filename: &[u8]| clonk_engine::NetworkResourceCore {
        id,
        loadable: true,
        filename: LegacyCString::from_bytes(filename.to_vec()).unwrap(),
        ..Default::default()
    };
    let mut snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .expect("default host snapshot");
    snapshot.parameters.scenario = core(9, b"Scenarios/Round.c4s");
    snapshot.parameters.game_resources = vec![core(3, b"System.c4g"), core(5, b"Material.c4g")];
    snapshot.dynamic = core(7, b"Round.c4s/Material.c4g");
    let player = |id, flags, resource| clonk_engine::ControlPlayerInfoEntry {
        id,
        flags,
        resource: Some(resource),
        ..Default::default()
    };
    snapshot.parameters.player_infos.clients = vec![clonk_network::ClientPlayerInfosSnapshot {
        client_id: 0,
        flags: 0,
        players: vec![
            player(
                1,
                clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                core(4, b"Players/A.c4p"),
            ),
            player(
                2,
                clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE
                    | clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                core(2, b"Players/Removed.c4p"),
            ),
            player(
                3,
                clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE
                    | clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE,
                core(1, b"Players/Embedded.c4p"),
            ),
            player(4, 0, core(6, b"Players/NoResourceFlag.c4p")),
        ],
    }];

    let rows = initial_classic_lobby_resource_rows(Some(&snapshot));
    assert_eq!(
        rows.keys().copied().collect::<Vec<_>>(),
        vec![3, 4, 5, 7, 9]
    );
    assert!(rows.values().all(|row| row.present_percent == 100));
    assert_eq!(rows[&9].filename, "Scenarios/Round.c4s");
    assert_eq!(rows[&4].filename, "Players/A.c4p");
}

#[test]
fn lobby_resource_save_gate_matches_native_type_completion_and_locality_matrix() {
    let root = PathBuf::from("l105-root");
    let work = root.join("Network");
    let source = work.join("Downloaded.c4g");
    for resource_type in 0_u8..=6 {
        for local in [false, true] {
            for complete in [false, true] {
                for allow_player_save in [false, true] {
                    let type_allowed = resource_type
                        == clonk_network::HostResourceType::Scenario as u8
                        || resource_type == clonk_network::HostResourceType::Definitions as u8
                        || resource_type == clonk_network::HostResourceType::Player as u8
                            && allow_player_save;
                    assert_eq!(
                                lobby_resource_save_possible(
                                    local,
                                    complete,
                                    resource_type,
                                    allow_player_save,
                                    &source,
                                    &work,
                                ),
                                !local && complete && type_allowed,
                                "type={resource_type} local={local} complete={complete} allow_player_save={allow_player_save}"
                            );
                }
            }
        }
    }
    assert!(
        lobby_resource_save_possible(
            false,
            true,
            clonk_network::HostResourceType::Scenario as u8,
            false,
            &root.join("NetworkSibling/Downloaded.c4s"),
            &work,
        ),
        "SEqual2 accepts any literal raw prefix"
    );
    assert!(!lobby_resource_save_possible(
        false,
        true,
        clonk_network::HostResourceType::Scenario as u8,
        false,
        &root.join("network/Downloaded.c4s"),
        &work,
    ));

    let player_named_scenario = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        filename: LegacyCString::from_bytes(b"Remote/Upper.C4P".to_vec()).unwrap(),
        ..Default::default()
    };
    assert_eq!(
        lobby_resource_save_target(
            Path::new("install"),
            Path::new("Players"),
            &player_named_scenario,
        ),
        Some((
            Path::new("install/Players/Upper.C4P").to_path_buf(),
            "Upper.C4P".to_string(),
        )),
        "the advertised extension, not the resource type, selects PlayerPath"
    );

    let legacy_named_scenario = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        filename: LegacyCString::from_bytes(b"Remote/Gr\xfcnd.c4s".to_vec()).unwrap(),
        ..Default::default()
    };
    #[cfg(unix)]
    let legacy_target = Path::new("install").join(path_from_group_name_bytes(b"Gr\xfcnd.c4s"));
    #[cfg(not(unix))]
    let legacy_target = Path::new("install").join("Gründ.c4s");
    assert_eq!(
        lobby_resource_save_target(
            Path::new("install"),
            Path::new("Players"),
            &legacy_named_scenario,
        ),
        Some((legacy_target, "Gründ.c4s".to_string())),
        "the filesystem target preserves the legacy basename bytes"
    );
}

#[test]
fn installed_empty_joined_roster_does_not_revive_participant_fallback() {
    let mut lobby = NetworkLobbyState::new(7, "Client".to_string(), false);
    assert!(
        !lobby.visible_roster_rows().is_empty(),
        "the pre-projection adapter exposes its participant fallback"
    );

    lobby.roster_rows_authoritative = true;
    assert!(
        lobby.visible_roster_rows().is_empty(),
        "an authoritative empty projection remains empty"
    );
}

#[test]
fn lobby_resource_save_dialogs_cover_overwrite_decline_accept_success_and_failure() {
    use clonk_frontend::message_dialog::{
        MessageDialogButtons, MessageDialogIcon, MessageDialogResult,
    };

    let root = tempdir().expect("resource save root");
    let work = root.path().join("Network");
    fs::create_dir(&work).expect("network work directory");
    let source = work.join("Downloaded.c4s");
    let target = root.path().join("Downloaded.c4s");
    fs::write(&source, b"first").expect("downloaded resource");

    let mut app = new_menu_app(640, 480);
    let mut settings = ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client");
    settings.resource_directory = work.clone();
    app.network_mode = Some(NetworkMode::Client(settings));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let core = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 47,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Remote/Downloaded.c4s".to_vec()).unwrap(),
        ..Default::default()
    };
    app.admission_resources.register_lobby_resource(&core);
    app.admission_resources
        .mark_complete_with_locality(core.id, source.clone(), false);
    app.register_classic_lobby_resource(&core, 100);
    assert!(app.network_lobby.as_ref().unwrap().resource_rows[&core.id].save_possible);

    app.request_lobby_resource_save(core.id, false)
        .expect("copy new target");
    assert_eq!(fs::read(&target).unwrap(), b"first");
    let success = app.message_dialogs.last().unwrap();
    assert_eq!(success.state.caption(), "Resource saved");
    assert_eq!(success.state.icon(), MessageDialogIcon::Standard(13));
    assert!(success.state.message().ends_with("Downloaded.c4s"));
    app.finish_message_dialog(MessageDialogResult::Ok)
        .expect("close success");

    fs::write(&source, b"replacement").expect("replace source");
    fs::write(&target, b"keep").expect("existing target");
    app.request_lobby_resource_save(core.id, false)
        .expect("prompt before overwrite");
    let confirmation = app.message_dialogs.last().unwrap();
    assert_eq!(confirmation.state.caption(), "Save resource");
    assert_eq!(confirmation.state.buttons(), MessageDialogButtons::YES_NO);
    assert_eq!(confirmation.state.icon(), MessageDialogIcon::CONFIRM);
    assert!(matches!(
        &confirmation.continuation,
        MessageDialogContinuation::LobbyResourceOverwrite { resource_id: 47 }
    ));
    app.finish_message_dialog(MessageDialogResult::No)
        .expect("decline overwrite");
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    assert!(app.message_dialogs.is_empty());

    app.request_lobby_resource_save(core.id, false)
        .expect("prompt for accepted overwrite");
    app.finish_message_dialog(MessageDialogResult::Yes)
        .expect("accept overwrite");
    assert_eq!(fs::read(&target).unwrap(), b"replacement");
    assert_eq!(
        app.message_dialogs.last().unwrap().state.caption(),
        "Resource saved"
    );
    app.finish_message_dialog(MessageDialogResult::Ok)
        .expect("close overwrite success");

    fs::remove_file(&source).expect("force copy failure");
    fs::remove_file(&target).expect("avoid overwrite prompt");
    app.request_lobby_resource_save(core.id, false)
        .expect("present copy failure");
    let failure = app.message_dialogs.last().unwrap();
    assert_eq!(failure.state.caption(), "Error copying file");
    assert_eq!(failure.state.message(), "Error copying file");
    assert_eq!(failure.state.icon(), MessageDialogIcon::ERROR);
}

#[test]
fn classic_lobby_client_removal_evicts_its_resource_namespace() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let remote_resource = (7 << 16) | 1;
    let host_resource = 2;
    for resource_id in [host_resource, remote_resource] {
        app.classic_host_lobby
            .as_mut()
            .unwrap()
            .resource_rows
            .insert(
                resource_id,
                LobbyResourceRow {
                    id: resource_id,
                    filename: format!("Resource{resource_id}.c4p"),
                    present_percent: 100,
                    save_possible: false,
                },
            );
        app.admission_resources.resources.insert(
            resource_id,
            AdmissionResourceState::Complete {
                path: PathBuf::from(format!("Resource{resource_id}.c4p")),
                removed: false,
                local: true,
            },
        );
        app.admission_resources.resource_cores.insert(
            resource_id,
            clonk_engine::NetworkResourceCore {
                id: resource_id,
                resource_type: clonk_network::HostResourceType::Player as u8,
                ..Default::default()
            },
        );
        app.admission_resources
            .present_percent
            .insert(resource_id, 100);
    }
    app.control_clients.replace_snapshot([
        clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            ..Default::default()
        },
        clonk_engine::ClientCoreControlData {
            client_id: 7,
            activated: true,
            ..Default::default()
        },
    ]);
    app.select_classic_lobby_sheet(LobbySheet::Resources);
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    events
        .send(NetworkEvent::DirectControl(NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: LegacyCString::default(),
                by_client: 0,
            },
        )))
        .unwrap();
    app.process_network_events()
        .expect("authoritative client removal updates resource rows");

    let lobby = app.classic_host_lobby.as_ref().unwrap();
    assert_eq!(
        lobby.resource_rows.keys().copied().collect::<Vec<_>>(),
        vec![host_resource]
    );
    assert_eq!(
        lobby
            .controller
            .resource_rows()
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![host_resource]
    );
    assert!(!app
        .admission_resources
        .resources
        .contains_key(&remote_resource));
}

#[test]
fn joined_lobby_chrome_routes_exit_and_right_tab_context() {
    fn joined_app() -> GameApp {
        let mut app = new_menu_app(640, 480);
        app.startup_view = StartupView::NetworkLobby;
        app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
        let (network, _events) = NetworkManager::test_stub_for_client_id(7);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app
    }

    fn right_caption_context_point(app: &mut GameApp) -> GuiPoint {
        let surface = app.graphics.surface();
        let lobby = app.network_lobby.as_mut().expect("joined lobby");
        let (controller, _) = lobby
            .classic_render_state(surface, app.assets.as_ref(), &app.scenario_game_options)
            .expect("build the production joined-client lobby");
        let fonts = app.assets.clonk_fonts.as_deref().expect("classic fonts");
        let layout = controller.layout(640, 480, fonts);
        GuiPoint::new(
            (layout.right_caption.x + 1) as f32,
            (layout.right_caption.y + layout.right_caption.h / 2) as f32,
        )
    }

    // MainDlg::OnRightTabContext adds Players, optional Teams, Resources
    // and Options for every participant (C4GameLobby.cpp:844-866).
    let joined_entries = GameApp::lobby_tab_context_entries(false, true);
    assert_eq!(
        joined_entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>(),
        vec![
            AppContextMenuCommand::LobbySheet(LobbySheet::Players),
            AppContextMenuCommand::LobbySheet(LobbySheet::Resources),
            AppContextMenuCommand::LobbySheet(LobbySheet::Options),
        ]
    );
    assert_eq!(
        joined_entries
            .iter()
            .map(|entry| entry.icon)
            .collect::<Vec<_>>(),
        vec![
            ContextMenuIcon::Phase(9),
            ContextMenuIcon::Phase(10),
            ContextMenuIcon::Phase(14),
        ]
    );
    let joined_team_entries = GameApp::lobby_tab_context_entries(true, true);
    assert_eq!(
        joined_team_entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>(),
        vec![
            AppContextMenuCommand::LobbySheet(LobbySheet::Players),
            AppContextMenuCommand::LobbySheet(LobbySheet::Teams),
            AppContextMenuCommand::LobbySheet(LobbySheet::Resources),
            AppContextMenuCommand::LobbySheet(LobbySheet::Options),
        ]
    );
    assert_eq!(
        joined_team_entries
            .iter()
            .map(|entry| entry.icon)
            .collect::<Vec<_>>(),
        vec![
            ContextMenuIcon::Phase(9),
            ContextMenuIcon::Phase(19),
            ContextMenuIcon::Phase(10),
            ContextMenuIcon::Phase(14),
        ]
    );

    let mut app = joined_app();

    let caption = right_caption_context_point(&mut app);
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .handle_panel_pointer_move(caption);
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .expect("right-click opens the available joined tab context");
    assert_eq!(
        app.context_menu
            .as_ref()
            .expect("joined tab context")
            .layout()
            .panels[0]
            .rows
            .len(),
        3,
        "without teams: Players, Resources and Options"
    );
    app.close_context_menu_silently();

    app.network_lobby.as_mut().expect("joined lobby").has_teams = true;
    let caption = right_caption_context_point(&mut app);
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .handle_panel_pointer_move(caption);
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .expect("right-click opens the team-aware joined tab context");
    let resource_point = {
        let layout = app
            .context_menu
            .as_ref()
            .expect("joined tab context")
            .layout();
        assert_eq!(
            layout.panels[0].rows.len(),
            4,
            "Players, Teams, Resources and Options match the native popup"
        );
        let row = &layout.panels[0].rows[2];
        GuiPoint::new(
            (row.rect.x + row.rect.w / 2) as f32,
            (row.rect.y + row.rect.h / 2) as f32,
        )
    };
    assert!(app
        .handle_context_menu_pointer_move(resource_point)
        .expect("hover Resources"));
    assert!(app
        .handle_context_menu_pointer_button(ElementState::Pressed, ContextMenuPointerButton::Left,)
        .expect("dispatch Resources"));
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .active_sheet,
        LobbySheet::Resources
    );
    assert!(app.context_menu.is_none());

    let exit = {
        let rect = app
            .network_lobby
            .as_ref()
            .and_then(|lobby| lobby.layout.as_ref())
            .expect("joined lobby layout")
            .exit_button;
        GuiPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        )
    };
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .handle_panel_pointer_move(exit);
    app.ui_sound_log.clear();
    app.handle_mouse_button(ElementState::Pressed)
        .expect("press the production joined Exit button");
    assert_eq!(app.ui_sound_log, ["ArrowHit".to_string()]);
    app.handle_mouse_button(ElementState::Released)
        .expect("release the production joined Exit button");
    assert_eq!(
        app.ui_sound_log,
        ["ArrowHit".to_string(), "Click".to_string()]
    );
    assert_eq!(app.startup_view, StartupView::MainMenu);
    assert!(app.network_lobby.is_none());
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());

    let mut escape = joined_app();
    escape.ui_sound_log.clear();
    escape
        .handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("Escape exits the production joined lobby");
    assert_eq!(escape.startup_view, StartupView::MainMenu);
    assert!(escape.network_lobby.is_none());
    assert!(escape.network.is_none());
    assert!(escape.network_mode.is_none());
    assert!(escape.ui_sound_log.is_empty(), "Escape is silent");

    let mut hotkey = joined_app();
    hotkey.keyboard_modifiers = ModifiersState::ALT;
    hotkey.ui_sound_log.clear();
    hotkey
        .handle_key(VirtualKeyCode::KeyX, ElementState::Pressed)
        .expect("Alt+X exits the production joined lobby");
    assert_eq!(hotkey.startup_view, StartupView::MainMenu);
    assert!(hotkey.network_lobby.is_none());
    assert!(hotkey.network.is_none());
    assert!(hotkey.network_mode.is_none());
    assert!(hotkey.ui_sound_log.is_empty(), "the Exit hotkey is silent");
}

#[test]
fn l102_joined_client_roster_context_reaches_mute_and_info_without_host_actions() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Client")]);

    let entries = app
        .classic_lobby_client_context_entries(0)
        .expect("remote host row is visible to the joined client");
    assert_eq!(
        entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>(),
        vec![
            AppContextMenuCommand::LobbyClientToggleMute(0),
            AppContextMenuCommand::LobbyClientInfo(0),
        ]
    );
    assert!(entries
        .iter()
        .all(|entry| entry.icon == ContextMenuIcon::None));
    let mut labels = entries
        .iter()
        .map(|entry| entry.text.clone())
        .collect::<Vec<_>>();
    for label in &mut labels {
        Markup::strip_markup(label);
    }
    assert_eq!(labels, vec!["Mute", "Info"]);

    let point = {
        let surface = app.graphics.surface();
        let lobby = app.network_lobby.as_mut().expect("joined lobby");
        let (mut controller, _) = lobby
            .classic_render_state(surface, app.assets.as_ref(), &app.scenario_game_options)
            .expect("build the production joined-client roster");
        let fonts = app.assets.clonk_fonts.as_deref().expect("classic fonts");
        let layout = controller.layout(640, 480, fonts);
        let roster = controller.roster_layout(&layout, fonts.text.line_height);
        let row = roster
            .rows
            .iter()
            .find(|row| {
                matches!(
                    controller.rows().get(row.index),
                    Some(LobbyRosterRow::Client(client)) if client.id == 0
                )
            })
            .expect("host client row layout");
        GuiPoint::new(
            (row.rect.x + row.rect.w / 2) as f32,
            (row.rect.y + row.rect.h / 2) as f32,
        )
    };
    app.network_lobby
        .as_mut()
        .expect("joined lobby")
        .handle_panel_pointer_move(point);
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .expect("right-click reaches the joined-client roster");
    assert_eq!(
        app.context_menu
            .as_ref()
            .expect("client context menu")
            .layout()
            .panels[0]
            .rows
            .len(),
        2
    );
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::KeyM, ElementState::Pressed)
        .expect("select Mute"));
    assert!(app.control_messages.is_muted(0));
    assert!(commands.take_submitted_client_updates().is_empty());
    assert!(commands.take_submitted_client_removes().is_empty());
    assert!(commands.take_submitted_votes().is_empty());

    let entries = app
        .classic_lobby_client_context_entries(0)
        .expect("muted host row remains visible");
    let mut label = entries[0].text.clone();
    Markup::strip_markup(&mut label);
    assert_eq!(label, "Unmute");
}

#[test]
fn joined_lobby_roster_routes_and_retains_classic_interactions() {
    fn row_point(app: &mut GameApp, id: LobbyRosterId) -> GuiPoint {
        let (_, roster) = app.joined_lobby_layouts().expect("joined roster layout");
        let lobby = app.network_lobby.as_ref().expect("joined lobby");
        let row = roster
            .rows
            .iter()
            .find(|layout_row| {
                lobby
                    .controller
                    .rows()
                    .get(layout_row.index)
                    .is_some_and(|row| row.id() == id)
            })
            .expect("semantic joined roster row");
        GuiPoint::new((row.rect.x + 2) as f32, (row.rect.y + 2) as f32)
    }

    fn tab_point(app: &mut GameApp, sheet: LobbySheet) -> GuiPoint {
        let layout = app
            .network_lobby
            .as_mut()
            .expect("joined lobby")
            .update_layout(640.0, 480.0)
            .clone();
        let rect = layout
            .sheet_buttons
            .iter()
            .find(|(candidate, _)| *candidate == sheet)
            .expect("joined lobby tab")
            .1;
        GuiPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        )
    }

    let _lock = env_lock().lock();
    let user_data = tempdir().expect("joined lobby user data");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app(640, 480);
    app.app_paths = Some(paths);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_max_players = 64;

    let chooser = clonk_engine::ControlPlayerInfoEntry {
        id: 31,
        name: LegacyCString::from_bytes(b"Chooser".to_vec()).unwrap(),
        team: 1,
        color: 0x0012_3456,
        original_color: 0x0065_4321,
        league_rank_symbol: 5,
        ..Default::default()
    };
    let companion = clonk_engine::ControlPlayerInfoEntry {
        id: 32,
        name: LegacyCString::from_bytes(b"Companion".to_vec()).unwrap(),
        team: 3,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED,
        ..Default::default()
    };
    let script = clonk_engine::ControlPlayerInfoEntry {
        id: 33,
        name: LegacyCString::from_bytes(b"Script".to_vec()).unwrap(),
        team: 1,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
        ..Default::default()
    };
    let foreign = clonk_engine::ControlPlayerInfoEntry {
        id: 41,
        name: LegacyCString::from_bytes(b"Foreign".to_vec()).unwrap(),
        team: 2,
        ..Default::default()
    };
    let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
    app.control_player_infos.replace_snapshot(
        50,
        [
            clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: 0,
                players: vec![foreign.clone()],
                by_client: 0,
            },
            clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: packet_flags,
                players: vec![chooser.clone(), companion.clone(), script.clone()],
                by_client: 7,
            },
        ],
    );

    let mut clients = vec![message_client(0, b"Host"), message_client(7, b"Client")];
    for id in 8..30 {
        clients.push(message_client(id, format!("Filler {id}").as_bytes()));
    }
    app.control_clients.replace_snapshot(clients.clone());

    let free_restore = clonk_engine::ControlPlayerInfoEntry {
        id: 50,
        name: LegacyCString::from_bytes(b"Free restore".to_vec()).unwrap(),
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
        color: 0x0012_3456,
        original_color: 0x0012_3456,
        ..Default::default()
    };
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host JoinData");
    snapshot.parameters.max_players = 64;
    snapshot.parameters.league_address =
        LegacyCString::from_bytes(b"https://league.example/".to_vec()).unwrap();
    snapshot.parameters.clients = clonk_network::JoinClientRegistrySnapshot {
        clients: clients.clone(),
        local_client_id: Some(7),
    };
    snapshot.parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: 50,
        clients: vec![
            clonk_network::ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![foreign.clone()],
            },
            clonk_network::ClientPlayerInfosSnapshot {
                client_id: 7,
                flags: packet_flags,
                players: vec![chooser.clone(), companion.clone(), script.clone()],
            },
        ],
    };
    snapshot.parameters.restore_player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: 50,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: vec![free_restore],
        }],
    };
    snapshot.parameters.teams = clonk_network::join_team_list_snapshot(set_control_test_metadata(
        false,
        vec![
            set_control_test_team(2, vec![41], 1),
            set_control_test_team(1, vec![31, 33], 0),
            set_control_test_team(3, vec![32], 0),
        ],
    ));
    app.pending_network_join_data = Some(clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 0,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    });
    app.network_is_league = true;
    app.sync_classic_lobby_roster();
    app.joined_lobby_layouts()
        .expect("synchronize retained joined player count");

    let lobby = app.network_lobby.as_ref().expect("joined lobby");
    assert_eq!(
        lobby.controller.players_title(),
        "&Players (4/64)",
        "free restore rows do not inflate the authoritative player count"
    );
    assert!(lobby.controller.league_mode());
    let chooser_index = lobby
        .controller
        .rows()
        .iter()
        .position(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 31))
        .expect("league chooser row");
    assert!(matches!(
        &lobby.controller.rows()[chooser_index],
        LobbyRosterRow::Player(player) if player.league_rank == Some(5)
    ));
    assert!(lobby.controller.rows().iter().any(|row| matches!(
        row,
        LobbyRosterRow::Header(LobbyHeaderRow {
            kind: LobbyRosterHeader::UnassignedSavegamePlayers,
            ..
        })
    )));
    assert!(lobby.controller.rows().iter().any(|row| matches!(
        row,
        LobbyRosterRow::Player(player) if player.id == 50 && player.client_id == -1
    )));

    let free_point = row_point(&mut app, LobbyRosterId::Player(50));
    app.handle_network_lobby_pointer_move(free_point)
        .expect("hover free savegame player");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press free savegame player");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("select free savegame player");
    let lobby = app.network_lobby.as_ref().unwrap();
    assert_eq!(
        lobby.controller.selected_roster_id(),
        Some(&LobbyRosterId::Player(50))
    );
    assert_eq!(lobby.controller.focus(), LobbyControl::Roster);

    for modifiers in [ModifiersState::empty(), ModifiersState::SHIFT] {
        app.network_lobby.as_mut().unwrap().chat_edit = LobbyChatEditView::default();
        app.joined_lobby_layouts()
            .expect("synchronize empty joined chat before Space");
        app.keyboard_modifiers = modifiers;
        // Dialog::CharIn refocuses the default edit for unprocessed
        // characters EXCEPT space, which buttons consume on key-up
        // (src/C4GuiDialogs.cpp:552-567); the focused listbox binds no
        // confirm keys, so Space stays inert on the roster.
        app.handle_key(VirtualKeyCode::Space, ElementState::Pressed)
            .expect("unbound roster Space is eaten without focus change");
        app.handle_text_input(' ')
            .expect("a space character never starts type-to-chat");
        app.handle_key(VirtualKeyCode::Space, ElementState::Released)
            .expect("release inert roster Space");
        let lobby = app.network_lobby.as_ref().unwrap();
        assert_eq!(lobby.controller.focus(), LobbyControl::Roster);
        assert_eq!(lobby.chat_edit.text, "");
        assert_eq!(
            lobby.controller.selected_roster_id(),
            Some(&LobbyRosterId::Player(50))
        );
    }
    app.keyboard_modifiers = ModifiersState::empty();

    app.network_lobby.as_mut().unwrap().chat_edit = LobbyChatEditView {
        text: "draft".into(),
        caret: 5,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    };
    app.joined_lobby_layouts()
        .expect("synchronize joined chat draft");
    app.keyboard_modifiers = ModifiersState::CONTROL;
    app.handle_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("unfocused Ctrl-C stays outside the joined chat edit");
    app.handle_key(VirtualKeyCode::KeyC, ElementState::Released)
        .expect("release unfocused joined Ctrl-C");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::Roster
    );
    app.keyboard_modifiers = ModifiersState::empty();
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("Return with roster focus is eaten like C4GUI::Dialog");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("release eaten roster Return");
    assert!(commands.take_submitted_messages().is_empty());
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::Roster
    );

    app.handle_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed)
        .expect("Left is an edit key only while the chat edit is focused");
    app.handle_key(VirtualKeyCode::ArrowLeft, ElementState::Released)
        .expect("release unfocused Left");
    let lobby = app.network_lobby.as_ref().unwrap();
    assert_eq!(lobby.chat_edit.text, "draft");
    assert_eq!(lobby.chat_edit.caret, 5);
    assert_eq!(lobby.controller.focus(), LobbyControl::Roster);

    app.network_lobby.as_mut().unwrap().chat_edit = LobbyChatEditView::default();
    app.joined_lobby_layouts()
        .expect("synchronize empty joined chat draft");
    app.handle_text_input('x')
        .expect("printable roster input refocuses and inserts into chat");
    let lobby = app.network_lobby.as_ref().unwrap();
    assert_eq!(lobby.controller.focus(), LobbyControl::ChatInput);
    assert_eq!(lobby.chat_edit.text, "x");

    assert!(
        app.joined_lobby_layouts()
            .expect("scrollable joined roster")
            .1
            .max_scroll
            > 0
    );

    let selected_point = row_point(&mut app, LobbyRosterId::Player(50));
    app.handle_network_lobby_pointer_move(selected_point)
        .expect("restore joined roster focus before refresh");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press selected joined roster row before refresh");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("restore selected joined roster row before refresh");
    let wheel_hover = row_point(
        &mut app,
        LobbyRosterId::Header(LobbyRosterHeader::UnassignedSavegamePlayers),
    );
    app.handle_network_lobby_pointer_move(wheel_hover)
        .expect("reactivate joined roster hover before wheel");
    assert!(app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
        .expect("scroll joined roster");
    assert!(
        app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_none(),
        "a joined ScrollWindow wheel releases tooltip hover ownership"
    );
    let retained_scroll = app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .roster_scroll();
    assert!(retained_scroll > 0);
    app.joined_lobby_layouts().expect("first retained frame");
    app.joined_lobby_layouts().expect("second retained frame");
    clients.push(message_client(40, b"Late filler"));
    app.control_clients.replace_snapshot(clients);
    app.sync_classic_lobby_roster();
    app.joined_lobby_layouts().expect("refreshed joined roster");
    let lobby = app.network_lobby.as_ref().unwrap();
    assert_eq!(
        lobby.controller.selected_roster_id(),
        Some(&LobbyRosterId::Player(50)),
        "row refresh retains semantic selection"
    );
    assert_eq!(lobby.controller.focus(), LobbyControl::Roster);
    assert_eq!(lobby.controller.roster_scroll(), retained_scroll);

    app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 100.0), 1.0)
        .expect("return joined roster to top");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .roster_scroll(),
        0
    );

    let last_roster_id = app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .rows()
        .last()
        .expect("nonempty joined roster")
        .id();
    app.handle_key(VirtualKeyCode::End, ElementState::Pressed)
        .expect("native joined ListBox End");
    app.handle_key(VirtualKeyCode::End, ElementState::Released)
        .expect("release joined ListBox End");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .selected_roster_id(),
        Some(&last_roster_id)
    );
    assert!(
        app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .roster_scroll()
            > 0
    );
    app.handle_key(VirtualKeyCode::Home, ElementState::Pressed)
        .expect("native joined ListBox Home");
    app.handle_key(VirtualKeyCode::Home, ElementState::Released)
        .expect("release joined ListBox Home");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .unwrap()
            .controller
            .roster_scroll(),
        0
    );
    app.handle_key(VirtualKeyCode::PageDown, ElementState::Pressed)
        .expect("select the last fully visible joined roster row");
    app.handle_key(VirtualKeyCode::PageDown, ElementState::Released)
        .expect("release first joined roster PageDown");
    let first_page_selection = app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .selected_roster_id()
        .cloned();
    app.handle_key(VirtualKeyCode::PageDown, ElementState::Pressed)
        .expect("scroll a second joined roster page");
    app.handle_key(VirtualKeyCode::PageDown, ElementState::Released)
        .expect("release second joined roster PageDown");
    let lobby = app.network_lobby.as_ref().unwrap();
    assert_ne!(
        lobby.controller.selected_roster_id().cloned(),
        first_page_selection
    );
    assert!(lobby.controller.roster_scroll() > 0);
    app.handle_key(VirtualKeyCode::Home, ElementState::Pressed)
        .expect("restore joined roster top after paging");
    app.handle_key(VirtualKeyCode::Home, ElementState::Released)
        .expect("release restored joined roster Home");

    app.network_lobby.as_mut().unwrap().chat_edit = LobbyChatEditView {
        text: "focus me".to_string(),
        caret: 0,
        cursor_visible: false,
        ..LobbyChatEditView::default()
    };
    app.joined_lobby_layouts()
        .expect("synchronize joined default-focus draft");
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
        .expect("a header row has no ListBox context, so Apps is eaten");
    let lobby = app.network_lobby.as_ref().unwrap();
    assert!(app.context_menu.is_none());
    assert_eq!(lobby.controller.focus(), LobbyControl::Roster);
    assert_eq!(lobby.chat_edit.text, "focus me");
    assert_eq!(lobby.chat_edit.caret, 0);
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Released)
        .expect("release eaten joined Apps");
    app.handle_network_lobby_pointer_move(free_point).unwrap();
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .unwrap();
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .unwrap();

    app.keyboard_modifiers = ModifiersState::SHIFT;
    app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("Shift-Tab leaves the joined roster in dialog order");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::ScenarioTab
    );
    app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
        .expect("release joined reverse traversal");
    app.keyboard_modifiers = ModifiersState::empty();

    let client_point = row_point(&mut app, LobbyRosterId::Client(7));
    app.handle_network_lobby_pointer_move(client_point)
        .expect("hover local client before cancellation checks");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press local client before cancellation checks");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("select local client before cancellation checks");
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("focus joined Add Player control"));
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::RosterAddPlayer
    );
    app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("Tab advances past the joined Add Player child");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::Exit
    );
    app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
        .expect("release joined child forward traversal");
    app.keyboard_modifiers = ModifiersState::SHIFT;
    app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("reverse traversal refocuses joined Add Player control");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::RosterAddPlayer
    );
    app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
        .expect("release joined child reverse traversal");
    app.keyboard_modifiers = ModifiersState::empty();
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("latch joined Add Player key"));
    assert!(app.definition_selector.is_none());
    app.handle_focus_lost()
        .expect("focus loss cancels retained joined key latch");
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Released)
        .expect("release canceled joined Add Player key"));
    assert!(app.definition_selector.is_none());
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("relatch joined Add Player key"));
    app.handle_gamepad_event(GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    })
    .expect("controller clear cancels retained joined key latch");
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Released)
        .expect("release controller-canceled joined Add Player key"));
    assert!(app.definition_selector.is_none());

    let chooser_point = row_point(&mut app, LobbyRosterId::Player(31));
    app.handle_network_lobby_pointer_move(chooser_point)
        .expect("hover local player");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press local player");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("select local player");
    let (_, league_roster) = app
        .joined_lobby_layouts()
        .expect("expanded joined league player layout");
    let chooser_index = app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .rows()
        .iter()
        .position(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 31))
        .unwrap();
    assert!(
        league_roster
            .rows
            .iter()
            .find(|row| row.index == chooser_index)
            .and_then(|row| row.rank)
            .is_some(),
        "expanded joined league rows reserve the native rank-symbol cell"
    );
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("focus local team control"));
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::RosterTeam
    );
    assert!(!app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Released)
        .expect("release local team focus key"));
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("open joined local team selector"));
    assert_eq!(
        app.context_menu.as_ref().unwrap().layout().panels[0]
            .rows
            .len(),
        2
    );
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select current joined team"));
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select available joined team"));
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("activate joined team selection"));
    let mut team_selected = chooser.clone();
    team_selected.team = 3;
    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: packet_flags,
            players: vec![team_selected, companion.clone(), script.clone()],
        }],
        "joined team combo submits one full packet without optimistic mutation"
    );
    app.keyboard_modifiers = ModifiersState::SHIFT;
    assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("reverse focus to roster"));
    assert_eq!(
        app.network_lobby.as_ref().unwrap().controller.focus(),
        LobbyControl::Roster
    );
    assert!(!app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Released)
        .expect("release reverse roster focus key"));
    app.keyboard_modifiers = ModifiersState::empty();
    app.pending_network_join_data
        .as_mut()
        .unwrap()
        .parameters
        .teams
        .team_distribution = 1;
    app.sync_classic_lobby_roster();
    app.submit_classic_lobby_team_selection(31, 3);
    app.move_local_classic_lobby_players_into_team(3);
    assert!(
        commands.take_player_info_updates().is_empty(),
        "a joined client cannot choose teams under host-only distribution"
    );
    app.pending_network_join_data
        .as_mut()
        .unwrap()
        .parameters
        .teams
        .team_distribution = 0;
    app.sync_classic_lobby_roster();
    let chooser_point = row_point(&mut app, LobbyRosterId::Player(31));
    app.handle_network_lobby_pointer_move(chooser_point)
        .expect("rehover local player after focus and permission checks");
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .expect("open local player context");
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::KeyR, ElementState::Pressed)
        .expect("activate joined local Remove"));
    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: packet_flags,
            players: vec![script.clone(), companion.clone()],
        }],
        "joined Remove submits the remaining full owner packet"
    );
    let chooser_point = row_point(&mut app, LobbyRosterId::Player(31));
    app.handle_network_lobby_pointer_move(chooser_point)
        .expect("rehover local player for New Color");
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .expect("reopen local player context");
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("activate New Color"));
    let mut recolored = chooser.clone();
    recolored.color = recolored.original_color;
    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: packet_flags,
            players: vec![recolored, companion.clone(), script.clone()],
        }]
    );
    assert!(
        app.classic_lobby_player_context_entries(41)
            .expect("foreign joined roster player")
            .1
            .is_empty(),
        "joined clients cannot mutate a foreign player's context"
    );

    let free_point = row_point(&mut app, LobbyRosterId::Player(50));
    app.handle_network_lobby_pointer_move(free_point)
        .expect("rehover free savegame player");
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .expect("open takeover context");
    let root = app.context_menu.as_ref().unwrap().layout().panels[0].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((root.x + 1) as f32, (root.y + 1) as f32))
        .expect("open takeover submenu");
    assert_eq!(
        app.context_menu.as_ref().unwrap().layout().panels[1]
            .rows
            .len(),
        1
    );
    let child = app.context_menu.as_ref().unwrap().layout().panels[1].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((child.x + 1) as f32, (child.y + 1) as f32))
        .expect("select takeover player");
    assert!(app
        .handle_context_menu_pointer_button(ElementState::Pressed, ContextMenuPointerButton::Left,)
        .expect("activate takeover player"));
    let mut associated = chooser.clone();
    associated.savegame_player = 50;
    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: packet_flags,
            players: vec![associated, companion.clone(), script.clone()],
        }]
    );
    assert_eq!(
        app.control_player_infos
            .client_update_request(7)
            .unwrap()
            .players[0]
            .savegame_player,
        0,
        "takeover waits for the authoritative echo"
    );
    assert!(app
        .handle_context_menu_pointer_button(ElementState::Released, ContextMenuPointerButton::Left,)
        .expect("consume takeover activation release"));

    let teams_tab = tab_point(&mut app, LobbySheet::Teams);
    app.handle_network_lobby_pointer_move(teams_tab)
        .expect("hover Teams tab");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press Teams tab");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("activate Teams tab");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().active_sheet,
        LobbySheet::Teams
    );
    for click in 0..2 {
        let team_point = row_point(&mut app, LobbyRosterId::Header(LobbyRosterHeader::Team(2)));
        let (team_layout, team_roster) = app.joined_lobby_layouts().unwrap();
        app.handle_network_lobby_pointer_move(team_point)
            .expect("hover target team");
        app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
            .expect("press target team");
        assert_eq!(
            app.network_lobby
                .as_ref()
                .unwrap()
                .controller
                .selected_roster_id(),
            Some(&LobbyRosterId::Header(LobbyRosterHeader::Team(2)))
        );
        assert_eq!(
            app.network_lobby
                .as_ref()
                .unwrap()
                .controller
                .accepted_roster_click_id(team_point, &team_layout, &team_roster),
            Some(LobbyRosterId::Header(LobbyRosterHeader::Team(2)))
        );
        app.handle_network_lobby_pointer_button(ElementState::Released, false)
            .expect("release target team");
        assert_eq!(
            app.network_lobby
                .as_ref()
                .unwrap()
                .last_roster_click
                .as_ref()
                .map(|(id, _)| id),
            (click == 0).then_some(&LobbyRosterId::Header(LobbyRosterHeader::Team(2)))
        );
    }
    let mut moved_chooser = chooser.clone();
    moved_chooser.team = 2;
    let mut moved_companion = companion.clone();
    moved_companion.team = 2;
    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: packet_flags,
            players: vec![moved_chooser, moved_companion, script.clone()],
        }],
        "joined team double-click mutates every local User exactly once"
    );

    let players_tab = tab_point(&mut app, LobbySheet::Players);
    app.handle_network_lobby_pointer_move(players_tab)
        .expect("hover Players tab");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press Players tab");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("activate Players tab");
    let chooser_point = row_point(&mut app, LobbyRosterId::Player(31));
    app.handle_network_lobby_pointer_move(chooser_point)
        .expect("hover local player before authoritative reorder");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press local player before authoritative reorder");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("select local player before authoritative reorder");
    let chooser_index_before = app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .rows()
        .iter()
        .position(|row| row.id() == LobbyRosterId::Player(31))
        .unwrap();
    app.control_player_infos.replace_snapshot(
        50,
        [
            clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: 0,
                players: vec![foreign.clone()],
                by_client: 0,
            },
            clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: packet_flags,
                players: vec![companion.clone(), chooser.clone(), script.clone()],
                by_client: 7,
            },
        ],
    );
    app.sync_classic_lobby_roster();
    let lobby = app.network_lobby.as_ref().unwrap();
    let chooser_index_after = lobby
        .controller
        .rows()
        .iter()
        .position(|row| row.id() == LobbyRosterId::Player(31))
        .unwrap();
    assert_ne!(chooser_index_after, chooser_index_before);
    assert_eq!(
        lobby.controller.selected_roster_id(),
        Some(&LobbyRosterId::Player(31)),
        "authoritative row reordering retains semantic joined selection"
    );
    assert_eq!(lobby.controller.focus(), LobbyControl::Roster);
    app.control_player_infos.replace_snapshot(
        50,
        [
            clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: 0,
                players: vec![foreign],
                by_client: 0,
            },
            clonk_engine::PlayerInfoControlData {
                client_id: 7,
                flags: packet_flags,
                players: vec![chooser, companion, script],
                by_client: 7,
            },
        ],
    );
    app.sync_classic_lobby_roster();
    let (_, roster) = app.joined_lobby_layouts().expect("Players roster layout");
    let lobby = app.network_lobby.as_ref().unwrap();
    let add_player = roster
        .rows
        .iter()
        .find(|layout_row| {
            matches!(
                lobby.controller.rows().get(layout_row.index),
                Some(LobbyRosterRow::Client(client)) if client.id == 7
            )
        })
        .and_then(|row| row.add_player)
        .expect("local Add Player button");
    let add_point = GuiPoint::new(
        (add_player.x + add_player.w / 2) as f32,
        (add_player.y + add_player.h / 2) as f32,
    );
    app.handle_network_lobby_pointer_move(add_point)
        .expect("hover local Add Player");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press local Add Player");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("open local Add Player selector");
    assert!(app.definition_selector.is_some());
    assert_eq!(
        app.pending_lobby_player_selection
            .as_ref()
            .expect("pending joined player selection")
            .client_id,
        7
    );
}

#[test]
fn joined_roster_double_click_is_roster_scoped() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_max_players = 8;

    let chooser = set_control_test_player(31, 1, 0);
    let companion = set_control_test_player(32, 3, clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED);
    let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
    let clients = vec![message_client(0, b"Host"), message_client(7, b"Client")];
    app.control_player_infos.replace_snapshot(
        40,
        [clonk_engine::PlayerInfoControlData {
            client_id: 7,
            flags: packet_flags,
            players: vec![chooser.clone(), companion.clone()],
            by_client: 7,
        }],
    );
    app.control_clients.replace_snapshot(clients.clone());

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host JoinData");
    snapshot.parameters.max_players = 8;
    snapshot.parameters.clients = clonk_network::JoinClientRegistrySnapshot {
        clients,
        local_client_id: Some(7),
    };
    snapshot.parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: 32,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 7,
            flags: packet_flags,
            players: vec![chooser.clone(), companion.clone()],
        }],
    };
    snapshot.parameters.teams = clonk_network::join_team_list_snapshot(set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![31], 0),
            set_control_test_team(2, vec![], 0),
            set_control_test_team(3, vec![32], 0),
        ],
    ));
    app.pending_network_join_data = Some(clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 0,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    });
    app.sync_classic_lobby_roster();

    let teams_tab = {
        let layout = app
            .network_lobby
            .as_mut()
            .expect("joined lobby")
            .update_layout(640.0, 480.0)
            .clone();
        let rect = layout
            .sheet_buttons
            .iter()
            .find(|(sheet, _)| *sheet == LobbySheet::Teams)
            .expect("joined Teams tab")
            .1;
        GuiPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        )
    };
    app.handle_network_lobby_pointer_move(teams_tab)
        .expect("hover Teams tab");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press Teams tab");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("activate Teams tab");
    assert_eq!(
        app.network_lobby.as_ref().unwrap().active_sheet,
        LobbySheet::Teams
    );

    let header_point = |app: &mut GameApp, team_id: i32| {
        let (_, roster) = app.joined_lobby_layouts().expect("joined roster layout");
        let lobby = app.network_lobby.as_ref().expect("joined lobby");
        let row = roster
            .rows
            .iter()
            .find(|layout_row| {
                matches!(
                    lobby.controller.rows().get(layout_row.index),
                    Some(LobbyRosterRow::Header(LobbyHeaderRow {
                        kind: LobbyRosterHeader::Team(id),
                        ..
                    })) if *id == team_id
                )
            })
            .expect("joined team header row");
        GuiPoint::new((row.rect.x + 2) as f32, (row.rect.y + 2) as f32)
    };

    // Two completed clicks on DIFFERENT team headers inside the 400 ms
    // window stay single clicks: the synthesized LeftDouble is scoped to
    // the retained semantic row, exactly like the persistent host path.
    let other_point = header_point(&mut app, 3);
    app.handle_network_lobby_pointer_move(other_point)
        .expect("hover other team header");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press other team header");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release other team header");
    let target_point = header_point(&mut app, 2);
    app.handle_network_lobby_pointer_move(target_point)
        .expect("hover target team header");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press target team header");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release target team header");
    assert!(
        commands.take_player_info_updates().is_empty(),
        "fast clicks across two roster rows never classify as a double click"
    );

    // A second completed click on the same header fires one bulk move.
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press target team header again");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release target team header again");
    let mut moved_chooser = chooser.clone();
    moved_chooser.team = 2;
    let mut moved_companion = companion.clone();
    moved_companion.team = 2;
    assert_eq!(
        commands.take_player_info_updates(),
        vec![clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: packet_flags,
            players: vec![moved_chooser, moved_companion],
        }],
        "the roster-scoped double click clones one full local packet"
    );

    // A press-classified LeftDouble (the SDL/X11 global press clock;
    // C4FullScreen.cpp:327-350) reaches the hovered team header directly,
    // and its release never double-fires.
    app.handle_network_lobby_pointer_button(ElementState::Pressed, true)
        .expect("press-classified LeftDouble on target team header");
    assert_eq!(
        commands.take_player_info_updates().len(),
        1,
        "LeftDouble runs the hovered header action exactly once"
    );
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("release after the press-classified LeftDouble");
    assert!(
        commands.take_player_info_updates().is_empty(),
        "the release after a LeftDouble neither activates nor re-fires"
    );
}

#[test]
fn unstaged_host_retained_roster_routes_script_player_add() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    app.control_player_infos.replace_snapshot(
        0,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: Vec::new(),
            by_client: 0,
        }],
    );
    let mut metadata = set_control_test_metadata(false, Vec::new());
    metadata.max_script_players = 1;
    metadata.script_player_names =
        LegacyCString::from_bytes(b"Bot".to_vec()).expect("valid script player name");
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.sync_classic_lobby_roster();

    let (_, roster) = app
        .joined_lobby_layouts()
        .expect("unstaged host roster layout");
    let lobby = app.network_lobby.as_ref().expect("unstaged host lobby");
    let add = roster
        .rows
        .iter()
        .find(|layout_row| {
            matches!(
                lobby.controller.rows().get(layout_row.index),
                Some(LobbyRosterRow::Header(LobbyHeaderRow {
                    kind: LobbyRosterHeader::ScriptPlayers,
                    can_add_player: true,
                    ..
                }))
            )
        })
        .and_then(|row| row.add_player)
        .expect("host Script Players Add button");
    let point = GuiPoint::new((add.x + add.w / 2) as f32, (add.y + add.h / 2) as f32);
    app.handle_network_lobby_pointer_move(point)
        .expect("hover retained Script Players Add button");
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .expect("press retained Script Players Add button");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .expect("activate retained Script Players Add button");

    let requests = commands.take_player_info_updates();
    let [request] = requests.as_slice() else {
        panic!("expected one retained-host script request, got {requests:?}");
    };
    assert_eq!(request.client_id, 0);
    assert_eq!(
        request.flags,
        clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
    );
    let [player] = request.players.as_slice() else {
        panic!("expected one retained-host script player");
    };
    assert_eq!(player.name.as_bytes(), b"Bot");
    assert_eq!(player.player_type, clonk_engine::PLAYER_INFO_TYPE_SCRIPT);
    assert_eq!(player.original_color, player.color);
}

fn joined_option_strip_app() -> GameApp {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.sync_network_lobby_game_option_state();
    app
}

fn joined_option_center(app: &GameApp, button: GameOptionButton) -> PhysicalPosition<f64> {
    let rect = app
        .scenario_game_options
        .layout()
        .rect(button)
        .expect("button present in the joined strip");
    PhysicalPosition::new(
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    )
}

fn joined_option_point(app: &GameApp, button: GameOptionButton) -> GuiPoint {
    let rect = app
        .scenario_game_options
        .layout()
        .rect(button)
        .expect("button present in the joined strip");
    GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
}

fn joined_option_controller_focus(app: &mut GameApp) -> LobbyControl {
    let lobby = app.network_lobby.as_mut().expect("joined lobby");
    lobby.sync_classic_controller();
    lobby.controller.focus()
}

#[test]
fn joined_lobby_game_option_strip_routes_input() {
    let mut app = joined_option_strip_app();

    // C4GameLobby.cpp:214 builds the client strip (fNetwork, !fHost,
    // fLobby): League and Fair Crew stay locked while Record is live.
    assert_eq!(
        app.scenario_game_options.context(),
        GameOptionContext::LobbyClient
    );
    assert!(
        !app.scenario_game_options
            .view(GameOptionButton::League)
            .expect("League is visible")
            .enabled
    );
    assert!(
        !app.scenario_game_options
            .view(GameOptionButton::FairCrew)
            .expect("Fair Crew is visible")
            .enabled
    );
    assert!(
        app.scenario_game_options
            .view(GameOptionButton::Record)
            .expect("Record is visible")
            .enabled
    );

    // Pointer: an enabled joined control presses with the native sounds
    // and toggles Config.General.Record on release. Buttons deliberately
    // keep the chat focus on mouse clicks.
    app.handle_cursor_moved(joined_option_center(&app, GameOptionButton::Record))
        .expect("hover Record");
    assert_eq!(
        app.scenario_game_options.hovered_button(),
        Some(GameOptionButton::Record),
        "the retained strip tracks controller-routed hover for its tooltip"
    );
    app.ui_sound_log.clear();
    app.handle_mouse_button(ElementState::Pressed)
        .expect("press Record");
    assert_eq!(app.ui_sound_log, ["ArrowHit".to_string()]);
    app.handle_mouse_button(ElementState::Released)
        .expect("release Record");
    assert_eq!(
        app.ui_sound_log,
        ["ArrowHit".to_string(), "Click".to_string()]
    );
    assert!(app.scenario_game_options.values().record);
    assert!(app.startup_view_flags.record);
    assert_eq!(app.scenario_game_options.focused_button(), None);
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::ChatInput
    );

    // Pointer: host-only/locked controls stay inert and visually exact.
    // Each press resets the native double-click clock so the classic
    // LeftDouble path (which never presses buttons) stays out of the way.
    let fair_crew_before = app.scenario_game_options.values().fair_crew;
    for locked in [GameOptionButton::League, GameOptionButton::FairCrew] {
        app.handle_cursor_moved(joined_option_center(&app, locked))
            .expect("hover a locked joined control");
        app.ui_sound_log.clear();
        app.last_application_left_press = None;
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press a locked joined control");
        app.handle_mouse_button(ElementState::Released)
            .expect("release a locked joined control");
        assert!(app.ui_sound_log.is_empty(), "{locked:?} is silent");
    }
    assert!(!app.scenario_game_options.values().lobby_is_league);
    assert_eq!(
        app.scenario_game_options.values().fair_crew,
        fair_crew_before
    );
    assert_eq!(
        app.scenario_game_options
            .view(GameOptionButton::League)
            .expect("League stays visible")
            .icon,
        clonk_frontend::game_option_buttons::GameOptionIcon::LeagueOff
    );
    assert_eq!(
        app.scenario_game_options
            .view(GameOptionButton::FairCrew)
            .expect("Fair Crew stays visible")
            .icon,
        if fair_crew_before {
            clonk_frontend::game_option_buttons::GameOptionIcon::FairCrewGray
        } else {
            clonk_frontend::game_option_buttons::GameOptionIcon::NormalCrewGray
        }
    );

    // A press that drags off the enabled button pops the visual with the
    // native ArrowHit and releases without an activation.
    app.handle_cursor_moved(joined_option_center(&app, GameOptionButton::Record))
        .expect("re-hover Record");
    app.ui_sound_log.clear();
    app.last_application_left_press = None;
    app.handle_mouse_button(ElementState::Pressed)
        .expect("hold Record");
    app.handle_cursor_moved(joined_option_center(&app, GameOptionButton::FairCrew))
        .expect("drag off the held Record");
    app.handle_mouse_button(ElementState::Released)
        .expect("release outside the held Record");
    assert_eq!(
        app.ui_sound_log,
        ["ArrowHit".to_string(), "ArrowHit".to_string()]
    );
    assert!(
        app.scenario_game_options.values().record,
        "an aborted click keeps the Record preference"
    );

    // Keyboard: Tab walks the C4GUI dialog focus order, in which every
    // strip button is a stop (Control::IsFocusElement holds while
    // disabled). Space activates only enabled buttons; an unhandled key
    // on a locked stop falls back to the chat default per
    // Dialog::KeyFocusDefault.
    let tab = |app: &mut GameApp| {
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Tab down");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("Tab up");
    };
    let mut guard = 0;
    while joined_option_controller_focus(&mut app)
        != LobbyControl::GameOption(GameOptionButton::League)
    {
        tab(&mut app);
        guard += 1;
        assert!(guard < 16, "the dialog focus cycle reaches the strip");
    }
    assert_eq!(
        app.scenario_game_options.focused_button(),
        Some(GameOptionButton::League)
    );
    app.ui_sound_log.clear();
    app.handle_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("Space on the locked League");
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::ChatInput,
        "KeyFocusDefault returns an unhandled option key to the chat edit"
    );
    assert_eq!(app.scenario_game_options.focused_button(), None);
    assert!(app.ui_sound_log.is_empty());
    app.handle_key(VirtualKeyCode::Space, ElementState::Released)
        .expect("Space release after the focus fallback");
    let mut guard = 0;
    while joined_option_controller_focus(&mut app)
        != LobbyControl::GameOption(GameOptionButton::League)
    {
        tab(&mut app);
        guard += 1;
        assert!(guard < 16, "the dialog focus cycle reaches the strip again");
    }
    tab(&mut app);
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::GameOption(GameOptionButton::FairCrew)
    );
    tab(&mut app);
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::GameOption(GameOptionButton::Record)
    );
    assert_eq!(
        app.scenario_game_options.focused_button(),
        Some(GameOptionButton::Record)
    );
    app.ui_sound_log.clear();
    app.handle_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("Space press on Record");
    app.handle_key(VirtualKeyCode::Space, ElementState::Released)
        .expect("Space release on Record");
    assert_eq!(
        app.ui_sound_log,
        ["ArrowHit".to_string(), "Click".to_string()]
    );
    assert!(!app.scenario_game_options.values().record);

    // A typed character on a focused option button refocuses the chat
    // edit and inserts (Dialog::KeyFocusDefault with CharIn).
    app.handle_text_input('y')
        .expect("char while strip focused");
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::ChatInput
    );
    assert_eq!(app.scenario_game_options.focused_button(), None);
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .chat_edit
            .text,
        "y"
    );

    // Alt hotkeys reach enabled controls silently and skip locked ones.
    app.keyboard_modifiers = ModifiersState::ALT;
    app.ui_sound_log.clear();
    app.handle_key(VirtualKeyCode::KeyR, ElementState::Pressed)
        .expect("Alt+R toggles Record");
    app.handle_key(VirtualKeyCode::KeyR, ElementState::Released)
        .expect("Alt+R release");
    assert!(app.scenario_game_options.values().record);
    assert!(app.ui_sound_log.is_empty(), "dialog hotkeys are silent");
    app.handle_key(VirtualKeyCode::KeyL, ElementState::Pressed)
        .expect("Alt+L stays inert");
    app.handle_key(VirtualKeyCode::KeyL, ElementState::Released)
        .expect("Alt+L release");
    assert!(!app.scenario_game_options.values().lobby_is_league);
    app.keyboard_modifiers = ModifiersState::empty();

    // Touch mirrors the pointer path.
    let record_point = joined_option_point(&app, GameOptionButton::Record);
    app.ui_sound_log.clear();
    app.last_application_left_press = None;
    app.handle_touch(TouchPhase::Started, record_point)
        .expect("touch Record");
    app.handle_touch(TouchPhase::Ended, record_point)
        .expect("lift off Record");
    assert_eq!(
        app.ui_sound_log,
        ["ArrowHit".to_string(), "Click".to_string()]
    );
    assert!(!app.scenario_game_options.values().record);
    let league_point = joined_option_point(&app, GameOptionButton::League);
    app.ui_sound_log.clear();
    app.last_application_left_press = None;
    app.handle_touch(TouchPhase::Started, league_point)
        .expect("touch the locked League");
    app.handle_touch(TouchPhase::Ended, league_point)
        .expect("lift off the locked League");
    assert!(app.ui_sound_log.is_empty());

    // Gamepad: Left/Right traverse the strip stops, Select activates the
    // focused button, and Select on a locked stop falls back to the chat
    // default.
    let mut guard = 0;
    while joined_option_controller_focus(&mut app)
        != LobbyControl::GameOption(GameOptionButton::League)
    {
        tab(&mut app);
        guard += 1;
        assert!(guard < 16, "the dialog focus cycle reaches the strip");
    }
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .expect("gamepad Right advances the strip focus");
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::GameOption(GameOptionButton::FairCrew)
    );
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .expect("gamepad Right reaches Record");
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::GameOption(GameOptionButton::Record)
    );
    app.ui_sound_log.clear();
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Pressed,
    )
    .expect("gamepad Select holds Record");
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Released,
    )
    .expect("gamepad Select clicks Record");
    assert_eq!(
        app.ui_sound_log,
        ["ArrowHit".to_string(), "Click".to_string()]
    );
    assert!(app.scenario_game_options.values().record);
    let mut guard = 0;
    while joined_option_controller_focus(&mut app)
        != LobbyControl::GameOption(GameOptionButton::League)
    {
        tab(&mut app);
        guard += 1;
        assert!(guard < 16, "the dialog focus cycle reaches League");
    }
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Pressed,
    )
    .expect("gamepad Select on a locked stop");
    assert_eq!(
        joined_option_controller_focus(&mut app),
        LobbyControl::ChatInput,
        "Select on a locked option falls back to the chat default"
    );

    // A league round grays Record exactly like the host rules; the
    // joined control then rejects pointer and hotkey input.
    app.network_is_league = true;
    app.sync_network_lobby_game_option_state();
    assert!(app.scenario_game_options.values().lobby_is_league);
    assert!(
        !app.scenario_game_options
            .view(GameOptionButton::Record)
            .expect("Record stays visible")
            .enabled
    );
    assert_eq!(
        app.scenario_game_options
            .view(GameOptionButton::Record)
            .expect("Record stays visible")
            .icon,
        clonk_frontend::game_option_buttons::GameOptionIcon::RecordOn
    );
    let record_before = app.scenario_game_options.values().record;
    app.handle_cursor_moved(joined_option_center(&app, GameOptionButton::Record))
        .expect("hover the league-locked Record");
    app.ui_sound_log.clear();
    app.last_application_left_press = None;
    app.handle_mouse_button(ElementState::Pressed)
        .expect("press the league-locked Record");
    app.handle_mouse_button(ElementState::Released)
        .expect("release the league-locked Record");
    app.keyboard_modifiers = ModifiersState::ALT;
    app.handle_key(VirtualKeyCode::KeyR, ElementState::Pressed)
        .expect("Alt+R on the league-locked Record");
    app.handle_key(VirtualKeyCode::KeyR, ElementState::Released)
        .expect("Alt+R release on the league-locked Record");
    app.keyboard_modifiers = ModifiersState::empty();
    assert_eq!(app.scenario_game_options.values().record, record_before);
    assert!(app.ui_sound_log.is_empty());
}

#[test]
fn network_lobby_game_option_state_matches_role_and_render_focus() {
    // The retained strip and joined controller keep the recursive focus
    // mirrored, so ClassicGameLobby::render's focus/context checks hold
    // on the projected render state.
    let mut app = joined_option_strip_app();
    let tab = |app: &mut GameApp| {
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Tab down");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("Tab up");
    };
    let mut seen = Vec::new();
    loop {
        let focus = joined_option_controller_focus(&mut app);
        if focus == LobbyControl::GameOption(GameOptionButton::League) {
            break;
        }
        seen.push(focus);
        tab(&mut app);
        assert!(
            seen.len() < 16,
            "focus cycle never reaches the strip: {seen:?}"
        );
    }
    let surface = app.graphics.surface();
    let (controller, options) = app
        .network_lobby
        .as_mut()
        .expect("joined lobby")
        .classic_render_state(surface, app.assets.as_ref(), &app.scenario_game_options)
        .expect("build the joined render state");
    assert_eq!(
        controller.focus(),
        LobbyControl::GameOption(GameOptionButton::League)
    );
    assert_eq!(options.focused_button(), Some(GameOptionButton::League));
    assert_eq!(options.context(), GameOptionContext::LobbyClient);

    // A host-role generic lobby retains the LobbyHost context so the
    // render-time context checks and native enable rules hold.
    let mut host = new_menu_app(640, 480);
    host.startup_view = StartupView::NetworkLobby;
    host.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    host.sync_network_lobby_game_option_state();
    assert_eq!(
        host.scenario_game_options.context(),
        GameOptionContext::LobbyHost
    );
    assert!(
        !host
            .scenario_game_options
            .view(GameOptionButton::League)
            .expect("host League is visible")
            .enabled,
        "the lobby locks the League toggle for the host too"
    );

    // The exact classic host lobby owns the strip: the generic sync must
    // not clobber it.
    let mut classic = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut classic);
    classic.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    let before = classic.scenario_game_options.values().clone();
    classic.network_is_league = true;
    classic.sync_network_lobby_game_option_state();
    assert_eq!(*classic.scenario_game_options.values(), before);
}

// C4Network2ClientDlg is constructed from an id and resolves the client in
// UpdateText, so an id that no longer resolves opens on
// IDS_NET_CLIENT_INFO_UNKNOWNID instead of doing nothing
// (src/C4Network2Dialogs.cpp:42-59).
#[test]
fn client_info_dialog_shows_unknown_id_and_host_unacknowledged_marker() {
    let mut app = new_real_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Client")]);

    assert!(
        app.open_classic_lobby_client_info(42)
            .expect("a stale client id still opens the native dialog"),
        "C++ never refuses to construct C4Network2ClientDlg"
    );
    let info = app.runtime_client_list.as_ref().expect("info dialog");
    assert_eq!(info.info_client_id(), Some(42));
    assert_eq!(info.info_lines(), ["Unknown client ID #42.".to_string()]);

    // A known id still resolves, and a joined client is never the network
    // host, so it cannot show the acknowledgement marker.
    assert!(app
        .open_classic_lobby_client_info(0)
        .expect("known client id opens"));
    let info = app.runtime_client_list.as_ref().expect("info dialog");
    assert_eq!(info.info_client_id(), Some(0));
    assert!(
        info.info_lines()
            .iter()
            .all(|line| !line.contains("(!ack)")),
        "only Game.Network.isHost() adds the marker (src/C4Network2Dialogs.cpp:71)"
    );

    // The host's own row has no C4Network2Client, so it never carries the
    // marker either (src/C4Network2Dialogs.cpp:62).
    let mut host = new_real_menu_app(640, 480);
    host.startup_view = StartupView::NetworkLobby;
    let (host_network, _host_events) = NetworkManager::test_stub_for_client_id(0);
    host.network = Some(host_network);
    host.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    host.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    assert!(host
        .open_classic_lobby_client_info(0)
        .expect("host opens its own client information"));
    assert!(host
        .runtime_client_list
        .as_ref()
        .expect("host info dialog")
        .info_lines()
        .iter()
        .all(|line| !line.contains("(!ack)")));
}

#[test]
fn l102_lobby_client_info_renders_modally_and_escape_release_cannot_exit_lobby() {
    let mut app = new_real_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.sync_network_lobby_game_option_state();
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Client")]);

    let mut base = vec![0_u8; 640 * 480 * 4];
    app.render(&mut base).expect("render joined-client lobby");
    assert!(app
        .open_classic_lobby_client_info(0)
        .expect("open client information"));
    let info = app.runtime_client_list.as_ref().expect("info dialog");
    assert!(info.is_info_only());
    assert_eq!(info.info_client_id(), Some(0));

    let mut with_info = vec![0_u8; 640 * 480 * 4];
    app.render(&mut with_info)
        .expect("render client information over joined lobby");
    assert_ne!(with_info, base);

    app.handle_text_input('x')
        .expect("covered lobby chat cannot receive text");
    assert!(app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .chat_edit
        .text
        .is_empty());
    app.handle_other_mouse_button(ElementState::Pressed)
        .expect("middle press belongs to client information");
    app.handle_other_mouse_button(ElementState::Released)
        .expect("middle release belongs to client information");
    assert!(app.runtime_client_list.is_some());
    assert!(app
        .network_lobby
        .as_ref()
        .expect("joined lobby")
        .chat_edit
        .text
        .is_empty());
    app.running_pointer_position = Some(GuiPoint::new(0.0, 0.0));
    app.handle_mouse_button(ElementState::Pressed)
        .expect("outside press belongs to the modal info dialog");
    app.handle_mouse_button(ElementState::Released)
        .expect("outside release belongs to the modal info dialog");
    assert!(app.runtime_client_list.is_some());
    assert!(app.context_menu.is_none());

    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("Return is swallowed by client information");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("Return release remains owned by client information");
    assert!(app.runtime_client_list.is_some());
    assert!(commands.take_submitted_client_updates().is_empty());
    assert!(commands.take_submitted_client_removes().is_empty());

    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("Escape closes client information");
    assert!(app.runtime_client_list.is_none());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("an auto-repeated closing Escape remains owned");
    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    assert!(app.network_lobby.is_some());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
        .expect("closing Escape release stays latched");
    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    assert!(app.network_lobby.is_some());

    assert!(app
        .open_classic_lobby_client_info(0)
        .expect("reopen client information for gamepad ownership"));
    let slot = GamepadSlot::new(0);
    app.process_gamepad_event_batch([
        GamepadEvent::GuiButton {
            slot,
            class: GuiButtonClass::High,
            state: ElementState::Pressed,
        },
        GamepadEvent::Action {
            slot,
            action: GamepadActionType::Cancel,
            state: ElementState::Pressed,
        },
    ])
    .expect("the modal owns the complete physical cancel cluster");
    assert!(app.runtime_client_list.is_none());
    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    assert!(app.network_lobby.is_some());
}

#[test]
fn classic_lobby_add_player_picker_publishes_relative_file_and_projects_echo() {
    let _lock = env_lock().lock();
    let install = tempdir().expect("install root");
    let user_data = tempdir().expect("user data");
    install_global_gui_and_loader_test_root(install.path());
    let player_dir = install.path().join("Players");
    fs::create_dir_all(&player_dir).expect("create player directory");
    let player_path = player_dir.join("Alice.c4p");
    let mut group = clonk_resources::MutableGroup::new("Alice.c4p");
    group
        .add_file_with_metadata(
            "Player.txt",
            b"[Player]\nName=Alice\n[Preferences]\nColorDw=1193046\n".to_vec(),
            1,
            false,
        )
        .expect("add player core");
    fs::write(&player_path, group.pack().expect("pack player")).expect("write player");
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = AppPaths::discover().expect("discover temporary install");
    paths.ensure_user_dirs().expect("create user directories");
    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=US\nPlayerPath=Players\n[Network]\nLocalName=Host\n",
    )
    .expect("configure player selector");
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_test_classic_host_lobby(&mut app);
    app.control_clients
        .replace_snapshot([clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            name: LegacyCString::from_bytes(b"Host".to_vec()).unwrap(),
            ..Default::default()
        }]);
    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::AddPlayerRequested {
        client_id: 0,
    }])
    .expect("C4PlayerSelDlg opens");
    let selected = app
        .definition_selector
        .as_ref()
        .expect("player selector")
        .rows()[0]
        .full_path()
        .to_string();
    assert_eq!(
        app.definition_selector.as_ref().unwrap().root_path(),
        player_dir.to_string_lossy(),
        "C4PlayerSelDlg snapshots the configured PlayerPath root"
    );
    assert_eq!(
        app.pending_lobby_player_selection
            .as_ref()
            .unwrap()
            .candidates[&selected]
            .wire_filename,
        "Players/Alice.c4p",
        "the physical picker path must not leak onto the wire"
    );
    assert_eq!(
        app.pending_lobby_player_selection
            .as_ref()
            .unwrap()
            .candidates[&selected]
            .source_path,
        player_path,
        "the exact physical path must survive the string-keyed selector"
    );

    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=US\nPlayerPath=ChangedAfterOpen\n",
    )
    .expect("mutate PlayerPath after selector construction");
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::RefreshRequested,
    ])
    .expect("F5 refreshes the frozen PlayerPath roots");
    assert_eq!(
        app.definition_selector.as_ref().unwrap().rows()[0].full_path(),
        selected,
        "F5 must not re-read PlayerPath"
    );

    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Players/Alice.c4p".to_vec()).unwrap(),
        ..Default::default()
    };
    let (direct_ready, direct_wait) = std::sync::mpsc::channel();
    let command_observer = thread::spawn(move || {
        commands.complete_runtime_host_join(resource, event_tx, direct_ready)
    });
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Accepted(vec![selected]),
    ])
    .expect("selected player is submitted");
    direct_wait
        .recv_timeout(Duration::from_secs(1))
        .expect("authoritative PlayerInfo is broadcast");
    app.process_network_events()
        .expect("authoritative player echo is applied");
    drop(app.network.take());
    let (_, _, infos, _) = command_observer.join().expect("command observer");
    assert_eq!(infos.len(), 1);
    assert!(app
        .classic_host_lobby
        .as_ref()
        .unwrap()
        .controller
        .rows()
        .iter()
        .any(|row| matches!(row, LobbyRosterRow::Player(player) if player.name == "Alice")));
}

#[test]
fn classic_lobby_team_combo_filters_teams_and_submits_the_full_player_packet() {
    let mut app = new_real_menu_app(640, 480);
    let (chooser, companion) = install_test_classic_host_team_lobby(&mut app);
    let team_rect = test_lobby_team_rect(&mut app);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
        player_id: 7,
    }])
    .expect("team combo opens without crossing a parity fence");

    let menu = app.context_menu.as_ref().expect("open team context menu");
    let panel = &menu.layout().panels[0];
    assert_eq!(panel.bounds.x, team_rect.x);
    assert_eq!(panel.bounds.y, team_rect.y + team_rect.h);
    assert!(panel.bounds.w >= team_rect.w);
    assert_eq!(
        panel.rows.len(),
        2,
        "the full current team stays visible; full and negative-limit alternatives are filtered"
    );
    assert_eq!(app.context_menu_lobby_team_player, Some(7));
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby")
            .controller
            .open_team_combo_player(),
        Some(7)
    );

    assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select first team row"));
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select second team row"));
    assert!(app
        .handle_context_menu_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("activate selected team"));

    let updates = commands.take_player_info_updates();
    assert_eq!(updates.len(), 1);
    let mut changed = chooser;
    changed.team = 2;
    assert_eq!(
        updates[0],
        clonk_network::PlayerInfoUpdateRequest {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![changed, companion],
        },
        "OnTeamComboSelChange clones the complete client packet and mutates only Team"
    );
    assert!(app.context_menu.is_none());
    assert_eq!(app.context_menu_lobby_team_player, None);
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby")
            .controller
            .rows()
            .iter()
            .find_map(|row| match row {
                LobbyRosterRow::Player(player) if player.id == 7 => {
                    player.team.as_ref().map(|team| team.id)
                }
                _ => None,
            }),
        Some(1),
        "the combo text waits for the authoritative player-info echo"
    );

    app.network_lobby = Some(NetworkLobbyState::new(0, "Peer view".to_string(), false));
    app.apply_direct_player_info_control(
        clonk_engine::PlayerInfoControlData {
            client_id: updates[0].client_id,
            flags: updates[0].flags,
            players: updates[0].players.clone(),
            by_client: 0,
        },
        false,
    );
    let projected_team = |rows: &[LobbyRosterRow]| {
        rows.iter().find_map(|row| match row {
            LobbyRosterRow::Player(player) if player.id == 7 => {
                player.team.as_ref().map(|team| team.id)
            }
            _ => None,
        })
    };
    assert_eq!(
        projected_team(
            app.classic_host_lobby
                .as_ref()
                .expect("host projection")
                .controller
                .rows()
        ),
        Some(2)
    );
    assert_eq!(
        projected_team(
            &app.network_lobby
                .as_ref()
                .expect("peer projection")
                .roster_rows
        ),
        Some(2),
        "the same authoritative PlayerInfo converges every lobby projection"
    );
}

fn test_ping_connection(
    connection_id: u32,
    client_id: ClientId,
    usage: &str,
    ping_ms: i32,
    lag_ms: i32,
) -> clonk_network::RuntimeNetworkConnection {
    clonk_network::RuntimeNetworkConnection {
        connection_id,
        client_id,
        usage: usage.to_string(),
        protocol: clonk_network::NetworkProtocol::Tcp,
        peer_address: None,
        packet_loss: 0,
        ping_ms,
        lag_ms,
    }
}

#[test]
fn classic_lobby_ping_prefers_positive_message_connection_over_data() {
    // The message route has an unanswered ping: its getLag value (70)
    // outgrew the measured round trip (33). UpdatePing reads getLag
    // (src/C4PlayerInfoListBox.cpp:894-905), so the roster shows 70.
    let pings = classic_lobby_client_ping_ms_by_id(
        &[
            test_ping_connection(1, 7, "Msg", 33, 70),
            test_ping_connection(2, 7, "Data", 20, 20),
        ],
        0,
    );

    assert_eq!(pings.get(&7), Some(&70));
}

#[test]
fn classic_lobby_ping_uses_data_fallback_and_hides_only_minus_one_or_local() {
    let connection = |connection_id, client_id, usage: &str, lag_ms| {
        test_ping_connection(connection_id, client_id, usage, lag_ms, lag_ms)
    };
    let pings = classic_lobby_client_ping_ms_by_id(
        &[
            connection(1, 7, "Msg", 0),
            connection(2, 7, "Data", 20),
            connection(3, 8, "Data/Msg", 0),
            connection(4, 9, "Msg", -1),
            connection(5, 10, "Data", -1),
            connection(6, 11, "Msg", 0),
            connection(7, 11, "Data", -1),
            connection(8, 12, "Data", 13),
            connection(9, 42, "Data/Msg", 99),
        ],
        42,
    );

    assert_eq!(pings.get(&7), Some(&20));
    assert_eq!(pings.get(&8), Some(&0), "zero is a visible ping");
    assert!(!pings.contains_key(&9));
    assert!(!pings.contains_key(&10));
    assert!(
        !pings.contains_key(&11),
        "a present data route replaces a nonpositive message value"
    );
    assert_eq!(pings.get(&12), Some(&13));
    assert!(
        !pings.contains_key(&42),
        "the local client has no ping label"
    );
}

#[test]
fn classic_lobby_client_telemetry_refreshes_on_the_one_second_timer() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let client_row = |id, name: &str| {
        LobbyRosterRow::Client(LobbyClientRow {
            id,
            name: name.to_string(),
            nick: String::new(),
            color: [255, 255, 255, 255],
            status: LobbyClientStatus::Client,
            local: false,
            connected: true,
            resource_progress: None,
            ping_ms: None,
        })
    };
    let lobby = app.classic_host_lobby.as_mut().expect("test lobby");
    let mut rows = lobby.controller.rows().to_vec();
    rows.push(client_row(7, "Downloading"));
    rows.push(client_row(8, "Disconnected"));
    lobby.controller.set_rows(rows);

    let (network, _events, _commands) = NetworkManager::test_stub_with_commands();
    network.set_test_lobby_client_telemetry(clonk_network::RuntimeLobbyClientTelemetry {
        connections: vec![
            // An unanswered message-route ping whose getLag wait (70)
            // outgrew the cached round trip (33): the roster label shows
            // the live 70 (src/C4PlayerInfoListBox.cpp:894-905).
            clonk_network::RuntimeNetworkConnection {
                connection_id: 1,
                client_id: 7,
                usage: "Msg".to_string(),
                protocol: clonk_network::NetworkProtocol::Udp,
                peer_address: None,
                packet_loss: 0,
                ping_ms: 33,
                lag_ms: 70,
            },
            clonk_network::RuntimeNetworkConnection {
                connection_id: 2,
                client_id: 7,
                usage: "Data".to_string(),
                protocol: clonk_network::NetworkProtocol::Tcp,
                peer_address: None,
                packet_loss: 0,
                ping_ms: 20,
                lag_ms: 20,
            },
        ],
        resource_progress: vec![(7, 30), (8, 25)],
    });
    app.network = Some(network);

    assert!(app.sec1_timer().expect("refresh lobby telemetry"));
    let clients = app
        .classic_host_lobby
        .as_ref()
        .unwrap()
        .controller
        .rows()
        .iter()
        .filter_map(|row| match row {
            LobbyRosterRow::Client(client) => Some((client.id, client)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(clients[&0].ping_ms, None);
    assert_eq!(clients[&0].resource_progress, None);
    assert_eq!(clients[&7].ping_ms, Some(70));
    assert_eq!(clients[&7].resource_progress, Some(30));
    assert!(clients[&7].connected);
    assert_eq!(clients[&8].ping_ms, None);
    assert_eq!(clients[&8].resource_progress, None);
    assert!(!clients[&8].connected);

    app.network
        .as_ref()
        .unwrap()
        .set_test_lobby_client_telemetry(clonk_network::RuntimeLobbyClientTelemetry {
            connections: vec![clonk_network::RuntimeNetworkConnection {
                connection_id: 1,
                client_id: 7,
                usage: "Data/Msg".to_string(),
                protocol: clonk_network::NetworkProtocol::Tcp,
                peer_address: None,
                packet_loss: 0,
                ping_ms: 15,
                lag_ms: 15,
            }],
            resource_progress: vec![(7, 100)],
        });
    assert!(app.sec1_timer().expect("refresh completed resources"));
    let completed = app
        .classic_host_lobby
        .as_ref()
        .unwrap()
        .controller
        .rows()
        .iter()
        .find_map(|row| match row {
            LobbyRosterRow::Client(client) if client.id == 7 => Some(client),
            _ => None,
        })
        .unwrap();
    assert_eq!(completed.ping_ms, Some(15));
    assert_eq!(
        completed.resource_progress,
        Some(100),
        "native keeps the (100%) prefix while the remote remains connected"
    );
}

#[test]
fn client_roster_projection_hides_foreign_team_controls_and_random_assignments() {
    let teams = vec![
        clonk_engine::InitialNetworkTeam {
            id: 1,
            name: LegacyCString::from_bytes(b"One".to_vec()).unwrap(),
            player_start_index: 0,
            player_ids: vec![1],
            color: 0x00f4_0000,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        },
        clonk_engine::InitialNetworkTeam {
            id: 2,
            name: LegacyCString::from_bytes(b"Two".to_vec()).unwrap(),
            player_start_index: 0,
            player_ids: vec![2],
            color: 0x0000_c800,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        },
    ];
    let mut metadata = clonk_engine::InitialNetworkTeamMetadata {
        active: true,
        custom: true,
        allow_hostility_change: false,
        allow_team_switch: false,
        auto_generate_teams: false,
        last_team_id: 2,
        team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
        team_colors: true,
        max_script_players: 1,
        script_player_names: LegacyCString::default(),
        random_team_count: 0,
        teams,
    };
    let project = |metadata: &clonk_engine::InitialNetworkTeamMetadata, local_client_id| {
        let mut clients = ControlClientRegistry::default();
        clients.replace_snapshot([
            clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 7,
                activated: true,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 8,
                activated: false,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 9,
                activated: true,
                ..Default::default()
            },
        ]);
        let mut infos = ControlPlayerInfoRegistry::default();
        let random_invisible = matches!(
            metadata.team_distribution,
            clonk_engine::InitialNetworkTeamDistribution::RandomInvisible
        );
        infos.replace_snapshot(
            3,
            [
                clonk_engine::PlayerInfoControlData {
                    client_id: 0,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 1,
                        team: 1,
                        flags: if random_invisible {
                            clonk_engine::PLAYER_INFO_FLAG_JOINED
                        } else {
                            0
                        },
                        savegame_player: if random_invisible { 41 } else { 0 },
                        name: LegacyCString::from_bytes(b"Host player".to_vec()).unwrap(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 7,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 2,
                        team: 2,
                        flags: if random_invisible {
                            clonk_engine::PLAYER_INFO_FLAG_JOINED
                        } else {
                            0
                        },
                        savegame_player: if random_invisible { 42 } else { 0 },
                        name: LegacyCString::from_bytes(b"Own player".to_vec()).unwrap(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                clonk_engine::PlayerInfoControlData {
                    client_id: 8,
                    players: Vec::new(),
                    ..Default::default()
                },
            ],
        );
        classic_lobby_roster_projection(
            &clients,
            &infos,
            Some(metadata),
            local_client_id,
            LobbySheet::Players,
        )
        .0
    };
    let selectable = |rows: &[LobbyRosterRow], player_id| {
        rows.iter().find_map(|row| match row {
            LobbyRosterRow::Player(player) if player.id == player_id => {
                player.team.as_ref().map(|team| team.selectable)
            }
            _ => None,
        })
    };

    let client_rows = project(&metadata, 7);
    assert_eq!(selectable(&client_rows, 1), Some(false));
    assert_eq!(selectable(&client_rows, 2), Some(true));
    assert!(client_rows.iter().any(|row| matches!(
        row,
        LobbyRosterRow::Header(LobbyHeaderRow {
            kind: LobbyRosterHeader::ScriptPlayers,
            can_add_player: false,
            ..
        })
    )));
    assert!(client_rows.iter().any(|row| matches!(
        row,
        LobbyRosterRow::Client(LobbyClientRow {
            id: 8,
            status: LobbyClientStatus::Observer,
            ..
        })
    )));
    assert!(client_rows.iter().any(|row| matches!(
        row,
        LobbyRosterRow::Client(LobbyClientRow {
            id: 9,
            status: LobbyClientStatus::Unknown,
            ..
        })
    )));

    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Host;
    assert_eq!(selectable(&project(&metadata, 7), 2), Some(false));
    assert_eq!(selectable(&project(&metadata, 0), 1), Some(true));
    assert_eq!(selectable(&project(&metadata, 0), 2), Some(true));

    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::RandomInvisible;
    let random_rows = project(&metadata, 7);
    assert!(random_rows
        .iter()
        .filter_map(|row| match row {
            LobbyRosterRow::Player(player) => player.team.as_ref(),
            _ => None,
        })
        .all(|team| team.name == "Random team" && !team.selectable));
}

#[test]
fn classic_lobby_script_team_selectability_honors_restore_and_capacity_rules() {
    let team = |id, player_ids, max_players| clonk_network::JoinTeamSnapshot {
        id,
        name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
        player_start_index: 0,
        player_ids,
        color: 0,
        icon_spec: LegacyCString::default(),
        max_players,
    };
    let mut teams = clonk_network::JoinTeamListSnapshot {
        active: 1,
        custom: 1,
        allow_hostility_change: 0,
        allow_team_switch: 0,
        auto_generate_teams: 0,
        last_team_id: 2,
        team_distribution: 0,
        team_colors: 0,
        max_script_players: 1,
        script_player_names: LegacyCString::default(),
        random_team_count: 0,
        teams: vec![team(1, vec![7], 1), team(2, Vec::new(), 1)],
    };
    let mut script = clonk_engine::ControlPlayerInfoEntry {
        id: 7,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        team: 1,
        ..clonk_engine::ControlPlayerInfoEntry::default()
    };

    assert!(classic_lobby_player_can_choose_team(&teams, &script, false));
    assert!(
        !classic_lobby_player_can_choose_team(&teams, &script, true),
        "an associated savegame script row has joined info"
    );
    teams.teams[1].player_ids.push(8);
    assert!(
        !classic_lobby_player_can_choose_team(&teams, &script, false),
        "the only other team is full"
    );
    teams.auto_generate_teams = 1;
    assert!(classic_lobby_player_can_choose_team(&teams, &script, false));
    script.flags |= clonk_engine::PLAYER_INFO_FLAG_JOINED;
    assert!(!classic_lobby_player_can_choose_team(
        &teams, &script, false
    ));
}

#[test]
fn clicking_an_open_lobby_team_combo_closes_without_reopening() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_team_lobby(&mut app);
    let team_rect = test_lobby_team_rect(&mut app);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
        player_id: 7,
    }])
    .expect("open team combo");

    let point = PhysicalPosition::new(f64::from(team_rect.x + 2), f64::from(team_rect.y + 2));
    app.handle_cursor_moved(point)
        .expect("move from menu back onto owning combo");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("same-combo down closes the menu");

    assert!(app.context_menu.is_none());
    assert_eq!(app.context_menu_lobby_team_player, None);
    assert_eq!(app.context_menu_pointer_dismissed_lobby_team_player, None);
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby")
            .controller
            .open_team_combo_player(),
        None
    );
    assert!(commands.take_player_info_updates().is_empty());
    app.handle_mouse_button(ElementState::Released)
        .expect("release stays with the closed combo gesture");
    assert!(app.context_menu.is_none());

    app.context_menu_pointer_dismissed_lobby_team_player = Some(7);
    app.handle_gamepad_event(GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    })
    .expect("gamepad input starts a distinct combo gesture");
    assert_eq!(app.context_menu_pointer_dismissed_lobby_team_player, None);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
        player_id: 7,
    }])
    .expect("reopen team combo");
    assert!(app.context_menu.is_some());
    assert!(app.select_classic_lobby_sheet(LobbySheet::Resources));
    assert!(app.context_menu.is_none());
    assert_eq!(app.context_menu_lobby_team_player, None);
}

#[test]
fn classic_lobby_team_combo_rechecks_cpp_team_permissions_before_opening() {
    let mut app = new_menu_app(640, 480);
    let (mut chooser, companion) = install_test_classic_host_team_lobby(&mut app);
    let metadata = app
        .network_team_assignment
        .as_mut()
        .expect("team assignment")
        .teams_mut();
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
        player_id: 7,
    }])
    .expect("random distribution is an inert combo request");
    assert!(app.context_menu.is_none());

    let metadata = app
        .network_team_assignment
        .as_mut()
        .expect("team assignment")
        .teams_mut();
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Free;
    metadata.teams.retain(|team| team.id == 1);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
        player_id: 7,
    }])
    .expect("no alternative team is an inert combo request");
    assert!(app.context_menu.is_none());

    app.network_team_assignment
        .as_mut()
        .expect("team assignment")
        .teams_mut()
        .auto_generate_teams = true;
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
        player_id: 7,
    }])
    .expect("auto-generated teams permit opening without an alternative");
    assert!(app.context_menu.is_some());
    app.close_context_menu_silently();

    chooser.savegame_player = 99;
    app.control_player_infos.replace_snapshot(
        8,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![chooser, companion],
            by_client: 0,
        }],
    );
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::TeamSelectionRequested {
        player_id: 7,
    }])
    .expect("savegame-associated players remain read-only");
    assert!(app.context_menu.is_none());
}

#[test]
fn focused_lobby_team_combo_opens_from_cpp_keyboard_bindings_and_escape_closes() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_team_lobby(&mut app);
    let (_, roster) = app.classic_host_lobby_layouts().expect("team lobby layout");
    let player_row = roster
        .rows
        .iter()
        .find(|row| row.index == 1)
        .expect("player row")
        .rect;
    app.handle_cursor_moved(PhysicalPosition::new(
        f64::from(player_row.x + player_row.w / 2),
        f64::from(player_row.y + 2),
    ))
    .expect("hover player row");
    app.handle_mouse_button(ElementState::Pressed)
        .expect("select player row");
    app.handle_mouse_button(ElementState::Released)
        .expect("finish row selection");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby")
            .controller
            .focus(),
        LobbyControl::Roster
    );
    app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("focus team combo");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby")
            .controller
            .focus(),
        LobbyControl::RosterTeam
    );

    app.handle_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("Down opens combo");
    assert!(app.context_menu.is_some());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("Escape aborts combo");
    assert!(app.context_menu.is_none());

    app.handle_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("Space opens combo");
    assert!(app.context_menu.is_some());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("close Space-opened combo");

    app.keyboard_modifiers = ModifiersState::ALT;
    app.handle_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("Alt+Down opens combo");
    assert!(app.context_menu.is_some());
    app.keyboard_modifiers = ModifiersState::empty();
}

#[test]
fn l037_paste_scanner_preserves_edit_rules_and_skips_empty_lines() {
    let mut view = LobbyChatEditView {
        text: "abZZcd".into(),
        caret: 4,
        selection: Some((2, 4)),
        cursor_visible: true,
        ..LobbyChatEditView::default()
    };
    let mut submissions = Vec::new();
    let outcome = lobby_chat_paste_text(
        &mut view,
        "x|y\t\u{1}",
        LobbyChatPasteMode::Lobby,
        |_| {},
        |submission| {
            submissions.push(submission);
            Ok::<bool, ()>(true)
        },
    )
    .expect("infallible paste callback");
    assert_eq!(outcome.completed_lines, 0);
    assert!(submissions.is_empty());
    assert_eq!(view.text, "abx¦y\t\u{1}cd");
    assert_eq!(view.caret, 8);
    assert_eq!(view.selection, None);

    let mut typed = LobbyChatEditView::default();
    assert!(!lobby_chat_insert_text(&mut typed, "\t"));
    assert!(lobby_chat_insert_text(&mut typed, "\u{80}"));
    assert_eq!(typed.text, "\u{80}");

    let mut view = LobbyChatEditView {
        text: "draft".into(),
        caret: 5,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    };
    let mut submissions = Vec::new();
    let outcome = lobby_chat_paste_text(
        &mut view,
        "\r\nmore",
        LobbyChatPasteMode::Lobby,
        |_| {},
        |submission| {
            submissions.push(submission);
            Ok::<bool, ()>(true)
        },
    )
    .expect("infallible paste callback");
    assert_eq!(outcome.completed_lines, 0);
    assert!(submissions.is_empty());
    assert_eq!(view.text, "draftmore");

    let mut view = LobbyChatEditView::default();
    let oversized = format!("{}\ntrailing", "a".repeat(300));
    let mut submissions = Vec::new();
    let outcome = lobby_chat_paste_text(
        &mut view,
        &oversized,
        LobbyChatPasteMode::Lobby,
        |_| {},
        |submission| {
            submissions.push(submission);
            Ok::<bool, ()>(true)
        },
    )
    .expect("infallible paste callback");
    assert_eq!(outcome.completed_lines, 1);
    assert_eq!(submissions.len(), 1);
    assert_eq!(clonk_script::c4_string_byte_len(&submissions[0]), 254);
    assert_eq!(view.text, "trailing");
    assert_eq!(view.caret, view.text.len());

    let mut view = LobbyChatEditView::default();
    let mut submissions = Vec::new();
    let outcome = lobby_chat_paste_text(
        &mut view,
        "one\ntwo\nthree",
        LobbyChatPasteMode::Lobby,
        |_| {},
        |submission| {
            submissions.push(submission);
            Ok::<bool, ()>(true)
        },
    )
    .expect("infallible paste callback");
    assert_eq!(outcome.completed_lines, 2);
    assert_eq!(submissions, ["one", "two"]);
    assert_eq!(view.text, "three");

    let mut view = LobbyChatEditView::default();
    let mut submissions = Vec::new();
    let outcome = lobby_chat_paste_text(
        &mut view,
        "first\nnever-inserted",
        LobbyChatPasteMode::Lobby,
        |_| {},
        |submission| {
            submissions.push(submission);
            Ok::<bool, ()>(false)
        },
    )
    .expect("infallible paste callback");
    assert!(outcome.stopped);
    assert_eq!(submissions, ["first"]);
    assert!(view.text.is_empty());
}

#[test]
fn l037_lobby_paste_submits_each_line_and_retains_the_tail() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);

    app.paste_classic_lobby_chat_text("hello|there\nsecond\nworld")
        .expect("paste classic lobby chat");
    let submitted = commands.take_submitted_messages();
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[0].message.as_bytes(), "hello¦there".as_bytes());
    assert_eq!(submitted[1].message.as_bytes(), b"second");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("classic lobby remains")
            .controller
            .chat_edit_view()
            .text,
        "world"
    );

    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.paste_network_lobby_chat_text("one\r\ntwo\nthree")
        .expect("paste generic lobby chat");
    let submitted = commands.take_submitted_messages();
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[0].message.as_bytes(), b"one");
    assert_eq!(submitted[1].message.as_bytes(), b"two");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("generic lobby remains")
            .chat_edit
            .text,
        "three"
    );
}

#[test]
fn l037_running_paste_obeys_finish_result_and_crlf_more_flag() {
    let mut app = new_running_sandbox_app();
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    let paste = |app: &mut GameApp, text: &str| {
        let layout = app.game_option_input_layout().expect("chat layout");
        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        let actions = app
            .running_chat_controller_mut()
            .expect("chat controller")
            .apply_context_command(
                InputDialogContextCommand::Paste,
                Some(text),
                &layout,
                &fonts.text,
            );
        app.finish_game_option_input_dialog_actions(actions)
            .expect("process running-chat paste");
    };

    app.start_running_chat(RunningChatMode::All);
    paste(&mut app, "hello\nsecond\nworld");
    let submitted = commands.take_submitted_messages();
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[0].message.as_bytes(), b"hello");
    assert_eq!(submitted[1].message.as_bytes(), b"second");
    assert_eq!(app.running_chat_text(), Some("world"));

    app.close_running_chat().expect("close retained chat");
    app.start_running_chat(RunningChatMode::All);
    paste(&mut app, "done\n");
    let submitted = commands.take_submitted_messages();
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0].message.as_bytes(), b"done");
    assert!(app.running_chat.is_none());

    app.start_running_chat(RunningChatMode::All);
    paste(&mut app, "stay\r\n");
    let submitted = commands.take_submitted_messages();
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0].message.as_bytes(), b"stay");
    assert_eq!(app.running_chat_text(), Some("stay"));
    assert_eq!(
        app.running_chat_controller()
            .expect("CRLF reports more at the first delimiter")
            .selection(),
        Some((0, 4))
    );
}

#[test]
fn classic_lobby_chat_edits_parses_and_submits_private_delivery_controls() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(
        LobbyChatRequest::InsertText("/me hello".to_string()),
    )])
    .expect("edit lobby chat");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby remains")
            .controller
            .chat_edit_view()
            .text,
        "/me hello"
    );
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(LobbyChatRequest::Submit(
        "/me hello".to_string(),
    ))])
    .expect("submit lobby chat");

    assert_eq!(
        commands.take_submitted_messages(),
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_ME,
            player: -1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"hello".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 0,
        }]
    );
    assert!(app
        .classic_host_lobby
        .as_ref()
        .expect("lobby remains")
        .controller
        .chat_edit_view()
        .text
        .is_empty());

    assert!(app.engine.set_team_distribution(4));
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(LobbyChatRequest::Submit(
        "^surprise".to_string(),
    ))])
    .expect("hidden random teams reject team chat without closing the lobby");
    assert!(commands.take_submitted_messages().is_empty());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby remains")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some("Can't send team message: Teams not known.")
    );
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(LobbyChatRequest::Submit(
        "^".to_string(),
    ))])
    .expect("empty hidden-team syntax rejects before payload trimming");
    assert!(commands.take_submitted_messages().is_empty());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("lobby remains")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()),
        Some("Can't send team message: Teams not known.")
    );

    let alert = parse_lobby_message_control(" /alert ")
        .expect("parse")
        .expect("ordinary leading space keeps this a normal message");
    assert_eq!(alert.message_type, MESSAGE_TYPE_NORMAL);
    let alert = parse_lobby_message_control("/alert")
        .expect("parse alert")
        .expect("empty alert is still submitted");
    assert_eq!(alert.message_type, MESSAGE_TYPE_ALERT);
    assert!(alert.message.is_empty());

    app.control_clients
        .replace_snapshot([message_client(7, b"Remote")]);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(LobbyChatRequest::Submit(
        "/mute Remote".to_string(),
    ))])
    .expect("mute is a local classic message command");
    assert!(app.control_messages.is_muted(7));
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::Chat(LobbyChatRequest::Submit(
        "/unmute Remote".to_string(),
    ))])
    .expect("unmute is a local classic message command");
    assert!(!app.control_messages.is_muted(7));
    assert!(commands.take_submitted_messages().is_empty());
}

#[test]
fn classic_host_lobby_exit_directly_tears_down_and_returns_to_startup() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated lobby exit user data");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = prepare_tutorial_host_lobby(&app, repository);
    app.staged_network_host_scenario = Some(staged);
    install_test_classic_host_lobby(&mut app);
    let _ = app
        .classic_host_lobby
        .as_mut()
        .expect("classic host lobby")
        .controller
        .apply_countdown_packet(clonk_frontend::game_lobby::LobbyCountdownPacket::Seconds(3));
    assert!(app
        .classic_host_lobby
        .as_ref()
        .expect("classic host lobby")
        .controller
        .countdown()
        .is_any());

    let (manager, _events) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Exact Host".to_string(),
        prepared: None,
    }));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Exact Host".to_string(), true));
    app.network_ticks.queue(0, 4, Vec::new());
    app.network_sync.queue(0, 5, Vec::new());
    let local_check = SyncCheckPacket {
        frame: 4,
        control_tick: 4,
        random3: 0,
        random_count: 0,
        crew_positions_sum: 0,
        pxs_count: 0,
        mass_mover_index: 0,
        object_count: 0,
        object_enumeration_index: 0,
        sector_shape_sum: 0,
        by_client: 0,
    };
    let mut remote_check = local_check.clone();
    remote_check.frame = 5;
    app.sync_checks.local.insert(local_check.frame, local_check);
    app.sync_checks
        .remote
        .insert(remote_check.frame, remote_check);
    app.admission_resources.resources.insert(
        7,
        AdmissionResourceState::Unavailable(AdmissionResourceUnavailable::Unloadable),
    );
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 8,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 9,
                ..Default::default()
            }],
            ..Default::default()
        });
    app.executing_ready_tick = Some(6);
    assert!(app.loader_screen.is_some());

    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("Escape closes the lobby directly without a confirmation");

    assert_eq!(app.startup_view, StartupView::MainMenu);
    assert!(app.classic_host_lobby.is_none());
    assert!(app.network_lobby.is_none());
    assert!(app.staged_network_host_scenario.is_none());
    // The return through PreInit re-initializes the loader screen for the
    // next game (src/C4Application.cpp:242-247,373-389).
    assert!(app.loader_screen.is_some());
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.startup_network_connection.is_none());
    assert!(app.network_ticks.ready.is_empty());
    assert!(app.network_sync.scheduled.is_empty());
    assert!(app.sync_checks.local.is_empty() && app.sync_checks.remote.is_empty());
    assert!(app.admission_resources.resources.is_empty());
    assert!(app.executing_ready_tick.is_none());
    assert!(app.control_player_infos.client_info_ids(8).is_empty());
    assert!(app.network_control_running);
    assert_eq!(
        app.scenario_game_options.context(),
        GameOptionContext::LocalSelector
    );
    assert!(app.message_dialogs.is_empty());
    assert!(app.status_text.is_empty());
    reset_cached_app_paths();
}

#[test]
fn classic_host_lobby_chat_keyboard_routes_edit_locally() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated lobby key user data");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_test_classic_host_lobby(&mut app);

    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
        .expect("Apps opens the classic chat context menu");
    assert!(app.context_menu.is_some());
    app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
        .expect("Escape closes the classic chat context menu");
    assert!(app.context_menu.is_none());

    for (key, modifiers) in [
        (VirtualKeyCode::KeyA, ModifiersState::CONTROL),
        (VirtualKeyCode::ArrowLeft, ModifiersState::CONTROL),
        (VirtualKeyCode::Delete, ModifiersState::empty()),
        (VirtualKeyCode::Home, ModifiersState::SHIFT),
    ] {
        app.handle_modifiers_changed(modifiers)
            .expect("set keyboard modifiers");
        app.handle_key(key, ElementState::Pressed)
            .expect("classic chat keyboard action is handled");
    }
    app.handle_modifiers_changed(ModifiersState::empty())
        .expect("set keyboard modifiers");
    app.handle_text_input('x')
        .expect("classic chat text input is handled");
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .chat_edit_view()
            .text,
        "x"
    );

    for _ in 0..10 {
        if app
            .classic_host_lobby
            .as_ref()
            .is_some_and(|lobby| lobby.controller.focus() == LobbyControl::Roster)
        {
            break;
        }
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("focus traversal to roster is local and safe");
    }
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .focus(),
        LobbyControl::Roster
    );
    app.handle_key(VirtualKeyCode::ContextMenu, ElementState::Pressed)
        .expect("local client context is safely ignored");
    assert!(app.context_menu.is_none());
}

#[test]
fn generic_client_lobby_chat_submits_private_delivery_message_controls() {
    let mut app = new_menu_app(640, 480);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);

    for character in "hello".chars() {
        app.handle_text_input(character).expect("type client chat");
    }
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("submit client chat");

    assert_eq!(
        commands.take_submitted_messages(),
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"hello".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 7,
        }]
    );
    assert!(app
        .network_lobby
        .as_ref()
        .expect("client lobby remains")
        .chat_edit
        .text
        .is_empty());

    for character in "ab".chars() {
        app.handle_text_input(character)
            .expect("type editable client chat");
    }
    app.handle_key(VirtualKeyCode::Backspace, ElementState::Pressed)
        .expect("client chat backspace");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("client lobby remains")
            .chat_edit
            .text,
        "a"
    );
    app.handle_key(VirtualKeyCode::ArrowUp, ElementState::Pressed)
        .expect("shared lobby history");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("client lobby remains")
            .chat_edit
            .text,
        "hello"
    );
    app.mode = AppMode::Running;
    app.start_running_chat(RunningChatMode::All);
    app.browse_running_chat_history(true);
    assert_eq!(
        app.running_chat_text(),
        Some("hello"),
        "C4MessageInput history survives the lobby-to-game transition"
    );
}

#[test]
fn classic_host_lobby_network_events_update_supported_live_state() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (manager, events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Exact Host".to_string(),
        prepared: None,
    }));

    events
        .send(NetworkEvent::PeerConnected {
            client_id: 0,
            name: "Exact Host".to_string(),
            kind: ParticipantKind::Player,
        })
        .expect("queue whitelisted local-host event");
    app.process_network_events()
        .expect("local client-zero notification is already represented");
    assert!(app.status_text.is_empty());
    assert!(app.network_lobby.is_none());

    app.control_player_infos.replace_snapshot(
        7,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 7,
                color: 0x00ff_0000,
                original_color: 0x00ff_0000,
                league_projected_gain: -1,
                ..Default::default()
            }],
            ..Default::default()
        }],
    );
    events
        .send(NetworkEvent::LeagueUpdate(
            clonk_network::LeagueUpdateResponse {
                player_infos: clonk_network::ClientPlayerInfosSnapshot {
                    client_id: -1,
                    flags: 0,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 7,
                        league_projected_gain: 4,
                        ..Default::default()
                    }],
                },
                ..Default::default()
            },
        ))
        .expect("queue lobby league update");
    app.process_network_events()
        .expect("league update remains valid in the classic lobby");
    assert_eq!(
        app.control_player_infos
            .get(7)
            .unwrap()
            .league_projected_gain,
        4
    );
    let league_broadcasts = commands.take_broadcast_player_infos();
    let [league_info] = league_broadcasts.as_slice() else {
        panic!("expected one projected-gain PlayerInfo broadcast");
    };
    assert_eq!(league_info.client_id, 0);
    assert_eq!(league_info.players.len(), 1);
    assert_eq!(league_info.players[0].id, 7);
    assert_eq!(league_info.players[0].league_projected_gain, 4);

    events
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 1,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 1,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Remote player".to_vec())
                        .expect("valid player name"),
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue supported classic-lobby PlayerInfo request");
    app.process_network_events()
        .expect("classic lobby admits PlayerInfo requests");
    let broadcasts = commands.take_broadcast_player_infos();
    let [gain_reset, admitted] = broadcasts.as_slice() else {
        panic!("expected gain reset then authoritative admission, got {broadcasts:?}");
    };
    assert_eq!(gain_reset.client_id, 0);
    assert_eq!(gain_reset.players[0].id, 7);
    assert_eq!(gain_reset.players[0].league_projected_gain, -1);
    assert_eq!(admitted.client_id, 1);
    for info in broadcasts {
        events
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                original: info.clone(),
                info,
                join_players_on_echo: Vec::new(),
            })
            .expect("queue supported classic-lobby PlayerInfo echo");
    }
    app.process_network_events()
        .expect("classic lobby applies authoritative PlayerInfo echoes");
    assert!(app.control_player_infos.contains_client(1));

    events
        .send(NetworkEvent::PeerConnected {
            client_id: 1,
            name: "Remote".to_string(),
            kind: ParticipantKind::Player,
        })
        .expect("queue remote transport row");
    app.process_network_events()
        .expect("remote transport notification is nonfatal");
    events
        .send(NetworkEvent::DirectControl(NetworkControl::ClientJoin(
            clonk_engine::ClientJoinControlData {
                core: clonk_engine::ClientCoreControlData {
                    client_id: 1,
                    activated: true,
                    observer: false,
                    name: LegacyCString::from_bytes(b"Remote".to_vec()).unwrap(),
                    nick: LegacyCString::from_bytes(b"Remote nick".to_vec()).unwrap(),
                    lobby_ready: false,
                },
                by_client: 0,
            },
        )))
        .expect("queue authoritative remote client");
    app.process_network_events()
        .expect("authoritative remote row is projected");
    // Raw transport callbacks stay presentation-silent. The accepted
    // host-authored control owns C++'s localized lobby log
    // (src/C4GameLobby.cpp:669-675; src/C4Control.cpp:554-565;
    // src/C4Log.cpp:227-239).
    assert!(app.status_text.is_empty());
    assert!(app.network_lobby.is_none());
    assert!(app.classic_host_lobby.is_some());
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .unwrap()
            .controller
            .logs()
            .last(),
        Some(&LobbyLogLine {
            text: "Client Remote connected.".to_string(),
            color: [0xaf, 0xaf, 0xaf, 0xff],
        })
    );
    assert!(app
        .classic_host_lobby
        .as_ref()
        .unwrap()
        .controller
        .rows()
        .iter()
        .any(|row| matches!(row, LobbyRosterRow::Client(client) if client.id == 1)));
}

#[test]
fn classic_host_lobby_cancel_paths_clear_pressed_activation_latches() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated lobby cancel user data");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_test_classic_host_lobby(&mut app);
    for _ in 0..20 {
        if app
            .classic_host_lobby
            .as_ref()
            .is_some_and(|lobby| lobby.controller.focus() == LobbyControl::Exit)
        {
            break;
        }
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("focus traversal to Exit is local and safe");
    }
    assert_eq!(
        app.classic_host_lobby
            .as_ref()
            .expect("test lobby")
            .controller
            .focus(),
        LobbyControl::Exit
    );

    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("latch Exit key down");
    app.handle_focus_lost().expect("cancel on focus loss");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("focus loss prevents delayed Exit activation");

    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("latch Exit before resize");
    app.resize(650, 490).expect("resize cancels lobby input");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("resize prevents delayed Exit activation");

    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Pressed,
    )
    .expect("latch gamepad low action");
    app.handle_gamepad_event(GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    })
    .expect("controller clear cancels lobby input");
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Released,
    )
    .expect("controller clear prevents delayed Exit activation");

    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("latch Exit before pointer leave");
    app.pointer_left().expect("process cursor exit");
    app.handle_key(VirtualKeyCode::Enter, ElementState::Released)
        .expect("ordinary cursor leave preserves and activates the Exit latch");
    assert_eq!(app.startup_view, StartupView::MainMenu);
    assert!(app.classic_host_lobby.is_none());
}

#[test]
fn connected_client_enters_exact_classic_lobby() {
    // The client worker announces its own established transport after
    // JoinData. C++ MainDlg::OnClientConnect is presentation-silent; only
    // authoritative C4ControlClientJoin writes the lobby log
    // (src/C4GameLobby.cpp:669-675; src/C4Control.cpp:554-565).
    let mut app = new_menu_app(640, 480);
    let (manager, events) = NetworkManager::test_stub_for_client_id(7);
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Client(ClientSettings::new(
                SocketAddr::from(([127, 0, 0, 1], 11112)),
                "Client",
            )),
            manager,
        )))
        .expect("queue completed client connection");
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));
    app.poll_startup_network_connection()
        .expect("poll joined network transition");
    assert!(app.network.is_some());
    assert!(matches!(app.network_mode, Some(NetworkMode::Client(_))));
    assert!(app.classic_host_lobby.is_none());
    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    let lobby = app
        .network_lobby
        .as_ref()
        .expect("connected client lobby model");
    assert_eq!(lobby.local_client_id, 7);
    assert!(!lobby.is_host);
    assert!(app.status_text.is_empty());
    assert!(!app.network_control_running);
    assert!(app.control_clients.contains(7));
    assert!(!app.control_clients.is_activated(7));

    events
        .send(NetworkEvent::PeerConnected {
            client_id: 7,
            name: "Client".to_string(),
            kind: ParticipantKind::Player,
        })
        .expect("queue the worker's local transport announcement");
    app.process_network_events()
        .expect("local transport announcement keeps the lobby active");
    assert!(
        app.status_text.is_empty(),
        "raw transport establishment must not poison the exact lobby renderer"
    );
}

#[test]
fn network_lobby_renders_classic_base_without_enabling_generic_fallback() {
    // MainDlg is a FullscreenDialog with exact client margins and a
    // ComponentAligner-owned bottom row. A non-host gets no Start button;
    // its ready checkbox occupies the rightmost 110x32 cell
    // (pristine 9ffa0a5d src/C4GuiDialogs.cpp:813-822,858-862;
    // src/C4GameLobby.cpp:141-218).
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let user_data = tempdir().expect("user data");
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = AppPaths::discover().expect("discover installed assets");
    let mut app = GameApp::new(
        640,
        480,
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
            player_name: "Client".to_string(),
            network: None,
            record_enabled: false,
        },
    )
    .expect("initialise installed app");
    wait_for_menu(&mut app);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    app.sync_network_lobby_game_option_state();
    let mut frame = vec![0x5a; 640 * 480 * 4];

    app.render(&mut frame)
        .expect("classic network lobby base renders");

    let layout = app
        .network_lobby
        .as_ref()
        .and_then(|lobby| lobby.layout.as_ref())
        .expect("render computes lobby layout");
    assert_eq!(
        (
            layout.ready_button.origin.x as i32,
            layout.ready_button.origin.y as i32,
            layout.ready_button.size.width as i32,
            layout.ready_button.size.height as i32,
        ),
        (508, 400, 110, 32)
    );
    assert!(layout.start_button.is_none());
    assert!(frame.iter().any(|byte| *byte != 0x5a));
    assert!(
        app.menu_frame_cache.is_none(),
        "live joined lobbies cannot cache away timers, held scroll, or tooltips"
    );

    let (classic_layout, roster) = app.joined_lobby_layouts().expect("joined layout");
    let exit = GuiPoint::new(
        (classic_layout.exit_button.x + 1) as f32,
        (classic_layout.exit_button.y + 1) as f32,
    );
    app.network_lobby
        .as_mut()
        .unwrap()
        .controller
        .pointer_move(exit, &classic_layout, &roster);
    assert!(app
        .network_lobby
        .as_ref()
        .unwrap()
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    let cached = vec![0x45; 640 * 480 * 4];
    app.menu_frame_cache = Some(MenuFrameCache {
        view: StartupView::NetworkLobby,
        version: app.menu_render_version,
        width: 640,
        height: 480,
        native_text_deferred: false,
        frame: cached.clone(),
    });
    let mut refreshed = cached.clone();
    assert!(app
        .render(&mut refreshed)
        .expect("joined lobby bypasses a matching startup frame cache"));
    assert_ne!(refreshed, cached);

    // The classic renderer remains fail-closed when its required assets
    // are absent; NetworkLobby must not re-enable the old generic pane.
    let mut assetless = new_menu_app(320, 200);
    Arc::get_mut(&mut assetless.assets)
        .expect("frontend assets are app-owned")
        .startup_dialog_images
        .remove("GUIButtonDown.png")
        .expect("classic fixture includes the pressed button sheet");
    assetless.startup_view = StartupView::NetworkLobby;
    assetless.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let mut untouched = vec![0x3c; 320 * 200 * 4];
    let error = assetless
        .render(&mut untouched)
        .expect_err("assetless lobby refuses generic fallback");
    assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::GlobalGuiBootstrapResources { issues })
            if issues.contains(&ClassicGuiBootstrapIssue::missing("GUIButtonDown"))
    ));
    assert!(untouched.iter().all(|byte| *byte == 0x3c));
    reset_cached_app_paths();
}

#[test]
fn l094_saving_a_file_picture_preserves_an_unchecked_lobby_icon() {
    let _lock = env_lock().lock();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let user_data = tempdir().expect("portrait user data");
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(repository)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = AppPaths::discover().expect("discover portrait paths");
    paths.ensure_user_dirs().expect("create portrait user path");
    let player_root = user_data.path().join("Players");
    persist_config_value(
        &paths,
        "General",
        "PlayerPath",
        player_root.to_string_lossy().into_owned(),
    )
    .expect("configure isolated player path");
    let selected_path = paths.user_data_dir().join("Selected.PNG");
    write_preview_image(&selected_path, [0, 0, 255, 255], image::ImageFormat::Png);

    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    app.open_new_startup_player_properties();
    let old_icon = ImageData::new(2, 1, vec![4, 5, 6, 255, 7, 8, 9, 255]);
    let pending = app
        .startup_player_properties_dialog
        .as_mut()
        .expect("new player properties");
    pending.controller.set_name("UncheckedIcon");
    pending
        .controller
        .replace_images(ImageData::new(1, 1, vec![1, 2, 3, 255]), old_icon.clone());

    app.apply_startup_player_portrait_selection(
        clonk_frontend::startup_portraitsel::PortraitSelCommit {
            choice: clonk_frontend::startup_portraitsel::PortraitChoice::File(selected_path),
            set_picture: true,
            set_big_icon: false,
        },
    );
    assert_eq!(
        app.startup_player_properties_dialog
            .as_ref()
            .expect("properties remain open")
            .controller
            .big_icon_update(),
        &clonk_frontend::startup_plrproperties::PlayerImageUpdate::Replace(old_icon.clone())
    );
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    let saved =
        Group::open(player_root.join("UncheckedIcon.c4p")).expect("open saved player group");
    let encoded_icon = saved
        .read_file("BigIcon.png")
        .expect("read saved lobby icon");
    let decoded_icon = image::load_from_memory(&encoded_icon)
        .expect("decode saved lobby icon")
        .into_rgba8();
    assert_eq!(decoded_icon.dimensions(), (2, 1));
    assert_eq!(decoded_icon.into_raw(), old_icon.pixels());
    reset_cached_app_paths();
}

#[test]
fn network_lobby_live_render_bypasses_matching_exact_cache() {
    let mut app = new_real_classic_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Host".to_string(), true));
    app.sync_network_lobby_game_option_state();
    assert!(app.status_text.is_empty());

    let cached = vec![0x31; 320 * 200 * 4];
    app.menu_frame_cache = Some(MenuFrameCache {
        view: StartupView::NetworkLobby,
        version: app.menu_render_version,
        width: 320,
        height: 200,
        native_text_deferred: false,
        frame: cached.clone(),
    });
    let mut frame = vec![0x73; 320 * 200 * 4];

    // A retained lobby advances tooltip clocks, held scrollbars and
    // transient status icons without input, so a matching cache must
    // neither replay nor be replaced by the live frame.
    assert!(app.render(&mut frame).expect("live lobby renders"));
    assert_ne!(frame, cached, "stale lobby pixels must not replay");
    assert_eq!(
        app.menu_frame_cache
            .as_ref()
            .expect("bypassed cache remains available for diagnostics")
            .frame,
        cached
    );

    let mut native_frame = vec![0x47; 960 * 600 * 4];
    app.render_native_main_menu_text(&mut native_frame, 960, 600)
        .expect("network lobby has no deferred main-menu text pass");
    assert!(native_frame.iter().all(|byte| *byte == 0x47));
}

#[test]
fn options_program_round_trips_bound_values_and_raw_fair_crew_strength() {
    use clonk_frontend::startup_options_dlg::{fair_crew_slider_to_strength, OptionsDlgAction};

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
                "[General]\nFontName=Endeavour\nFontSize=14\nUseWhiteIngameChat=1\nUseWhiteLobbyChat=1\nShowLogTimestamps=0\nPreloading=0\nDefCrewStrength=1000\nVendorProgramKey=keep\n",
            )
            .expect("seed Program config");

    let mut app =
        test_game_app(1280, 720, AudioOptions::default(), Some(&paths)).expect("initialise app");
    wait_for_menu(&mut app);
    app.open_options_menu();

    let program = app.startup_options_dialog.as_ref().unwrap().program();
    assert_eq!(program.font_face, "Endeavour");
    assert_eq!(program.font_size, "14");
    assert!(program.white_chat_ingame);
    assert!(program.white_chat_lobby);
    assert!(!program.preloading);
    assert_eq!(program.fair_crew_strength, 1_000);
    assert_eq!(program.fair_crew_slider, 9);

    let strength = fair_crew_slider_to_strength(10);
    {
        let program = app.startup_options_dialog.as_mut().unwrap().program_mut();
        program.white_chat_ingame = false;
        program.white_chat_lobby = false;
        program.preloading = true;
        program.fair_crew_slider = 10;
        program.fair_crew_strength = strength;
    }
    app.process_options_dialog_actions(vec![
        OptionsDlgAction::WhiteChatIngameChanged(false),
        OptionsDlgAction::WhiteChatLobbyChanged(false),
        OptionsDlgAction::PreloadingChanged(true),
        OptionsDlgAction::FairCrewStrengthChanged(strength),
    ])
    .expect("apply Program callbacks");
    assert!(!app.display_flags.white_chat);
    assert!(!app.white_lobby_chat);
    app.process_options_dialog_actions(vec![OptionsDlgAction::Back])
        .expect("save Program options");

    let config = Config::load(paths.config_file()).expect("reload Program config");
    assert_eq!(
        config.get_in(Some("General"), "FontName"),
        Some("Endeavour")
    );
    assert_eq!(config.get_in(Some("General"), "FontSize"), Some("14"));
    assert_eq!(
        config.get_in(Some("General"), "UseWhiteIngameChat"),
        Some("0")
    );
    assert_eq!(
        config.get_in(Some("General"), "UseWhiteLobbyChat"),
        Some("0")
    );
    assert_eq!(config.get_in(Some("General"), "Preloading"), Some("1"));
    let strength_string = strength.to_string();
    assert_eq!(
        config.get_in(Some("General"), "DefCrewStrength"),
        Some(strength_string.as_str())
    );
    assert_eq!(
        config.get_in(Some("General"), "VendorProgramKey"),
        Some("keep")
    );
}

#[test]
fn selected_network_scenario_installs_prepared_host_before_admission() {
    // OpenScenario and InitHost finish before Players.Init authors the
    // empty Initial PlayerInfo; AllowJoin follows that direct local
    // execution (src/C4Game.cpp:421-438,3847-3876;
    // src/C4Network2Players.cpp:38-49,78-123,160-239).
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated selected-host user data");
    let content = tempdir().expect("minimal selected-host content");
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    persist_config_value(&paths, "Network", "PortUDP", "0")
        .expect("disable selected-host UDP listener");
    persist_config_value(&paths, "Network", "PortDiscovery", "0")
        .expect("disable selected-host multicast discovery");
    persist_config_value(&paths, "Network", "EnableUPnP", "0")
        .expect("disable selected-host UPnP probe");
    // The enabled async path has its own preload-reuse regression; this test
    // isolates host preparation and admission ordering.
    persist_config_value(&paths, "General", "Preloading", "0")
        .expect("keep this admission test independent of platform preload defaults");
    let reference_port = std::net::TcpListener::bind("[::1]:0")
        .expect("reserve selected-host reference port")
        .local_addr()
        .expect("selected-host reference address")
        .port();
    persist_config_value(
        &paths,
        "Network",
        "PortRefServer",
        reference_port.to_string(),
    )
    .expect("configure selected-host reference listener");
    let mut app = new_menu_app_with_paths(1280, 720, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    app.staged_network_host_scenario = Some(staged);

    app.activate_prepared_network_host(scenario.clone(), SocketAddr::from(([127, 0, 0, 1], 0)));
    assert!(app.network.is_none(), "preparation must precede bind");
    assert!(app.startup_network_connection.is_some());
    // OpenScenario publishes 4 before InitNetworkHost begins, so the
    // loader installed around host preparation must retain that value
    // (src/C4Game.cpp:124-270,421-440).
    assert_eq!(
        app.loader_screen
            .as_ref()
            .expect("prepared host loader")
            .state()
            .progress(),
        4
    );

    for _ in 0..3_000 {
        app.poll_startup_network_connection()
            .expect("poll selected network host");
        if app.network.is_some() && app.network_control_clock.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(app.network.is_some(), "{}", app.status_text);
    let NetworkMode::Host(settings) = app.network_mode.as_ref().expect("host mode") else {
        panic!("prepared network selection must install a host");
    };
    let prepared = settings.prepared.as_ref().expect("canonical host state");
    assert!(!prepared.host_config().allow_join);
    assert!(prepared.host_config().initial_join_snapshot.is_some());
    assert!(app.control_player_infos.contains_client(0));
    assert_eq!(app.control_player_infos.player_count(), 0);
    assert_eq!(
        prepared.scenario_wire_name().as_bytes(),
        scenario.identifier.as_bytes(),
        "the prepared host retains the selected scenario's wire identity"
    );
    assert!(
        app.network_lobby.is_none(),
        "a staged host uses the exact C++ lobby instead of the generic projection"
    );
    assert!(app.classic_host_lobby.is_some());
    let local_addresses = app
        .network
        .as_ref()
        .expect("live prepared host")
        .local_addresses();
    assert!(matches!(local_addresses.len(), 1 | 2));
    let tcp = local_addresses.first().expect("prepared host TCP address");
    assert_eq!(tcp.protocol, clonk_network::NetworkProtocol::Tcp);
    assert_ne!(tcp.endpoint.port(), 0);
    if let Some(udp) = local_addresses.get(1) {
        // C4Network2IO starts TCP and UDP independently and publishes only
        // live transports. A parallel test or process may own PortUDP;
        // that is a valid TCP-only host, not an admission failure.
        assert_eq!(udp.protocol, clonk_network::NetworkProtocol::Udp);
        assert_eq!(udp.endpoint.ip(), tcp.endpoint.ip());
        assert_eq!(udp.endpoint.port(), 11_113);
    }
    let advertised = app
        .advertised_game_reference
        .as_ref()
        .expect("prepared host publishes an exact reference");
    assert!(app.network_game_advertiser.is_some());
    assert!(advertised.summary().join_allowed);
    assert_eq!(advertised.metadata().icon, 2);
    assert_eq!(advertised.metadata().addresses, local_addresses);
    assert_eq!(
        advertised.parameters(),
        &prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters
    );
    let prepared_parameters = &prepared
        .host_config()
        .initial_join_snapshot
        .as_ref()
        .expect("prepared JoinData")
        .parameters;
    let prepared_random_seed = u64::from(prepared_parameters.random_seed as u32);
    let prepared_fair_crew = (
        prepared_parameters.use_fair_crew,
        prepared_parameters.fair_crew_strength,
    );
    assert_eq!(
        app.network_control_clock,
        Some(NetworkControlClock::new(
            i32::try_from(prepared.host_config().start_tick).expect("start tick fits i32"),
            prepared_parameters.control_rate,
        ))
    );

    let expected_go = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: prepared.host_config().initial_status.control_mode,
        target_tick: 0,
    };
    let (manager, events, mut commands) = NetworkManager::test_stub_with_commands();
    // The live manager owns the temporary published definition packs.
    // Keep it alive while the command stub observes the countdown; C++
    // likewise retains its resource list through game activation.
    let _prepared_resource_owner = app
        .network
        .replace(manager)
        .expect("prepared host retains its live resource manager");
    install_test_classic_host_lobby(&mut app);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: DEFAULT_LOBBY_COUNTDOWN_SECONDS,
        check_league_rules: true,
        confirm_unassociated_savegame_players: false,
    }])
    .expect("prepared classic host starts the C++ countdown");
    assert!(app.select_classic_lobby_sheet(LobbySheet::Options));
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::OptionSelectionRequested {
        option: LobbyOptionKind::ControlRate,
        anchor: GuiPoint::new(400.0, 240.0),
        minimum_width: 120,
    }])
    .expect("open an option ComboBox during the countdown");
    assert!(app.context_menu.is_some());
    assert_eq!(
        app.context_menu_lobby_option,
        Some(LobbyOptionKind::ControlRate)
    );
    let go_observer = thread::spawn(move || {
        let observed = commands.complete_lobby_start(Ok(()));
        (commands, observed)
    });
    for _ in 0..DEFAULT_LOBBY_COUNTDOWN_SECONDS {
        assert!(
            app.sec1_timer().expect("advance global second timer"),
            "global second timer advances countdown"
        );
    }
    // Countdown::OnSec1Timer broadcasts/applies zero before calling
    // Network.Start, which is what submits the GS_Go status barrier
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:1140-1173).
    let countdown_command = |countdown| {
        network::TestLobbyStartCommand::Countdown(clonk_network::LobbyCountdownPacket::new(
            countdown,
        ))
    };
    let (mut commands, observed_start) = go_observer.join().expect("atomic Go observer");
    assert_eq!(
        observed_start,
        vec![
            countdown_command(5),
            countdown_command(4),
            countdown_command(3),
            countdown_command(2),
            countdown_command(1),
            countdown_command(0),
            network::TestLobbyStartCommand::BeginGo {
                status: expected_go,
                join_allowed: false,
            },
        ]
    );
    assert!(
        app.host_lobby_countdown.is_none(),
        "natural zero releases C4Network2::pLobbyCountdown ownership before GO"
    );
    app.sec1_timer().expect("pulse inactive second timer");
    assert!(
        commands.take_lobby_start_commands().is_empty(),
        "later second pulses cannot repeat zero or GO"
    );
    assert!(matches!(app.mode, AppMode::Loading));
    assert!(app.loading_state.is_some());
    assert!(app.context_menu.is_none());
    assert_eq!(app.context_menu_lobby_option, None);
    // Init returns from InitNetworkHost/DoLobby at 7 before beginning
    // InitGame's script and definition phases
    // (src/C4Game.cpp:438-457,3872-3913).
    assert_eq!(
        app.loading_state
            .as_ref()
            .expect("prepared host loading state")
            .last_progress,
        7
    );
    assert_eq!(
        app.loader_screen
            .as_ref()
            .expect("prepared host loader after lobby")
            .state()
            .progress(),
        7
    );
    assert!(app
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| !wait.visible));
    // C4GameParameters chooses the host seed before InitNetworkHost and
    // the same Parameters.RandomSeed is serialized to every client. The
    // retained host scenario must therefore enter InitGame with that
    // exact bit pattern (pristine 9ffa0a5d
    // src/C4GameParameters.cpp:418-432,555;
    // src/C4Game.cpp:2617-2627).
    assert_eq!(
        app.loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|loading| loading.random_seed),
        Some(prepared_random_seed),
        "prepared host must retain Parameters.RandomSeed for scenario activation"
    );
    assert_eq!(
        app.loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|loading| (loading.use_fair_crew, loading.fair_crew_strength)),
        Some(prepared_fair_crew),
        "prepared host must retain synchronized fair-crew parameters"
    );

    // FinalInit reports the host's local arrival, but OnStatusAck is what
    // starts network control after every waited-for client has reached Go
    // (src/C4Network2.cpp:2017-2077,2091-2110). The initialized game must
    // therefore remain behind the loading screen until that exact commit.
    let loading_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll_loading()
            .expect("initialize the retained prepared scenario");
        if app
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .is_some_and(|pending| pending.local_reached)
        {
            break;
        }
        assert!(
            Instant::now() < loading_deadline,
            "prepared host InitGame worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        app.loader_screen
            .as_ref()
            .expect("prepared host loader through Go wait")
            .state()
            .progress(),
        97
    );
    assert!(app.loading_state.as_ref().is_some_and(|loading| loading
        .log
        .iter()
        .any(|line| line == "Definition selection resolved")));
    assert_eq!(
                app.engine.random_seed(),
                prepared_random_seed,
                "the network host seed remains authoritative over offline defaults (status: {:?}, mode: {:?}, loading: {})",
                app.status_text,
                app.mode,
                app.loading_state.is_some(),
            );
    assert_eq!(
        (app.engine.use_fair_crew(), app.engine.fair_crew_strength(),),
        prepared_fair_crew,
    );
    assert_eq!(commands.take_status_reached(), 1);
    assert!(matches!(app.mode, AppMode::Loading));
    assert!(app.loading_state.is_some());
    assert!(app
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| wait.visible));
    assert!(
        app.message_dialogs.is_empty(),
        "the host uses its roster wait instead of the client message dialog"
    );
    events
        .send(NetworkEvent::StatusRequested(expected_go))
        .expect("queue the host session's delayed self-echo");
    app.process_network_events()
        .expect("preserve an already reached prepared barrier");
    assert_eq!(
        app.loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|pending| pending.local_reached),
        Some(true)
    );
    assert_eq!(
        commands.take_status_reached(),
        0,
        "an identical host status echo must not report local reach twice"
    );
    assert!(
                app.engine.snapshot().players.is_empty(),
                "network InitPlayers must not directly join the local player before host-issued JoinPlr controls"
            );

    events
        .send(NetworkEvent::StatusCommitted(expected_go))
        .expect("commit the exact Go barrier");
    app.process_network_events()
        .expect("apply the committed Go barrier");
    assert!(matches!(app.mode, AppMode::Running));
    assert!(app.loading_state.is_none());
    assert!(app.network_start_wait.is_none());
    assert!(
        app.network_game_advertiser.is_some(),
        "native keeps the reference listener alive during play"
    );
    let running_reference = app
        .advertised_game_reference
        .as_ref()
        .expect("running reference remains retained");
    assert_eq!(running_reference.summary().state, "Running");
    assert!(!running_reference.summary().join_allowed);
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 41,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(clonk_engine::NetworkResourceCore {
                    resource_type: 3,
                    id: 41 << 16,
                    loadable: true,
                    filename: clonk_engine::LegacyCString::from_bytes(b"Player.c4p".to_vec())
                        .expect("valid resource filename"),
                    ..clonk_engine::NetworkResourceCore::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
    assert!(app.control_player_infos.mark_joined(41, 3, 77));
    assert!(app
        .control_clients
        .apply_join(&clonk_engine::ClientJoinControlData {
            core: clonk_engine::ClientCoreControlData {
                client_id: 7,
                activated: true,
                name: clonk_engine::LegacyCString::from_bytes(b"Remote".to_vec()).unwrap(),
                nick: clonk_engine::LegacyCString::from_bytes(b"R".to_vec()).unwrap(),
                lobby_ready: true,
                ..Default::default()
            },
            by_client: 0,
        }));
    app.engine.set_teams(vec![clonk_engine::TeamInfo::new(
        4,
        "Live team",
        0x0012_3456,
    )
    .with_player_ids(vec![41])]);
    app.engine.set_max_players(13);
    app.snapshot.game_time = 123;
    app.snapshot.players = vec![clonk_engine::PlayerState {
        id: 3,
        player_info_id: 41,
        won: true,
        ..Default::default()
    }];
    app.snapshot.round_results = clonk_engine::RoundResultsState {
        league_performance: -7,
        players: vec![clonk_engine::RoundResultsPlayerState {
            player_info_id: 41,
            league_performance: 19,
            ..Default::default()
        }],
        ..Default::default()
    };
    let final_reference = game_over_host_reference(
        app.advertised_game_reference
            .as_ref()
            .expect("retained reference template"),
        app.host_join_snapshot
            .as_ref()
            .expect("live host parameters")
            .parameters
            .clone(),
        &app.control_clients,
        &app.control_player_infos,
        app.engine.teams(),
        app.engine.max_players().expect("live maximum is set"),
        &app.snapshot,
    )
    .expect("game-over reference projection validates");
    assert_eq!(final_reference.summary().state, "Running");
    assert!(!final_reference.summary().join_allowed);
    assert_eq!(final_reference.metadata().time, 123);
    assert_eq!(final_reference.metadata().league_performance, -7);
    assert_eq!(final_reference.parameters().max_players, 13);
    assert!(final_reference
        .parameters()
        .clients
        .clients
        .iter()
        .any(|client| {
            client.client_id == 7 && client.name.as_bytes() == b"Remote" && client.lobby_ready
        }));
    assert_eq!(
        final_reference.parameters().teams.teams[0].player_ids,
        vec![41]
    );
    let player_packet = &final_reference.parameters().player_infos.clients[0];
    assert_eq!(
        player_packet.flags,
        clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL
    );
    let player = &player_packet.players[0];
    assert_eq!(player.league_performance, 19);
    assert_eq!((player.game_number, player.game_join_frame), (3, 77));
    assert_ne!(player.flags & clonk_engine::PLAYER_INFO_FLAG_WON, 0);
    assert_eq!(
        player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0
    );
    assert!(player.resource.is_none());
    app.advertised_game_reference = Some(final_reference);
    app.snapshot.game_over = true;
    assert!(app.control_clients.set_lobby_ready(7, false));
    app.publish_updated_host_join_snapshot();
    let republished_player = &app
        .advertised_game_reference
        .as_ref()
        .expect("final reference survives a later live publication")
        .parameters()
        .player_infos
        .clients[0]
        .players[0];
    assert_eq!(republished_player.league_performance, 19);
    assert_ne!(
        republished_player.flags & clonk_engine::PLAYER_INFO_FLAG_WON,
        0
    );
    let live_player = app
        .control_player_infos
        .get(41)
        .expect("live PlayerInfo remains retained");
    assert_ne!(
        live_player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        0
    );
    assert!(live_player.resource.is_some());
}

#[test]
fn l038_network_lobby_does_not_displace_join_or_host_startup_dialog() {
    let mut joined = new_menu_app(640, 480);
    joined.open_network_game_dialog();
    joined.open_network_lobby();
    joined
        .start_sandbox_scenario(FrontendScenario::fallback())
        .expect("start joined L038 sandbox round");
    confirm_abort_dialog(&mut joined);
    assert!(matches!(joined.mode, AppMode::Menu));
    assert_eq!(joined.startup_view, StartupView::NetworkGame);
    assert_eq!(joined.last_startup_dialog, StartupDialog::NetworkGame);

    let mut hosted = l038_running_browser_sandbox(ScenarioSelectorMode::NetworkHost);
    hosted.open_network_lobby();
    confirm_abort_dialog(&mut hosted);
    assert_l038_browser_return(&hosted, ScenarioSelectorMode::NetworkHost);
    hosted
        .scensel_do_back()
        .expect("fresh hosted selector has no retained NetDlg");
    assert_eq!(hosted.startup_view, StartupView::MainMenu);
    assert_eq!(hosted.last_startup_dialog, StartupDialog::MainMenu);

    let mut reused = new_menu_app(640, 480);
    reused.open_network_game_dialog();
    reused.open_network_host_scenario_browser();
    reused
        .scensel_do_back()
        .expect("reuse the retained network dialog");
    assert_eq!(reused.startup_view, StartupView::NetworkGame);
    assert_eq!(
        reused.last_startup_dialog,
        StartupDialog::ScenarioBrowser(ScenarioSelectorMode::NetworkHost)
    );
    reused.open_network_lobby();
    reused
        .start_sandbox_scenario(FrontendScenario::fallback())
        .expect("start joined round after backing out of host selection");
    confirm_abort_dialog(&mut reused);
    assert_l038_browser_return(&reused, ScenarioSelectorMode::NetworkHost);
}

#[test]
fn finishing_recording_deletes_stale_final_infos_for_an_empty_roster() {
    let directory = tempdir().expect("record directory");
    let output_path = directory.path().join("001-Empty.c4s");
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    install_test_recording_template(&mut app, output_path.clone());
    app.recording_template
        .as_mut()
        .unwrap()
        .group
        .add_file("RecPlayerInfos.txt", b"stale final roster".to_vec())
        .unwrap();

    assert!(app.start_recording(true).expect("start empty recording"));
    assert!(app.finish_recording().is_none());

    let group = Group::open(&output_path).expect("open finished record");
    assert!(!group.exists("RecPlayerInfos.txt"));
}

#[test]
fn runtime_client_drains_ready_target_before_retargeted_ack() {
    // Native's reach predicate also requires !CtrlReady(ControlTick). If
    // target control is complete already, execute it and acknowledge the
    // later actual tick at the next cadence boundary.
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 2,
    };
    events
        .send(NetworkEvent::StatusRequested(pause))
        .expect("request runtime Pause");
    for tick in 0..=2 {
        events
            .send(NetworkEvent::ReadyTick {
                tick,
                controls: Vec::new(),
            })
            .expect("queue complete control through requested target");
    }
    for _ in 0..7 {
        app.update()
            .expect("drive queued target and cadence frames");
    }

    let reached = clonk_network::NetworkStatus {
        target_tick: 3,
        ..pause
    };
    assert_eq!(
        (app.engine.frame(), app.expected_network_control_tick()),
        (6, 3)
    );
    assert!(!app.network_control_running);
    assert_eq!(
        commands.take_framed_status_acknowledgements(),
        vec![(reached, 6)]
    );
}

#[test]
fn runtime_status_reach_uses_preflight_ready_not_raw_packet_presence() {
    // CheckCompleteCtrl advances CtrlReady only after Control.PreExecute.
    // A raw JoinPlayer blocked on its resource therefore does not prevent
    // CheckStatusReached from stopping at the barrier.
    let mut app = new_state_only_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 0, 0, 1);
    app.control_clients.register(0, true, false);
    let resource_id = 91;
    let core = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: resource_id,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"Pending.c4p".to_vec())
            .expect("valid resource filename"),
        ..Default::default()
    };
    events
        .send(NetworkEvent::ReadyTick {
            tick: 0,
            controls: vec![NetworkControl::JoinPlayer(
                clonk_engine::JoinPlayerControlData {
                    at_client: 0,
                    info_id: 9,
                    source: clonk_engine::JoinPlayerSource::Resource(core),
                    by_client: 0,
                    ..Default::default()
                },
            )],
        })
        .expect("queue raw resource-blocked target control");
    let pause = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_PAUSE,
        control_mode: 1,
        target_tick: 0,
    };
    events
        .send(NetworkEvent::StatusRequested(pause))
        .expect("request Pause at blocked control");

    app.process_network_events()
        .expect("preflight target while handling Pause");

    assert!(!app.network_control_running);
    assert!(app.network_ticks.ready.contains_key(&0));
    assert_eq!(
        app.admission_resources.status(resource_id),
        Some(&AdmissionResourceState::Loading { removed: false })
    );
    assert_eq!(
        commands.take_runtime_status_commands(),
        vec![network::TestRuntimeStatusCommand::Reached {
            status: pause,
            actual_control_tick: 0,
        }]
    );
}

#[test]
fn lobby_message_keeps_markup_timestamp_and_makes_chat_color_readable() {
    // MainDlg::OnMessage forwards the first user player's lobby color to
    // AddTextLine with fMakeReadableOnBlack=true. MultilineLabel::AddLine
    // then applies the weighted lightness floor before storing the line
    // (src/C4GameLobby.cpp:706-721; src/C4GuiLabels.cpp:293-299;
    // src/C4Gui.cpp:71-89).
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app.white_lobby_chat = false;
    app.show_log_timestamps = false;
    app.control_clients
        .replace_snapshot([message_client(0, b"Local"), message_client(7, b"Remote")]);
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                team: 4,
                color: 0x0000_0000,
                original_color: 0x0065_4321,
                ..Default::default()
            }],
            ..Default::default()
        });

    assert!(
        app.execute_message_control(message_control(MESSAGE_TYPE_NORMAL, -1, -1, b"hello", 7,))
            .displayed
    );
    let line = &app
        .classic_host_lobby
        .as_ref()
        .expect("lobby remains")
        .controller
        .logs()[0];
    assert_eq!(line.text, "<Remote> hello");
    assert_eq!(line.color, [0x65, 0x65, 0x65, 0xff]);

    assert!(app.engine.set_team_distribution(4));
    app.engine.set_team_colors(true);
    app.execute_message_control(message_control(
        MESSAGE_TYPE_NORMAL,
        -1,
        -1,
        b"hidden team color",
        7,
    ));
    let line = &app
        .classic_host_lobby
        .as_ref()
        .expect("lobby remains")
        .controller
        .logs()[1];
    assert_eq!(line.color, [0x65, 0x65, 0x65, 0xff]);

    app.engine.set_teams(vec![clonk_engine::TeamInfo::new(
        4,
        "Existing",
        0x00f4_0000,
    )]);
    app.execute_message_control(message_control(
        MESSAGE_TYPE_NORMAL,
        -1,
        -1,
        b"existing hidden team color",
        7,
    ));
    let line = &app
        .classic_host_lobby
        .as_ref()
        .expect("lobby remains")
        .controller
        .logs()[2];
    assert_eq!(line.color, [0x82, 0x60, 0x3e, 0xff]);

    app.show_log_timestamps = true;
    app.execute_message_control(message_control(MESSAGE_TYPE_SYSTEM, -1, -1, b"notice", 0));
    let line = &app
        .classic_host_lobby
        .as_ref()
        .expect("lobby remains")
        .controller
        .logs()[3];
    assert!(line.text.starts_with("<c 909090>["));
    assert!(line.text.ends_with("</c> Network: notice"));
    assert_eq!(line.color, [0xaf, 0xaf, 0xaf, 0xff]);

    app.show_log_timestamps = false;
    app.white_lobby_chat = true;
    app.execute_message_control(message_control(
        MESSAGE_TYPE_NORMAL,
        -1,
        -1,
        b"white body",
        7,
    ));
    let line = app
        .classic_host_lobby
        .as_ref()
        .expect("lobby remains")
        .controller
        .logs()
        .last()
        .expect("white-chat line");
    assert_eq!(line.text, "<Remote> <c ffffff>white body");
    assert_eq!(line.color, [0x82, 0x60, 0x3e, 0xff]);
}

#[test]
fn lobby_roster_makes_black_client_and_player_names_readable() {
    // Lobby player and client labels pass their raw lobby colors through
    // MakeColorReadableOnBlack before drawing (src/C4PlayerInfoListBox.cpp:
    // 72-87, 143, 648-685, 737-750, 824-825;
    // src/C4PlayerInfoListBox.h:176-179; src/C4Gui.cpp:71-89).
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app.control_clients
        .replace_snapshot([message_client(0, b"Local"), message_client(7, b"Remote")]);
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 41,
                player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                color: 0,
                original_color: 0,
                ..Default::default()
            }],
            ..Default::default()
        });

    app.sync_classic_lobby_roster();

    let rows = app
        .classic_host_lobby
        .as_ref()
        .expect("lobby remains")
        .controller
        .rows();
    assert!(rows
        .iter()
        .any(|row| matches!(row, LobbyRosterRow::Client(client)
                if client.id == 7 && client.color == [0x65, 0x65, 0x65, 0xff])));
    assert!(rows
        .iter()
        .any(|row| matches!(row, LobbyRosterRow::Player(player)
                if player.id == 41 && player.color == [0x65, 0x65, 0x65, 0xff])));
}

#[test]
fn direct_lobby_player_info_survives_network_game_initialization() {
    // Network PlayerInfo is applied directly in the lobby and the same
    // registry is reused when the game starts; the later synchronized
    // JoinPlayer resolves its InfoID there (src/C4Network2Players.cpp:245-269;
    // src/C4Game.cpp:2392-2423).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    let info_id = 41;
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData {
                client_id: 3,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: info_id,
                    ..Default::default()
                }],
                by_client: 0,
                ..Default::default()
            },
        )))
        .expect("queue direct lobby PlayerInfo");

    app.process_network_events()
        .expect("apply direct lobby PlayerInfo");
    assert!(app.control_player_infos.get(info_id).is_some());

    app.configure_running_state("Network".to_string(), DEFAULT_GROUND_HEIGHT);

    assert!(
        app.control_player_infos.get(info_id).is_some(),
        "network game initialization must retain lobby PlayerInfo"
    );
}

#[test]
fn client_join_data_replaces_the_lobby_participant_snapshot() {
    // Assigning Game.Parameters.Clients removes absent clients and copies
    // every authoritative C4ClientCore field before DoLobby renders the
    // participant list (src/C4Network2.cpp:1595-1602;
    // src/C4Client.cpp:284-290,321-350).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    let mut lobby = NetworkLobbyState::new(7, "stale local".to_string(), false);
    lobby.register_peer(99, "stale peer".to_string(), ParticipantKind::Player);
    app.network_lobby = Some(lobby);

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    snapshot.parameters.title = clonk_engine::LegacyCString::from_bytes(b"Caf\xe9 Arena".to_vec())
        .expect("valid legacy scenario title");
    snapshot.parameters.clients = clonk_network::JoinClientRegistrySnapshot {
        clients: vec![
            clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: clonk_engine::LegacyCString::from_bytes(b"Exact Andr\xe9".to_vec())
                    .expect("valid host name"),
                lobby_ready: true,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 7,
                name: clonk_engine::LegacyCString::from_bytes(b"Exact local".to_vec())
                    .expect("valid local name"),
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 9,
                observer: true,
                name: clonk_engine::LegacyCString::from_bytes(b"Exact observer".to_vec())
                    .expect("valid observer name"),
                lobby_ready: false,
                ..Default::default()
            },
        ],
        local_client_id: Some(7),
    };
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: snapshot.dynamic_tick,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    event_tx
        .send(NetworkEvent::JoinData(join_data))
        .expect("queue JoinData");

    app.process_network_events().expect("apply JoinData");

    let participants = &app
        .network_lobby
        .as_ref()
        .expect("client remains in lobby")
        .participants;
    assert_eq!(participants.keys().copied().collect::<Vec<_>>(), [0, 7, 9]);
    assert_eq!(participants[&0].name, "Exact Andr\u{e9}");
    assert!(participants[&0].ready);
    assert_eq!(participants[&7].name, "Exact local");
    assert!(!participants[&7].ready);
    assert_eq!(participants[&9].name, "Exact observer");
    assert_eq!(participants[&9].kind, ParticipantKind::Observer);
    assert!(!participants[&9].ready);
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("client remains in lobby")
            .scenario_label(),
        "Caf\u{e9} Arena"
    );
    assert_eq!(app.scenario_label, "Caf\u{e9} Arena");
    assert_eq!(
        app.control_clients
            .state(0)
            .expect("raw host core remains registered")
            .name
            .as_bytes(),
        b"Exact Andr\xe9"
    );
    assert_eq!(
        app.pending_network_join_data
            .as_ref()
            .expect("raw JoinData remains pending")
            .parameters
            .title
            .as_bytes(),
        b"Caf\xe9 Arena"
    );
}

#[test]
fn lobby_ready_gate_waits_for_registered_non_player_resource() {
    // MainDlg::UpdateResourceProgress keeps Ready disabled while any
    // registered non-player C4Network2Res is incomplete
    // (src/C4GameLobby.cpp:779-802).
    let mut resources = AdmissionResourceStore::default();
    let scenario = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 41,
        loadable: true,
        ..Default::default()
    };

    resources.register_lobby_resource(&scenario);
    assert!(!resources.lobby_ready_available());

    resources.mark_complete(scenario.id, PathBuf::from("Scenario.c4s"));
    assert!(resources.lobby_ready_available());
}

#[test]
fn lobby_ready_gate_ignores_incomplete_player_resource() {
    // MainDlg::UpdateResourceProgress explicitly excludes NRT_Player from
    // the resource-completeness gate (src/C4GameLobby.cpp:781-790).
    let mut resources = AdmissionResourceStore::default();
    resources.register_lobby_resource(&clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 42,
        loadable: true,
        ..Default::default()
    });

    assert!(resources.lobby_ready_available());
}

#[test]
fn lobby_preload_gate_uses_one_shared_eligibility_edge_and_success_is_one_shot() {
    let mut automatic = LobbyPreloadState::new(true);
    assert!(!automatic.synchronize(false, true));
    assert!(!automatic.synchronize(true, false));
    assert!(automatic.synchronize(true, true));
    assert!(!automatic.synchronize(true, true));
    automatic.record_result(false);
    assert!(
        automatic.eligible,
        "failure remains eligible but does not spin"
    );
    assert!(!automatic.synchronize(true, true));
    assert!(!automatic.synchronize(false, true));
    assert!(automatic.synchronize(true, true));

    let mut manual = LobbyPreloadState::new(false);
    assert!(!manual.synchronize(true, true));
    assert!(manual.manual_button_present);
    assert!(manual.eligible);
    manual.record_result(true);
    assert!(manual.spent);
    assert!(!manual.eligible);
    assert!(!manual.manual_button_present);
    assert!(!manual.synchronize(true, true));
    manual.reset_for_context();
    assert!(!manual.spent);
    assert!(manual.manual_button_present);
    assert!(!manual.synchronize(true, true));
    assert!(manual.eligible);
}

#[test]
fn queued_client_lobby_preload_cleanup_removes_uncommitted_staging_file() {
    let directory = tempdir().expect("client preload staging directory");
    let staging_path = directory.path().join(".Combined7.c4s.preload.tmp");
    fs::write(&staging_path, b"staged scenario").expect("write staged scenario");
    let artifact = ClientLobbyPreloadArtifact {
        client_id: 7,
        dynamic_resource_id: 23,
        random_seed: 41,
        scenario: None,
        material_groups: Vec::new(),
        staging_path: Some(staging_path.clone()),
    };
    let (sender, receiver) = mpsc::channel();
    assert!(sender.send(artifact).is_ok(), "queue completed preload");

    drop(receiver);

    assert!(
        !staging_path.exists(),
        "dropping an unread completed result must retire its staging file"
    );
}

#[test]
fn clearing_client_lobby_preload_removes_only_its_committed_combined_file() {
    let directory = tempdir().expect("client preload combined directory");
    let owned_path = directory.path().join("Combined7.c4s");
    fs::write(&owned_path, b"preload-owned scenario").expect("write committed scenario");
    let mut app = new_state_only_menu_app(320, 200);
    app.client_combined_scenario_path = Some(owned_path.clone());
    app.client_combined_preload_file.replace(owned_path.clone());
    app.network_material_resource_groups = Some(Vec::new());

    app.clear_lobby_preload();

    assert!(!owned_path.exists());
    assert!(app.client_combined_scenario_path.is_none());
    assert!(!app.client_combined_preload_file.is_owned());
    assert!(app.network_material_resource_groups.is_none());

    let existing_path = directory.path().join("Combined8.c4s");
    fs::write(&existing_path, b"pre-existing scenario").expect("write existing scenario");
    app.client_combined_scenario_path = Some(existing_path.clone());
    app.clear_lobby_preload();

    assert!(
        existing_path.exists(),
        "clearing preload state must not remove a pack it did not create"
    );
    assert!(app.client_combined_scenario_path.is_none());

    let dropped_path = directory.path().join("Combined9.c4s");
    fs::write(&dropped_path, b"drop-owned scenario").expect("write drop-owned scenario");
    {
        let mut dropped_app = new_state_only_menu_app(320, 200);
        dropped_app.client_combined_scenario_path = Some(dropped_path.clone());
        dropped_app
            .client_combined_preload_file
            .replace(dropped_path.clone());
    }
    assert!(
        !dropped_path.exists(),
        "dropping the app must retire its preload-owned combined pack"
    );
}

#[test]
fn configured_automatic_lobby_preload_runs_off_thread_and_activation_reuses_it() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated preload config");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "Preloading", "1")
        .expect("enable automatic preloading");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let mut staged = prepare_tutorial_host_lobby(&app, repository);
    app.loader_screen = staged.loader_screen.take();
    app.staged_network_host_scenario = Some(staged);
    let (manager, _events) = NetworkManager::test_stub();
    let mode = NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name: "Exact Host".to_string(),
        prepared: None,
    });
    let (lobby, options) = app
        .build_classic_host_lobby(&mode, &manager)
        .expect("build exact host lobby");
    assert!(lobby.preload.automatic);
    assert!(!lobby.preload.manual_button_present);
    app.classic_host_lobby = Some(lobby);
    app.scenario_game_options = options;
    app.network_mode = Some(mode);
    app.network = Some(manager);

    app.sync_classic_lobby_resource_ready();
    let preload = app
        .classic_host_lobby
        .as_ref()
        .map(|lobby| lobby.preload)
        .expect("live host lobby");
    assert!(preload.spent, "successful worker launch is one-shot");
    assert!(app.lobby_preload_task.is_some());

    let deadline = Instant::now() + Duration::from_secs(180);
    while app.lobby_preload_task.is_some() {
        app.poll_lobby_preload().expect("poll lobby preload");
        assert!(Instant::now() < deadline, "lobby preload did not finish");
        thread::yield_now();
    }
    let artifact = app
        .lobby_preload_artifact
        .as_ref()
        .expect("completed preload artifact");
    let expected_hud = Arc::clone(&artifact.game_graphics.hud_graphics);
    let expected_textures = Arc::clone(&artifact.material_texture_images);
    let expected_render_info = Arc::clone(&artifact.material_render_info);

    app.network = None;
    app.network_mode = None;
    app.classic_host_lobby = None;
    let staged = app
        .staged_network_host_scenario
        .take()
        .expect("same staged scenario");
    app.activate_loaded_scenario(staged.frontend, &staged.scenario)
        .expect("activate preloaded scenario");
    assert!(Arc::ptr_eq(
        &expected_hud,
        &app.active_game_graphics
            .as_ref()
            .expect("active game graphics")
            .hud_graphics
    ));
    assert!(Arc::ptr_eq(
        &expected_textures,
        &app.material_texture_images
    ));
    assert!(Arc::ptr_eq(
        &expected_render_info,
        &app.material_render_info
    ));
}

#[test]
fn catalog_host_lobby_preload_is_eligible_and_caches_the_selected_scenario() {
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated catalog-host preload config");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let frontend = tutorial_frontend(repository);
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.scenario_catalog
        .insert(frontend.identifier.clone(), frontend.clone());
    let mut lobby = NetworkLobbyState::new(0, "Catalog Host".to_string(), true)
        .with_preloading(true, LobbyLabels::default());
    lobby.select_scenario(&frontend.identifier, &frontend.title);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name: "Catalog Host".to_string(),
        prepared: None,
    }));
    let (manager, _events) = NetworkManager::test_stub();
    app.network = Some(manager);

    app.sync_classic_lobby_resource_ready();
    assert!(app.lobby_preload_task.is_some());
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.lobby_preload_task.is_some() {
        app.poll_lobby_preload().expect("poll catalog preload");
        assert!(Instant::now() < deadline, "catalog preload did not finish");
        thread::yield_now();
    }
    assert!(
        app.lobby_preload_artifact
            .as_ref()
            .and_then(|artifact| artifact.catalog_host.as_ref())
            .is_some_and(|catalog_host| catalog_host.scenario.is_some()),
        "catalog preload retains the parsed scenario for the start path"
    );
    let catalog_host = app
        .lobby_preload_artifact
        .as_mut()
        .and_then(|artifact| artifact.catalog_host.as_mut())
        .unwrap();
    let mut stale_key = catalog_host.key.clone();
    stale_key.languages.push("DE".to_string());
    assert!(catalog_host.take_matching_scenario(&stale_key).is_none());
    assert!(
        catalog_host.scenario.is_some(),
        "a changed raw load key must leave the cached scenario untouched"
    );
    let artifact = app.lobby_preload_artifact.as_ref().unwrap();
    let expected_hud = Arc::clone(&artifact.game_graphics.hud_graphics);
    let expected_textures = Arc::clone(&artifact.material_texture_images);
    let expected_render_info = Arc::clone(&artifact.material_render_info);

    let definition_load = app.scenario_seed_definition_load();
    app.begin_loading_scenario(frontend, definition_load)
        .expect("start the preloaded catalog scenario");

    assert!(
        app.lobby_preload_artifact
            .as_ref()
            .and_then(|artifact| artifact.catalog_host.as_ref())
            .is_some_and(|catalog_host| catalog_host.scenario.is_none()),
        "the regular loading path consumes the cached scenario"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.loading_state.is_some() {
        app.poll_loading().expect("finish catalog scenario loading");
        assert!(
            Instant::now() < deadline,
            "preloaded catalog scenario did not activate"
        );
        thread::yield_now();
    }
    assert!(Arc::ptr_eq(
        &expected_hud,
        &app.active_game_graphics
            .as_ref()
            .expect("active catalog graphics")
            .hud_graphics
    ));
    assert!(Arc::ptr_eq(
        &expected_textures,
        &app.material_texture_images
    ));
    assert!(Arc::ptr_eq(
        &expected_render_info,
        &app.material_render_info
    ));
}

#[test]
fn lobby_preload_launch_failure_logs_red_without_error_sound_and_stays_retryable() {
    let mut app = new_state_only_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    {
        let lobby = app.classic_host_lobby.as_mut().unwrap();
        lobby.preload = LobbyPreloadState::new(false);
        assert!(!lobby.preload.synchronize(true, true));
        lobby.controller.set_preload_button_state(true, true);
    }
    let sounds_before = app.ui_sound_log.len();

    app.request_lobby_preload();

    let lobby = app.classic_host_lobby.as_ref().unwrap();
    assert!(lobby.preload.eligible);
    assert!(lobby.preload.manual_button_present);
    assert_eq!(app.ui_sound_log.len(), sounds_before);
    assert_eq!(
        lobby.controller.logs().last(),
        Some(&LobbyLogLine {
            text: "Preloading error.".to_string(),
            color: [255, 32, 32, 255],
        })
    );
}

#[test]
fn lobby_ready_toggle_is_disabled_while_non_player_resource_loads() {
    // UpdatePreloadingGUIState disables the Ready checkbox until every
    // registered non-player resource is complete, so OnReadyCheck cannot
    // broadcast or mutate local readiness (src/C4GameLobby.cpp:779-824,
    // 329-343).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local client".to_string(), false));
    app.admission_resources
        .register_lobby_resource(&clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Scenario as u8,
            id: 43,
            loadable: true,
            ..Default::default()
        });

    app.process_lobby_action(LobbyAction::ToggleReady)
        .expect("disabled Ready is ignored");

    assert!(!app.network_lobby.as_ref().unwrap().local_ready());
    assert!(commands.take_submitted_ready_checks().is_empty());
}

#[test]
fn join_data_resources_keep_lobby_ready_disabled_until_completion_events() {
    // InitClient registers GameRes and Dynamic from JoinData before DoLobby;
    // UpdateResourceProgress then observes those same resources until all
    // non-player loads finish (src/C4Network2.cpp:1612-1620;
    // src/C4GameLobby.cpp:779-802).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Observer",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Observer".to_string(), false));
    let resource = |resource_type, id| clonk_engine::NetworkResourceCore {
        resource_type,
        id,
        loadable: true,
        ..Default::default()
    };
    let scenario = resource(clonk_network::HostResourceType::Scenario as u8, 44);
    let dynamic = resource(clonk_network::HostResourceType::Dynamic as u8, 45);
    let definitions = resource(clonk_network::HostResourceType::Definitions as u8, 46);
    let player = resource(clonk_network::HostResourceType::Player as u8, 47);
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    snapshot.parameters.clients = clonk_network::JoinClientRegistrySnapshot {
        clients: vec![
            clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                ..Default::default()
            },
            clonk_engine::ClientCoreControlData {
                client_id: 7,
                observer: true,
                ..Default::default()
            },
        ],
        local_client_id: Some(7),
    };
    snapshot.parameters.scenario = scenario.clone();
    snapshot.parameters.game_resources = vec![definitions.clone()];
    snapshot.parameters.player_infos = clonk_network::PlayerInfoListSnapshot {
        last_player_id: 1,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(player.clone()),
                ..Default::default()
            }],
        }],
    };
    snapshot.dynamic = dynamic.clone();
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: snapshot.dynamic_tick,
        status: host_config.initial_status,
        dynamic,
        parameters: snapshot.parameters,
    };
    event_tx
        .send(NetworkEvent::JoinData(join_data))
        .expect("queue JoinData");

    app.process_network_events().expect("install JoinData");
    assert!(!app.admission_resources.lobby_ready_available());
    assert_eq!(
        app.admission_resources.status(player.id),
        Some(&AdmissionResourceState::Loading { removed: false })
    );

    for (core, path) in [
        (scenario, "Scenario.c4s"),
        (definitions, "Objects.c4d"),
        (snapshot.dynamic, "Dynamic.c4s"),
    ] {
        event_tx
            .send(NetworkEvent::ResourceComplete {
                resource_id: core.id,
                core,
                path: PathBuf::from(path),
                local: false,
            })
            .expect("queue resource completion");
    }
    app.process_network_events()
        .expect("apply resource completions");

    assert!(app.admission_resources.lobby_ready_available());
    assert_eq!(
        app.admission_resources.status(player.id),
        Some(&AdmissionResourceState::Loading { removed: false }),
        "an incomplete player resource is tracked but does not block lobby readiness"
    );
}

#[test]
fn host_ready_check_request_aborts_countdown_and_clears_only_nonhosts() {
    // RequestReadyCheck aborts an active countdown, leaves the host's
    // readiness untouched, clears every non-host, and broadcasts Request
    // from the local host (src/C4GameLobby.cpp:1072-1088).
    let mut app = new_state_only_menu_app(320, 200);
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.register_peer(7, "Player".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Observer".to_string(), ParticipantKind::Observer);
    for participant in lobby.participants.values_mut() {
        participant.ready = true;
    }
    lobby.countdown = Some(5);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.host_lobby_countdown = Some(HostLobbyCountdown::new());
    for client_id in [0, 7, 9] {
        app.control_clients.register(client_id, true, false);
        assert!(app.control_clients.set_lobby_ready(client_id, true));
    }
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);

    assert!(app
        .request_lobby_ready_check_at(Instant::now())
        .expect("host ready check starts"));

    let lobby = app.network_lobby.as_ref().unwrap();
    assert!(lobby.participants[&0].ready);
    assert!(!lobby.participants[&7].ready);
    assert!(!lobby.participants[&9].ready);
    assert!(app.control_clients.state(0).unwrap().lobby_ready);
    assert!(!app.control_clients.state(7).unwrap().lobby_ready);
    assert!(!app.control_clients.state(9).unwrap().lobby_ready);
    assert_eq!(lobby.countdown, None);
    assert!(app.host_lobby_countdown.is_none());
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }]
    );
}

#[test]
fn host_ready_check_uses_cpp_ten_second_default_cooldown() {
    // /readycheck calls Config.Cooldowns.ReadyCheck.TryReset before it
    // mutates lobby state; the stock configured default is ten seconds
    // (src/C4GameLobby.cpp:614-627; src/C4Config.cpp:394-400;
    // src/C4Cooldown.h:54-64).
    let mut app = new_state_only_menu_app(320, 200);
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.register_peer(7, "Player".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).unwrap().ready = true;
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let now = Instant::now();

    assert!(app.request_lobby_ready_check_at(now).unwrap());
    assert_eq!(commands.take_submitted_ready_checks().len(), 1);
    app.network_lobby
        .as_mut()
        .unwrap()
        .participants
        .get_mut(&7)
        .unwrap()
        .ready = true;
    app.host_lobby_countdown = Some(HostLobbyCountdown::new());

    assert!(!app
        .request_lobby_ready_check_at(now + Duration::from_secs(9))
        .unwrap());
    assert!(app.network_lobby.as_ref().unwrap().participants[&7].ready);
    assert!(app.host_lobby_countdown.is_some());
    assert!(commands.take_submitted_ready_checks().is_empty());
    assert_eq!(app.status_text, "Too early! Please wait 1 seconds.");

    assert!(app
        .request_lobby_ready_check_at(now + Duration::from_secs(10))
        .unwrap());
    assert!(!app.network_lobby.as_ref().unwrap().participants[&7].ready);
    assert!(app.host_lobby_countdown.is_none());
    assert_eq!(commands.take_submitted_ready_checks().len(), 1);
}

#[test]
fn ready_check_config_clamps_below_cpp_five_second_minimum() {
    // mkParAdapt compiles Config.Cooldowns.ReadyCheck with a five-second
    // minimum, independently of its ten-second missing-value default
    // (src/C4Config.cpp:394-400; src/C4Cooldown.h:85-93).
    let cooldown = LobbyReadyCheckCooldown::from_config_seconds(2);

    assert_eq!(cooldown.duration, Duration::from_secs(5));
}

#[test]
fn ready_check_cooldown_reads_cpp_cooldowns_config_key() {
    // C4ConfigCooldowns compiles this value as [Cooldowns] ReadyCheck
    // (src/C4Config.cpp:394-400,865).
    let mut config = Config::new();
    config.set_in(Some("Cooldowns"), "ReadyCheck", "17");

    let cooldown = lobby_ready_check_cooldown_from_config(Some(&config));

    assert_eq!(cooldown.duration, Duration::from_secs(17));
}

#[test]
fn l030_ready_check_toast_config_uses_cpp_boolean_grammar_and_default() {
    assert!(ready_check_toasts_enabled_from_config(b""));
    assert!(ready_check_toasts_enabled_from_config(
        b"[Toasts]\nReadyCheck=true\n"
    ));
    assert!(!ready_check_toasts_enabled_from_config(
        b"[Toasts]\nReadyCheck=false\n"
    ));
    assert!(
        ready_check_toasts_enabled_from_config(b"[Toasts]\nReadyCheck=invalid\n"),
        "malformed values retain C++'s enabled default"
    );
}

#[test]
fn l030_unfocused_ready_check_queues_one_enabled_desktop_notification() {
    fn client_app(window_active: bool, toasts_enabled: bool) -> GameApp {
        let mut app = new_menu_app(320, 200);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
        app.window_active = window_active;
        app.ready_check_toasts_enabled = toasts_enabled;
        app
    }

    let packet = clonk_network::ReadyCheckPacket {
        client_id: 0,
        data: clonk_network::ReadyCheckData::Request,
    };
    let mut app = client_app(false, true);
    app.handle_lobby_ready_check_request(packet)
        .expect("open ready-check prompt");

    assert_eq!(
        app.take_desktop_notification(),
        Some(DesktopNotification::new(
            "Are you ready?",
            "The host wants to know whether you're ready.\n15 seconds remaining.",
            Duration::from_secs(15),
        ))
    );
    app.handle_lobby_ready_check_request(packet)
        .expect("ignore duplicate prompt");
    assert!(app.take_desktop_notification().is_none());

    for (window_active, toasts_enabled) in [(true, true), (false, false)] {
        let mut app = client_app(window_active, toasts_enabled);
        app.handle_lobby_ready_check_request(packet)
            .expect("open ready-check prompt without a desktop alert");
        assert!(app.take_desktop_notification().is_none());
    }
}

#[test]
fn l030_desktop_notification_delivery_failure_is_nonfatal() {
    let mut app = new_state_only_menu_app(320, 200);
    app.pending_desktop_notifications
        .push_back(DesktopNotification::new(
            "Ready check",
            "Synthetic failure",
            Duration::from_secs(15),
        ));
    let mut attempts = 0;

    deliver_desktop_notifications(&mut app, |_| {
        attempts += 1;
        Err(anyhow!("synthetic notification backend failure"))
    });

    assert_eq!(attempts, 1);
    assert!(app.take_desktop_notification().is_none());
}

#[test]
fn client_ready_check_request_replies_not_ready_while_resources_load() {
    // HandleReadyCheck clears every non-host readiness flag, and when
    // MainDlg::CanBeReady is false it skips the dialog and immediately
    // broadcasts NotReady for the local client
    // (src/C4Network2.cpp:1635-1688).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let mut lobby = NetworkLobbyState::new(7, "Client".to_string(), false);
    lobby.register_peer(9, "Peer".to_string(), ParticipantKind::Player);
    for participant in lobby.participants.values_mut() {
        participant.ready = true;
    }
    app.network_lobby = Some(lobby);
    app.admission_resources
        .register_lobby_resource(&clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Scenario as u8,
            id: 51,
            loadable: true,
            ..Default::default()
        });
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }))
        .expect("queue host ready check request");

    app.process_network_events().expect("handle ready check");

    let lobby = app.network_lobby.as_ref().unwrap();
    assert!(lobby.participants[&0].ready);
    assert!(!lobby.participants[&7].ready);
    assert!(!lobby.participants[&9].ready);
    assert!(app.message_dialogs.is_empty());
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::NotReady,
        }]
    );
}

#[test]
fn complete_client_ready_check_opens_one_exact_fifteen_second_prompt() {
    // A resource-complete client creates one ReadyCheckDialog for fifteen
    // seconds. While its nested modal loop handles packets, another Request
    // is ignored before readiness is cleared again
    // (src/C4Network2.cpp:129-173,1635-1643,1657-1688).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let mut lobby = NetworkLobbyState::new(7, "Client".to_string(), false);
    lobby.register_peer(9, "Peer".to_string(), ParticipantKind::Player);
    for participant in lobby.participants.values_mut() {
        participant.ready = true;
    }
    app.network_lobby = Some(lobby);
    let request = NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
        client_id: 0,
        data: clonk_network::ReadyCheckData::Request,
    });
    event_tx.send(request).expect("queue host request");

    app.process_network_events().expect("open ready prompt");

    assert_eq!(app.message_dialogs.len(), 1);
    let prompt = &app.message_dialogs[0].state;
    assert_eq!(prompt.caption(), "Are you ready?");
    assert_eq!(
        prompt.message(),
        "The host wants to know whether you're ready.|15 seconds remaining."
    );
    assert_eq!(
        prompt.buttons(),
        clonk_frontend::message_dialog::MessageDialogButtons::YES_NO
    );
    assert_eq!(
        prompt.icon(),
        clonk_frontend::message_dialog::MessageDialogIcon::Standard(30)
    );
    assert_eq!(prompt.focused_button(), None);
    assert!(commands.take_submitted_ready_checks().is_empty());

    app.network_lobby
        .as_mut()
        .unwrap()
        .participants
        .get_mut(&9)
        .unwrap()
        .ready = true;
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }))
        .expect("queue duplicate request");
    app.process_network_events()
        .expect("ignore duplicate request");

    assert_eq!(app.message_dialogs.len(), 1);
    assert!(app.network_lobby.as_ref().unwrap().participants[&9].ready);
    assert!(commands.take_submitted_ready_checks().is_empty());
}

#[test]
fn accepting_ready_check_sets_local_ready_and_submits_cpp_ready_reply() {
    // ShowModalDlg(true) broadcasts Ready for the local client, checks the
    // Ready checkbox, and then applies that local C4Client transition
    // (src/C4Network2.cpp:1673-1695,1721-1729).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }))
        .expect("queue host request");
    app.process_network_events().expect("open ready prompt");

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .expect("accept ready check");

    assert!(app.message_dialogs.is_empty());
    assert!(app.network_lobby.as_ref().unwrap().local_ready());
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }]
    );
}

#[test]
fn declining_ready_check_keeps_local_unready_and_submits_cpp_reply() {
    // ShowModalDlg(false), including the explicit No button, broadcasts
    // NotReady for the local client and leaves the checkbox clear
    // (src/C4Network2.cpp:1673-1695).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    let mut lobby = NetworkLobbyState::new(7, "Client".to_string(), false);
    lobby.participants.get_mut(&7).unwrap().ready = true;
    app.network_lobby = Some(lobby);
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }))
        .expect("queue host request");
    app.process_network_events().expect("open ready prompt");

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .expect("decline ready check");

    assert!(app.message_dialogs.is_empty());
    assert!(!app.network_lobby.as_ref().unwrap().local_ready());
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::NotReady,
        }]
    );
}

#[test]
fn ready_check_prompt_sends_no_reply_after_lobby_ends() {
    // The modal loop may outlive the lobby. C++ rechecks
    // C4Network2::isLobbyActive after the dialog closes and returns before
    // broadcasting or applying the local ready state when the game has
    // already started (src/C4Network2.cpp:1673-1695).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }))
        .expect("queue host request");
    app.process_network_events().expect("open ready prompt");

    app.mode = AppMode::Running;
    app.network_lobby = None;
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .expect("close stale ready prompt");

    assert!(app.message_dialogs.is_empty());
    assert!(commands.take_submitted_ready_checks().is_empty());
}

#[test]
fn go_status_request_deletes_client_lobby_and_suppresses_stale_ready_reply() {
    // HandleStatus installs GS_Go before resource preparation finishes.
    // DoLobby therefore closes and deletes pLobby immediately, and a
    // modal ready-check callback that unwinds afterward cannot broadcast
    // a reply (src/C4Network2.cpp:475-515,1673-1695,2010-2029).
    let mut app = new_menu_app(320, 200);
    let fonts = app
        .assets
        .clonk_fonts
        .clone()
        .expect("synthetic client loader fonts");
    app.loader_screen = Some(
        LoaderScreen::new(
            LoaderSelection::startup("LoaderClientGo.png").expect("valid client loader selection"),
            ImageData::new(1, 1, vec![7, 8, 9, 255]),
            LoaderResources::new(fonts, ImageData::new(3, 1, vec![255; 12]))
                .expect("valid client loader resources"),
            LoaderState::initial("Loading"),
        )
        .expect("valid client loader"),
    );
    app.loader_error = None;
    app.loader_render_error = None;
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }))
        .expect("queue host request");
    app.process_network_events().expect("open ready prompt");
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Stale lobby child",
            "Lobby",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .expect("open a second lobby-owned child");
    app.open_game_option_input_dialog(GameOptionInputDialogRequest {
        kind: GameOptionInputKind::Password,
        message: "Password",
        caption: "Password",
        icon: clonk_frontend::game_option_buttons::GameOptionIcon::Locked,
        max_text: 31,
        initial_text: String::new(),
        chat_layout: false,
    })
    .expect("open lobby option input");
    let go = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        control_mode: 2,
        target_tick: 23,
    };
    event_tx
        .send(NetworkEvent::StatusRequested(go))
        .expect("queue GO status request");
    app.process_network_events()
        .expect("retain pending client preparation");
    assert!(matches!(app.mode, AppMode::Loading));
    assert_eq!(app.pending_client_start_status, Some(go));
    assert!(
        app.network_lobby.is_none(),
        "native pLobby is deleted as soon as GS_Go is installed"
    );
    assert!(
        app.message_dialogs.is_empty(),
        "DoLobby closes lobby-owned dialogs while entering the loader"
    );
    assert!(
        app.game_option_input_dialog.is_none(),
        "CloseAllDialogs also removes lobby option input"
    );
    assert!(commands.take_submitted_ready_checks().is_empty());
    let mut frame = vec![0x4c; 320 * 200 * 4];
    app.render(&mut frame)
        .expect("post-lobby GO renders the loader instead of a dead lobby");
    assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn ready_check_prompt_counts_down_and_times_out_not_ready_at_fifteen_seconds() {
    // ReadyCheckDialog is a TimedDialog{15}; each one-second callback
    // updates the remaining text and the fifteenth closes false, producing
    // NotReady (src/C4Network2.cpp:129-146,1673-1695;
    // src/C4GuiDialogs.cpp:1279-1299).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 0,
            data: clonk_network::ReadyCheckData::Request,
        }))
        .expect("queue host request");
    app.process_network_events().expect("open ready prompt");

    for remaining in (1..LOBBY_READY_CHECK_PROMPT_SECONDS).rev() {
        app.sec1_timer().expect("advance ready-check prompt");
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(
            app.message_dialogs[0].state.message(),
            lobby_ready_check_message(remaining)
        );
    }
    assert!(commands.take_submitted_ready_checks().is_empty());

    app.sec1_timer().expect("expire ready-check prompt");

    assert!(app.message_dialogs.is_empty());
    assert!(!app.network_lobby.as_ref().unwrap().local_ready());
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::NotReady,
        }]
    );
}

#[test]
fn lobby_ready_toggle_broadcasts_cpp_ready_packet() {
    // MainDlg::OnReadyCheck broadcasts the local Client ID and new
    // Ready/NotReady state, then updates the local lobby row
    // (src/C4GameLobby.cpp:329-343).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local client".to_string(), false));

    app.process_lobby_action(LobbyAction::ToggleReady)
        .expect("toggle ready");
    assert!(app.network_lobby.as_ref().unwrap().local_ready());
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }]
    );

    app.process_lobby_action(LobbyAction::ToggleReady)
        .expect("toggle not ready");
    assert!(!app.network_lobby.as_ref().unwrap().local_ready());
    assert_eq!(
        commands.take_submitted_ready_checks(),
        vec![clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::NotReady,
        }]
    );
}

#[test]
fn joined_lobby_long_countdown_keeps_game_options_unlocked_and_renderable() {
    // MainDlg maps values above AlmostStartCountdownTime to
    // CDS_LongCountdown. SetCountdownState immediately passes
    // IsCountdown()==false to the retained game-option strip because only
    // the final ten seconds and Start lock it
    // (src/C4GameLobby.cpp:346-425; src/C4GameLobby.h:93).
    let mut app = new_real_menu_app(320, 200);
    app.graphics.set_runtime_sprite_filtering(1.0, false);
    app.configure_native_startup_fonts(1.0, false);
    app.startup_view = StartupView::NetworkLobby;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local client".to_string(), false));
    app.sync_network_lobby_game_option_state();

    event_tx
        .send(NetworkEvent::LobbyCountdown(
            clonk_network::LobbyCountdownPacket::new(12),
        ))
        .expect("queue long lobby countdown");
    app.process_network_events()
        .expect("apply long lobby countdown");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .countdown(),
        clonk_frontend::game_lobby::LobbyCountdownState::Long { seconds: 12 }
    );
    assert!(!app.scenario_game_options.values().countdown);

    app.sync_network_lobby_game_option_state();
    assert!(
        !app.scenario_game_options.values().countdown,
        "ordinary lobby synchronization must preserve LongCountdown's unlocked strip"
    );
    let assets = Arc::clone(&app.assets);
    let (_, options) = app
        .network_lobby
        .as_mut()
        .expect("joined lobby")
        .classic_render_state(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
        )
        .expect("long countdown remains renderable");
    assert!(!options.values().countdown);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.render(&mut frame)
        .expect("render joined lobby during a long countdown");
    let presentation = retained_test_presentation(&app);
    let retained = app
        .render_retained_gpu_frame(presentation)
        .expect("retain joined lobby during a long countdown");
    assert_retained_frame_has_commands("joined lobby long countdown", &retained);

    event_tx
        .send(NetworkEvent::LobbyCountdown(
            clonk_network::LobbyCountdownPacket::new(10),
        ))
        .expect("queue final lobby countdown");
    app.process_network_events()
        .expect("apply final lobby countdown");
    assert_eq!(
        app.network_lobby
            .as_ref()
            .expect("joined lobby")
            .controller
            .countdown(),
        clonk_frontend::game_lobby::LobbyCountdownState::Final { seconds: 10 }
    );
    assert!(app.scenario_game_options.values().countdown);
    let final_frame = app
        .render_retained_gpu_frame(presentation)
        .expect("retain joined lobby during the final countdown");
    assert_retained_frame_has_commands("joined lobby final countdown", &final_frame);
}

#[test]
fn inbound_lobby_countdown_updates_cpp_countdown_start_and_abort_states() {
    // MainDlg maps -1 to no countdown, zero to the start transition, and
    // values through ten to the active countdown state
    // (src/C4GameLobby.cpp:392-418).
    let mut app = new_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.startup_view = StartupView::NetworkLobby;
    app.show_log_timestamps = false;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local client".to_string(), false));

    for (countdown, expected, expected_controller, expected_logs) in [
        (
            5,
            Some(5),
            clonk_frontend::game_lobby::LobbyCountdownState::Final { seconds: 5 },
            vec!["The game will start in 5 seconds."],
        ),
        (
            0,
            Some(0),
            clonk_frontend::game_lobby::LobbyCountdownState::Start,
            vec!["The game will start in 5 seconds."],
        ),
        (
            -1,
            None,
            clonk_frontend::game_lobby::LobbyCountdownState::None,
            vec!["The game will start in 5 seconds.", "Game start aborted."],
        ),
    ] {
        event_tx
            .send(NetworkEvent::LobbyCountdown(
                clonk_network::LobbyCountdownPacket::new(countdown),
            ))
            .expect("queue lobby countdown");
        app.process_network_events().expect("apply lobby countdown");
        let retained_logs = {
            let lobby = app.network_lobby.as_ref().expect("joined lobby");
            assert_eq!(lobby.countdown, expected);
            assert_eq!(lobby.controller.countdown(), expected_controller);
            assert_eq!(
                lobby
                    .logs
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                expected_logs,
            );
            assert_eq!(lobby.logs, lobby.controller.logs());
            lobby.logs.clone()
        };
        let assets = Arc::clone(&app.assets);
        let (projection, _) = app
            .network_lobby
            .as_mut()
            .expect("joined lobby")
            .classic_render_state(
                app.graphics.surface(),
                assets.as_ref(),
                &app.scenario_game_options,
            )
            .expect("project retained joined countdown");
        assert_eq!(projection.countdown(), expected_controller);
        assert_eq!(projection.logs(), retained_logs);
        assert_eq!(
            app.network_lobby
                .as_ref()
                .expect("joined lobby")
                .controller
                .logs(),
            retained_logs,
        );
        assert!(matches!(app.mode, AppMode::Menu));
        assert!(app.host_lobby_countdown.is_none());
        assert!(
            !app.sec1_timer().expect("pulse client second timer"),
            "client packet state never installs a one-second callback"
        );
        assert_eq!(app.network_lobby.as_ref().unwrap().countdown, expected);
    }
}

#[test]
fn host_start_begins_default_cpp_lobby_countdown_without_leaving_lobby() {
    // MainDlg::OnRunBtn starts Config.Lobby.CountdownTime, whose stock
    // value is five; Countdown broadcasts and locally applies that initial
    // value before installing its one-second callback
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:442-472,1111-1131;
    // src/C4Config.cpp:276).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);

    app.process_lobby_action(LobbyAction::StartGame)
        .expect("begin host countdown");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, Some(5));
    assert_eq!(
        app.host_lobby_countdown,
        Some(HostLobbyCountdown {
            remaining: DEFAULT_LOBBY_COUNTDOWN_SECONDS,
        })
    );
    assert!(matches!(app.mode, AppMode::Menu));
}

#[test]
fn host_start_preserves_scenario_selection_guards_before_arming_countdown() {
    // C++ opens the selected scenario before InitNetworkHost and therefore
    // cannot enter its lobby without that concrete source. The Rust manual
    // Start guard preserves the same prerequisite before creating the
    // host-owned countdown
    // (pristine 9ffa0a5d src/C4StartupNetDlg.cpp:1111-1114;
    // src/C4StartupScenSelDlg.cpp:1635-1666; src/C4Game.cpp:421-438).
    let mut app = new_state_only_menu_app(320, 200);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);

    app.process_lobby_action(LobbyAction::StartGame)
        .expect("reject missing scenario selection");
    assert_eq!(app.status_text, "Select a scenario before starting");
    assert!(app.host_lobby_countdown.is_none());
    assert!(commands.take_lobby_start_commands().is_empty());

    app.network_lobby
        .as_mut()
        .unwrap()
        .select_scenario("missing.c4s", "Missing");
    app.process_lobby_action(LobbyAction::StartGame)
        .expect("reject unavailable selected scenario");
    assert_eq!(
        app.status_text,
        "Scenario `missing.c4s` is not available in the catalog"
    );
    assert!(app.host_lobby_countdown.is_none());
    assert!(commands.take_lobby_start_commands().is_empty());
    assert!(matches!(app.mode, AppMode::Menu));
}

#[test]
fn host_start_cancels_an_active_cpp_lobby_countdown() {
    // OnRunBtn checks the active countdown before attempting another
    // start. Abort broadcasts -1, locally applies it, and deletes the
    // timer without entering Network.Start
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:442-450,1176-1193;
    // src/C4Network2.cpp:3046-3051).
    let mut app = new_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.process_lobby_action(LobbyAction::StartGame)
        .expect("begin host countdown");
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );

    app.process_lobby_action(LobbyAction::StartGame)
        .expect("cancel host countdown");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(-1)]
    );
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, None);
    assert!(app.host_lobby_countdown.is_none());
    assert!(matches!(app.mode, AppMode::Menu));
    assert!(
        !app.sec1_timer().expect("pulse aborted countdown timer"),
        "abort releases the one-second callback"
    );
    assert!(commands.take_lobby_start_commands().is_empty());
}

#[test]
fn host_sec1_timer_counts_cpp_lobby_down_through_one() {
    // Countdown::OnSec1Timer decrements first and broadcasts every value
    // in the final ten seconds. The callback is driven by the process-wide
    // second timer, not by a private interval
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:1140-1160;
    // src/C4Application.cpp:495-506; src/StdAppUnix.cpp:261-291).
    let mut app = new_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.process_lobby_action(LobbyAction::StartGame)
        .expect("begin host countdown");
    commands.take_submitted_lobby_countdowns();

    let mut observed = Vec::new();
    for expected in (1..DEFAULT_LOBBY_COUNTDOWN_SECONDS).rev() {
        assert!(
            app.sec1_timer().expect("advance host countdown"),
            "countdown changes visible lobby state"
        );
        observed.extend(commands.take_submitted_lobby_countdowns());
        assert_eq!(
            app.network_lobby.as_ref().unwrap().countdown,
            Some(expected)
        );
        assert!(matches!(app.mode, AppMode::Menu));
    }

    assert_eq!(
        observed,
        [4, 3, 2, 1]
            .map(clonk_network::LobbyCountdownPacket::new)
            .to_vec()
    );
}

#[test]
fn inbound_countdown_packet_cannot_arm_the_host_owned_timer() {
    // C4Network2::pLobbyCountdown is created only by the host's
    // StartLobbyCountdown. MainDlg::OnCountdownPacket updates presentation
    // state only, so a received or late packet cannot install a callback
    // that eventually enters Network.Start
    // (pristine 9ffa0a5d src/C4Network2.cpp:3038-3051;
    // src/C4GameLobby.cpp:392-418,1111-1131).
    let mut app = new_menu_app(320, 200);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::LobbyCountdown(
            clonk_network::LobbyCountdownPacket::new(2),
        ))
        .expect("queue a packet-derived countdown");
    app.process_network_events()
        .expect("apply countdown presentation");

    assert!(
        !app.sec1_timer().expect("pulse packet-only countdown"),
        "no host timer callback was installed"
    );
    assert!(commands.take_lobby_start_commands().is_empty());
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, Some(2));
    assert!(matches!(app.mode, AppMode::Menu));
}

#[test]
fn connected_observer_starts_not_ready_and_retains_explicit_ready_state() {
    // C4ClientCore initializes LobbyReady=false independently of Observer;
    // only C4PacketReadyCheck changes that field
    // (src/C4Client.cpp:32-36; src/C4Network2.cpp:1625-1635,1703-1731).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local".to_string(), false));

    event_tx
        .send(NetworkEvent::PeerConnected {
            client_id: 9,
            name: "Observer".to_string(),
            kind: ParticipantKind::Observer,
        })
        .expect("queue observer connection");
    app.process_network_events()
        .expect("register observer in lobby");
    assert!(!app.network_lobby.as_ref().unwrap().participants[&9].ready);

    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 9,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue explicit observer ready state");
    app.process_network_events()
        .expect("apply observer ready state");
    app.network_lobby.as_mut().unwrap().register_peer(
        9,
        "Renamed observer".to_string(),
        ParticipantKind::Observer,
    );

    let observer = &app.network_lobby.as_ref().unwrap().participants[&9];
    assert!(observer.ready);
    assert_eq!(observer.name, "Renamed observer");
}

#[test]
fn inbound_ready_check_updates_the_claimed_lobby_participant() {
    // HandleReadyCheck looks up packet.Client and applies IsReady to that
    // exact C4Client; it does not substitute the transport sender
    // (src/C4Network2.cpp:1625-1635,1703-1731).
    let mut app = new_state_only_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    let mut lobby = NetworkLobbyState::new(7, "Local".to_string(), false);
    lobby.register_peer(9, "Remote".to_string(), ParticipantKind::Player);
    app.network_lobby = Some(lobby);
    app.control_clients.register(9, true, false);

    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 9,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue remote ready state");
    app.process_network_events().expect("apply ready state");

    let lobby = app.network_lobby.as_ref().unwrap();
    assert!(lobby.participants[&9].ready);
    assert!(!lobby.participants[&7].ready);
    assert!(app.control_clients.state(9).unwrap().lobby_ready);
}

#[test]
fn final_remote_ready_transition_starts_cpp_default_countdown() {
    // MainDlg::OnClientReadyStateChange starts Config.Lobby.CountdownTime
    // after the changed client leaves every relevant client ready
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:868-893;
    // src/C4Network2.cpp:1721-1729).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.participants.get_mut(&0).unwrap().ready = true;
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue final ready transition");

    app.process_network_events()
        .expect("apply final ready transition");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, Some(5));
    assert_eq!(
        app.host_lobby_countdown,
        Some(HostLobbyCountdown {
            remaining: DEFAULT_LOBBY_COUNTDOWN_SECONDS,
        })
    );
}

#[test]
fn empty_nonhost_does_not_block_ready_autostart() {
    // MainDlg::OnClientReadyStateChange skips a non-host client when
    // GetInfoByClientID has no players for it
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-888).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.participants.get_mut(&0).unwrap().ready = true;
    lobby.register_peer(7, "Player client".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Empty client".to_string(), ParticipantKind::Observer);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue final relevant ready transition");

    app.process_network_events()
        .expect("apply final relevant ready transition");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );
    assert!(!app.network_lobby.as_ref().unwrap().participants[&9].ready);
}

#[test]
fn host_without_players_still_blocks_ready_autostart() {
    // MainDlg::OnClientReadyStateChange always includes the host in its
    // readiness scan, even when the host has no C4ClientPlayerInfos entry
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-887).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue remote ready transition");

    app.process_network_events()
        .expect("apply remote ready transition");

    assert!(commands.take_submitted_lobby_countdowns().is_empty());
    assert!(app.host_lobby_countdown.is_none());
    assert!(!app.network_lobby.as_ref().unwrap().participants[&0].ready);
}

#[test]
fn unready_nonhost_with_player_blocks_ready_autostart() {
    // MainDlg::OnClientReadyStateChange includes a non-host client when
    // its C4ClientPlayerInfos contains at least one player
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-887).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.participants.get_mut(&0).unwrap().ready = true;
    lobby.register_peer(7, "Unready".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Changed".to_string(), ParticipantKind::Player);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    for (client_id, info_id) in [(7, 1), (9, 2)] {
        app.control_player_infos
            .apply(clonk_engine::PlayerInfoControlData {
                client_id,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: info_id,
                    ..Default::default()
                }],
                ..Default::default()
            });
    }
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 9,
            data: clonk_network::ReadyCheckData::Ready,
        }))
        .expect("queue later client ready transition");

    app.process_network_events()
        .expect("apply later client ready transition");

    assert!(commands.take_submitted_lobby_countdowns().is_empty());
    assert!(app.host_lobby_countdown.is_none());
    assert!(!app.network_lobby.as_ref().unwrap().participants[&7].ready);
}

#[test]
fn final_local_host_ready_transition_starts_cpp_default_countdown() {
    // MainDlg::OnReadyCheck applies the host's own ready packet through
    // HandleReadyCheck, which invokes OnClientReadyStateChange for the
    // actual state transition
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:329-344,868-893;
    // src/C4Network2.cpp:1721-1729).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).unwrap().ready = true;
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);

    app.process_lobby_action(LobbyAction::ToggleReady)
        .expect("toggle final local host ready");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );
    assert!(app.network_lobby.as_ref().unwrap().local_ready());
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, Some(5));
}

#[test]
fn changed_relevant_client_becoming_unready_aborts_active_countdown() {
    // The first relevant unready client aborts an active host countdown
    // only when it is the client whose ready state actually changed
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-885;
    // src/C4Network2.cpp:1721-1729).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.participants.get_mut(&0).unwrap().ready = true;
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).unwrap().ready = true;
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.process_lobby_action(LobbyAction::StartGame)
        .expect("begin manual countdown");
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );
    event_tx
        .send(NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::NotReady,
        }))
        .expect("queue changed relevant unready state");

    app.process_network_events()
        .expect("apply changed relevant unready state");

    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(-1)]
    );
    assert!(app.host_lobby_countdown.is_none());
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, None);
}

fn host_countdown_disconnect_fixture(
    client_id: ClientId,
    kind: ParticipantKind,
    remaining: i32,
    player_ids: &[i32],
) -> (
    GameApp,
    network::NetworkEventSender,
    crate::network::TestNetworkCommands,
) {
    let mut app = new_menu_app(320, 200);
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.register_peer(client_id, "Remote".to_string(), kind);
    lobby.countdown = Some(remaining);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.host_lobby_countdown = Some(HostLobbyCountdown { remaining });
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: i32::try_from(client_id).expect("fixture client fits i32"),
            players: player_ids
                .iter()
                .map(|id| clonk_engine::ControlPlayerInfoEntry {
                    id: *id,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    (app, event_tx, commands)
}

#[test]
fn host_disconnect_of_player_owning_client_aborts_lobby_countdown() {
    // C4Network2 captures GetPrimaryInfoByClientID before CtrlRemove,
    // then aborts after removing that client (src/C4Network2.cpp:1787-1800).
    // Display kind is intentionally Observer: player infos are authoritative.
    let client_id = 7;
    let (mut app, event_tx, mut commands) = host_countdown_disconnect_fixture(
        client_id,
        ParticipantKind::Observer,
        ALMOST_START_LOBBY_COUNTDOWN_SECONDS,
        &[1],
    );
    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id,
            reason: Some("connection lost".to_string()),
        })
        .expect("queue player-owning client disconnect");

    app.process_network_events()
        .expect("apply player-owning client disconnect");

    // C4Network2's raw disconnect callback only mutates transport/control
    // state. Presentation belongs to the later authoritative ClientRemove
    // control (src/C4Network2.cpp:1774-1833; src/C4Control.cpp:637-670).
    assert!(
        app.status_text.is_empty(),
        "raw disconnect must not poison the exact lobby renderer"
    );
    assert!(!app
        .network_lobby
        .as_ref()
        .unwrap()
        .participants
        .contains_key(&client_id));
    assert!(app.host_lobby_countdown.is_none());
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, None);
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(
            clonk_network::LobbyCountdownPacket::ABORT,
        )]
    );

    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id,
            reason: None,
        })
        .expect("queue duplicate disconnect");
    app.process_network_events()
        .expect("ignore duplicate countdown abort");
    assert!(commands.take_submitted_lobby_countdowns().is_empty());
}

#[test]
fn host_disconnect_of_playerless_observer_keeps_lobby_countdown() {
    let client_id = 9;
    let (mut app, event_tx, mut commands) =
        host_countdown_disconnect_fixture(client_id, ParticipantKind::Observer, 5, &[]);
    assert!(app.control_player_infos.client_info_ids(9).is_empty());
    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id,
            reason: None,
        })
        .expect("queue observer disconnect");

    app.process_network_events()
        .expect("apply observer disconnect");

    assert!(!app
        .network_lobby
        .as_ref()
        .unwrap()
        .participants
        .contains_key(&client_id));
    assert_eq!(app.host_lobby_countdown, Some(HostLobbyCountdown::new()));
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, Some(5));
    assert!(commands.take_submitted_lobby_countdowns().is_empty());
}

#[test]
fn host_disconnect_during_long_countdown_keeps_native_timer() {
    // MainDlg::IsCountdown excludes CDS_LongCountdown (>10 seconds)
    // (src/C4GameLobby.h:43,92-94; src/C4GameLobby.cpp:392-425).
    let (mut app, event_tx, mut commands) =
        host_countdown_disconnect_fixture(7, ParticipantKind::Player, 11, &[1]);
    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id: 7,
            reason: None,
        })
        .expect("queue long-countdown disconnect");

    app.process_network_events()
        .expect("apply long-countdown disconnect");

    assert_eq!(
        app.host_lobby_countdown,
        Some(HostLobbyCountdown { remaining: 11 })
    );
    assert_eq!(app.network_lobby.as_ref().unwrap().countdown, Some(11));
    assert!(commands.take_submitted_lobby_countdowns().is_empty());
}

#[test]
fn client_host_disconnect_aborts_lobby_and_restores_network_dialog() {
    // A client host loss clears C4Network2. While DoLobby is active that
    // makes DoLobby return false, so C4Game::Init aborts back through the
    // remembered startup dialog instead of continuing locally
    // (src/C4Network2.cpp:477-515,1809-1833;
    // src/C4Game.cpp:405-411).
    let mut app = new_real_classic_menu_app(320, 200);
    app.startup_view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.control_clients.replace_snapshot([
        clonk_engine::ClientCoreControlData {
            client_id: 0,
            name: clonk_engine::LegacyCString::from_bytes(b"Oracle Host".to_vec())
                .expect("valid host name"),
            ..Default::default()
        },
        clonk_engine::ClientCoreControlData {
            client_id: 7,
            name: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec())
                .expect("valid client name"),
            ..Default::default()
        },
    ]);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::PeerDisconnected {
            client_id: 0,
            reason: Some("all client transport routes closed".to_string()),
        })
        .expect("queue host socket loss");

    app.process_network_events()
        .expect("restore startup after lobby host loss");

    assert_eq!(app.mode, AppMode::Menu);
    assert_eq!(app.startup_view, StartupView::NetworkGame);
    assert!(app.startup_network_dialog.is_some());
    assert!(app.network.is_none());
    assert!(app.network_mode.is_none());
    assert!(app.network_lobby.is_none());
    assert_startup_error_log(&app, "Network: host Oracle Host disconnected!");
    let engine_results = app.engine.snapshot().round_results;
    assert_eq!(
        engine_results.network_result,
        Some(clonk_engine::RoundResultsNetworkResult::NetworkError)
    );
    assert_eq!(
        engine_results.network_result_message,
        b"Network: host Oracle Host disconnected!"
    );
    assert_eq!(app.snapshot.round_results, engine_results);
    let mut frame = vec![0x4c; 320 * 200 * 4];
    app.render(&mut frame)
        .expect("render restored network dialog and host-loss error");
    assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn repeated_or_irrelevant_unready_does_not_abort_manual_countdown() {
    // HandleReadyCheck invokes the lobby callback only for an actual
    // state change. OnClientReadyStateChange returns at the first relevant
    // unready client and aborts only when that exact client changed
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-885;
    // src/C4Network2.cpp:1721-1729).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.participants.get_mut(&0).unwrap().ready = true;
    lobby.register_peer(7, "Unready player".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Empty client".to_string(), ParticipantKind::Observer);
    lobby.participants.get_mut(&9).unwrap().ready = true;
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 7,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            ..Default::default()
        });
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.process_lobby_action(LobbyAction::StartGame)
        .expect("manual start ignores readiness");
    assert_eq!(
        commands.take_submitted_lobby_countdowns(),
        vec![clonk_network::LobbyCountdownPacket::new(5)]
    );

    for packet in [
        clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::NotReady,
        },
        clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        },
        clonk_network::ReadyCheckPacket {
            client_id: 9,
            data: clonk_network::ReadyCheckData::NotReady,
        },
    ] {
        event_tx
            .send(NetworkEvent::ReadyCheck(packet))
            .expect("queue ready state");
        app.process_network_events().expect("apply ready state");
        assert!(commands.take_submitted_lobby_countdowns().is_empty());
        assert!(app.host_lobby_countdown.is_some());
        assert_eq!(app.network_lobby.as_ref().unwrap().countdown, Some(5));
    }
}

#[test]
fn player_info_add_or_remove_alone_does_not_trigger_ready_autostart() {
    // C4Network2 invokes OnClientReadyStateChange only from an actual
    // ReadyCheck state transition; changing C4ClientPlayerInfos merely
    // changes which clients a later ready transition will consider
    // (pristine 9ffa0a5d src/C4Network2.cpp:1721-1729;
    // src/C4GameLobby.cpp:868-893).
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scenario_catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.participants.get_mut(&0).unwrap().ready = true;
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).unwrap().ready = true;
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData {
                client_id: 7,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )))
        .expect("queue PlayerInfo addition");

    app.process_network_events()
        .expect("apply PlayerInfo addition");

    assert_eq!(app.control_player_infos.client_info_ids(7), vec![1]);
    assert!(commands.take_submitted_lobby_countdowns().is_empty());
    assert!(app.host_lobby_countdown.is_none());

    app.network_lobby
        .as_mut()
        .unwrap()
        .participants
        .get_mut(&7)
        .unwrap()
        .ready = false;
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            clonk_engine::PlayerInfoControlData {
                client_id: 7,
                players: Vec::new(),
                ..Default::default()
            },
        )))
        .expect("queue PlayerInfo removal");

    app.process_network_events()
        .expect("apply PlayerInfo removal");

    assert!(app.control_player_infos.client_info_ids(7).is_empty());
    assert!(commands.take_submitted_lobby_countdowns().is_empty());
    assert!(app.host_lobby_countdown.is_none());
}

#[test]
fn team_selection_execute_queues_the_only_non_full_team() {
    // C4Player::Execute asks GetForcedTeamSelection while in
    // PS_TeamSelection, closes C4MN_TeamSelection, and queues
    // DoTeamSelection for the sole joinable team. Full alternatives do
    // not prevent the forced choice (C4Player.cpp:159-173;
    // C4Teams.cpp:876-914).
    let mut app = new_running_sandbox_app();
    let occupant = app.local_owner;
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Full", 0x00f4_0000).with_max_players(1),
        clonk_engine::TeamInfo::new(2, "Open", 0x0000_c800),
    ]);
    app.engine
        .player_mut(occupant)
        .expect("sandbox player remains")
        .set_team(Some(1));
    let chooser = app
        .engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: "Chooser".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0000_c800,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .expect("chooser waits for team selection");
    app.local_controls.initialize(LocalControlInit {
        owner: chooser,
        preferred_set: 1,
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.engine.set_local_players([occupant, chooser]);
    app.open_initial_team_selection(chooser);
    assert!(app.ingame_menu.is_some(), "selection menu starts open");
    let (manager, events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    events
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: Vec::new(),
        })
        .expect("complete control tick is ready");

    app.update().expect("team-selection execute succeeds");

    assert_eq!(
        app.engine.player(chooser).map(clonk_engine::Player::status),
        Some(PlayerStatus::TeamSelectionPending)
    );
    assert!(
        app.ingame_menu.is_none(),
        "forced selection closes the menu"
    );
    assert_eq!(
        commands.take_submitted_init_scenario_players(),
        vec![(
            tick,
            clonk_engine::InitScenarioPlayerControlData {
                team: 2,
                player: chooser,
                by_client: 0,
            },
        )]
    );
}

#[test]
fn synchronized_team_selection_and_runtime_switch_refresh_live_parameters() {
    let mut app = new_state_only_synthetic_crew_running_sandbox_app();
    app.engine.set_team_colors(true);
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Red", 0x00f4_0000),
        clonk_engine::TeamInfo::new(2, "Green", 0x0000_c800),
    ]);
    let chooser = app
        .engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: "Chooser".to_string(),
            player_info_id: 41,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0012_3456,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .expect("chooser waits for team selection");
    app.control_player_infos
        .apply(clonk_engine::PlayerInfoControlData {
            client_id: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 41,
                color: 0x0012_3456,
                original_color: 0x0012_3456,
                ..Default::default()
            }],
            ..Default::default()
        });
    let (host_snapshot, reference) = default_exact_host_reference();
    app.control_clients
        .replace_snapshot(host_snapshot.parameters.clients.clients.clone());
    app.host_join_snapshot = Some(host_snapshot);
    app.advertised_game_reference = Some(reference);
    let (manager, _events) = NetworkManager::test_stub_for_client_id(0);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));

    app.execute_init_scenario_player_control(chooser, 2)
        .expect("synchronized team choice executes");

    let info = app
        .control_player_infos
        .get(41)
        .expect("PlayerInfo retained");
    assert_eq!(
        (info.team, info.color, info.original_color),
        (2, 0x0000_c800, 0x0012_3456)
    );
    assert_eq!(
        app.engine
            .teams()
            .iter()
            .find(|team| team.id == 2)
            .expect("green team")
            .player_ids,
        vec![41]
    );
    let parameters = &app.host_join_snapshot.as_ref().unwrap().parameters;
    assert_eq!(parameters.player_infos.clients[0].players[0].team, 2);
    assert_eq!(parameters.teams.teams[1].player_ids, vec![41]);

    app.engine
        .set_player_team(chooser, Some(1))
        .expect("script-style runtime team switch");
    app.handle_script_player_info_updates()
        .expect("post-script parameter refresh");

    assert_eq!(app.control_player_infos.get(41).unwrap().team, 1);
    let parameters = &app.host_join_snapshot.as_ref().unwrap().parameters;
    assert_eq!(parameters.player_infos.clients[0].players[0].team, 1);
    assert_eq!(parameters.teams.teams[0].player_ids, vec![41]);
    assert!(parameters.teams.teams[1].player_ids.is_empty());
}

// C4Menu::Execute refills every active menu when Game.iTick35 wraps
// (C4Menu.cpp:990-1000), and C4MainMenu's team case rebuilds every caption
// plus the generated-team row from live state (C4MainMenu.cpp:175-232).
// ClearItems keeps the numeric selection; only AdjustSelection clamps it
// (C4Menu.cpp:947-987).
#[test]
fn team_switch_menu_refills_membership_and_preserves_selection_like_tick35() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    app.engine
        .player_mut(owner)
        .expect("sandbox player")
        .set_name(clonk_script::c4_string_from_bytes(b"Chooser"));
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Alpha", 0x0011_2233),
        clonk_engine::TeamInfo::new(2, "Beta", 0x0044_5566),
    ]);
    let mut configuration = app.engine.team_configuration();
    configuration.allow_team_switch = true;
    app.engine.set_team_configuration(configuration);

    app.apply_ingame_menu_action_for_player(owner, MenuAction::ActivateTeamSelection)
        .expect("open the mid-round team switch page");
    {
        let menu = app.ingame_menu.get(owner).expect("team switch page");
        assert_eq!(menu.page(), ingame_menu::MenuPage::TeamSelection);
        assert!(menu.is_team_switch());
        assert_eq!(
            menu.items()
                .iter()
                .map(|item| item.caption.clone())
                .collect::<Vec<_>>(),
            ["Alpha", "Beta"]
        );
    }
    app.ingame_menu
        .get_mut(owner)
        .expect("team switch page")
        .set_selection(1);

    // Membership changes without any menu control executing.
    app.engine
        .player_mut(owner)
        .expect("sandbox player")
        .set_team(Some(2));

    assert_eq!(
        app.ingame_menu
            .get(owner)
            .expect("team switch page")
            .items()[1]
            .caption,
        "Beta",
        "native waits for the periodic refill"
    );
    app.refresh_team_menus();
    let menu = app.ingame_menu.get(owner).expect("refilled page");
    assert_eq!(
        menu.items()
            .iter()
            .map(|item| item.caption.clone())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta (Chooser)"]
    );
    assert!(
        menu.is_team_switch(),
        "a refill keeps dispatching TeamSwitch:<id>"
    );
    assert_eq!(menu.selection(), 1, "ClearItems(false) keeps the row index");

    // Auto-generated teams add the New Team row only while no configured
    // team is empty (C4MainMenu.cpp:182-197), so the row appears and
    // disappears across refills.
    let mut configuration = app.engine.team_configuration();
    configuration.auto_generate_teams = true;
    app.engine.set_team_configuration(configuration);
    app.refresh_team_menus();
    assert_eq!(
        app.ingame_menu
            .get(owner)
            .expect("refilled page")
            .items()
            .len(),
        2,
        "team Alpha is still empty, so no New Team row is offered"
    );

    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Alpha", 0x0011_2233).with_player_ids(vec![41]),
        clonk_engine::TeamInfo::new(2, "Beta", 0x0044_5566).with_player_ids(vec![42]),
    ]);
    app.refresh_team_menus();
    let menu = app.ingame_menu.get(owner).expect("refilled page");
    assert_eq!(
        menu.items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>(),
        [
            MenuAction::SwitchTeam(1),
            MenuAction::SwitchTeam(2),
            MenuAction::SwitchTeam(-1),
        ]
    );
    assert_eq!(menu.selection(), 1);

    // A shrinking refill clamps an out-of-range selection exactly like
    // AdjustSelection.
    app.ingame_menu
        .get_mut(owner)
        .expect("team switch page")
        .set_selection(2);
    app.engine
        .set_teams(vec![clonk_engine::TeamInfo::new(1, "Alpha", 0x0011_2233)]);
    app.refresh_team_menus();
    let menu = app.ingame_menu.get(owner).expect("refilled page");
    assert_eq!(menu.items().len(), 1);
    assert_eq!(menu.selection(), 0);
}

#[test]
fn l135_team_selection_entries_cache_icon_specs_and_player_info_occupancy() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let mut definition =
        Definition::from_script("TICO", "Team icon", "").expect("team icon definition");
    definition.set_picture(Some(clonk_engine::DefinitionPicture {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }));
    definition.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
        width: 1,
        height: 1,
        pixels: Arc::from([0xff, 0xff, 0xff, 0xff]),
        color_mask: Some(Arc::from([0xff_u8])),
    }));
    app.engine
        .register_definition(definition)
        .expect("register team icon definition");
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Declared", 0x0011_2233)
            .with_player_ids(vec![999])
            .with_icon_spec("TICO"),
        clonk_engine::TeamInfo::new(2, "Missing", 0x0044_5566).with_icon_spec("MISS"),
    ]);

    let entries = app.team_selection_entries();
    assert_eq!(
        (
            entries[0].icon_spec.as_deref(),
            entries[0].color,
            entries[0].has_participants,
        ),
        (Some("TICO"), 0x0011_2233, true),
        "C4Team::GetPlayerCount includes retained, not-yet-joined PlayerInfo IDs"
    );
    assert_eq!(
        (
            entries[1].icon_spec.as_deref(),
            entries[1].color,
            entries[1].has_participants,
        ),
        (Some("MISS"), 0x0044_5566, false)
    );

    app.engine
        .set_player_status(owner, PlayerStatus::TeamSelection)
        .expect("open initial team selection for the local player");
    app.open_initial_team_selection(owner);
    let team_icons = &app
        .ingame_menu_gfx
        .as_ref()
        .expect("team menu graphics cache")
        .team_icons;
    assert_eq!(
        team_icons.get(&1).map(ImageData::pixels),
        Some([0x11, 0x22, 0x33, 0xff].as_slice())
    );
    assert!(
        !team_icons.contains_key(&2),
        "an unresolved IconSpec must remain eligible for the renderer fallback"
    );
}

#[test]
fn team_selection_participant_names_decode_native_bytes_for_presentation() {
    let mut app = new_running_sandbox_app();
    let occupant = app.local_owner;
    app.engine
        .player_mut(occupant)
        .expect("sandbox player remains")
        .set_name(clonk_script::c4_string_from_bytes(b"Andr\xe9"));
    app.engine
        .player_mut(occupant)
        .expect("sandbox player remains")
        .set_team(Some(1));
    app.engine.set_teams(vec![clonk_engine::TeamInfo::new(
        1,
        clonk_script::c4_string_from_bytes(b"Bl\xe5"),
        0x0000_00ff,
    )]);
    let chooser = app
        .engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: "Chooser".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0000_00ff,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .expect("chooser waits for team selection");

    app.open_initial_team_selection(chooser);
    let item = app
        .ingame_menu
        .get(chooser)
        .and_then(|menu| menu.items().first())
        .expect("occupied team row");
    assert_eq!(item.caption, "Bl\u{e5} (Andr\u{e9})");
    assert_eq!(
        item.info_caption.as_deref(),
        Some("Join team Bl\u{e5} (Andr\u{e9})")
    );
    assert_eq!(
        clonk_script::c4_string_bytes(
            app.engine
                .player(occupant)
                .expect("occupant remains")
                .name()
        ),
        b"Andr\xe9"
    );
    assert_eq!(
        clonk_script::c4_string_bytes(&app.engine.teams()[0].name),
        b"Bl\xe5"
    );
}

#[test]
fn team_selection_execute_keeps_an_ambiguous_local_choice_open() {
    // With two joinable teams GetForcedTeamSelection returns zero;
    // C4Player::Execute leaves C4MN_TeamSelection active instead of
    // submitting either choice (C4Player.cpp:159-173;
    // C4Teams.cpp:887-894).
    let mut app = new_running_sandbox_app();
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Left", 0x00f4_0000),
        clonk_engine::TeamInfo::new(2, "Right", 0x0000_c800),
    ]);
    let chooser = app
        .engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: "Chooser".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0000_c800,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .expect("chooser waits for team selection");
    app.local_controls.initialize(LocalControlInit {
        owner: chooser,
        preferred_set: 1,
        prefers_mouse: false,
        gamepads_enabled: true,
        replay: false,
        disable_mouse: false,
    });
    app.open_initial_team_selection(chooser);
    app.ingame_menu
        .as_mut()
        .expect("team menu opens")
        .set_selection(1);
    let (manager, events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    events
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: Vec::new(),
        })
        .expect("complete control tick is ready");

    app.update().expect("ambiguous selection executes");

    assert_eq!(
        app.engine.player(chooser).map(clonk_engine::Player::status),
        Some(PlayerStatus::TeamSelection)
    );
    let menu = app.ingame_menu.as_ref().expect("selection remains open");
    assert_eq!(menu.selection(), 1, "the local choice is not reset");
    assert!(commands.take_submitted_init_scenario_players().is_empty());

    app.engine.set_teams(vec![clonk_engine::TeamInfo::new(
        1,
        "Existing",
        0x00f4_0000,
    )]);
    app.engine.set_auto_generate_teams(true);
    let local_owner = app.local_owner;
    app.engine
        .set_player_team(local_owner, Some(1))
        .expect("sandbox player joins the existing team");
    let tick = app.local_control_submission_tick();
    events
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: Vec::new(),
        })
        .expect("next complete control tick is ready");

    app.update().expect("generated alternative executes");

    assert_eq!(
        app.ingame_menu
            .as_ref()
            .expect("generated alternative keeps menu open")
            .items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>(),
        [MenuAction::SelectTeam(1), MenuAction::SelectTeam(-1)]
    );
    assert!(commands.take_submitted_init_scenario_players().is_empty());
}

#[test]
fn simultaneous_local_team_selection_menus_route_independently() {
    // Every C4Player owns its C4MainMenu, and LocalPlayerControl converts
    // input through the addressed player's menu. Each viewport likewise
    // draws only its associated player's menu (pristine 9ffa0a5d
    // src/C4Player.h:85; src/C4Game.cpp:3572-3624;
    // src/C4Viewport.cpp:965-1017).
    let mut app = new_state_only_synthetic_crew_running_sandbox_app();
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "West", 0xff0000),
        clonk_engine::TeamInfo::new(2, "East", 0x0000ff),
    ]);
    let first = app
        .engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: "First chooser".to_string(),
            player_info_id: 31,
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
            startup_player_count: 2,
        })
        .expect("first player waits for a team");
    let second = app
        .engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: "Second chooser".to_string(),
            player_info_id: 32,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0000ff,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .expect("second player waits for a team");
    app.engine.set_local_players([first, second]);
    app.local_controls = LocalControlRegistry::default();
    for (owner, preferred_set) in [(first, 0), (second, 1)] {
        app.local_controls.initialize(LocalControlInit {
            owner,
            preferred_set,
            prefers_mouse: false,
            gamepads_enabled: true,
            replay: false,
            disable_mouse: false,
        });
        app.open_initial_team_selection(owner);
    }

    assert!(
        [first, second]
            .into_iter()
            .all(|owner| app.ingame_menu_belongs_to(owner)),
        "both local players retain their own initial team menu"
    );
    // The team page is a five-column C4MN_Style_Normal grid, so COM_MenuDown
    // on a two-item menu computes iData = 0 and MoveSelection returns
    // without moving; COM_MenuRight is the within-row step
    // (C4Menu.cpp:444-473).
    assert!(app
        .handle_menu_command(first, ControlCommand::MenuDown, CommandKind::Press,)
        .expect("first player consumes the vertical command"));
    assert!(app
        .handle_menu_command(first, ControlCommand::MenuRight, CommandKind::Press,)
        .expect("first player navigates own menu"));
    assert!(app
        .handle_menu_command(first, ControlCommand::MenuEnter, CommandKind::Press,)
        .expect("first player selects own team"));
    assert_eq!(
        app.engine
            .player(first)
            .map(|player| (player.status(), player.team())),
        Some((PlayerStatus::Active, Some(2)))
    );
    assert!(
        app.engine.crew_cursor(first).is_some(),
        "first team activation spawns its native crew"
    );
    assert_eq!(
        app.engine
            .player(second)
            .map(|player| (player.status(), player.team())),
        Some((PlayerStatus::TeamSelection, None)),
        "first player's selection must not mutate the second player"
    );
    assert!(
        app.ingame_menu_belongs_to(second),
        "second player's menu survives the first player's selection"
    );

    assert!(app
        .handle_menu_command(second, ControlCommand::MenuEnter, CommandKind::Press,)
        .expect("second player selects own team"));
    assert_eq!(
        app.engine
            .player(second)
            .map(|player| (player.status(), player.team())),
        Some((PlayerStatus::Active, Some(1)))
    );
    assert!(
        app.engine.crew_cursor(second).is_some(),
        "second team activation spawns its native crew"
    );
    assert!(!app.ingame_menu_belongs_to(first));
    assert!(!app.ingame_menu_belongs_to(second));
}

#[test]
fn client_join_publishes_selected_players_before_info_and_lobby_ack() {
    // InitNetworkFromReference initializes Network.Players before DoLobby;
    // C4Game copies the raw configured module list and loads it directly;
    // each resource is published first, all successful player infos travel
    // in one CIF_Initial request, and only then may GS_Lobby be
    // acknowledged (pristine 9ffa0a5d src/C4Game.cpp:361-364,3823-3844;
    // src/C4PlayerInfo.cpp:70-104,357-395;
    // src/C4Network2Players.cpp:38-49,78-136).
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
    let bravo = write_player("Bravo.c4p", "Bravo", 0x11_22_33);
    let alpha = write_player("Alpha.c4p", "Alpha", 0x44_55_66);
    let mut config = b"[General]\nName=\"Maker\"\nParticipants=\"".to_vec();
    config.extend_from_slice(bravo.as_os_str().as_encoded_bytes());
    config.push(b';');
    config.extend_from_slice(alpha.as_os_str().as_encoded_bytes());
    config.extend_from_slice(b"\"\n");
    fs::write(paths.config_file(), config).expect("write raw configured participants");

    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths))
        .expect("initialize app with configured participants");
    wait_for_menu(&mut app);
    app.freeze_configured_client_players_for_game()
        .expect("C4Game::Init freezes configured participants");
    fs::write(
        paths.config_file(),
        b"[General]\nName=Changed\nParticipants=\"\"\n",
    )
    .expect("mutate persisted config after C4Game::Init snapshot");
    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    app.startup_view = StartupView::NetworkLobby;
    // The visible startup list is deliberately stale: production joining
    // must use Config.General.Participants directly, not this UI model.
    app.startup_player_files.clear();
    app.startup_player_models.clear();
    app.selected_player_file = None;

    let configured_paths = [bravo, alpha];
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
            .expect("fixture path is NUL-free"),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let expected_cores = cores.clone();
    let command_observer = thread::spawn(move || commands.complete_initial_client_join(cores));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    snapshot
        .parameters
        .clients
        .clients
        .push(clonk_engine::ClientCoreControlData {
            client_id: 7,
            name: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec())
                .expect("valid client name"),
            ..Default::default()
        });
    snapshot.parameters.clients.local_client_id = Some(7);
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 23,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    event_tx
        .send(NetworkEvent::JoinData(join_data))
        .expect("queue JoinData");

    app.process_network_events()
        .expect("initialize client lobby");
    for (core, path) in expected_cores.iter().zip(&configured_paths) {
        assert_eq!(
            app.admission_resources.complete_path(core.id),
            Some(path.as_path()),
            "the publishing client keeps each initial player resource complete"
        );
    }

    let (order, publications, player_infos, acknowledgements) =
        command_observer.join().expect("command observer");
    assert_eq!(
        order,
        vec!["publish", "publish", "player-info", "status-ack"]
    );
    assert_eq!(
        publications
            .iter()
            .map(|request| request.wire_name.as_bytes())
            .collect::<Vec<_>>(),
        configured_paths
            .iter()
            .map(|path| path.as_os_str().as_encoded_bytes())
            .collect::<Vec<_>>()
    );
    assert!(publications
        .iter()
        .all(|request| request.group_maker.as_bytes() == b"Maker"));
    assert_eq!(player_infos.len(), 1);
    assert_eq!(
        player_infos[0]
            .players
            .iter()
            .map(|player| {
                (
                    player.name.as_bytes(),
                    player.color,
                    player.resource.clone(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                b"Bravo".as_slice(),
                0x11_22_33,
                Some(expected_cores[0].clone())
            ),
            (
                b"Alpha".as_slice(),
                0x44_55_66,
                Some(expected_cores[1].clone())
            ),
        ]
    );
    assert_eq!(acknowledgements.len(), 1);
    assert_eq!(acknowledgements[0].target_tick, 23);
}

#[test]
fn startup_network_client_enters_and_acknowledges_lobby_when_boot_completes() {
    // A command-line direct join completes network initialization before
    // C4Game::Init enters C4Network2::DoLobby. DoLobby then marks the lobby
    // running so the initial GS_Lobby can be acknowledged
    // (src/C4Game.cpp:366-409; src/C4Network2.cpp:445-461,2017-2052).
    let _lock = env_lock().lock();
    let user_data = tempdir().expect("isolated client startup user data");
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(320, 200, &paths);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Observer",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Observer".to_string(), false));
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    snapshot
        .parameters
        .clients
        .clients
        .push(clonk_engine::ClientCoreControlData {
            client_id: 7,
            observer: true,
            name: clonk_engine::LegacyCString::from_bytes(b"Observer".to_vec())
                .expect("valid client name"),
            ..Default::default()
        });
    snapshot.parameters.clients.local_client_id = Some(7);
    event_tx
        .send(NetworkEvent::JoinData(clonk_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: 23,
            status: host_config.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        }))
        .expect("receive JoinData while startup assets still load");
    app.process_network_events()
        .expect("retain pre-lobby JoinData");
    assert!(commands.take_status_acknowledgements().is_empty());

    app.mode = AppMode::Loading;
    let (boot_tx, boot_rx) = mpsc::channel();
    app.boot_loading = Some(BootLoadingState::new(boot_rx));
    boot_tx
        .send(BootLoadingEvent::Finished(None))
        .expect("finish command-line client boot");

    app.poll_boot_loading();

    assert_eq!(app.startup_view, StartupView::NetworkLobby);
    assert!(app.network.is_some());
    assert!(app.network_lobby.is_some());
    assert_eq!(
        commands.take_framed_status_acknowledgements(),
        vec![(
            clonk_network::NetworkStatus {
                target_tick: 23,
                ..host_config.initial_status
            },
            0,
        )]
    );
}

#[test]
fn client_lobby_acknowledges_join_status_at_the_initialized_control_tick_once() {
    // DoLobby marks GS_Lobby reached only after the lobby is running, then
    // rewrites the reference status target to the initialized ControlTick
    // and sends one PID_StatusAck after initial PlayerInfo submission
    // (src/C4Network2.cpp:445-461,2041-2058;
    // src/C4Network2Players.cpp:124-136).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Observer",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Observer".to_string(), false));
    app.startup_view = StartupView::NetworkLobby;
    app.selected_player_file = None;
    for _ in 0..3 {
        app.engine.tick().expect("advance C++ frame counter");
    }

    let host_config = clonk_network::HostConfig::default();
    let mut reference_status = host_config.initial_status;
    reference_status.target_tick = -1;
    let mut snapshot = host_config
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    snapshot
        .parameters
        .clients
        .clients
        .push(clonk_engine::ClientCoreControlData {
            client_id: 7,
            name: clonk_engine::LegacyCString::from_bytes(b"Observer".to_vec())
                .expect("valid client name"),
            ..Default::default()
        });
    snapshot.parameters.clients.local_client_id = Some(7);
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 23,
        status: reference_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    event_tx
        .send(NetworkEvent::JoinData(join_data))
        .expect("queue JoinData");

    app.process_network_events().expect("enter network lobby");

    assert_eq!(
        app.network_control_clock
            .map(NetworkControlClock::current_tick),
        Some(23),
        "PID_StatusAck uses Game.Control.ControlTick"
    );
    assert_eq!(
        app.engine.frame(),
        3,
        "the distinct Game.FrameCounter is reserved for ClientActReq"
    );

    assert_eq!(
        commands.take_framed_status_acknowledgements(),
        vec![(
            clonk_network::NetworkStatus {
                target_tick: 23,
                ..reference_status
            },
            3,
        )]
    );
    app.process_network_events()
        .expect("poll empty event queue");
    assert!(commands.take_status_acknowledgements().is_empty());
}

#[test]
fn later_admission_reuses_shifted_initial_hosts_persisted_alternate_color() {
    // `ResolvePlayerAttributeConflicts` revisits every retained packet on
    // each admission. The synchronized row omits AlternateColorDw, but
    // the host's original C4PlayerInfo keeps it for the whole session
    // (src/C4PlayerInfo.cpp:82-90,177-230;
    // src/C4PlayerInfoConflicts.cpp:249-296).
    let legacy = |bytes: &[u8]| {
        clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).expect("NUL-free fixture")
    };
    let resource = |id| clonk_engine::NetworkResourceCore {
        id,
        ..Default::default()
    };
    let mut app = new_state_only_menu_app(320, 200);
    app.network_max_players = 8;
    app.control_player_infos.replace_snapshot(
        2,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![
                clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    name: legacy(b"Blocker"),
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    resource: Some(resource(11)),
                    ..Default::default()
                },
                clonk_engine::ControlPlayerInfoEntry {
                    id: 2,
                    name: legacy(b"Shifted host"),
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    color: 0x0000_00e8,
                    original_color: 0x00f4_0000,
                    resource: Some(resource(22)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
    );
    app.host_local_alternate_colors_by_resource = HashMap::from([(11, 0), (22, 0x0000_00e8)]);
    app.host_local_player_info_ids = HashSet::from([1, 2]);
    let aliased_remote = clonk_engine::ControlPlayerInfoEntry {
        id: 99,
        resource: Some(resource(22)),
        ..Default::default()
    };
    assert_eq!(
        host_runtime_alternate_color(
            &app.host_local_alternate_colors_by_resource,
            &app.host_local_player_info_ids,
            &aliased_remote,
        ),
        Some(0),
        "a remote row sharing the resource ID still has native's wire default"
    );
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: legacy(b"Later remote"),
                    color: 0x0000_c800,
                    original_color: 0x0000_c800,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue later PlayerInfo request");

    app.process_network_events()
        .expect("persisted host alternate resolves the revisited packet");

    let broadcasts = commands.take_broadcast_player_infos();
    let [admitted] = broadcasts.as_slice() else {
        panic!("expected one admitted packet, got {broadcasts:?}");
    };
    assert_eq!(admitted.client_id, 3);
    assert_eq!(admitted.players[0].id, 3);
    assert_eq!(
        app.control_player_infos
            .get(2)
            .expect("shifted initial host remains retained")
            .color,
        0x0000_00e8
    );
}

#[test]
fn host_player_info_conflict_update_precedes_admission_and_converges_join_data() {
    // ResolvePlayerAttributeConflicts may change an older, lower-priority
    // packet while admitting a higher-priority player. SendUpdatedPlayers
    // submits those retained packets before the new packet's CID_PlrInfo;
    // each direct control updates the live JoinData projection in the same
    // order (src/C4Network2Players.cpp:203-239;
    // src/C4PlayerInfoConflicts.cpp:193-344).
    let same_name =
        clonk_engine::LegacyCString::from_bytes(b"Same".to_vec()).expect("valid player name");
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos.replace_snapshot(
        1,
        [clonk_engine::PlayerInfoControlData {
            client_id: 2,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: same_name.clone(),
                color: 0x00f4_0000,
                original_color: 0x00f4_0000,
                ..Default::default()
            }],
            ..Default::default()
        }],
    );
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: same_name,
                    color: 0x0000_00f4,
                    original_color: 0x0000_00f4,
                    league_score: 10,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue conflicting PlayerInfo update request");

    app.process_network_events()
        .expect("resolve conflicting PlayerInfo request");

    let (broadcasts, published) = commands.take_team_control_updates();
    let [updated_existing, admitted] = broadcasts.as_slice() else {
        panic!("expected existing update followed by admitted PlayerInfo");
    };
    assert_eq!((updated_existing.client_id, admitted.client_id), (2, 3));
    assert_eq!(
        updated_existing.players[0].forced_name.as_bytes(),
        b"Same (2)"
    );
    assert!(admitted.players[0].forced_name.is_empty());
    assert_eq!(admitted.players[0].id, 2);
    assert!(app.control_player_infos.get(2).is_some());

    for info in broadcasts.clone() {
        event_tx
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                join_players_on_echo: Vec::new(),
                original: info.clone(),
                info,
            })
            .expect("queue authoritative direct PlayerInfo");
    }
    app.process_network_events()
        .expect("apply ordered authoritative PlayerInfo controls");

    assert_eq!(
        app.control_player_infos
            .get(1)
            .expect("existing player remains retained")
            .forced_name
            .as_bytes(),
        b"Same (2)"
    );
    assert_eq!(
        app.control_player_infos
            .get(2)
            .expect("new player control is applied")
            .name
            .as_bytes(),
        b"Same"
    );
    assert_eq!(published.len(), 2);
    let latest = published.last().expect("final JoinData is published");
    assert_eq!(latest.parameters.player_infos.last_player_id, 2);
    let existing = latest
        .parameters
        .player_infos
        .clients
        .iter()
        .find(|client| client.client_id == 2)
        .expect("existing client row is published");
    assert_eq!(existing.players[0].forced_name.as_bytes(), b"Same (2)");
    let admitted = latest
        .parameters
        .player_infos
        .clients
        .iter()
        .find(|client| client.client_id == 3)
        .expect("admitted client row is published");
    assert_eq!(admitted.players[0].id, 2);
    assert_eq!(published.last(), app.host_join_snapshot.as_ref());
}

#[test]
fn updated_existing_echo_cannot_issue_new_admission_before_its_echo() {
    let same_name =
        clonk_engine::LegacyCString::from_bytes(b"Same".to_vec()).expect("valid player name");
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"New.c4p".to_vec()).unwrap(),
        ..Default::default()
    };
    let mut app = new_state_only_running_sandbox_app();
    app.control_clients.register(3, true, false);
    app.control_player_infos.replace_snapshot(
        1,
        [clonk_engine::PlayerInfoControlData {
            client_id: 3,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: same_name.clone(),
                color: 0x00f4_0000,
                original_color: 0x00f4_0000,
                ..Default::default()
            }],
            ..Default::default()
        }],
    );
    app.admission_resources.register_lobby_resource(&resource);
    app.admission_resources
        .mark_complete(resource.id, PathBuf::from("New.c4p"));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: same_name,
                    color: 0x0000_00f4,
                    original_color: 0x0000_00f4,
                    league_score: 10,
                    resource: Some(resource),
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue same-client conflict admission");

    app.process_network_events()
        .expect("preexecute same-client conflict admission");
    let controls = commands.take_preexecuted_player_infos();
    let [(updated_existing, updated_ids), (admitted, admitted_ids)] = controls.as_slice() else {
        panic!("expected non-joining update echo before joining admission echo");
    };
    assert!(updated_ids.is_empty());
    assert_eq!(
        admitted_ids
            .iter()
            .map(|player| player.id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: updated_existing.clone(),
            info: updated_existing.clone(),
            join_players_on_echo: updated_ids.clone(),
        })
        .expect("queue updated-existing echo");
    app.process_network_events()
        .expect("merge updated-existing echo");
    assert!(commands.take_submitted_join_players().is_empty());

    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: admitted.clone(),
            info: admitted.clone(),
            join_players_on_echo: admitted_ids.clone(),
        })
        .expect("queue admitted echo");
    app.process_network_events().expect("merge admitted echo");
    let joins = commands.take_submitted_join_players();
    assert_eq!(joins.len(), 1);
    assert_eq!(joins[0].1.info_id, admitted.players[0].id);
}

#[test]
fn delayed_admission_joins_parent_before_other_client_rebalance_follow_up() {
    let resource = |id, filename: &[u8]| clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(filename.to_vec()).unwrap(),
        ..Default::default()
    };
    let parent_resource = resource(17, b"Parent.c4p");
    let other_resource = resource(18, b"Other.c4p");
    let fixed = clonk_engine::PLAYER_INFO_FLAG_ATTRIBUTES_FIXED;
    let mut other_unjoined = set_control_test_player(10, 1, fixed);
    other_unjoined.color = 0x00f4_0000;
    other_unjoined.original_color = 0x00f4_0000;
    other_unjoined.resource = Some(other_resource.clone());
    let mut joined_twenty =
        set_control_test_player(20, 1, fixed | clonk_engine::PLAYER_INFO_FLAG_JOINED);
    joined_twenty.color = 0x0000_c800;
    joined_twenty.original_color = 0x0000_c800;
    let mut joined_thirty =
        set_control_test_player(30, 1, fixed | clonk_engine::PLAYER_INFO_FLAG_JOINED);
    joined_thirty.color = 0x0000_00f4;
    joined_thirty.original_color = 0x0000_00f4;

    let mut app = new_state_only_running_sandbox_app();
    app.control_clients.register(3, true, false);
    app.control_clients.register(4, true, false);
    app.control_player_infos.replace_snapshot(
        30,
        [clonk_engine::PlayerInfoControlData {
            client_id: 4,
            players: vec![other_unjoined, joined_twenty, joined_thirty],
            ..Default::default()
        }],
    );
    let mut metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![10, 20, 30], 0),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    for core in [&parent_resource, &other_resource] {
        app.admission_resources.register_lobby_resource(core);
        app.admission_resources.mark_complete(
            core.id,
            PathBuf::from(core.filename.to_string_lossy().into_owned()),
        );
    }
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Parent".to_vec()).unwrap(),
                    flags: fixed,
                    color: 0x00f4_f400,
                    original_color: 0x00f4_f400,
                    resource: Some(parent_resource),
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue delayed parent admission");

    app.process_network_events()
        .expect("preexecute admission and random-team follow-up");
    let controls = commands.take_preexecuted_player_infos();
    assert_eq!(
        controls
            .iter()
            .map(|(info, _)| info.client_id)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(
        controls
            .iter()
            .map(|(_, players)| players.iter().map(|player| player.id).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec![31], vec![10]]
    );
    for (info, join_players_on_echo) in controls {
        event_tx
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                original: info.clone(),
                info,
                join_players_on_echo,
            })
            .expect("queue normalized preexecuted echo");
    }

    app.process_network_events()
        .expect("issue joins in direct-control order");
    assert_eq!(
        commands
            .take_submitted_join_players()
            .into_iter()
            .map(|(_, join)| (join.at_client, join.info_id))
            .collect::<Vec<_>>(),
        vec![(3, 31), (4, 10)]
    );
}

#[test]
fn host_updated_admission_emits_clean_full_follow_up_after_direct_apply() {
    let same_name =
        clonk_engine::LegacyCString::from_bytes(b"Same".to_vec()).expect("valid player name");
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos.replace_snapshot(
        1,
        [clonk_engine::PlayerInfoControlData {
            client_id: 2,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: same_name.clone(),
                color: 0x00f4_0000,
                original_color: 0x00f4_0000,
                league_score: 10,
                ..Default::default()
            }],
            ..Default::default()
        }],
    );
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: same_name,
                    color: 0x0000_00f4,
                    original_color: 0x0000_00f4,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue lower-priority duplicate admission");
    app.process_network_events()
        .expect("resolve the incoming duplicate name");

    let broadcasts = commands.take_broadcast_player_infos();
    let [admitted, clean_follow_up] = broadcasts.as_slice() else {
        panic!("expected updated admission and clean full follow-up");
    };
    assert_ne!(
        admitted.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED,
        0
    );
    assert_eq!(admitted.players[0].forced_name.as_bytes(), b"Same (2)");
    assert_eq!(clean_follow_up.client_id, 3);
    assert_eq!(
        clean_follow_up.flags
            & (clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
                | clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED),
        0
    );
    assert_eq!(clean_follow_up.players, admitted.players);
    assert!(app.control_player_infos.get(2).is_some());
}

#[test]
fn host_remote_lobby_player_info_assigns_the_unique_least_used_free_team() {
    // HandlePlayerInfoUpdRequest allocates the ID before AssignTeams, and
    // a lobby needs a team preset even under TEAMDIST_Free. AddPlayer also
    // records the ID and forces the current team color
    // (src/C4Network2Players.cpp:160-205;
    // src/C4Teams.cpp:53-81,446-542).
    let team = |id, player_ids, color| clonk_engine::InitialNetworkTeam {
        id,
        name: clonk_engine::LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
        player_start_index: 0,
        player_ids,
        color,
        icon_spec: clonk_engine::LegacyCString::default(),
        max_players: 0,
    };
    let mut app = new_state_only_menu_app(320, 200);
    app.control_player_infos.replace_snapshot(
        1,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: clonk_engine::LegacyCString::from_bytes(b"Existing".to_vec())
                    .expect("valid existing player name"),
                team: 1,
                ..Default::default()
            }],
            ..Default::default()
        }],
    );
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(
        clonk_engine::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
            team_colors: true,
            max_script_players: 0,
            script_player_names: clonk_engine::LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![1], 0x00f4_0000),
                team(2, Vec::new(), 0x0000_c800),
            ],
        },
    ));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    let original_color = 0x0012_3456;
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 9,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"New".to_vec())
                        .expect("valid new player name"),
                    color: original_color,
                    original_color,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue teamless remote PlayerInfo update request");

    app.process_network_events()
        .expect("process teamless remote PlayerInfo update request");

    let broadcasts = commands.take_broadcast_player_infos();
    let [info] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo broadcast");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted player");
    };
    assert_eq!((player.id, player.team), (2, 2));
    assert_eq!(
        (player.color, player.original_color),
        (0x0000_c800, original_color)
    );
    let teams = app
        .network_team_assignment
        .as_mut()
        .expect("prepared host team state remains installed")
        .teams_mut();
    assert_eq!(teams.teams[0].player_ids, vec![1]);
    assert_eq!(teams.teams[1].player_ids, vec![2]);
}

#[test]
fn loading_host_with_retained_lobby_does_not_assign_a_free_team() {
    let mut app = new_state_only_menu_app(320, 200);
    let mut metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, Vec::new(), 0),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    metadata.custom = true;
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    app.mode = AppMode::Loading;
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Loading".to_vec()).unwrap(),
                    color: 0x0012_3456,
                    original_color: 0x0012_3456,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue loading admission");

    app.process_network_events()
        .expect("process loading admission");

    let broadcasts = commands.take_broadcast_player_infos();
    let [info] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo");
    };
    assert_eq!(info.players[0].team, 0);
    let teams = app.network_team_assignment.as_ref().unwrap().teams();
    assert!(teams.teams.iter().all(|team| team.player_ids.is_empty()));
}

#[test]
fn lobby_admission_does_not_gain_join_eligibility_before_delayed_echo() {
    let mut app = new_state_only_menu_app(320, 200);
    app.control_clients.register(3, true, false);
    let resource = clonk_engine::NetworkResourceCore {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 17,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"Lobby.c4p".to_vec()).unwrap(),
        ..Default::default()
    };
    app.admission_resources.register_lobby_resource(&resource);
    app.admission_resources
        .mark_complete(resource.id, PathBuf::from("Lobby.c4p"));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Lobby".to_vec()).unwrap(),
                    color: 0x0012_3456,
                    original_color: 0x0012_3456,
                    resource: Some(resource),
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue lobby admission");

    app.process_network_events()
        .expect("preexecute lobby admission");
    let controls = commands.take_preexecuted_player_infos();
    let [(info, join_players)] = controls.as_slice() else {
        panic!("expected one preexecuted lobby admission");
    };
    assert!(join_players.is_empty());

    app.mode = AppMode::Running;
    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: info.clone(),
            info: info.clone(),
            join_players_on_echo: join_players.clone(),
        })
        .expect("queue delayed lobby echo");
    app.process_network_events()
        .expect("merge delayed lobby echo after GO");
    assert!(commands.take_submitted_join_players().is_empty());
}

#[test]
fn queued_lobby_admissions_observe_synchronously_filled_explicit_team() {
    let mut app = new_state_only_menu_app(320, 200);
    let mut metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, Vec::new(), 1),
            set_control_test_team(2, Vec::new(), 1),
        ],
    );
    metadata.custom = true;
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    for (client_id, name, color) in [
        (3, b"First".as_slice(), 0x00f4_0000),
        (4, b"Second".as_slice(), 0x0000_00f4),
    ] {
        event_tx
            .send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: client_id as u32,
                request: clonk_network::PlayerInfoUpdateRequest {
                    client_id,
                    flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        name: clonk_engine::LegacyCString::from_bytes(name.to_vec()).unwrap(),
                        team: 2,
                        color,
                        original_color: color,
                        ..Default::default()
                    }],
                },
                by_host: false,
            })
            .expect("queue competing lobby admission");
    }

    app.process_network_events()
        .expect("process competing lobby admissions");

    let broadcasts = commands.take_broadcast_player_infos();
    assert_eq!(
        broadcasts
            .iter()
            .map(|info| info.players[0].team)
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    for info in &broadcasts {
        event_tx
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                original: info.clone(),
                info: info.clone(),
                join_players_on_echo: Vec::new(),
            })
            .expect("queue normalized PlayerInfo echo");
    }
    app.process_network_events()
        .expect("merge normalized PlayerInfo echoes");
    let (_, clients) = app.control_player_infos.retained_rows_snapshot();
    assert_eq!(
        clients
            .iter()
            .map(|(_, _, players)| players.len())
            .sum::<usize>(),
        2,
        "preexecuted AddPlayers echoes must not duplicate retained rows"
    );
}

#[test]
fn admission_that_requires_generated_team_generates_and_broadcasts() {
    let mut app = new_state_only_menu_app(320, 200);
    app.control_player_infos.replace_snapshot(
        1,
        [clonk_engine::PlayerInfoControlData {
            client_id: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                name: clonk_engine::LegacyCString::from_bytes(b"Existing".to_vec()).unwrap(),
                team: 1,
                color: 0x00f4_0000,
                original_color: 0x00f4_0000,
                ..Default::default()
            }],
            ..Default::default()
        }],
    );
    let metadata = set_control_test_metadata(true, vec![set_control_test_team(1, vec![1], 0)]);
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), true));
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    name: clonk_engine::LegacyCString::from_bytes(b"Generated".to_vec()).unwrap(),
                    color: 0x0000_00f4,
                    original_color: 0x0000_00f4,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue admission requiring generated team");

    app.process_network_events()
        .expect("generate a team and admit the player transactionally");

    let broadcasts = commands.take_broadcast_player_infos();
    let [admitted] = broadcasts.as_slice() else {
        panic!("expected one generated-team admission, got {broadcasts:?}");
    };
    assert_eq!((admitted.players[0].id, admitted.players[0].team), (2, 2));
    let teams = app.network_team_assignment.as_ref().unwrap().teams();
    assert_eq!(teams.teams.len(), 2);
    assert_eq!(teams.teams[1].id, 2);
    assert_eq!(teams.teams[1].name.as_bytes(), b"Team 2");
    assert_eq!(teams.teams[1].player_ids, vec![2]);
}

#[test]
fn joined_player_automatic_admission_updates_live_engine_before_loopback() {
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos.replace_snapshot(41, []);
    let mut metadata = set_control_test_metadata(
        false,
        vec![
            set_control_test_team(1, vec![41], 0),
            set_control_test_team(2, Vec::new(), 0),
        ],
    );
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    metadata.team_colors = true;
    app.engine
        .set_teams(runtime_teams_from_initial_metadata(&metadata));
    app.engine
        .register_player(
            PlayerConfig::new(17, "Joined")
                .with_player_info_id(41)
                .with_team(Some(1))
                .with_color(Some(RgbColor::new(0xf4, 0, 0))),
        )
        .expect("register joined live player");
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    event_tx
        .send(NetworkEvent::PlayerInfoUpdateRequest {
            origin: 3,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id: 3,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 41,
                    flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                    name: clonk_engine::LegacyCString::from_bytes(b"Joined".to_vec()).unwrap(),
                    team: 0,
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                }],
            },
            by_host: false,
        })
        .expect("queue joined PlayerInfo admission");

    app.process_network_events()
        .expect("process joined PlayerInfo admission");

    let live = app.engine.player(17).expect("joined player remains live");
    assert_eq!(live.team(), Some(2));
    assert_eq!(live.color(), Some(RgbColor::new(0, 0, 0xf4)));
    let broadcasts = commands.take_broadcast_player_infos();
    assert_eq!(broadcasts[0].players[0].team, 2);
}

#[test]
fn offline_set_max_player_raise_reaches_admission_before_queued_script_player() {
    // SetMaxPlayer mutates Game.Parameters directly; CreateScriptPlayer
    // queues its PlayerInfo later in the same callback. The app must copy
    // the new limit out of Engine before applying that deferred request
    // (src/C4Script.cpp:2882-2902,3693-3705).
    let mut app = new_state_only_running_sandbox_app();
    app.network_max_players = 1;
    app.engine.set_max_players(1);
    app.engine
        .install_scenario_script_with_convention(
            "SetMaxPlayer raised admission fixture",
            r#"
                    static set_result;

                    global func RaiseLimitAndSpawn()
                    {
                        set_result = SetMaxPlayer(2);
                        CreateScriptPlayer("Admitted Bot", 0x445566, 2, 15, __AI);
                    }
                    "#,
            true,
        )
        .expect("fixture script installs");

    let before = app.engine.snapshot().players.len();
    app.engine
        .call_scenario_script_function("RaiseLimitAndSpawn", Vec::new())
        .expect("SetMaxPlayer and CreateScriptPlayer execute in order");

    let globals = app.engine.snapshot().script_globals.named;
    assert_eq!(
        globals.get("set_result"),
        Some(&Value::Int(1)),
        "FnSetMaxPlayer has the C4ValueInt success result"
    );
    assert_eq!(app.engine.max_players(), Some(2));
    assert_eq!(
        app.engine.snapshot().players.len(),
        before,
        "CreateScriptPlayer remains deferred until app admission"
    );

    app.handle_script_player_info_updates()
        .expect("raised cap admits the queued script player");

    assert_eq!(
        app.network_max_players, 2,
        "the app admission cap follows Game.Parameters.MaxPlayers"
    );
    let admitted = app
        .control_player_infos
        .client_info_ids(0)
        .into_iter()
        .filter_map(|id| app.control_player_infos.get(id))
        .find(|info| info.name.as_bytes() == b"Admitted Bot")
        .expect("script PlayerInfo is admitted under the raised cap");
    assert!(
        app.engine
            .snapshot()
            .players
            .iter()
            .any(|player| player.player_info_id == admitted.id),
        "the admitted PlayerInfo reaches JoinPlayer"
    );
}

#[test]
fn synchronized_host_script_control_executes_at_the_ready_tick() {
    let mut app = new_state_only_running_sandbox_app();
    assert_ne!(app.engine.physics().gravity, 77);

    app.apply_ready_controls(
        12,
        vec![NetworkControl::Script(clonk_engine::ScriptControlData {
            target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: clonk_engine::ScriptStrictness::Strict3,
            script: clonk_engine::LegacyCString::from_bytes(b"SetGravity(77)".to_vec())
                .expect("script is NUL-free"),
            by_client: 0,
        })],
    )
    .expect("host-authored script control executes");

    assert_eq!(app.engine.physics().gravity, 77);
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn synchronized_em_draw_tool_executes_at_the_ready_tick() {
    let mut app = new_state_only_running_sandbox_app();
    assert_ne!(
        app.engine
            .landscape()
            .expect("sandbox landscape exists")
            .mode(),
        clonk_engine::LANDSCAPE_MODE_EXACT
    );

    app.apply_ready_controls(
        12,
        vec![NetworkControl::EmDrawTool(
            clonk_engine::EmDrawToolControlData {
                action: clonk_engine::EMDT_SET_MODE,
                mode: clonk_engine::LANDSCAPE_MODE_EXACT,
                material: clonk_engine::LegacyCString::default(),
                texture: clonk_engine::LegacyCString::default(),
                by_client: 0,
                ..Default::default()
            },
        )],
    )
    .expect("synchronized editor draw control executes");

    assert_eq!(
        app.engine
            .landscape()
            .expect("sandbox landscape remains")
            .mode(),
        clonk_engine::LANDSCAPE_MODE_EXACT
    );
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn synchronized_em_drop_def_executes_at_the_ready_tick() {
    let mut app = new_state_only_running_sandbox_app();
    let mut definition =
        Definition::from_script("DROP", "Drop", "#strict\n").expect("definition compiles");
    definition.set_category(clonk_engine::CATEGORY_OBJECT);
    app.engine
        .register_definition(definition)
        .expect("definition registers");
    assert!(app
        .engine
        .first_active_object_for_definition("DROP")
        .is_none());

    app.apply_ready_controls(
        12,
        vec![NetworkControl::EmDropDef(
            clonk_engine::EmDropDefControlData {
                id: *b"DROP",
                x: 23,
                y: 17,
                by_client: 0,
            },
        )],
    )
    .expect("synchronized editor definition drop executes");

    let object = app
        .engine
        .first_active_object_for_definition("DROP")
        .and_then(|id| app.engine.object_snapshot(id))
        .expect("dropped object exists");
    assert_eq!(object.position, Vector2::new(23, 17));
    assert_eq!(object.owner, clonk_engine::OWNER_NONE);
    assert_eq!(app.executing_ready_tick, None);
}

#[test]
fn message_board_query_opens_on_tick35_and_routes_ui_answer_at_ready_tick() {
    let mut app = new_synthetic_running_sandbox_app();
    let player = app.local_owner;
    app.engine
        .player_mut(player)
        .expect("local player remains")
        .set_at_client(clonk_engine::PlayerAtClient::HOST);
    app.engine
        .register_definition(
            Definition::from_script(
                "MBUI",
                "Message-board UI target",
                r#"#strict 2
    local callback_answer, callback_count;
    public func Open(int player) { return CallMessageBoard(this(), true, "Exact prompt", player); }
    protected func InputCallback(string answer, int player)
    {
        callback_answer = answer;
        callback_count = callback_count + 1;
        return 1;
    }
    "#,
            )
            .expect("message-board target compiles"),
        )
        .expect("message-board target registers");
    let target = app
        .engine
        .spawn_object(SpawnConfig::new("MBUI"))
        .expect("message-board target spawns");
    let target_index = app
        .engine
        .find_object_index(target)
        .expect("message-board target is live");
    assert_eq!(
        app.engine
            .call_object_function(target_index, "Open", vec![Value::Int(player)])
            .expect("query opens"),
        Value::Bool(true)
    );

    let updates_to_tick35 = 35 - app.engine.frame() % 35;
    for update in 1..updates_to_tick35 {
        app.update()
            .unwrap_or_else(|error| panic!("pre-activation update {update} succeeds: {error}"));
        assert!(app.engine.active_message_board_input().is_none());
        assert!(app.running_chat_controller().is_none());
    }
    app.update().expect("Tick35 update activates query");
    assert_eq!(app.engine.frame() % 35, 0);
    assert_eq!(
        app.engine
            .active_message_board_input()
            .expect("engine query activates")
            .prompt,
        "Exact prompt"
    );
    let controller = app
        .running_chat_controller()
        .expect("the same Tick35 update opens the input dialog");
    assert_eq!(controller.message(), "Exact prompt");
    assert_eq!(controller.text(), "");

    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    for character in "mixed".chars() {
        app.handle_text_input(character)
            .expect("type message-board answer");
    }
    let submission_tick = app.local_control_submission_tick();
    app.handle_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("submit message-board answer");
    assert!(app.running_chat_controller().is_none());
    assert!(app.engine.active_message_board_input().is_none());
    let target_before_ready = app
        .engine
        .object_snapshot(target)
        .expect("target remains live before the ready tick");
    assert_ne!(
        target_before_ready.local_vars.get("callback_count"),
        Some(&Value::Int(1)),
        "submission closes the local UI without running the callback"
    );
    let mut submitted = commands.take_submitted_message_board_answers();
    assert_eq!(submitted.len(), 1);
    let (queued_tick, answer) = submitted
        .pop()
        .expect("network worker receives the UI answer");
    assert_eq!(queued_tick, submission_tick);
    assert_eq!(answer.answer.as_bytes(), b"MIXED");
    assert_eq!(answer.by_client, 0);

    app.apply_ready_controls(
        queued_tick,
        vec![NetworkControl::MessageBoardAnswer(answer)],
    )
    .expect("message-board answer executes at its ready tick");
    let target_after_ready = app
        .engine
        .object_snapshot(target)
        .expect("target remains live after the ready tick");
    assert_eq!(
        target_after_ready.local_vars.get("callback_answer"),
        Some(&Value::String("MIXED".to_string().into()))
    );
    assert_eq!(
        target_after_ready.local_vars.get("callback_count"),
        Some(&Value::Int(1))
    );

    app.network = None;
    assert_eq!(
        app.engine
            .call_object_function(target_index, "Open", vec![Value::Int(player)])
            .expect("second query opens"),
        Value::Bool(true)
    );
    let updates_to_tick35 = 35 - app.engine.frame() % 35;
    for _ in 0..updates_to_tick35 {
        app.update().expect("second query reaches Tick35");
    }
    assert!(app.running_chat_controller().is_some());
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(manager);
    let cancel_tick = app.local_control_submission_tick();
    app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
        .expect("F2 cancels the script query");
    let mut submitted = commands.take_submitted_message_board_answers();
    assert_eq!(submitted.len(), 1);
    let (queued_tick, answer) = submitted.pop().expect("F2 queues an empty answer");
    assert_eq!(queued_tick, cancel_tick);
    assert!(answer.answer.is_empty());
    app.apply_ready_controls(
        queued_tick,
        vec![NetworkControl::MessageBoardAnswer(answer)],
    )
    .expect("empty answer removes the query at its ready tick");
    assert!(app.running_chat_controller().is_none());
    assert!(app.engine.active_message_board_input().is_none());
    assert!(app
        .engine
        .player(player)
        .expect("local player remains after cancellation")
        .message_board_queries()
        .is_empty());
    let target_after_cancel = app
        .engine
        .object_snapshot(target)
        .expect("target remains live after cancellation");
    assert_eq!(
        target_after_cancel.local_vars.get("callback_count"),
        Some(&Value::Int(1)),
        "an empty answer removes the query without invoking InputCallback"
    );
}

#[test]
fn goal_rule_activation_on_unresolved_definition_returns_typed_boundary() {
    let mut app = new_running_sandbox_app();
    let player = app.local_owner;

    for (action, child) in [
        (
            MenuAction::GoalInfo("MISS".to_string()),
            ClassicIngameMenuChild::GoalInfo("MISS".to_string()),
        ),
        (
            MenuAction::RuleInfo("MISS".to_string()),
            ClassicIngameMenuChild::RuleInfo("MISS".to_string()),
        ),
    ] {
        let error = app
            .apply_ingame_menu_action_for_player(player, action)
            .expect_err("an unresolved goal/rule must reach its typed menu boundary");
        let EngineError::ClassicMenuParityBoundary { detail } = error else {
            panic!("unexpected unresolved goal/rule error: {error:?}");
        };
        assert_eq!(
                    detail,
                    format!(
                        "classic in-game menu child {child:?} is not implemented; refusing status/no-op substitute"
                    )
                );
    }
}

#[test]
fn ready_ticks_follow_plus_and_minus_one_control_rate_changes() {
    let mut app = new_running_sandbox_app();
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_control_clock = Some(NetworkControlClock::new(9, 2));
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(9, 2).expect("valid timing"),
    );

    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 9,
            controls: vec![NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type: 0,
                data: 1,
                by_client: 0,
            })],
        })
        .expect("queue rate increase");
    app.update().expect("execute rate increase");
    assert_eq!((app.engine.frame(), app.engine.control_rate()), (1, 3));

    app.update().expect("simulate frame one");
    app.update().expect("simulate frame two");
    app.update().expect("stall on the new frame-three cadence");
    assert_eq!(app.engine.frame(), 3);
    assert_eq!(
        app.network_control_clock
            .map(NetworkControlClock::current_tick),
        Some(10)
    );

    event_tx
        .send(NetworkEvent::ReadyTick {
            tick: 10,
            controls: vec![NetworkControl::Set(clonk_network::LegacyControlSet {
                value_type: 0,
                data: -1,
                by_client: 0,
            })],
        })
        .expect("queue rate decrease");
    app.update().expect("execute rate decrease");
    assert_eq!((app.engine.frame(), app.engine.control_rate()), (4, 2));

    app.update()
        .expect("stall immediately on the new even-frame cadence");
    assert_eq!(app.engine.frame(), 4);
    assert_eq!(
        app.network_control_clock
            .map(NetworkControlClock::current_tick),
        Some(11)
    );
}

#[test]
fn host_lobby_sets_update_published_join_data_and_obey_fair_crew_lock() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    let mut snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .expect("default host publishes JoinData");
    snapshot.parameters.fair_crew_forced = true;
    app.host_join_snapshot = Some(snapshot);
    app.engine.set_fair_crew_forced(false);
    let before = (app.engine.use_fair_crew(), app.engine.fair_crew_strength());

    app.execute_control_set(clonk_network::LegacyControlSet {
        value_type: 5,
        data: 777,
        by_client: 0,
    });
    assert_eq!(
        (app.engine.use_fair_crew(), app.engine.fair_crew_strength()),
        before,
        "the synchronized FairCrewForced parameter owns the lobby gate"
    );
    assert!(commands.take_published_join_snapshots().is_empty());

    app.host_join_snapshot
        .as_mut()
        .expect("live snapshot")
        .parameters
        .fair_crew_forced = false;
    for (value_type, data) in [(0, 1), (1, 0), (2, 37), (3, 4), (4, 1), (5, 777)] {
        app.execute_control_set(clonk_network::LegacyControlSet {
            value_type,
            data,
            by_client: 0,
        });
    }

    let parameters = &app
        .host_join_snapshot
        .as_ref()
        .expect("live snapshot")
        .parameters;
    assert_eq!(parameters.control_rate, 2);
    assert!(!parameters.allow_debug);
    assert_eq!(parameters.max_players, 37);
    assert_eq!(parameters.teams.team_distribution, 4);
    assert_eq!(parameters.teams.team_colors, 1);
    assert!(parameters.use_fair_crew);
    assert_eq!(parameters.fair_crew_strength, 777);
    let published = commands.take_published_join_snapshots();
    assert_eq!(published.len(), 6);
    assert_eq!(published.last(), app.host_join_snapshot.as_ref());
}

#[test]
fn fair_crew_set_clears_cached_projection_even_when_parameters_are_unchanged() {
    let mut app = new_state_only_lightweight_running_sandbox_app();
    app.engine.set_use_fair_crew(true);
    app.engine.set_fair_crew_strength(1_000);
    let mut definition = Definition::from_script(
        "FCRW",
        "Fair crew",
        r#"#strict
    static fill_count;
    protected func GetFairCrewPhysical(string name, int rank, &value)
    {
        if (name eq "Energy")
        {
            fill_count += 1;
            value += fill_count;
        }
        return true;
    }
    "#,
    )
    .expect("definition compiles");
    definition.set_crew_member(true);
    definition.set_physical(clonk_engine::PhysicalInfo {
        energy: 50_000,
        scale: 30_000,
        hangle: 30_000,
        swim: 60_000,
        fight: 50_000,
        ..clonk_engine::PhysicalInfo::default()
    });
    app.engine
        .register_definition(definition)
        .expect("definition registers");
    app.engine
        .join_player(clonk_engine::JoinPlayerConfig {
            name: "Fair crew player".to_string(),
            player_info_id: 902,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff0000,
            pref_color: 0,
            pref_position: 0,
            crew: vec![clonk_engine::player_file::CrewInfo {
                core: Default::default(),
                id: "FCRW".to_string(),
                name: "Henry".to_string(),
                death_message: String::new(),
                rank: 1,
                rank_name: "Ensign".to_string(),
                experience: 120,
                rounds: 0,
                physical: clonk_engine::PhysicalInfo {
                    energy: 55_000,
                    scale: 30_000,
                    hangle: 30_000,
                    swim: 60_000,
                    fight: 50_000,
                    ..clonk_engine::PhysicalInfo::default()
                },
                death_count: 0,
                total_playing_time: 0,
                birthday: 0,
                age: 0,
                participation: 1,
                in_action: false,
                was_in_action: false,
                in_action_time: 0,
                has_died: false,
                extra_data: Vec::new(),
                portraits: Default::default(),
            }],
            startup_player_count: 1,
            control_style: false,
            auto_context_menu: false,
        })
        .expect("player joins");
    let crew = app
        .engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "FCRW")
        .map(|object| object.id)
        .expect("fair-crew object exists");
    let crew_index = app
        .engine
        .find_object_index(crew)
        .expect("crew remains live");
    let before = app.engine.object_physical(crew_index);
    assert_eq!(before.energy, 55_001);
    assert_eq!(before.scale, 33_500);

    app.apply_ready_controls(
        14,
        vec![NetworkControl::Set(clonk_network::LegacyControlSet {
            value_type: 5,
            data: 1_000,
            by_client: 0,
        })],
    )
    .expect("FairCrew executes");

    let after = app.engine.object_physical(crew_index);
    assert_eq!(after.energy, 55_002);
    assert_eq!(after.scale, 33_500);
    assert!(app.engine.use_fair_crew());
    assert_eq!(app.engine.fair_crew_strength(), 1_000);
}

#[test]
fn host_deactivates_playerless_remote_before_the_ready_control_gate() {
    // C4Game::Execute calls Network.Execute before Control.Prepare. The
    // player-less client is therefore selected at frame 501 even when
    // network control is currently stopped. OnSec1Timer independently
    // calls Execute too, and native does not suppress a repeated update
    // before the synchronized first one executes (src/C4Game.cpp:776-787;
    // src/C4Network2.cpp:674-700,2148-2159).
    let mut app = new_running_sandbox_app();
    for _ in 0..500 {
        app.engine.tick().expect("advance to strict delay boundary");
    }
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.network_control_running = false;
    app.control_clients.register(3, true, false);

    app.update().expect("scan frame 500 before ready gate");
    assert!(commands.take_submitted_client_updates().is_empty());

    app.engine.tick().expect("advance to first stale frame");
    app.update().expect("scan frame 501 before ready gate");
    let expected = clonk_engine::ClientUpdateControlData {
        update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
        client_id: 3,
        data: 0,
        by_client: 0,
    };
    assert_eq!(
        commands.take_submitted_client_updates(),
        vec![expected.clone()]
    );
    assert!(
        app.control_clients.is_activated(3),
        "submission must not bypass synchronized execution"
    );

    app.sec1_timer()
        .expect("run independent native second callback");
    assert_eq!(commands.take_submitted_client_updates(), vec![expected]);
}

#[test]
fn frozen_lobby_executes_synchronized_activation_immediately() {
    // HandleControlPkt executes synchronized controls immediately while
    // the network is frozen in GS_Lobby, rather than waiting for a game
    // simulation tick (pristine 9ffa0a5d
    // src/C4GameControlNetwork.cpp:558-588).
    let directory = tempdir().expect("record directory");
    let mut app = new_state_only_menu_app(320, 200);
    install_test_recording_template(&mut app, directory.path().join("001-FrozenLobbySync.c4s"));
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), false));
    app.startup_view = StartupView::NetworkLobby;
    app.control_clients.register(3, false, false);
    event_tx
        .send(NetworkEvent::ScheduledSync {
            tick: 0,
            controls: vec![
                NetworkControl::Synchronize(clonk_engine::SynchronizeControlData {
                    save_player_files: false,
                    sync_clearance: true,
                    by_client: 0,
                }),
                NetworkControl::ClientUpdate(clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    client_id: 3,
                    data: 1,
                    by_client: 0,
                }),
            ],
        })
        .expect("queue frozen activation");

    app.process_network_events()
        .expect("execute frozen activation");

    assert!(app.control_clients.is_activated(3));
    assert!(app.network_sync.scheduled.is_empty());
    assert!(app.recording.is_none());
    assert!(app.recording_template.is_some());
}

#[test]
fn frozen_classic_host_lobby_executes_synchronized_activation_immediately() {
    // The exact classic-host projection is the same frozen GS_Lobby
    // control state as the client lobby (src/C4GameControlNetwork.cpp:558-588).
    let mut app = new_menu_app(320, 200);
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    install_test_classic_host_lobby(&mut app);
    app.control_clients.register(3, false, false);
    event_tx
        .send(NetworkEvent::ScheduledSync {
            tick: 0,
            controls: vec![NetworkControl::ClientUpdate(
                clonk_engine::ClientUpdateControlData {
                    update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    client_id: 3,
                    data: 1,
                    by_client: 0,
                },
            )],
        })
        .expect("queue frozen classic-host activation");

    app.process_network_events()
        .expect("execute frozen classic-host activation");

    assert!(app.control_clients.is_activated(3));
    assert!(app.network_sync.scheduled.is_empty());
}

#[test]
fn network_tick_waits_for_exact_ready_batch_and_ignores_duplicate() {
    // C4Game::Execute returns before simulation when control preparation
    // is not ready (src/C4Game.cpp:786-787), then executes the retrieved
    // control before game ticks (src/C4Game.cpp:797-805).
    let mut app = new_running_sandbox_app();
    let remote_owner = app.local_owner + 1;
    app.engine
        .register_player(PlayerConfig::new(remote_owner, "Remote"))
        .expect("register remote player");
    app.engine
        .player_mut(app.local_owner)
        .expect("local player")
        .control
        .control_style = true;
    app.engine
        .player_mut(remote_owner)
        .expect("remote player")
        .control
        .control_style = true;
    let (manager, event_tx) = NetworkManager::test_stub();
    app.network = Some(manager);

    let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
    let initial_frame = app.engine.frame();
    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .expect("submit local input");
    assert_eq!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_RIGHT),
        0,
        "network input is submitted but not dispatched immediately"
    );

    app.update().expect("wait without ready batch");
    assert_eq!(
        app.engine.frame(),
        initial_frame,
        "simulation cannot advance before its exact ready tick"
    );

    let controls = vec![
        NetworkControl::Player {
            owner: app.local_owner,
            event: ControlEvent::Press(ControlButton::Right),
        },
        NetworkControl::Player {
            owner: remote_owner,
            event: ControlEvent::Press(ControlButton::Left),
        },
    ];
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: controls.clone(),
        })
        .expect("queue ready aggregate");
    app.update().expect("execute ready batch");
    assert_eq!(app.engine.frame(), initial_frame + 1);
    assert_ne!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_RIGHT),
        0,
        "local aggregate control is applied"
    );
    assert_ne!(
        app.engine
            .player(remote_owner)
            .expect("remote player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT),
        0,
        "remote aggregate control is applied"
    );
    let local_last_com = app
        .engine
        .player(app.local_owner)
        .expect("local player")
        .control
        .last_com;

    event_tx
        .send(NetworkEvent::ReadyTick { tick, controls })
        .expect("queue duplicate aggregate");
    app.update().expect("ignore duplicate ready batch");
    assert_eq!(
        app.engine.frame(),
        initial_frame + 1,
        "a stale duplicate cannot advance simulation twice"
    );
    assert_eq!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .last_com,
        local_last_com,
        "a stale duplicate cannot dispatch its controls twice"
    );
}

#[test]
fn synchronized_player_select_executes_at_the_ready_tick() {
    let mut app = new_running_sandbox_app();
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
    let (manager, event_tx, _commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: vec![NetworkControl::PlayerSelect(PlayerSelectControlData {
                player: owner,
                objects: vec![second.as_u64() as i32],
                by_client: 7,
            })],
        })
        .expect("queue synchronized selection");

    app.update().expect("execute ready selection");

    assert_eq!(app.engine.selected_crew(owner), vec![second]);
    assert_eq!(app.engine.crew_cursor(owner), Some(second));
    let stats = app.network_stats.as_ref().expect("running stats survive");
    let controls = stats
        .player_control_graph(owner)
        .expect("player control graph survives");
    let actions = stats
        .player_action_graph(owner)
        .expect("player action graph survives");
    assert_eq!(controls.raw_value(controls.end_time() - 1), 1.0);
    assert_eq!(actions.raw_value(actions.end_time() - 1), 1.0);
    let player = app.engine.player(owner).expect("player survives");
    assert_eq!(
        (player.control_count(), player.action_count()),
        (0, 0),
        "the running statistics sample drains native per-control-frame counters"
    );
}

#[test]
fn ready_tick_follow_on_uses_next_open_tick_and_clears_marker() {
    // Input generated while C4Control::Execute is running goes back into
    // Game.Input for the next unsent control tick
    // (src/C4GameControl.cpp:314-318; src/C4GameControlNetwork.cpp:145-176).
    let mut app = new_running_sandbox_app();
    let (manager, event_tx, mut commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    app.open_ingame_menu().expect("open local menu");
    assert!(
        commands.take_submitted_local().is_empty(),
        "opening the main menu does not clear controls"
    );
    let tick = u32::try_from(app.engine.frame()).expect("test tick fits u32");
    event_tx
        .send(NetworkEvent::ReadyTick {
            tick,
            controls: vec![NetworkControl::Player {
                owner: app.local_owner,
                event: ControlEvent::Command {
                    command: ControlCommand::MenuClose,
                    kind: CommandKind::Press,
                },
            }],
        })
        .expect("queue synchronized close");

    app.update().expect("apply ready close");

    assert_eq!(
        commands.take_submitted_local(),
        vec![(
            app.local_owner,
            ControlEvent::ClearPressed,
            tick.saturating_add(1),
        )],
        "a reentrant follow-on targets the next open tick"
    );
    assert_eq!(
        app.executing_ready_tick, None,
        "the ready marker is cleared after application"
    );
}
