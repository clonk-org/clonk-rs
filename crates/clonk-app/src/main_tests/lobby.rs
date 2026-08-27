// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! lobby_fixture {
    (player_named: $name:expr, color: $color:expr $(, $field:ident: $value:expr)* $(,)?) => {
        clonk_engine::ControlPlayerInfoEntry {
            name: $name,
            color: $color,
            original_color: $color,
            $($field: $value,)*
            ..Default::default()
        }
    };
    (player_bytes: $name:expr, color: $color:expr $(, $field:ident: $value:expr)* $(,)?) => {
        lobby_fixture!(player_named:
            clonk_engine::LegacyCString::from_bytes($name.to_vec()).test_value(),
            color: $color,
            $($field: $value,)*
        )
    };
    (player { $($field:ident $(: $value:expr)?),* $(,)? }) => {
        clonk_engine::ControlPlayerInfoEntry {
            $($field $(: $value)?,)*
            ..Default::default()
        }
    };
    (player: $id:expr, $flags:expr $(,)?) => {
        clonk_engine::ControlPlayerInfoEntry {
            id: $id,
            flags: $flags,
            ..Default::default()
        }
    };
    (player_data { $($field:ident $(: $value:expr)?),* $(,)? }) => {
        clonk_engine::PlayerInfoControlData {
            $($field $(: $value)?,)*
            ..Default::default()
        }
    };
    (client { $($field:ident $(: $value:expr)?),* $(,)? }) => {
        clonk_engine::ClientCoreControlData {
            $($field $(: $value)?,)*
            ..Default::default()
        }
    };
    (resource { $($field:ident $(: $value:expr)?),* $(,)? }) => {
        clonk_engine::NetworkResourceCore {
            $($field $(: $value)?,)*
            ..Default::default()
        }
    };
    (player_resource: $id:expr, $filename:expr $(,)?) => {
        clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
            id: $id,
            loadable: true,
            filename: $filename,
            ..Default::default()
        }
    };
    (player_data: $client_id:expr, $players:expr $(,)?) => {
        clonk_engine::PlayerInfoControlData {
            client_id: $client_id,
            players: $players,
            ..Default::default()
        }
    };
    (player_data: $client_id:expr, flags: $flags:expr, $players:expr, by: $by_client:expr $(,)?) => {
        clonk_engine::PlayerInfoControlData {
            client_id: $client_id,
            flags: $flags,
            players: $players,
            by_client: $by_client,
        }
    };
    (host: $port:expr, $player_name:expr, $prepared:expr $(,)?) => {
        HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], $port)),
            player_name: $player_name,
            prepared: $prepared,
        }
    };
    (status: $state:expr, $control_mode:expr, $target_tick:expr $(,)?) => {
        clonk_network::NetworkStatus {
            state: $state,
            control_mode: $control_mode,
            target_tick: $target_tick,
        }
    };
    (control_set: $value_type:expr, $data:expr, $by_client:expr $(,)?) => {
        clonk_network::LegacyControlSet {
            value_type: $value_type,
            data: $data,
            by_client: $by_client,
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
    (join_data: $client_id:expr, $start_tick:expr, $status:expr, $dynamic:expr, $parameters:expr $(,)?) => {
        clonk_network::JoinDataEnvelope {
            client_id: $client_id,
            start_control_tick: $start_tick,
            status: $status,
            dynamic: $dynamic,
            parameters: $parameters,
        }
    };
}

fn networked_client_lobby(
    mut app: GameApp,
    player_name: &str,
    lobby: NetworkLobbyState,
) -> (GameApp, network::NetworkEventSender) {
    let event_tx = install_client_network_stub(&mut app, 7);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name,
    )));
    app.network_lobby = Some(lobby);
    (app, event_tx)
}

fn joined_client_app(mut app: GameApp) -> GameApp {
    app.startup.view = StartupView::NetworkLobby;
    networked_client_lobby(
        app,
        "Client",
        NetworkLobbyState::new(7, "Client".to_string(), false),
    )
    .0
}

fn joined_client_app_with_events(mut app: GameApp) -> (GameApp, network::NetworkEventSender) {
    app.startup.view = StartupView::NetworkLobby;
    networked_client_lobby(
        app,
        "Client",
        NetworkLobbyState::new(7, "Client".to_string(), false),
    )
}

fn joined_client_app_with_commands(
    mut app: GameApp,
) -> (GameApp, crate::network::TestNetworkCommands) {
    app.startup.view = StartupView::NetworkLobby;
    let (app, _events, commands) = networked_client_lobby_with_commands(
        app,
        "Client",
        NetworkLobbyState::new(7, "Client".to_string(), false),
    );
    (app, commands)
}

fn networked_client_lobby_with_commands(
    mut app: GameApp,
    player_name: &str,
    lobby: NetworkLobbyState,
) -> (
    GameApp,
    network::NetworkEventSender,
    crate::network::TestNetworkCommands,
) {
    let (event_tx, commands) = install_client_network_commands(&mut app, 7);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        player_name,
    )));
    app.network_lobby = Some(lobby);
    (app, event_tx, commands)
}

fn networked_host_lobby_with_commands(
    app: GameApp,
    lobby: NetworkLobbyState,
) -> (
    GameApp,
    network::NetworkEventSender,
    crate::network::TestNetworkCommands,
) {
    let (mut app, event_tx, commands) = networked_host_with_commands(app);
    app.network_lobby = Some(lobby);
    (app, event_tx, commands)
}

fn networked_host_with_commands(
    mut app: GameApp,
) -> (
    GameApp,
    network::NetworkEventSender,
    crate::network::TestNetworkCommands,
) {
    let (event_tx, commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    (app, event_tx, commands)
}

fn install_network_stub(app: &mut GameApp) -> network::NetworkEventSender {
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    events
}

fn install_client_network_stub(
    app: &mut GameApp,
    client_id: ClientId,
) -> network::NetworkEventSender {
    let (manager, events) = NetworkManager::test_stub_for_client_id(client_id);
    app.network = Some(manager);
    events
}

fn install_network_commands(
    app: &mut GameApp,
) -> (
    network::NetworkEventSender,
    crate::network::TestNetworkCommands,
) {
    let (manager, events, commands) = NetworkManager::test_stub_with_commands();
    app.network = Some(manager);
    (events, commands)
}

fn install_client_network_commands(
    app: &mut GameApp,
    client_id: ClientId,
) -> (
    network::NetworkEventSender,
    crate::network::TestNetworkCommands,
) {
    let (manager, events, commands) =
        NetworkManager::test_stub_with_commands_for_client_id(client_id);
    app.network = Some(manager);
    (events, commands)
}

fn client_lobby_state() -> NetworkLobbyState {
    NetworkLobbyState::new(7, "Client".to_string(), false)
}

fn host_lobby_state() -> NetworkLobbyState {
    NetworkLobbyState::new(0, "Host".to_string(), true)
}

fn selected_host_lobby_with_commands(
    mut app: GameApp,
) -> (
    GameApp,
    network::NetworkEventSender,
    crate::network::TestNetworkCommands,
) {
    let scenario = FrontendScenario::fallback();
    app.scensel.catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = NetworkLobbyState::new(0, "Host".to_string(), true);
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    networked_host_lobby_with_commands(app, lobby)
}

fn lobby_player_infos(client_id: i32, player_ids: &[i32]) -> clonk_engine::PlayerInfoControlData {
    clonk_engine::PlayerInfoControlData {
        client_id,
        players: player_ids
            .iter()
            .map(|id| lobby_fixture!(player { id: *id }))
            .collect(),
        ..Default::default()
    }
}

fn send_ready_check(
    event_tx: &network::NetworkEventSender,
    client_id: i32,
    data: clonk_network::ReadyCheckData,
) {
    event_tx
        .send(NetworkEvent::ReadyCheck(
            clonk_network::ReadyCheckPacket::new(client_id, data),
        ))
        .test_value();
}

fn send_network_event(event_tx: &network::NetworkEventSender, event: NetworkEvent) {
    event_tx.send(event).test_value();
}

fn send_resource_progress(
    event_tx: &network::NetworkEventSender,
    resource_id: i32,
    present_percent: u8,
) {
    send_network_event(
        event_tx,
        NetworkEvent::ResourceProgress {
            resource_id,
            present_percent,
        },
    );
}

fn send_lobby_countdown(event_tx: &network::NetworkEventSender, countdown: i32) {
    send_network_event(
        event_tx,
        NetworkEvent::LobbyCountdown(clonk_network::LobbyCountdownPacket::new(countdown)),
    );
}

fn send_peer_disconnected(
    event_tx: &network::NetworkEventSender,
    client_id: ClientId,
    reason: Option<&str>,
) {
    send_network_event(
        event_tx,
        NetworkEvent::PeerDisconnected {
            client_id,
            reason: reason.map(str::to_owned),
        },
    );
}

fn send_added_player_infos(
    event_tx: &network::NetworkEventSender,
    origin: u32,
    client_id: i32,
    players: Vec<clonk_engine::ControlPlayerInfoEntry>,
) {
    send_network_event(
        event_tx,
        NetworkEvent::PlayerInfoUpdateRequest {
            origin,
            request: clonk_network::PlayerInfoUpdateRequest {
                client_id,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players,
            },
            by_host: false,
        },
    );
}

fn send_ready_tick(
    event_tx: &network::NetworkEventSender,
    tick: u32,
    controls: Vec<NetworkControl>,
) {
    send_network_event(event_tx, NetworkEvent::ReadyTick { tick, controls });
}

fn process_classic_lobby_action(app: &mut GameApp, action: ClassicLobbyAction) {
    app.process_classic_lobby_actions(vec![action]).test_value();
}

fn request_classic_lobby_start(app: &mut GameApp, countdown_seconds: i32) {
    process_classic_lobby_action(
        app,
        ClassicLobbyAction::StartRequested {
            countdown_seconds,
            check_league_rules: true,
            confirm_unassociated_savegame_players: false,
        },
    );
}

fn process_joined_lobby_action(app: &mut GameApp, action: LobbyAction) {
    app.process_lobby_action(action).test_value();
}

fn process_lobby_game_option(app: &mut GameApp, input: LobbyGameOptionInput) {
    process_classic_lobby_action(app, ClassicLobbyAction::GameOptions(input));
}

fn accept_game_option_input(app: &mut GameApp, text: &str) {
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        text.to_string(),
    )])
    .test_value();
}

fn select_classic_lobby_sheet(app: &mut GameApp, sheet: LobbySheet) {
    process_classic_lobby_action(app, ClassicLobbyAction::SheetRequested(sheet));
}

fn request_classic_lobby_team(app: &mut GameApp, player_id: i32) {
    process_classic_lobby_action(
        app,
        ClassicLobbyAction::TeamSelectionRequested { player_id },
    );
}

fn process_classic_lobby_chat_action(app: &mut GameApp, request: LobbyChatRequest) {
    process_classic_lobby_action(app, ClassicLobbyAction::Chat(request));
}

fn process_lobby_chat_request(app: &mut GameApp, request: LobbyChatRequest) {
    app.process_classic_lobby_chat_request(request).test_value();
}

fn click_network_lobby(app: &mut GameApp, point: GuiPoint) {
    app.handle_network_lobby_pointer_move(point).test_value();
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .test_value();
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .test_value();
}

fn joined_lobby_row_point(app: &mut GameApp, id: LobbyRosterId) -> GuiPoint {
    let (_, roster) = app.joined_lobby_layouts().test_value();
    let lobby = app_lobby(app);
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
        .test_value();
    GuiPoint::new((row.rect.x + 2) as f32, (row.rect.y + 2) as f32)
}

fn joined_lobby_tab_point(app: &mut GameApp, sheet: LobbySheet) -> GuiPoint {
    let layout = app_lobby_mut(&mut app.network_lobby)
        .update_layout(640.0, 480.0)
        .clone();
    let rect = layout
        .sheet_buttons
        .iter()
        .find(|(candidate, _)| *candidate == sheet)
        .test_value()
        .1;
    GuiPoint::new(
        rect.origin.x + rect.size.width / 2.0,
        rect.origin.y + rect.size.height / 2.0,
    )
}

fn assert_lobby_countdowns(commands: &mut crate::network::TestNetworkCommands, countdowns: &[i32]) {
    main_assert_eq!(commands.take_submitted_lobby_countdowns() => countdowns.iter().copied().map(clonk_network::LobbyCountdownPacket::new).collect::<Vec<_>>());
}

fn assert_ready_checks(
    commands: &mut crate::network::TestNetworkCommands,
    client_id: i32,
    data: clonk_network::ReadyCheckData,
) {
    main_assert_eq!(commands.take_submitted_ready_checks() => vec![clonk_network::ReadyCheckPacket { client_id, data }]);
}

fn join_team_selection_player(
    app: &mut GameApp,
    name: &str,
    player_info_id: i32,
    color_dw: u32,
) -> i32 {
    app.engine
        .join_player_for_team_selection(JoinPlayerConfig {
            name: name.to_string(),
            player_info_id,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .test_value()
}

fn app_lobby(app: &GameApp) -> &NetworkLobbyState {
    app.network_lobby.as_ref().test_value()
}

fn app_lobby_mut(lobby: &mut Option<NetworkLobbyState>) -> &mut NetworkLobbyState {
    lobby.as_mut().test_value()
}

fn app_classic_lobby(app: &GameApp) -> &ClassicHostLobbyState {
    app.classic_host_lobby.as_ref().test_value()
}

fn app_classic_lobby_mut(lobby: &mut Option<ClassicHostLobbyState>) -> &mut ClassicHostLobbyState {
    lobby.as_mut().test_value()
}

fn some<T>(value: &Option<T>) -> &T {
    value.as_ref().test_value()
}

fn some_mut<T>(value: &mut Option<T>) -> &mut T {
    value.as_mut().test_value()
}

#[test]
fn startup_player_count_uses_roster_only_at_frame_zero() {
    main_assert_eq!(startup_player_count_for_init(0, Some(7), Some(2)) => Some(2), "fresh games overwrite a stale serialized parameter");
    main_assert_eq!(startup_player_count_for_init(37, Some(7), Some(1)) => Some(7), "runtime restores retain their original startup scalar");
    main_assert_eq!(startup_player_count_for_init(-1, Some(0), Some(3)) => Some(0), "the native branch tests exact zero rather than positive frames");
    main_assert_eq!(startup_player_count_for_init(0, Some(4), None) => None);
}

#[test]
fn classic_command_line_lobby_timeout_starts_the_host_countdown() {
    main_assert_eq!(parse_classic_command_line(&[OsString::from("/network")]).lobby_timeout => None,);
    main_assert_eq!(parse_classic_command_line(&[OsString::from("/lobby")]).lobby_timeout => Some(None),);
    let mut app = new_state_only_menu_app(320, 200);
    let scenario = FrontendScenario::fallback();
    app.scensel.catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = host_lobby_state();
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    let (_events, mut commands) = install_network_commands(&mut app);
    app.apply_classic_command_line(&ClassicCommandLine {
        scenario: Some(PathBuf::from("Fixture.c4s")),
        network_active: Some(true),
        lobby_timeout: Some(Some(120)),
        ..ClassicCommandLine::default()
    })
    .test_value();

    app.finish_classic_command_line_host_entry().test_value();

    assert_lobby_countdowns(&mut commands, &[120]);
    main_assert_eq!(app.host_lobby_countdown => Some(HostLobbyCountdown::with_seconds(120)));
}

#[test]
fn classic_command_line_network_scenario_skips_unrequested_lobby() {
    let mut app = new_state_only_menu_app(320, 200);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
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

    app.finish_classic_command_line_host_entry().test_value();

    main_assert!(some(&app.lobby_preload_task).start_host_when_ready);
    main_assert_eq!(app.mode => AppMode::Loading);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
}

#[test]
fn already_failed_client_start_resource_never_opens_a_stale_progress_wait() {
    let mut app = new_menu_app(800, 600);
    let _event_tx = install_network_stub(&mut app);
    let core = lobby_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Definitions as u8,
        id: 12,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"Network\\Objects.c4d".to_vec())
            .test_value(),
    });
    app.admission_resources.register_lobby_resource(&core);
    app.admission_resources.mark_failed(core.id);

    app.wait_for_client_start_resource(PendingClientStartResource {
        role: ClientStartResourceRole::GameResource { index: 0 },
        core,
    })
    .test_value();

    main_assert!(app.network.is_none());
    main_assert!(app.blocking_resource_wait.is_none());
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.message_dialogs[0].state.caption() => "Error Log");
    main_assert_eq!(app.message_dialogs[0].state.message() => "Unable to retrieve Object Definition: Objects.c4d.");
}

#[test]
fn network_control_catch_up_drains_ten_ready_ticks_in_one_pass() {
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(11, 1);
    let frame_before = app.engine.frame();

    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();

    main_assert_eq!(outcome.executed_frames => 8);
    main_assert_eq!(app.engine.frame() => frame_before + 8);
    main_assert_eq!(app.network_control_pacing().behind => 3);
    main_assert!(!app.network_control_pacing().overflow);
    main_assert_eq!(accumulator => Duration::ZERO);
}

#[test]
fn automatic_frame_skip_is_consumed_by_an_already_suppressed_pass() {
    let mut frame_skip = AutomaticFrameSkip::default();
    frame_skip.finish_graphics_pass(true, Duration::from_millis(29), Duration::from_millis(28));

    frame_skip.consume_suppressed_graphics_pass();
    main_assert!(!frame_skip.begin_graphics_pass(true));
}

#[test]
fn network_control_catch_up_stops_at_a_ready_tick_gap() {
    let mut gate = NetworkTickGate::default();
    gate.queue(7, 7, Vec::new());
    gate.queue(7, 11, Vec::new());

    main_assert_eq!(gate.contiguous_ready_behind(7) => 1);

    gate.queue(7, 8, Vec::new());
    gate.queue(7, 9, Vec::new());
    let mut inspected = 0;
    main_assert_eq!(gate.contiguous_ready_behind_if(7, |_| {inspected += 1; inspected < 2}) => 1, "a future control that fails PreExecute ends the ready prefix");
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
    .test_value();

    let mut definition = Definition::from_script("WLKR", "Walker", walker_script()).test_value();
    definition.set_crew_member(true);
    app.engine.register_definition(definition).test_value();
    let start = clonk_engine::scenario::PlayerStart {
        ready_crew: vec![("WLKR".to_string(), 2)],
        ..Default::default()
    };
    app.engine.set_player_starts(vec![start]);

    app.join_local_player().test_value();

    // C4PlayerList::GetFreeNumber (C4PlayerList.cpp:189-201): the
    // first joining player takes number 0.
    main_assert_eq!(app.local_owner => 0, "local owner adopts the joined number");
    let snapshot = app.engine.snapshot();
    let crew: Vec<_> = snapshot
        .objects
        .iter()
        .filter(|object| object.crew_member && object.owner == app.local_owner)
        .collect();
    main_assert_eq!(crew.len() => 2, "Crew=WLKR=2 places two ready crew members");
    let selection = snapshot.crew_selection.get(&app.local_owner).test_value();
    main_assert!(
        selection.cursor.is_some(),
        "cursor lands on a crew member at join"
    );

    // A second call must not join (or place crew) twice.
    app.join_local_player().test_value();
    main_assert_eq!(app.engine.players().count() => 1, "rejoining does not add a second player");
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
    .test_value();
    app.engine
        .load_scenario_script_with_convention(
            "local join audio fixture",
            "#strict 3\nglobal func InitializePlayer(int plr) { Sound(\"LocalJoin\", true, nil, 100, plr + 1); }",
            true,
        ).test_value();
    app.engine.set_local_players([]);

    app.join_local_player().test_value();

    main_assert!(
        app.test_audio_ref()
            .missing_sounds
            .contains("request::localjoin"),
        "InitializePlayer sound reaches the local audio host"
    );
}

#[test]
fn network_too_few_warning_ok_stages_and_enters_exact_lobby() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let scenario_group = tempdir();
    fs::write(scenario_group.path().join("Scenario.txt"), "[Head]\nTitle=Needs players\nMinPlayer=2\nMaxPlayer=4\n\n[Definitions]\nAllowUserChange=true\n").test_value();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "Participants", "").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "NeedsPlayers.c4s".to_string();
    scenario.title = "Needs players".to_string();
    scenario.path = Some(scenario_group.path().to_path_buf());
    scenario.allow_user_change = Some(true);
    let scenarios = vec![scenario.clone()];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);
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
    .test_value();
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.message_dialogs[0].state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL);

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
        .test_value();

    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.definition_selector.is_none());
    let staged = some(&app.staged_network_host_scenario);
    main_assert_eq!(staged.frontend.identifier => scenario.identifier);
    main_assert_eq!(staged.frontend.title => scenario.title);

    let blocker = app.startup_network_connection.take().test_value();
    drop(blocker);
    drop(blocker_sender);

    let (manager, _events) = NetworkManager::test_stub();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Host(lobby_fixture!(host: 0, "Exact Host".to_string(), None)),
            manager,
        )))
        .test_value();
    app.begin_startup_network_connection(
        receiver,
        StartupNetworkPurpose::StagedHost,
        Some((scenario.identifier.clone(), scenario.title.clone())),
        None,
    )
    .test_value();
    app.poll_startup_network_connection().test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(app.network.is_some());
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.status_text.is_empty());
    let lobby = &app_classic_lobby(&app).controller;
    main_assert_eq!(lobby.role() => LobbyRole::Host);
    main_assert_eq!(lobby.title() => "Needs players - Lobby");
    main_assert!(matches!(
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
    app.network_lobby = Some(client_lobby_state().with_preloading(false, LobbyLabels::default()));
    app.startup.view = StartupView::NetworkLobby;
    app.sync_network_lobby_game_option_state();
    let assets = Arc::clone(&app.assets);
    let layout = app_lobby_mut(&mut app.network_lobby)
        .with_classic_controller_input(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
            |_, layout, _| layout.clone(),
        )
        .test_value();
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

    app.handle_network_lobby_pointer_move(point).test_value();
    // Below C4GUI_ToolTipShowTime the retained clock keeps the tip hidden.
    main_assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));
    main_assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));

    // The tracker survives the per-frame projection rather than being
    // rebuilt: a reconstructed controller would restart the clock at draw
    // time and lose the hover owner.
    let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
    app_lobby_mut(&mut app.network_lobby)
        .render_classic(
            &mut surface,
            assets.as_ref(),
            &app.scenario_game_options,
            false,
            true,
            &startup_identity_gamma().clone(),
        )
        .test_value();
    main_assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));
    main_assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));

    // Non-pointer input suppresses until real motion, exactly as on the host.
    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert!(!tooltip_at(&app, Instant::now() + Duration::from_secs(1)));
    app.handle_network_lobby_pointer_move(point).test_value();
    main_assert!(
        !tooltip_at(&app, Instant::now() + Duration::from_secs(1)),
        "same-pixel motion must not reactivate the tooltip"
    );
    point.x += 1.0;
    app.handle_network_lobby_pointer_move(point).test_value();
    main_assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));

    // A new hover target takes ownership and restarts the clock, so the
    // retained tracker reports the newly owned element's text.
    let chat_tip = tooltip_text(&app).test_value();
    let exit = GuiPoint::new(
        (layout.exit_button.x + layout.exit_button.w / 2) as f32,
        (layout.exit_button.y + layout.exit_button.h / 2) as f32,
    );
    app.handle_network_lobby_pointer_move(exit).test_value();
    main_assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));
    let exit_tip = tooltip_text(&app).test_value();
    main_assert_ne!(chat_tip => exit_tip);
    app.handle_network_lobby_pointer_move(point).test_value();

    // A wheel is mouse input, so `ResetToolTipTime` restarts the stillness
    // clock rather than disabling the tip outright.
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert!(!tooltip_at(
        &app,
        Instant::now() + Duration::from_millis(100)
    ));
    main_assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));

    // A covering modal owns the pointer, so the lobby beneath draws no tip.
    point.x += 1.0;
    app.handle_network_lobby_pointer_move(point).test_value();
    main_assert!(tooltip_at(&app, Instant::now() + Duration::from_secs(1)));
    main_assert!(app
        .render_startup_tooltips()
        .expect("uncovered lobby tooltip pass"));
    app.open_network_join_password_dialog().test_value();
    main_assert!(!app
        .render_startup_tooltips()
        .expect("covered lobby tooltip pass"));
}

#[test]
fn persistent_classic_lobby_non_pointer_input_suppresses_tooltip_until_motion() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (layout, _) = app.classic_host_lobby_layouts().test_value();
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

    app.handle_classic_lobby_pointer_move(point).test_value();
    main_assert!(tooltip_visible(&app));

    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert!(!tooltip_visible(&app));
    app.handle_classic_lobby_pointer_move(point).test_value();
    main_assert!(
        !tooltip_visible(&app),
        "same-pixel motion must not reactivate the tooltip"
    );

    point.x += 1.0;
    app.handle_classic_lobby_pointer_move(point).test_value();
    main_assert!(tooltip_visible(&app));
    app.test_text_input('x');
    main_assert!(!tooltip_visible(&app));

    point.x += 1.0;
    app.handle_classic_lobby_pointer_move(point).test_value();
    main_assert!(tooltip_visible(&app));
    app.handle_gamepad_event(GamepadEvent::Direction {
        slot: GamepadSlot::new(0),
        button: ControlButton::Right,
        state: ElementState::Pressed,
    })
    .test_value();
    main_assert!(!tooltip_visible(&app));

    point.x += 1.0;
    app.handle_classic_lobby_pointer_move(point).test_value();
    main_assert!(tooltip_visible(&app));

    let option_rect = app
        .scenario_game_options
        .layout()
        .rect(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
        .test_value();
    let option_point = GuiPoint::new(
        (option_rect.x + option_rect.w / 2) as f32,
        (option_rect.y + option_rect.h / 2) as f32,
    );
    app.handle_classic_lobby_pointer_move(option_point)
        .test_value();
    main_assert!(app
        .scenario_game_options
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert!(app
        .scenario_game_options
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_none());
    app.handle_classic_lobby_pointer_move(option_point)
        .test_value();
    main_assert!(
        app.scenario_game_options
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_none(),
        "the embedded option strip shares CMouse same-pixel suppression"
    );
    app.handle_classic_lobby_pointer_move(GuiPoint::new(option_point.x + 1.0, option_point.y))
        .test_value();
    main_assert!(app
        .scenario_game_options
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    app.handle_classic_lobby_secondary_button(ElementState::Released)
        .test_value();
    main_assert!(
        app.scenario_game_options
            .tooltip_state_at(Instant::now() + Duration::from_millis(400))
            .is_none(),
        "right-button release resets the embedded option tooltip clock"
    );
    app.handle_classic_lobby_middle_button(ElementState::Released)
        .test_value();
    main_assert!(
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
    let seed = app_classic_lobby(&app).controller.rows()[0].clone();
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
    app_classic_lobby_mut(&mut app.classic_host_lobby)
        .controller
        .set_rows(rows);

    let (layout, roster) = app.classic_host_lobby_layouts().test_value();
    main_assert!(roster.max_scroll > 0);
    let first_row = roster.rows.first().test_value();
    let point = GuiPoint::new((first_row.rect.x + 2) as f32, (first_row.rect.y + 2) as f32);
    app.handle_classic_lobby_pointer_move(point).test_value();
    main_assert!(app_classic_lobby(&app)
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());

    app.handle_classic_lobby_wheel(-60).test_value();
    main_assert!(
        app_classic_lobby(&app)
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
        .test_value();
    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    app.handle_classic_lobby_wheel(60).test_value();
    main_assert!(
        app_classic_lobby(&app)
            .controller
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_some(),
        "an unconsumed wheel re-establishes hover with a fresh delay"
    );

    let mut short_lobby = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut short_lobby);
    let (_, short_roster) = short_lobby.classic_host_lobby_layouts().test_value();
    main_assert_eq!(short_roster.max_scroll => 0);
    let row = short_roster.rows.first().test_value();
    let row_point = GuiPoint::new((row.rect.x + 2) as f32, (row.rect.y + 2) as f32);
    short_lobby
        .handle_classic_lobby_pointer_move(row_point)
        .test_value();
    main_assert!(short_lobby
        .classic_host_lobby
        .as_ref()
        .expect("test lobby")
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    short_lobby.handle_classic_lobby_wheel(60).test_value();
    main_assert!(
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
    let (_, roster) = app.classic_host_lobby_layouts().test_value();
    roster
        .rows
        .iter()
        .find(|row| row.index == 1)
        .and_then(|row| row.team)
        .test_value()
}

#[test]
fn staged_host_completion_enters_exact_lobby_over_loader_background() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "Network", "Nick", "Exact Nick").test_value();
    persist_config_value(&paths, "Lobby", "CountdownTime", "7").test_value();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    // The production launcher applies C4Config.cpp:1671-1674 before the
    // app starts, enabling shader gamma for every migrated installation.
    persist_config_value(&paths, "Graphics", "Shader", "1").test_value();
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    main_assert!(
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
    main_assert!(
        staged.loader_screen.is_some(),
        "loader is selected before bind"
    );
    main_assert_eq!(staged.lobby.local_name => "Exact Host");
    main_assert_ne!(staged.lobby.local_name => app.player_name);
    let expected_title = staged.loader_screen.test_ref().state().title().to_string();
    main_assert_eq!(expected_title => staged.frontend.title, "the selected loader's pack-aware title is retained for JoinData");
    app.staged_network_host_scenario = Some(staged);
    app.network_client_activity.mark_activated(99, 123);

    let (manager, _events) = NetworkManager::test_stub();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Host(lobby_fixture!(host: 11112, "Exact Host".to_string(), None)),
            manager,
        )))
        .test_value();
    app.begin_startup_network_connection(receiver, StartupNetworkPurpose::StagedHost, None, None)
        .test_value();
    main_assert_eq!(app.mode => AppMode::Loading);
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(some(&app.loader_screen).state().title() => expected_title);
    app.poll_startup_network_connection().test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert!(app.network_client_activity.last_frame.is_empty());
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(
        app.network.is_some(),
        "host listener remains owned by lobby"
    );
    main_assert!(matches!(app.network_mode, Some(NetworkMode::Host(_))));
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.classic_host_lobby.is_some());
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(app.scenario_game_options.context() => GameOptionContext::LobbyHost);
    main_assert_eq!(app.scenario_game_options.values().password => accepted.password);
    main_assert_eq!(app.scenario_game_options.values().comment => accepted.comment);
    main_assert_eq!(
        app.scenario_game_options.values().fair_crew_strength =>
        accepted.fair_crew_strength,
        "C++ fills zero scenario strength from accepted config when fair crew is active"
    );
    main_assert!(!app.scenario_game_options.values().countdown);

    let lobby = &app_classic_lobby(&app).controller;
    main_assert_eq!(lobby.role() => LobbyRole::Host);
    main_assert_eq!(lobby.title() => format!("{expected_title} - Lobby"));
    main_assert_eq!(lobby.focus() => LobbyControl::ChatInput);
    main_assert!(!lobby.ready());
    main_assert_eq!(lobby.countdown() => clonk_frontend::game_lobby::LobbyCountdownState::None);
    main_assert!(matches!(
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
    main_assert_eq!(projected_player_ids => authoritative_player_ids);
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let surface = app.graphics.surface();
    let layout = lobby.layout(surface.width() as i32, surface.height() as i32, fonts);
    main_assert_eq!(app.scenario_game_options.layout().bounds => layout.game_option_strip);

    let config = app.loader_render_config.test_value();
    let mut background = Surface::new(800, 600, PixelFormat::Rgba8888);
    some(&app.loader_screen).render_background(&mut background, config, app.loader_gamma.as_ref());
    let expected_corner = background.pixels()[..4].to_vec();
    let mut frame = vec![0_u8; 800 * 600 * 4];
    main_assert!(app.render(&mut frame).expect("render exact host lobby"));
    main_assert_eq!(&frame[..4] => expected_corner.as_slice());
    main_assert!(app.render(&mut frame).expect("render live lobby again"));

    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_ne!(app_classic_lobby(&app).controller.focus() => LobbyControl::ChatInput);
    app.test_cursor(PhysicalPosition::new(0.0, 0.0));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);
    app.test_touch(TouchPhase::Started, GuiPoint::new(0.0, 0.0));
    app.test_touch(TouchPhase::Ended, GuiPoint::new(0.0, 0.0));
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();

    app.show_main_menu();
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.network_control_running);
    main_assert!(app.control_clients.contains(0));
    main_assert!(app.control_clients.is_activated(0));
}

#[test]
fn staged_host_installs_activated_participant_before_building_lobby_roster() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
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
    .test_value();
    let expected_alternate_colors = prepared.local_player_alternate_colors_by_resource().clone();
    main_assert_eq!(expected_alternate_colors.len() => 1);
    app.staged_network_host_scenario = Some(staged);

    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
    let admission = thread::spawn(move || {
        let (allowed, completion) = commands.receive_join_allowed();
        main_assert!(allowed, "prepared participant opens lobby admission");
        completion.send(Ok(())).test_value();
    });
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Host(
                lobby_fixture!(host: 11112, "Exact Host".to_string(), Some(prepared)),
            ),
            manager,
        )))
        .test_value();
    app.begin_startup_network_connection(receiver, StartupNetworkPurpose::StagedHost, None, None)
        .test_value();
    app.poll_startup_network_connection().test_value();
    admission.test_join();

    main_assert!(app.status_text.is_empty(), "{}", app.status_text);
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(app.network_lobby.is_none());
    main_assert_eq!(
        app.host_local_alternate_colors_by_resource => expected_alternate_colors,
        "the prepared host retains its process-local AlternateColorDw sidecar"
    );
    main_assert_eq!(app.host_local_player_info_ids.len() => 1);
    let lobby = &app_classic_lobby(&app).controller;
    main_assert_eq!(lobby.players_title() => "&Players (1/1)");
    let [LobbyRosterRow::Client(client), LobbyRosterRow::Player(player)] = lobby.rows() else {
        panic!(
            "expected local client followed by activated player: {:?}",
            lobby.rows()
        );
    };
    main_assert_eq!(client.id => 0);
    main_assert_eq!(client.name => "Exact Host");
    main_assert_eq!(client.color => [0x3b, 0x3b, 0xff, 0xff]);
    main_assert_eq!(player.client_id => 0);
    main_assert_eq!(player.name => "Exact Player");
    main_assert_eq!(player.color => [0x3b, 0x3b, 0xff, 0xff]);
    main_assert!(
        matches!(&player.icon, LobbyRosterIcon::Raster(icon) if icon.width() == 1 && icon.height() == 1 && icon.pixels() == [12, 34, 56, 255])
    );

    let (_, retained) = app.control_player_infos.retained_rows_snapshot();
    let retained_player = &retained[0].2[0];
    main_assert_eq!(retained_player.id => player.id);
    let resource = retained_player.resource.test_ref();
    main_assert!(
        app.admission_resources.complete_path(resource.id).is_some(),
        "activated player resource is installed before lobby admission"
    );
}

#[test]
fn classic_lobby_identity_sanitizes_native_bytes() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let config_with_identity = |name: &[u8], nick: &[u8]| {
        let mut config = b"[Network]\nLocalName=\"".to_vec();
        config.extend_from_slice(name);
        config.extend_from_slice(b"\"\nNick=\"");
        config.extend_from_slice(nick);
        config.extend_from_slice(b"\"\n\n[Lobby]\nCountdownTime=7\n");
        config
    };
    let load_identity = |name: &[u8], nick: &[u8]| {
        fs::write(paths.config_file(), config_with_identity(name, nick)).test_value();
        load_classic_lobby_identity(&paths).test_value()
    };
    let maximum_name = b"\xc3\xa4".repeat(15);
    fs::write(
        paths.config_file(),
        config_with_identity(&maximum_name, b"N\xc3\xa4ck"),
    )
    .test_value();

    let (local_name, nick, countdown) =
        load_classic_lobby_identity_with_hostname_provider(&paths, || {
            panic!("explicit LocalName must not query the system hostname")
        })
        .test_value();

    main_assert_eq!(
        clonk_resources::encode_legacy_script_text(&local_name) =>
        Some(maximum_name.clone()),
        "valid UTF-8-shaped bytes must not collapse during CP1252 encoding"
    );
    main_assert_eq!(clonk_resources::encode_legacy_script_text(&nick) => Some(b"N\xc3\xa4ck".to_vec()));
    main_assert_eq!(countdown => 7);

    let overlong_name = b"\xc3\xa4".repeat(16);
    let (local_name, _, _) = load_identity(&overlong_name, b"Nick");
    main_assert_eq!(clonk_resources::encode_legacy_script_text(&local_name) => Some(maximum_name), "C4MaxName truncation counts native bytes");

    let mut removable_prefix = vec![b'{'; 1_025];
    removable_prefix.extend_from_slice(b"Alice");
    let (local_name, _, _) = load_identity(&removable_prefix, b"Nick");
    main_assert_eq!(local_name => "Alice");

    let dirty_name = b"  {<i>Guessed</i><c G> Host</c>}}<future>  ";
    let dirty_nick = b"  <i>Guessed Nick</i>}}  ";
    let (local_name, nick, _) = load_identity(dirty_name, dirty_nick);
    main_assert_eq!(local_name => "Guessed Host<future>");
    main_assert_eq!(nick => "Guessed Nick");

    let (local_name, nick, _) = load_identity(b"Exact Host", b" {<i></i>}} ");
    main_assert_eq!((local_name.as_str(), nick.as_str()) => ("Exact Host", "Exact Host"));

    let (_, nick, _) = load_identity(b"Exact Host", b"<i<i>>");
    main_assert_eq!(nick => "Unknown");

    let (local_name, nick, _) = load_identity(b"<i<i<i<i>>>>", b"");
    main_assert_eq!(local_name => "<i<i>>");
    main_assert_eq!(nick => "Unknown");

    fs::write(
        paths.config_file(),
        b"[Network]\nNick=\"\"\n\n[Lobby]\nCountdownTime=7\n",
    )
    .test_value();
    let (local_name, nick, _) =
        load_classic_lobby_identity_with_hostname(&paths, b"H\xc3\xa4st").test_value();
    main_assert_eq!(clonk_resources::encode_legacy_script_text(&local_name) => Some(b"H\xc3\xa4st".to_vec()));
    main_assert_eq!(nick => local_name);

    fs::write(
        paths.config_file(),
        config_with_identity(b"<i>Unknown</i>", b""),
    )
    .test_value();
    let (local_name, nick, _) =
        load_classic_lobby_identity_with_hostname(&paths, b"Tylers-MacBook-Pro-M4-Max.local")
            .test_value();
    main_assert_eq!(local_name => "Tylers-MacBook-Pro-M4-Ma");
    main_assert_eq!(nick => local_name);

    main_assert_eq!(
        sanitize_classic_lobby_name("", "test name", false).unwrap() =>
        "empty",
        "an initially empty VAL_NameNoEmpty input uses the generic guard literal"
    );
    main_assert_eq!(
        sanitize_classic_lobby_name("<i></i>", "test name", false).unwrap() =>
        "Unknown",
        "a nonempty input cleaned to empty uses the name-validator fallback"
    );
    main_assert_eq!(sanitize_classic_lobby_name("", "test nick", true).unwrap() => "");
    main_assert!(sanitize_classic_lobby_name("☃", "test name", false).is_err());
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
        main_assert!(
            error.to_string().contains(expected),
            "{expected} boundary missing from {error}"
        );
    }

    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    select_classic_lobby_sheet(&mut app, LobbySheet::Players);
    select_classic_lobby_sheet(&mut app, LobbySheet::Scenario);
    main_assert_eq!(app_classic_lobby(&app).controller.active_sheet() => LobbySheet::Scenario);
}

#[test]
fn lobby_options_refresh_only_while_the_sheet_is_active() {
    let mut app = new_menu_app(640, 480);
    let (_events, _commands) = install_classic_host_network_stub(&mut app);
    app.engine.set_control_rate(4);
    app.network_control_clock = Some(NetworkControlClock::new(0, 4));

    select_classic_lobby_sheet(&mut app, LobbySheet::Options);
    let option_value = |app: &GameApp, kind| {
        app_classic_lobby(app)
            .controller
            .option_rows()
            .iter()
            .find(|row| row.kind == kind)
            .test_value()
            .value
            .clone()
    };
    main_assert_eq!(option_value(&app, LobbyOptionKind::ControlRate) => "4");

    select_classic_lobby_sheet(&mut app, LobbySheet::Players);
    some_mut(&mut app.network_control_clock).set_control_rate(7);
    app.sec1_timer().test_value();
    main_assert_eq!(option_value(&app, LobbyOptionKind::ControlRate) => "4", "inactive options retain their last snapshot");

    select_classic_lobby_sheet(&mut app, LobbySheet::Options);
    main_assert_eq!(option_value(&app, LobbyOptionKind::ControlRate) => "7");

    some_mut(&mut app.network_control_clock).set_control_rate(8);
    app.sec1_timer().test_value();
    main_assert_eq!(option_value(&app, LobbyOptionKind::ControlRate) => "8");
}

#[test]
fn classic_lobby_internet_signup_is_pollable_and_rolls_back_failure() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    main_assert!(!app.scenario_game_options.values().master_server_signup);
    let enable = commands.receive_masterserver_signup();
    main_assert!(enable.enabled);
    main_assert!(!enable.config.league_server_signup);
    main_assert_eq!(enable.reference.summary().state => "Lobby");
    enable.complete(Ok(Some(clonk_network::LeagueStartResponse::default())));
    app.poll_live_masterserver_signup().test_value();
    main_assert!(app.scenario_game_options.values().master_server_signup);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    main_assert!(app.scenario_game_options.values().master_server_signup);
    let disable = commands.receive_masterserver_signup();
    main_assert!(!disable.enabled);
    disable.complete(Ok(None));
    app.poll_live_masterserver_signup().test_value();
    main_assert!(!app.scenario_game_options.values().master_server_signup);
    main_assert!(!app.scenario_game_options.values().league_server_signup);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    let rejected = commands.receive_masterserver_signup();
    main_assert!(rejected.enabled);
    rejected.complete(Err("masterserver rejected the game".to_string()));
    app.poll_live_masterserver_signup().test_value();
    main_assert!(!app.scenario_game_options.values().master_server_signup);
    main_assert!(app.status_text.contains("masterserver rejected the game"));
}

#[test]
fn aborting_live_internet_signup_keeps_the_prior_off_state() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    main_assert!(!app.scenario_game_options.values().master_server_signup);
    main_assert!(app.pending_lobby_internet_signup.is_some());
    main_assert!(
        !app.network_game_start_guard_passes(),
        "a host cannot launch while a Start or compensating End is unresolved"
    );
    let wait = app.message_dialogs.last().test_value();
    main_assert!(matches!(
        wait.continuation,
        MessageDialogContinuation::LiveMasterserverSignup
    ));
    main_assert_eq!(wait.state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Standard(3));
    main_assert_eq!(wait.state.button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel) => "Abort");

    let pending_command = commands.receive_masterserver_signup();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    pending_command.wait_for_cancellation();
    main_assert!(app.pending_lobby_internet_signup.is_none());
    main_assert!(!app.scenario_game_options.values().master_server_signup);
    main_assert_eq!(app.status_text => "Internet game signup cancelled.");
}

#[test]
fn live_signup_applies_every_start_response_field() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (snapshot, reference) = default_exact_host_reference();
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);
    let league = LegacyCString::from_bytes(b"Cup".to_vec()).test_value();
    let stream_to =
        LegacyCString::from_bytes(b"https://stream.example/upload?".to_vec()).test_value();
    let response = clonk_network::LeagueStartResponse {
        league: league.clone(),
        stream_to: stream_to.clone(),
        seed: Some(0x1234_5678),
        max_players: 4,
        ..clonk_network::LeagueStartResponse::default()
    };

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    commands
        .receive_masterserver_signup()
        .complete(Ok(Some(response)));
    app.poll_live_masterserver_signup().test_value();

    let parameters = &some(&app.host_join_snapshot).parameters;
    main_assert_eq!(parameters.league => league);
    main_assert_eq!(parameters.random_seed => 0x1234_5678);
    main_assert_eq!(parameters.max_players => 4);
    main_assert_eq!(app.network_stream_address => stream_to);
    main_assert_eq!(app.network_league_name => b"Cup");
    main_assert_eq!(app.network_max_players => 4);
    main_assert_eq!(app.engine.max_players() => Some(4));
    let reference = some(&app.advertised_game_reference);
    main_assert_eq!(reference.parameters().league.as_bytes() => b"Cup");
    main_assert_eq!(reference.parameters().random_seed => 0x1234_5678);
    main_assert_eq!(reference.parameters().max_players => 4);
    let published = commands.take_published_join_snapshots();
    main_assert_eq!(published.len() => 1);
    main_assert_eq!(published[0].parameters.league.as_bytes() => b"Cup");
    main_assert_eq!(published[0].parameters.random_seed => 0x1234_5678);
    main_assert_eq!(published[0].parameters.max_players => 4);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    let disable = commands.receive_masterserver_signup();
    main_assert!(!disable.enabled);
    disable.complete(Ok(None));
    app.poll_live_masterserver_signup().test_value();

    let parameters = &some(&app.host_join_snapshot).parameters;
    main_assert!(parameters.league.is_empty());
    main_assert!(parameters.league_address.is_empty());
    main_assert_eq!(parameters.random_seed => 0x1234_5678);
    main_assert_eq!(parameters.max_players => 4);
    main_assert_eq!(app.network_stream_address => stream_to);
    main_assert!(app.network_league_name.is_empty());
    let reference = some(&app.advertised_game_reference);
    main_assert!(reference.parameters().league.is_empty());
    main_assert!(reference.parameters().league_address.is_empty());
    main_assert_eq!(reference.parameters().random_seed => 0x1234_5678);
    main_assert_eq!(reference.parameters().max_players => 4);
    let published = commands.take_published_join_snapshots();
    main_assert_eq!(published.len() => 1);
    main_assert!(published[0].parameters.league.is_empty());
    main_assert!(published[0].parameters.league_address.is_empty());
    main_assert_eq!(published[0].parameters.random_seed => 0x1234_5678);
    main_assert_eq!(published[0].parameters.max_players => 4);
}

#[test]
fn committed_start_apply_failure_tears_down_when_cleanup_cannot_start() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    let signup = commands.receive_masterserver_signup();
    app.advertised_game_reference = None;
    signup.complete(Ok(Some(clonk_network::LeagueStartResponse::default())));
    app.poll_live_masterserver_signup().test_value();

    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.message_dialogs.last().is_some_and(|dialog| dialog
        .state
        .message()
        .contains("could not begin compensating Internet signup cleanup")));
}

#[test]
fn leaving_lobby_during_compensating_end_preserves_worker_cleanup() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (snapshot, reference) = default_exact_host_reference();
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    commands
        .receive_masterserver_signup()
        .complete(Ok(Some(clonk_network::LeagueStartResponse {
            league: LegacyCString::from_bytes(b"Cup".to_vec()).test_value(),
            seed: Some(0x1234_5678),
            max_players: -1,
            ..clonk_network::LeagueStartResponse::default()
        })));
    app.poll_live_masterserver_signup().test_value();
    main_assert!(
        app.pending_lobby_internet_signup.is_some(),
        "the committed Start must remain visible until End"
    );
    let cleanup = commands.receive_masterserver_signup();
    main_assert!(!cleanup.enabled);

    app.show_main_menu();

    main_assert!(app.pending_lobby_internet_signup.is_none());
    main_assert!(app.network.is_none());
    cleanup.wait_for_cleanup_preservation();
}

#[test]
fn failed_live_end_tears_the_host_down() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (snapshot, reference) = default_exact_host_reference();
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    commands
        .receive_masterserver_signup()
        .complete(Ok(Some(clonk_network::LeagueStartResponse::default())));
    app.poll_live_masterserver_signup().test_value();
    commands.take_published_join_snapshots();

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    commands
        .receive_masterserver_signup()
        .complete(Err("End transport failed".to_owned()));
    app.poll_live_masterserver_signup().test_value();

    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.message_dialogs.last().is_some_and(|dialog| dialog
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
        .test_value();
    let reference = prepared.initial_host_game_reference(true, &[]).test_value();
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    app.host_join_snapshot = Some(initial_snapshot);
    app.advertised_game_reference = Some(reference);
    app.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 11112, "Host".to_owned(), Some(prepared)),
    ));
    main_assert!(!app.scenario_game_options.values().master_server_signup);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('I'));
    let signup = commands.receive_masterserver_signup();
    main_assert!(signup.enabled);
    signup.complete(Ok(Some(clonk_network::LeagueStartResponse {
        league: LegacyCString::from_bytes(b"Cup".to_vec()).test_value(),
        seed: Some(1_784_903_470),
        ..clonk_network::LeagueStartResponse::default()
    })));
    app.poll_live_masterserver_signup().test_value();

    main_assert!(
        app.pending_lobby_internet_signup.is_some(),
        "the committed Start must retain a compensating End transaction"
    );
    main_assert!(app.scenario_game_options.values().master_server_signup);
    main_assert!(
        !app.network_game_start_guard_passes(),
        "the host remains blocked while the compensating End is unresolved"
    );
    app.start_network_game_now().test_value();
    main_assert!(matches!(app.mode, AppMode::Menu));
    main_assert!(app.status_text.contains("1784903470"));
    main_assert_eq!(some(&app.host_join_snapshot).parameters.random_seed => 1_784_903_471, "the rejected response must not partially replace the local seed");

    let rollback = commands.receive_masterserver_signup();
    main_assert!(!rollback.enabled);
    rollback.complete(Ok(None));
    app.poll_live_masterserver_signup().test_value();

    main_assert!(app.pending_lobby_internet_signup.is_none());
    main_assert!(!app.scenario_game_options.values().master_server_signup);
    main_assert_eq!(some(&app.host_join_snapshot).parameters.random_seed => 1_784_903_471);
    main_assert!(
        app.status_text.contains("1784903470"),
        "successful cleanup preserves the actionable rejection"
    );
    let scenario = match app.network_mode.as_ref() {
        Some(NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        })) => prepared.claim_scenario().test_value(),
        _ => panic!("cleanup retains the prepared host"),
    };
    main_assert!(!scenario.generated_landscape_requires_seed_retry());
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
            .test_value(),
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
            completion.send(result).test_value();
        }
        passwords
    });

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('P'));
    main_assert_eq!(app.scenario_game_options.values().password => "old password");
    main_assert!(
        some(&app.advertised_game_reference)
            .summary()
            .password_needed
    );

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('P'));
    main_assert!(app.scenario_game_options.values().password.is_empty());
    main_assert!(
        !some(&app.advertised_game_reference)
            .summary()
            .password_needed
    );

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('P'));
    main_assert_eq!(some(&app.game_option_input_dialog).controller.text() => "remembered password");
    accept_game_option_input(&mut app, "unsupported 🔒");
    main_assert!(app.scenario_game_options.values().password.is_empty());
    main_assert_eq!(app.scenario_game_options.values().last_password => "remembered password");
    main_assert!(
        !some(&app.advertised_game_reference)
            .summary()
            .password_needed
    );

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('P'));
    accept_game_option_input(&mut app, "new password");

    main_assert_eq!(app.scenario_game_options.values().password => "new password");
    main_assert_eq!(app.scenario_game_options.values().last_password => "new password");
    main_assert!(
        some(&app.advertised_game_reference)
            .summary()
            .password_needed
    );
    main_assert_eq!(observer.join().expect("password observer") => vec![Vec::<u8>::new(), Vec::<u8>::new(), b"new password".to_vec(),]);
}

#[test]
fn classic_lobby_comment_updates_and_invalidates_the_advertised_reference() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 11112, "Exact Host".to_string(), None),
    ));
    let (manager, _events, mut commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    app.network = Some(manager);
    app.scenario_game_options.set_comment("old comment");
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(
        reference
            .replacing_lobby_options(
                false,
                LegacyCString::from_bytes(b"old comment".to_vec()).test_value(),
            )
            .test_value(),
    );

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('M'));
    accept_game_option_input(&mut app, "old comment");
    main_assert_eq!(commands.take_league_update_effects().1 => 0);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('M'));
    accept_game_option_input(&mut app, "unsupported 💬");
    main_assert_eq!(app.scenario_game_options.values().comment => "old comment");
    main_assert_eq!(commands.take_league_update_effects().1 => 0);
    main_assert!(app_classic_lobby(&app).controller.logs().is_empty());
    main_assert_eq!(some(&app.advertised_game_reference).metadata().comment.as_bytes() => b"old comment");

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('M'));
    accept_game_option_input(&mut app, "new comment");

    main_assert_eq!(commands.take_league_update_effects().1 => 1);
    main_assert_eq!(some(&app.advertised_game_reference).metadata().comment.as_bytes() => b"new comment");
    main_assert_eq!(
        app_classic_lobby(&app)
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()) =>
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

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('F'));
    let sets = commands.take_submitted_control_sets();
    main_assert_eq!(sets => vec![lobby_fixture!(control_set: 5, 75, 0)]);
    main_assert!(!app.scenario_game_options.values().fair_crew);
    app.execute_control_set(sets[0]);
    main_assert!(app.scenario_game_options.values().fair_crew);

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('F'));
    main_assert_eq!(commands.take_submitted_control_sets()[0].data => -1);

    app.scenario_game_options.set_countdown(true);
    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('F'));
    main_assert!(commands.take_submitted_control_sets().is_empty());

    app.scenario_game_options.set_countdown(false);
    app.scenario_game_options.set_lobby_fair_crew(false, true);
    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('F'));
    main_assert!(commands.take_submitted_control_sets().is_empty());

    process_lobby_game_option(&mut app, LobbyGameOptionInput::Hotkey('R'));
    main_assert!(app.scenario_game_options.values().record);
    main_assert!(app.startup.view_flags.record);
}

/// `C4PacketCountdown::GetCountdownMsg` (src/C4GameLobby.cpp:50-60): under
/// `AlmostStartCountdownTime` the message is a bare `"n..."`, but the *initial*
/// packet is always the full `IDS_PRC_COUNTDOWN` sentence however small its
/// value — `MainDlg::OnCountdownPacket` passes `!fWasCountdown` for that flag
/// (`:415`), and a dedicated host with no dialog logs the same text
/// (`:1125-1127`, `:1150-1157`).
#[test]
fn lobby_countdown_messages_shorten_only_after_the_opening_packet() {
    let template = "Game starts in {seconds} seconds";

    // The opening packet spells it out at every value, including 0 and 1.
    for seconds in [0, 1, 9, 10, 30] {
        main_assert_eq!(lobby_countdown_message(seconds, true, template) => format!("Game starts in {seconds} seconds"), "initial packet at {seconds}");
    }

    // Later packets shorten strictly below AlmostStartCountdownTime (10).
    for seconds in [0, 1, 9] {
        main_assert_eq!(lobby_countdown_message(seconds, false, template) => format!("{seconds}..."));
    }
    for seconds in [10, 11, 60] {
        main_assert_eq!(lobby_countdown_message(seconds, false, template) => format!("Game starts in {seconds} seconds"), "the boundary is exclusive at {seconds}");
    }
}

/// `C4GameLobby::Countdown::OnSec1Timer` at zero: a host with no lobby dialog
/// — which is what a dedicated engine has, `fFullscreenLobby` being
/// `!Console.Active && (lpDDraw->GetEngine() != GFXENGN_NOGFX)`
/// (src/C4Network2.cpp:463) — logs `IDS_MSG_NOTENOUGHPLAYERSFORTHISRO` and
/// quits the application rather than starting a round short of
/// `Game.C4S.GetMinPlayer()` (src/C4GameLobby.cpp:1159-1170). A host that is
/// showing a lobby dialog always starts, because a person is watching it.
#[test]
fn countdown_zero_quits_a_dialogless_host_that_is_short_of_min_players() {
    let short_host = |dialogless: bool| {
        let mut app = new_menu_app(640, 480);
        let (events, commands) = install_classic_host_network_stub(&mut app);
        app.headless = dialogless;
        app.network_lobby_min_players = Some(2);
        app.control_player_infos.replace_snapshot(
            1,
            [lobby_fixture!(player_data:
                0,
                vec![lobby_fixture!(player {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Lonely".to_vec()).unwrap(),
                })],
            )],
        );
        app.host_lobby_countdown = Some(HostLobbyCountdown::with_seconds(1));
        (app, events, commands)
    };

    let (mut server, _events, _commands) = short_host(true);
    server.tick_network_lobby_countdown();
    main_assert!(
        server.take_exit_request(),
        "a dialogless host quits instead of starting a short round"
    );
    main_assert!(server.host_lobby_countdown.is_none());
    main_assert_ne!(server.mode => AppMode::Loading, "the aborted round must not begin loading");

    // The same shortfall in front of a lobby dialog still starts: C++ gates
    // the abort on `!Game.Network.GetLobby()` alone.
    let (mut interactive, _events, _commands) = short_host(false);
    interactive.tick_network_lobby_countdown();
    main_assert!(!interactive.take_exit_request());

    // And a dialogless host that has its minimum starts normally.
    let (mut ready, _events, _commands) = short_host(true);
    ready.network_lobby_min_players = Some(1);
    ready.tick_network_lobby_countdown();
    main_assert!(!ready.take_exit_request());
}

#[test]
fn classic_host_configured_countdown_uses_sparse_packets_and_abort_unlocks_options() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);

    request_classic_lobby_start(&mut app, 12);

    assert_lobby_countdowns(&mut commands, &[12]);
    main_assert_eq!(app_classic_lobby(&app).controller.countdown() => clonk_frontend::game_lobby::LobbyCountdownState::Long { seconds: 12 });
    main_assert!(!app.scenario_game_options.values().countdown);

    main_assert!(!app.tick_network_lobby_countdown());
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(app.tick_network_lobby_countdown());
    assert_lobby_countdowns(&mut commands, &[10]);
    main_assert!(app.scenario_game_options.values().countdown);

    process_classic_lobby_action(&mut app, ClassicLobbyAction::AbortCountdownRequested);
    assert_lobby_countdowns(&mut commands, &[-1]);
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert_eq!(app_classic_lobby(&app).controller.countdown() => clonk_frontend::game_lobby::LobbyCountdownState::None);
    main_assert!(!app.scenario_game_options.values().countdown);
    main_assert_eq!(app_classic_lobby(&app).controller.logs().last().map(|line| line.text.as_str()) => Some("Game start aborted."));
}

#[test]
fn classic_host_zero_countdown_enters_go_without_a_countdown_packet() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let prepared = prepare_staged_network_host(&app, &staged);
    let expected_go = lobby_fixture!(status:
        clonk_network::NETWORK_STATE_GO,
        prepared.host_config().initial_status.control_mode,
        0,
    );
    app.host_join_snapshot = prepared.host_config().initial_join_snapshot.clone();
    app.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 11112, "Exact Host".to_string(), Some(prepared)),
    ));
    let (_events, mut commands) = install_network_commands(&mut app);
    let mut generic_lobby = NetworkLobbyState::new(0, "Exact Host".to_string(), true);
    generic_lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(generic_lobby);
    install_test_classic_host_lobby(&mut app);
    let go_observer = thread::spawn(move || commands.complete_lobby_start(Ok(())));

    request_classic_lobby_start(&mut app, 0);

    main_assert_eq!(go_observer.join().expect("atomic Go observer") => vec![network::TestLobbyStartCommand::BeginGo {status: expected_go, join_allowed: false,}]);
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert!(matches!(app.mode, AppMode::Loading));
    main_assert!(app.classic_host_lobby.is_none());
    main_assert!(app
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| !wait.visible));
}

#[test]
fn atomic_go_worker_failure_is_reported_before_lobby_teardown() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    let prepared = prepare_staged_network_host(&app, &staged);
    let expected_go = lobby_fixture!(status:
        clonk_network::NETWORK_STATE_GO,
        prepared.host_config().initial_status.control_mode,
        0,
    );
    app.host_join_snapshot = prepared.host_config().initial_join_snapshot.clone();
    app.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 11112, "Exact Host".to_string(), Some(prepared)),
    ));
    let (_events, mut commands) = install_network_commands(&mut app);
    let mut generic_lobby = NetworkLobbyState::new(0, "Exact Host".to_string(), true);
    generic_lobby.select_scenario(&scenario.identifier, &scenario.title);
    app.network_lobby = Some(generic_lobby);
    install_test_classic_host_lobby(&mut app);
    let go_observer = thread::spawn(move || {
        commands.complete_lobby_start(Err("host loop rejected Go".to_string()))
    });

    request_classic_lobby_start(&mut app, 0);

    main_assert_eq!(go_observer.join().expect("atomic Go observer") => vec![network::TestLobbyStartCommand::BeginGo {status: expected_go, join_allowed: false,}]);
    main_assert!(!matches!(app.mode, AppMode::Loading));
    main_assert!(app.loading_state.is_none());
    main_assert!(app.classic_host_lobby.is_some());
    main_assert!(app.network_lobby.is_some());
    main_assert!(app.status_text.contains("host loop rejected Go"));
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
        [lobby_fixture!(player_data: 7, vec![lobby_fixture!(player { id: 1 })])],
    );
    app.sync_classic_lobby_roster();

    process_classic_lobby_action(&mut app, ClassicLobbyAction::ReadyChanged(true));
    assert_ready_checks(&mut commands, 0, clonk_network::ReadyCheckData::Ready);
    main_assert!(app_classic_lobby(&app).controller.ready());
    main_assert!(app.host_lobby_countdown.is_none());

    events
        .send(NetworkEvent::ReadyCheck(
            clonk_network::ReadyCheckPacket::new(7, clonk_network::ReadyCheckData::Ready),
        ))
        .test_value();
    app.test_network_events();

    main_assert!(app_classic_lobby(&app)
                .controller
                .rows()
                .iter()
                .any(|row| matches!(row, LobbyRosterRow::Client(client) if client.id == 7 && client.status == LobbyClientStatus::Ready)));
    assert_lobby_countdowns(&mut commands, &[5]);
    main_assert_eq!(app.host_lobby_countdown => Some(HostLobbyCountdown::new()));
    main_assert_eq!(
        app_classic_lobby(&app)
            .controller
            .logs()
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>() =>
        ["Client Remote ready.", "The game will start in 5 seconds."]
    );

    events
        .send(NetworkEvent::ReadyCheck(
            clonk_network::ReadyCheckPacket::new(7, clonk_network::ReadyCheckData::Ready),
        ))
        .test_value();
    app.test_network_events();
    main_assert_eq!(app_classic_lobby(&app).controller.logs().last().map(|line| line.text.as_str()) => Some("Client Remote ready."));
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
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
        .send(NetworkEvent::ReadyCheck(
            clonk_network::ReadyCheckPacket::new(7, clonk_network::ReadyCheckData::Ready),
        ))
        .test_value();
    app.test_network_events();
    request_classic_lobby_start(&mut app, 12);

    let logs = app_classic_lobby(&app).controller.logs();
    main_assert_eq!(logs.len() => 2);
    main_assert!(logs[0].text.ends_with(" Client Remote ready."));
    main_assert_ne!(logs[0].text => "Client Remote ready.");
    main_assert!(logs[1]
        .text
        .ends_with(" The game will start in 12 seconds."));
    main_assert_ne!(logs[1].text => "The game will start in 12 seconds.");
}

#[test]
fn classic_host_chat_start_abort_and_readycheck_use_live_lobby_actions() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);

    if let Some(lobby) = app.classic_host_lobby.as_mut() {
        lobby.chat_history_index = 3;
        lobby.controller.set_chat_draft("stale");
    }
    process_lobby_chat_request(&mut app, LobbyChatRequest::Submit(String::new()));
    main_assert_eq!(app.sound.ui_log => ["Error"]);
    main_assert!(commands.take_submitted_messages().is_empty());
    let lobby = app_classic_lobby(&app);
    main_assert_eq!(lobby.chat_history_index => -1);
    main_assert!(lobby.controller.chat_edit_view().text.is_empty());

    process_lobby_chat_request(&mut app, LobbyChatRequest::Submit("/start 12".to_string()));
    assert_lobby_countdowns(&mut commands, &[12]);

    process_lobby_chat_request(&mut app, LobbyChatRequest::Submit("/start 3".to_string()));
    assert_lobby_countdowns(&mut commands, &[-1, 3]);

    process_lobby_chat_request(&mut app, LobbyChatRequest::Submit("/abort".to_string()));
    assert_lobby_countdowns(&mut commands, &[-1]);
    process_lobby_chat_request(&mut app, LobbyChatRequest::Submit("/abort".to_string()));
    main_assert_eq!(app_classic_lobby(&app).controller.logs().last().map(|line| (&*line.text, line.color)) => Some(("Not in countdown!", [255, 32, 32, 255])));

    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::Submit("/readycheck".to_string()),
    );
    assert_ready_checks(&mut commands, 0, clonk_network::ReadyCheckData::Request);
}

#[test]
fn unknown_lobby_command_is_a_local_nonfatal_cpp_error() {
    let mut app = new_menu_app(640, 480);
    install_classic_host_network_stub(&mut app);
    app.show_log_timestamps = true;

    process_lobby_chat_request(&mut app, LobbyChatRequest::Submit("/xyz".to_string()));

    main_assert_eq!(
        app_classic_lobby(&app).controller.logs().last() =>
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
fn set_maxplayer_submits_sync_control_and_refreshes_lobby_count() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);

    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::Submit("/set maxplayer 4".to_string()),
    );
    let sets = commands.take_submitted_control_sets();
    main_assert_eq!(sets => [lobby_fixture!(control_set: 2, 4, 0)]);

    app.execute_control_set(sets[0]);
    main_assert_eq!(app.engine.max_players() => Some(4));
    main_assert_eq!(app.network_max_players => 4);
    let lobby = &app_classic_lobby(&app).controller;
    main_assert!(lobby.players_title().contains("/4"));
    main_assert_eq!(lobby.logs().last().map(|line| line.text.as_str()) => Some("MaxPlayer = 4"));
}

#[test]
fn set_faircrew_submits_native_values_and_obeys_lobby_gates() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "DefCrewStrength", "75").test_value();
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
        process_lobby_chat_request(&mut app, LobbyChatRequest::Submit(command.to_string()));
    }
    main_assert_eq!(
        commands.take_submitted_control_sets() =>
        [
            lobby_fixture!(control_set: 5, 75, 0),
            lobby_fixture!(control_set: 5, -1, 0),
            lobby_fixture!(control_set: 5, 42, 0),
        ]
    );

    app.network_is_league = true;
    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::Submit("/set faircrew on".to_string()),
    );
    main_assert!(commands.take_submitted_control_sets().is_empty());

    app.network_is_league = false;
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::Submit("/set faircrew on".to_string()),
    );
    main_assert!(commands.take_submitted_control_sets().is_empty());

    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::Submit("/set faircrew -1".to_string()),
    );
    main_assert!(commands.take_submitted_control_sets().is_empty());
}

#[test]
fn reached_network_start_wait_uses_host_roster_and_client_abort_dialog() {
    let status = lobby_fixture!(status: clonk_network::NETWORK_STATE_GO, 1, 4);

    let mut host = new_menu_app(640, 480);
    host.network_mode = Some(NetworkMode::Host(host_network_settings()));
    host.begin_network_start_wait(status);
    host.show_reached_network_start_wait().test_value();
    main_assert!(host
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| wait.visible && wait.expected_status == status));
    main_assert!(host.message_dialogs.is_empty());

    let mut client = new_menu_app(640, 480);
    client.mode = AppMode::Loading;
    client.network_mode = Some(NetworkMode::Client(client_network_settings()));
    client.show_reached_network_start_wait().test_value();
    main_assert!(client.network_start_wait.is_none());
    let [dialog] = client.message_dialogs.as_slice() else {
        panic!("client should have exactly one start-wait dialog");
    };
    main_assert!(matches!(
        dialog.continuation,
        MessageDialogContinuation::NetworkClientStartWait
    ));
    main_assert_eq!(dialog.state.message() => "Waiting for start...");
    main_assert_eq!(dialog.state.caption() => "Network");
    main_assert_eq!(dialog.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::CANCEL);
    main_assert_eq!(dialog.state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Standard(3));
    main_assert_eq!(dialog.state.size() => clonk_frontend::message_dialog::MessageDialogSize::Small);
    main_assert_eq!(dialog.state.focused_button() => None);
    main_assert_eq!(dialog.state.button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel) => "Cancel");

    let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
    let blank = surface.pixels().to_vec();
    let gamma = client.startup_fragment_gamma();
    client
        .render_loading_message_dialogs(&mut surface, &gamma)
        .test_value();
    main_assert_ne!(surface.pixels() => blank.as_slice());
}

#[test]
fn classic_lobby_resource_sheet_refreshes_only_while_active() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    app_classic_lobby_mut(&mut app.classic_host_lobby)
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
        select_classic_lobby_sheet(&mut app, sheet);
        main_assert_eq!(app_classic_lobby(&app).controller.active_sheet() => sheet);
    }

    app.select_classic_lobby_sheet(LobbySheet::Resources);
    main_assert_eq!(app_classic_lobby(&app).controller.resource_rows()[0].present_percent => 10);
    let events = install_network_stub(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    send_resource_progress(&events, 7, 33);
    app.test_network_events();
    main_assert_eq!(app_classic_lobby(&app).controller.resource_rows()[0].present_percent => 33);

    app.select_classic_lobby_sheet(LobbySheet::Players);
    send_resource_progress(&events, 7, 66);
    app.test_network_events();
    let lobby = app_classic_lobby(&app);
    main_assert_eq!(lobby.resource_rows[&7].present_percent => 66);
    main_assert_eq!(lobby.controller.resource_rows()[0].present_percent => 33, "inactive C4Network2ResDlg does not reconcile its visible rows");
    events
        .send(NetworkEvent::ResourceComplete {
            resource_id: 7,
            core: lobby_fixture!(resource {
                id: 7,
                loadable: true,
                filename: LegacyCString::from_bytes(b"Network/Scenario.c4s".to_vec()).unwrap(),
            }),
            path: PathBuf::from("Network/Scenario.c4s"),
            local: false,
        })
        .test_value();
    app.test_network_events();
    main_assert_eq!(app_classic_lobby(&app).resource_rows[&7].present_percent => 100);
    main_assert_eq!(app_classic_lobby(&app).controller.resource_rows()[0].present_percent => 33);
    app.select_classic_lobby_sheet(LobbySheet::Resources);
    main_assert_eq!(app_classic_lobby(&app).controller.resource_rows()[0].present_percent => 100, "activation forces an immediate resource-row refresh");
    events
        .send(NetworkEvent::ResourceLoadFailed { resource_id: 7 })
        .test_value();
    app.test_network_events();
    main_assert!(app_classic_lobby(&app).resource_rows.is_empty());
    main_assert!(app_classic_lobby(&app)
        .controller
        .resource_rows()
        .is_empty());
    app.register_classic_lobby_player_resources(&[lobby_fixture!(player {
        flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
        resource: Some(lobby_fixture!(resource {
            id: 7,
            loadable: true,
            filename: LegacyCString::from_bytes(b"Network/Scenario.c4s".to_vec()).test_value(),
        })),
    })]);
    main_assert!(
        app_classic_lobby(&app).resource_rows.is_empty(),
        "an authoritative PlayerInfo replay cannot resurrect a failed resource"
    );
}

#[test]
fn generic_client_lobby_external_irc_button_is_retained_and_emits_typed_action() {
    let mut lobby = client_lobby_state().with_external_chat(true);

    let pure_layout = lobby.update_layout(640.0, 480.0).clone();
    main_assert!(pure_layout.external_chat_button.is_some());

    let app = new_menu_app(640, 480);
    let (controller, _) = lobby
        .classic_render_state(
            app.graphics.surface(),
            app.assets.as_ref(),
            &app.scenario_game_options,
        )
        .test_value();
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    main_assert!(controller
        .layout(640, 480, fonts)
        .tab_buttons
        .iter()
        .any(|button| button.control == LobbyControl::ChatDialog));

    let rect = lobby
        .layout
        .as_ref()
        .and_then(|layout| layout.external_chat_button)
        .test_value();
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
        .test_value();
    main_assert_eq!(down => Vec::new());
    main_assert_eq!(lobby.controller.take_sounds() => [LobbySound::ArrowHit]);
    let up = lobby
        .with_classic_controller_input(
            app.graphics.surface(),
            app.assets.as_ref(),
            &app.scenario_game_options,
            |controller, layout, roster| {
                controller.pointer_up(point, layout, roster, Instant::now())
            },
        )
        .test_value();
    main_assert_eq!(up => [ClassicLobbyAction::Chat(LobbyChatRequest::OpenExternalDialog)], "the single controller emission reaches the routed Chat arm");
    main_assert_eq!(lobby.controller.take_sounds() => [LobbySound::Click]);

    let mut inactive = client_lobby_state();
    main_assert!(inactive
        .update_layout(640.0, 480.0)
        .external_chat_button
        .is_none());
}

#[test]
fn joined_lobby_chat_routes_pointer_context_and_log_scroll() {
    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(client_lobby_state());
    {
        let lobby = app_lobby_mut(&mut app.network_lobby);
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
    let (layout, max_scroll) = app_lobby_mut(&mut app.network_lobby)
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
        .test_value();
    main_assert!(max_scroll > 0);
    let font = &assets.clonk_fonts.as_deref().test_value().text;
    let edit_y = (layout.chat_edit.y + layout.chat_edit.h / 2) as f32;
    let beta = GuiPoint::new(
        (layout.chat_edit.x + 4 + font.measure("alpha b", false).0) as f32,
        edit_y,
    );

    app.handle_network_lobby_pointer_move(beta).test_value();
    app.handle_network_lobby_pointer_button(ElementState::Pressed, true)
        .test_value();
    main_assert_eq!(app_lobby(&app).chat_edit.selection => Some((6, 10)),);
    let label_point = GuiPoint::new(
        (layout.chat_label.x + layout.chat_label.w / 2) as f32,
        (layout.chat_label.y + layout.chat_label.h / 2) as f32,
    );
    click_network_lobby(&mut app, label_point);
    main_assert_eq!(app_lobby(&app).chat_edit.selection => Some((6, 10)), "C4GUI::Dialog::SetFocus preserves the edit selection when focus is unchanged",);
    app.handle_network_lobby_pointer_move(beta).test_value();
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .test_value();
    main_assert!(app.context_menu.is_some());
    app.handle_network_lobby_secondary_button(ElementState::Released)
        .test_value();
    app.handle_network_lobby_middle_button(ElementState::Released)
        .test_value();
    app.context_menu = None;
    app_lobby_mut(&mut app.network_lobby).pointer = None;
    app.handle_network_lobby_context_key().test_value();
    main_assert!(app.context_menu.is_some());
    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::ContextCommand(LobbyChatContextCommand::Clear),
    );
    main_assert_eq!(app_lobby(&app).chat_edit.text => "alpha ",);
    app.context_menu = None;

    let roster_point = GuiPoint::new(
        (layout.roster_client.x + 2) as f32,
        (layout.roster_client.y + 2) as f32,
    );
    click_network_lobby(&mut app, roster_point);
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::Roster,);
    app.keyboard_modifiers = ModifiersState::ALT;
    app.test_key(VirtualKeyCode::KeyT, ElementState::Pressed);
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::ChatInput,);
    app.keyboard_modifiers = ModifiersState::empty();
    click_network_lobby(&mut app, roster_point);
    app.keyboard_modifiers = ModifiersState::CONTROL;
    main_assert!(!app
        .handle_network_lobby_chat_key(VirtualKeyCode::KeyA, ElementState::Pressed)
        .expect("unfocused joined edit rejects Ctrl+A"));
    app.keyboard_modifiers = ModifiersState::empty();
    main_assert_eq!(app_lobby(&app).chat_edit.text => "alpha ",);
    {
        let lobby = app_lobby_mut(&mut app.network_lobby);
        let caret = lobby.chat_edit.caret;
        main_assert_eq!(lobby.handle_key(KeyCode::Enter, ElementState::Pressed) => None, "an unfocused joined edit must not submit through the reduced adapter",);
        main_assert_eq!(lobby.handle_key(KeyCode::Left, ElementState::Pressed) => None, "an unfocused joined edit must not consume cursor keys",);
        main_assert_eq!(lobby.chat_edit.caret => caret);
        main_assert_eq!(lobby.chat_edit.text => "alpha ");
    }
    let unfocused_view = app_lobby(&app).chat_edit.clone();
    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    main_assert_eq!(app_lobby(&app).chat_edit => unfocused_view,);
    app.handle_network_lobby_pointer_move(beta).test_value();
    app.handle_network_lobby_pointer_button(ElementState::Pressed, true)
        .test_value();
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::Roster, "C4GUI::Edit::LeftDouble does not run the ordinary focus-changing LeftDown path",);
    app.test_text_input('z');
    main_assert_eq!(app_lobby(&app).chat_edit.text => "z",);
    app.test_text_input('\u{80}');
    main_assert_eq!(app_lobby(&app).chat_edit.text => "z\u{80}",);
    let exact_modifier_view = app_lobby(&app).chat_edit.clone();
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
        app.test_key(key, ElementState::Pressed);
        main_assert_eq!(app_lobby(&app).chat_edit => exact_modifier_view,);
    }
    app.keyboard_modifiers = ModifiersState::empty();

    let log_point = GuiPoint::new(
        (layout.chat_log_client.x + 2) as f32,
        (layout.chat_log_client.y + 2) as f32,
    );
    app.handle_network_lobby_pointer_move(log_point)
        .test_value();
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);
    let wheel_scroll = app_lobby(&app).controller.chat_scroll();
    main_assert!(wheel_scroll < max_scroll);

    let assets = Arc::clone(&app.assets);
    app_lobby_mut(&mut app.network_lobby)
        .classic_render_state(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
        )
        .test_value();
    main_assert_eq!(app_lobby(&app).controller.chat_scroll() => wheel_scroll, "a frame projection must not repin the retained TextWindow",);

    let scrollbar_start = GuiPoint::new(
        (layout.chat_log_scrollbar.x + layout.chat_log_scrollbar.w / 2) as f32,
        (layout.chat_log_scrollbar.y + layout.chat_log_scrollbar.h / 2) as f32,
    );
    let scrollbar_end = GuiPoint::new(scrollbar_start.x, (layout.chat_log_scrollbar.y + 17) as f32);
    app.handle_network_lobby_touch(TouchPhase::Started, scrollbar_start, false)
        .test_value();
    app.handle_network_lobby_touch(TouchPhase::Moved, scrollbar_end, false)
        .test_value();
    app.handle_network_lobby_touch(TouchPhase::Ended, scrollbar_end, false)
        .test_value();
    let scrollbar_scroll = app_lobby(&app).controller.chat_scroll();
    main_assert!(scrollbar_scroll < wheel_scroll);
    let assets = Arc::clone(&app.assets);
    app_lobby_mut(&mut app.network_lobby)
        .classic_render_state(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
        )
        .test_value();
    main_assert_eq!(app_lobby(&app).controller.chat_scroll() => scrollbar_scroll,);

    app.handle_network_lobby_touch(TouchPhase::Started, scrollbar_start, false)
        .test_value();
    app.cancel_underlying_interaction();
    let cancelled_scroll = app_lobby(&app).controller.chat_scroll();
    app.handle_network_lobby_touch(TouchPhase::Moved, scrollbar_end, false)
        .test_value();
    main_assert_eq!(app_lobby(&app).controller.chat_scroll() => cancelled_scroll, "an elevated dialog cancellation must not strand the joined TextWindow drag",);

    let drag_start = GuiPoint::new((layout.chat_edit.x + 4) as f32, edit_y);
    let drag_end = GuiPoint::new(
        (layout.chat_edit.x + layout.chat_edit.w + 40) as f32,
        edit_y,
    );
    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::InsertText("touch selection".to_string()),
    );
    app.handle_network_lobby_touch(TouchPhase::Started, drag_start, false)
        .test_value();
    app.handle_network_lobby_touch(TouchPhase::Moved, drag_end, false)
        .test_value();
    app.handle_network_lobby_touch(TouchPhase::Cancelled, drag_end, false)
        .test_value();
    main_assert!(
        lobby_chat_selection(&app_lobby(&app).chat_edit,).is_some(),
        "touch cancel releases capture but retains the last selection",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView::default());
    process_lobby_chat_request(&mut app, LobbyChatRequest::InsertText("W".repeat(200)));
    main_assert!(
        app_lobby(&app).chat_edit.horizontal_scroll > 0,
        "user text insertion keeps the C++ caret visible",
    );
    process_lobby_chat_request(
        &mut app,
        LobbyChatRequest::EditKey {
            key: LobbyChatEditKey::Home,
            modifiers: LobbyChatKeyModifiers::default(),
        },
    );
    main_assert_eq!(
        app_lobby(&app).chat_edit.horizontal_scroll =>
        (font.measure("\u{a6}", false).0 / 2).saturating_sub(2),
        "ordinary cursor operations run C4GUI::Edit::ScrollCursorInView",
    );

    app.install_active_lobby_chat_view(LobbyChatEditView::default());
    app.paste_classic_lobby_chat_text(&"W".repeat(200))
        .test_value();
    main_assert!(
        app_lobby(&app).chat_edit.horizontal_scroll > 0,
        "clipboard insertion keeps the C++ caret visible",
    );
    let paste_scroll = app_lobby(&app).chat_edit.horizontal_scroll;
    app.paste_classic_lobby_chat_text("").test_value();
    main_assert_eq!(app_lobby(&app).chat_edit.horizontal_scroll => paste_scroll, "C4GUI::Edit::InsertText returns before scrolling an empty paste",);

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "W".repeat(254),
        caret: 254,
        selection: None,
        horizontal_scroll: 123,
        cursor_visible: true,
    });
    process_lobby_chat_request(&mut app, LobbyChatRequest::InsertText("X".to_string()));
    main_assert_eq!(app_lobby(&app).chat_edit.horizontal_scroll => 123, "a zero-byte C++ insertion preserves retained horizontal scroll",);

    app.message_input_history.clear();
    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "seed".to_string(),
        caret: 4,
        selection: None,
        horizontal_scroll: 37,
        cursor_visible: true,
    });
    process_lobby_chat_request(&mut app, LobbyChatRequest::History { older: true });
    let history_view = &app_lobby(&app).chat_edit;
    main_assert!(history_view.text.is_empty());
    main_assert_eq!(history_view.horizontal_scroll => 37, "C++ history miss clears through DeleteSelection without scrolling",);
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerDown(drag_start));
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerUp(drag_end));
    let unchanged_pointer_view = &app_lobby(&app).chat_edit;
    main_assert_eq!(unchanged_pointer_view.caret => 0);
    main_assert_eq!(unchanged_pointer_view.horizontal_scroll => 37, "same-caret down/up preserve C++ iXScroll",);

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "alpha".to_string(),
        caret: 0,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    });
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerDown(drag_start));
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerUp(drag_end));
    main_assert_eq!(lobby_chat_selection(&app_lobby(&app).chat_edit,) => Some(0..5), "C4GUI::Screen::StopDragging applies the final release coordinate",);

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "alpha".to_string(),
        caret: 0,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    });
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerDown(drag_start));
    process_lobby_chat_request(&mut app, LobbyChatRequest::InsertText("Z".to_string()));
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerMove(drag_end));
    main_assert_eq!(lobby_chat_selection(&app_lobby(&app).chat_edit,) => Some(0..6),);
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerUp(drag_end));

    app.install_active_lobby_chat_view(LobbyChatEditView {
        text: "alpha".to_string(),
        caret: 5,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    });
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerDown(drag_end));
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerMove(drag_start));
    process_lobby_chat_request(&mut app, LobbyChatRequest::InsertText("Z".to_string()));
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerMove(drag_end));
    main_assert_eq!(lobby_chat_selection(&app_lobby(&app).chat_edit,) => Some(0..1),);
    process_lobby_chat_request(&mut app, LobbyChatRequest::PointerUp(drag_end));

    app.install_active_lobby_chat_view(LobbyChatEditView::default());
    app.paste_classic_lobby_chat_text(&("W".repeat(200) + "\n"))
        .test_value();
    let completed_line_view = &app_lobby(&app).chat_edit;
    main_assert!(completed_line_view.text.is_empty());
    main_assert!(
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
    process_lobby_chat_request(&mut app, LobbyChatRequest::Submit("submitted".to_string()));
    let submitted_view = &app_lobby(&app).chat_edit;
    main_assert!(submitted_view.text.is_empty());
    main_assert_eq!(submitted_view.horizontal_scroll => 61);
    main_assert!(
        submitted_view.cursor_visible,
        "DeleteSelection refreshes the focused caret after nonempty submission",
    );
}

#[test]
fn completed_scenario_description_uses_exact_desc_or_title() {
    let app = new_state_only_menu_app(640, 480);
    let directory = tempdir();
    let scenario = directory.path().join("Remote.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(
        scenario.join("DescUS.rtf"),
        br"{\rtf1 Gold Mine\par Mine some gold.\par}",
    )
    .test_value();

    main_assert_eq!(
        app.completed_lobby_scenario_description(&scenario, "Remote title".to_string()) =>
        LobbyScenarioText::Description("Gold Mine\nMine some gold.\n".to_string())
    );

    fs::remove_file(scenario.join("DescUS.rtf")).test_value();
    fs::write(scenario.join("Scenario.txt"), b"Unrelated fallback").test_value();
    main_assert_eq!(app.completed_lobby_scenario_description(&scenario, "Remote title".to_string()) => LobbyScenarioText::Title("Remote title".to_string()));
    main_assert_eq!(
        app.completed_lobby_scenario_description(
            &directory.path().join("Missing.c4s"),
            "Remote title".to_string(),
        ) =>
        LobbyScenarioText::Message("scenario file load error".to_string())
    );
}

#[test]
fn lobby_scenario_description_ignores_bytes_after_native_nul() {
    let app = new_state_only_menu_app(640, 480);
    let directory = tempdir();
    let scenario = directory.path().join("Remote.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(
        scenario.join("DescUS.rtf"),
        b"{\\rtf1 Visible lobby description.\\par}\0}",
    )
    .test_value();

    main_assert_eq!(
        app.completed_lobby_scenario_description(&scenario, "Remote title".to_string()) =>
        LobbyScenarioText::Description("Visible lobby description.\n".to_string())
    );

    fs::write(scenario.join("DescUS.rtf"), b"\0ignored suffix").test_value();
    fs::write(
        scenario.join("DescDE.rtf"),
        br"{\rtf1 Later language must not win.\par}",
    )
    .test_value();
    main_assert_eq!(
        load_lobby_scenario_description(
            &scenario,
            &["US".to_string(), "DE".to_string()],
            &LanguagePacks::default(),
        )
        .expect("load native-NUL-first lobby description") =>
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
    let (mut app, events) = joined_client_app_with_events(new_menu_app(640, 480));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    let mut status = host_config.initial_status;
    status.control_mode = 1;
    snapshot.parameters.control_rate = 3;
    snapshot.parameters.teams.active = 1;
    snapshot.parameters.teams.team_colors = 1;
    snapshot.parameters.teams.team_distribution = 3;
    snapshot.parameters.teams.random_team_count = 4;
    events
        .send(NetworkEvent::JoinData(lobby_fixture!(join_data:
            7,
            0,
            status,
            snapshot.dynamic,
            snapshot.parameters,
        )))
        .test_value();
    app.test_network_events();

    process_joined_lobby_action(&mut app, LobbyAction::SelectSheet(LobbySheet::Options));
    main_assert_eq!(app_lobby(&app).active_sheet => LobbySheet::Options);

    let rows = app_lobby(&app).controller.option_rows().to_vec();
    main_assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>() =>
        [
            LobbyOptionKind::ControlMode,
            LobbyOptionKind::ControlRate,
            LobbyOptionKind::TeamDistribution,
            LobbyOptionKind::TeamColors,
        ],
        "no RuntimeJoin and no RandomTeamCount for a joined client"
    );
    main_assert!(
        rows.iter().all(|row| !row.editable),
        "every joined client row is a read-only ComboBox"
    );
    main_assert_eq!(rows[1].value => "3", "control rate follows the joined status");

    // C4GameOptionsList::Activate forces one Update, and its Sec1 timer
    // keeps the visible sheet current (src/C4GameOptions.cpp:302-308).
    {
        let lobby = app_lobby_mut(&mut app.network_lobby);
        lobby.controller.set_option_rows(Vec::new());
    }
    app.sec1_timer().test_value();
    main_assert_eq!(app_lobby(&app).controller.option_rows().len() => 4, "the one-second callback reprojects the visible sheet");

    // An inactive sheet does no periodic work.
    process_joined_lobby_action(&mut app, LobbyAction::SelectSheet(LobbySheet::Players));
    {
        let lobby = app_lobby_mut(&mut app.network_lobby);
        lobby.controller.set_option_rows(Vec::new());
    }
    app.sec1_timer().test_value();
    main_assert!(
        app_lobby(&app).controller.option_rows().is_empty(),
        "a hidden Options sheet retains its last projection"
    );
}

// The Options tab is added for every participant, so a joined client can
// click the cog itself (src/C4GameLobby.cpp:223). Exercise the pointer route
// the report used rather than the action it lowers to.
#[test]
fn joined_lobby_options_tab_click_opens_the_read_only_sheet() {
    let (mut app, events) = joined_client_app_with_events(new_menu_app(640, 480));

    let host_config = clonk_network::HostConfig::default();
    let snapshot = host_config.initial_join_snapshot.test_value();
    events
        .send(NetworkEvent::JoinData(lobby_fixture!(join_data: 7, 0, host_config.initial_status, snapshot.dynamic, snapshot.parameters)))
        .test_value();
    app.test_network_events();

    let assets = Arc::clone(&app.assets);
    let options_tab = app_lobby_mut(&mut app.network_lobby)
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
        .test_value();
    let cog = GuiPoint::new(
        (options_tab.x + options_tab.w / 2) as f32,
        (options_tab.y + options_tab.h / 2) as f32,
    );

    click_network_lobby(&mut app, cog);
    main_assert_eq!(app_lobby(&app).active_sheet => LobbySheet::Options);
    main_assert!(
        !app_lobby(&app).controller.option_rows().is_empty(),
        "the activated sheet projects its read-only rows"
    );

    let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
    app_lobby_mut(&mut app.network_lobby)
        .render_classic(
            &mut surface,
            assets.as_ref(),
            &app.scenario_game_options,
            false,
            true,
            &startup_identity_gamma().clone(),
        )
        .test_value();

    // Every joined row is a read-only ComboBox, so clicking its value opens
    // no selection popup (src/C4GameOptions.cpp:80,126).
    let combo = app_lobby_mut(&mut app.network_lobby)
        .with_classic_controller_input(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
            |_, _, roster| roster.rows.first().and_then(|row| row.option_value),
        )
        .expect("layout the joined Options sheet")
        .test_value();
    let value = GuiPoint::new(
        (combo.x + combo.w / 2) as f32,
        (combo.y + combo.h / 2) as f32,
    );
    click_network_lobby(&mut app, value);
    main_assert!(
        app.context_menu.is_none(),
        "a read-only joined ComboBox opens no selection popup"
    );
}

#[test]
fn client_lobby_resource_sheet_tracks_hidden_transfer_progress() {
    let (mut app, events) = joined_client_app_with_events(new_menu_app(640, 480));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    let scenario = lobby_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 9,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Scenarios/Remote.c4s".to_vec()).test_value(),
    });
    snapshot.parameters.scenario = scenario.clone();
    events
        .send(NetworkEvent::JoinData(lobby_fixture!(join_data: 7, 0, host_config.initial_status, snapshot.dynamic, snapshot.parameters)))
        .test_value();
    app.test_network_events();
    main_assert_eq!(app_lobby(&app).resource_rows[&9].present_percent => 0);

    send_resource_progress(&events, 9, 37);
    app.test_network_events();
    main_assert_eq!(app_lobby(&app).active_sheet => LobbySheet::Players);
    main_assert_eq!(app_lobby(&app).resource_rows[&9].present_percent => 37);

    {
        let lobby = app_lobby_mut(&mut app.network_lobby);
        let layout = lobby.update_layout(640.0, 480.0).clone();
        let rect = layout
            .sheet_buttons
            .iter()
            .find(|(sheet, _)| *sheet == LobbySheet::Resources)
            .test_value()
            .1;
        lobby.handle_panel_pointer_move(GuiPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        ));
    }
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app_lobby(&app).active_sheet => LobbySheet::Resources);
    let (controller, _) = {
        let surface = app.graphics.surface();
        app_lobby_mut(&mut app.network_lobby).classic_render_state(
            surface,
            app.assets.as_ref(),
            &app.scenario_game_options,
        )
    }
    .test_value();
    main_assert_eq!(controller.active_sheet() => LobbySheet::Resources);
    main_assert_eq!(controller.resource_rows().iter().find(|row| row.id == 9).unwrap().present_percent => 37);

    process_joined_lobby_action(&mut app, LobbyAction::SelectSheet(LobbySheet::Players));
    send_resource_progress(&events, 9, 73);
    app.test_network_events();
    main_assert_eq!(app_lobby(&app).resource_rows[&9].present_percent => 73);
    process_joined_lobby_action(&mut app, LobbyAction::SelectSheet(LobbySheet::Resources));
    let (controller, _) = {
        let surface = app.graphics.surface();
        app_lobby_mut(&mut app.network_lobby).classic_render_state(
            surface,
            app.assets.as_ref(),
            &app.scenario_game_options,
        )
    }
    .test_value();
    main_assert_eq!(controller.resource_rows().iter().find(|row| row.id == 9).unwrap().present_percent => 73);

    {
        let lobby = app_lobby_mut(&mut app.network_lobby);
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
        let fonts = app.assets.clonk_fonts.as_deref().test_value();
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
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert!(app_lobby(&app).resource_scroll > 0);
}

#[test]
fn prepared_lobby_resource_rows_seed_complete_eligible_cores_in_id_order() {
    let core = |id, filename: &[u8]| {
        lobby_fixture!(resource {
            id,
            loadable: true,
            filename: LegacyCString::from_bytes(filename.to_vec()).test_value(),
        })
    };
    let mut snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    snapshot.parameters.scenario = core(9, b"Scenarios/Round.c4s");
    snapshot.parameters.game_resources = vec![core(3, b"System.c4g"), core(5, b"Material.c4g")];
    snapshot.dynamic = core(7, b"Round.c4s/Material.c4g");
    let player = |id, flags, resource| {
        lobby_fixture!(player {
            id,
            flags,
            resource: Some(resource),
        })
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
    main_assert_eq!(rows.keys().copied().collect::<Vec<_>>() => vec![3, 4, 5, 7, 9]);
    main_assert!(rows.values().all(|row| row.present_percent == 100));
    main_assert_eq!(rows[&9].filename => "Scenarios/Round.c4s");
    main_assert_eq!(rows[&4].filename => "Players/A.c4p");
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
                    main_assert_eq!(
                        lobby_resource_save_possible(
                            local,
                            complete,
                            resource_type,
                            allow_player_save,
                            &source,
                            &work,
                        ) =>
                        !local && complete && type_allowed,
                        "type={resource_type} local={local} complete={complete} allow_player_save={allow_player_save}"
                    );
                }
            }
        }
    }
    main_assert!(
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
    main_assert!(!lobby_resource_save_possible(
        false,
        true,
        clonk_network::HostResourceType::Scenario as u8,
        false,
        &root.join("network/Downloaded.c4s"),
        &work,
    ));

    let player_named_scenario = lobby_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        filename: LegacyCString::from_bytes(b"Remote/Upper.C4P".to_vec()).test_value(),
    });
    main_assert_eq!(
        lobby_resource_save_target(
            Path::new("install"),
            Path::new("Players"),
            &player_named_scenario,
        ) =>
        Some((
            Path::new("install/Players/Upper.C4P").to_path_buf(),
            "Upper.C4P".to_string(),
        )),
        "the advertised extension, not the resource type, selects PlayerPath"
    );

    let legacy_named_scenario = lobby_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        filename: LegacyCString::from_bytes(b"Remote/Gr\xfcnd.c4s".to_vec()).test_value(),
    });
    #[cfg(unix)]
    let legacy_target = Path::new("install").join(path_from_group_name_bytes(b"Gr\xfcnd.c4s"));
    #[cfg(not(unix))]
    let legacy_target = Path::new("install").join("Gründ.c4s");
    main_assert_eq!(
        lobby_resource_save_target(
            Path::new("install"),
            Path::new("Players"),
            &legacy_named_scenario,
        ) =>
        Some((legacy_target, "Gründ.c4s".to_string())),
        "the filesystem target preserves the legacy basename bytes"
    );
}

#[test]
fn installed_empty_joined_roster_does_not_revive_participant_fallback() {
    let mut lobby = client_lobby_state();
    main_assert!(
        !lobby.visible_roster_rows().is_empty(),
        "the pre-projection adapter exposes its participant fallback"
    );

    lobby.roster_rows_authoritative = true;
    main_assert!(
        lobby.visible_roster_rows().is_empty(),
        "an authoritative empty projection remains empty"
    );
}

#[test]
fn lobby_resource_save_dialogs_cover_overwrite_decline_accept_success_and_failure() {
    use clonk_frontend::message_dialog::{
        MessageDialogButtons, MessageDialogIcon, MessageDialogResult,
    };

    let root = tempdir();
    let work = root.path().join("Network");
    fs::create_dir(&work).test_value();
    let source = work.join("Downloaded.c4s");
    let target = root.path().join("Downloaded.c4s");
    fs::write(&source, b"first").test_value();

    let mut app = new_menu_app(640, 480);
    let mut settings = ClientSettings::new(SocketAddr::from(([127, 0, 0, 1], 11_112)), "Client");
    settings.resource_directory = work.clone();
    app.network_mode = Some(NetworkMode::Client(settings));
    app.network_lobby = Some(client_lobby_state());
    let core = lobby_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 47,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Remote/Downloaded.c4s".to_vec()).test_value(),
    });
    app.admission_resources.register_lobby_resource(&core);
    app.admission_resources
        .mark_complete_with_locality(core.id, source.clone(), false);
    app.register_classic_lobby_resource(&core, 100);
    main_assert!(app_lobby(&app).resource_rows[&core.id].save_possible);

    app.request_lobby_resource_save(core.id, false).test_value();
    main_assert_eq!(fs::read(&target).unwrap() => b"first");
    let success = app.message_dialogs.last().test_value();
    main_assert_eq!(success.state.caption() => "Resource saved");
    main_assert_eq!(success.state.icon() => MessageDialogIcon::Standard(13));
    main_assert!(success.state.message().ends_with("Downloaded.c4s"));
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    fs::write(&source, b"replacement").test_value();
    fs::write(&target, b"keep").test_value();
    app.request_lobby_resource_save(core.id, false).test_value();
    let confirmation = app.message_dialogs.last().test_value();
    main_assert_eq!(confirmation.state.caption() => "Save resource");
    main_assert_eq!(confirmation.state.buttons() => MessageDialogButtons::YES_NO);
    main_assert_eq!(confirmation.state.icon() => MessageDialogIcon::CONFIRM);
    main_assert!(matches!(
        &confirmation.continuation,
        MessageDialogContinuation::LobbyResourceOverwrite { resource_id: 47 }
    ));
    app.finish_message_dialog(MessageDialogResult::No)
        .test_value();
    main_assert_eq!(fs::read(&target).unwrap() => b"keep");
    main_assert!(app.message_dialogs.is_empty());

    app.request_lobby_resource_save(core.id, false).test_value();
    app.finish_message_dialog(MessageDialogResult::Yes)
        .test_value();
    main_assert_eq!(fs::read(&target).unwrap() => b"replacement");
    main_assert_eq!(app.message_dialogs.last().unwrap().state.caption() => "Resource saved");
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();

    fs::remove_file(&source).test_value();
    fs::remove_file(&target).test_value();
    app.request_lobby_resource_save(core.id, false).test_value();
    let failure = app.message_dialogs.last().test_value();
    main_assert_eq!(failure.state.caption() => "Error copying file");
    main_assert_eq!(failure.state.message() => "Error copying file");
    main_assert_eq!(failure.state.icon() => MessageDialogIcon::ERROR);
}

#[test]
fn classic_lobby_client_removal_evicts_its_resource_namespace() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let remote_resource = (7 << 16) | 1;
    let host_resource = 2;
    for resource_id in [host_resource, remote_resource] {
        app_classic_lobby_mut(&mut app.classic_host_lobby)
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
            lobby_fixture!(resource {
                id: resource_id,
                resource_type: clonk_network::HostResourceType::Player as u8,
            }),
        );
        app.admission_resources
            .present_percent
            .insert(resource_id, 100);
    }
    app.control_clients.replace_snapshot([
        lobby_fixture!(client {
            client_id: 0,
            activated: true,
        }),
        lobby_fixture!(client {
            client_id: 7,
            activated: true,
        }),
    ]);
    app.select_classic_lobby_sheet(LobbySheet::Resources);
    let (manager, events) = NetworkManager::test_stub();
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    events
        .send(NetworkEvent::DirectControl(NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: LegacyCString::default(),
                by_client: 0,
            },
        )))
        .test_value();
    app.test_network_events();

    let lobby = app_classic_lobby(&app);
    main_assert_eq!(lobby.resource_rows.keys().copied().collect::<Vec<_>>() => vec![host_resource]);
    main_assert_eq!(lobby.controller.resource_rows().iter().map(|row| row.id).collect::<Vec<_>>() => vec![host_resource]);
    main_assert!(!app
        .admission_resources
        .resources
        .contains_key(&remote_resource));
}

#[test]
fn joined_lobby_chrome_routes_exit_and_right_tab_context() {
    fn joined_app() -> GameApp {
        joined_client_app(new_menu_app(640, 480))
    }

    fn right_caption_context_point(app: &mut GameApp) -> GuiPoint {
        let surface = app.graphics.surface();
        let lobby = app_lobby_mut(&mut app.network_lobby);
        let (controller, _) = lobby
            .classic_render_state(surface, app.assets.as_ref(), &app.scenario_game_options)
            .test_value();
        let fonts = app.assets.clonk_fonts.as_deref().test_value();
        let layout = controller.layout(640, 480, fonts);
        GuiPoint::new(
            (layout.right_caption.x + 1) as f32,
            (layout.right_caption.y + layout.right_caption.h / 2) as f32,
        )
    }

    // MainDlg::OnRightTabContext adds Players, optional Teams, Resources
    // and Options for every participant (C4GameLobby.cpp:844-866).
    let joined_entries = GameApp::lobby_tab_context_entries(false, true);
    main_assert_eq!(
        joined_entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        vec![
            AppContextMenuCommand::LobbySheet(LobbySheet::Players),
            AppContextMenuCommand::LobbySheet(LobbySheet::Resources),
            AppContextMenuCommand::LobbySheet(LobbySheet::Options),
        ]
    );
    main_assert_eq!(
        joined_entries
            .iter()
            .map(|entry| entry.icon)
            .collect::<Vec<_>>() =>
        vec![
            ContextMenuIcon::Phase(9),
            ContextMenuIcon::Phase(10),
            ContextMenuIcon::Phase(14),
        ]
    );
    let joined_team_entries = GameApp::lobby_tab_context_entries(true, true);
    main_assert_eq!(
        joined_team_entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        vec![
            AppContextMenuCommand::LobbySheet(LobbySheet::Players),
            AppContextMenuCommand::LobbySheet(LobbySheet::Teams),
            AppContextMenuCommand::LobbySheet(LobbySheet::Resources),
            AppContextMenuCommand::LobbySheet(LobbySheet::Options),
        ]
    );
    main_assert_eq!(
        joined_team_entries
            .iter()
            .map(|entry| entry.icon)
            .collect::<Vec<_>>() =>
        vec![
            ContextMenuIcon::Phase(9),
            ContextMenuIcon::Phase(19),
            ContextMenuIcon::Phase(10),
            ContextMenuIcon::Phase(14),
        ]
    );

    let mut app = joined_app();

    let caption = right_caption_context_point(&mut app);
    app_lobby_mut(&mut app.network_lobby).handle_panel_pointer_move(caption);
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .test_value();
    main_assert_eq!(some(&app.context_menu).layout().panels[0].rows.len() => 3, "without teams: Players, Resources and Options");
    app.close_context_menu_silently();

    app_lobby_mut(&mut app.network_lobby).has_teams = true;
    let caption = right_caption_context_point(&mut app);
    app_lobby_mut(&mut app.network_lobby).handle_panel_pointer_move(caption);
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .test_value();
    let resource_point = {
        let layout = some(&app.context_menu).layout();
        main_assert_eq!(layout.panels[0].rows.len() => 4, "Players, Teams, Resources and Options match the native popup");
        let row = &layout.panels[0].rows[2];
        GuiPoint::new(
            (row.rect.x + row.rect.w / 2) as f32,
            (row.rect.y + row.rect.h / 2) as f32,
        )
    };
    main_assert!(app
        .handle_context_menu_pointer_move(resource_point)
        .expect("hover Resources"));
    main_assert!(app
        .handle_context_menu_pointer_button(ElementState::Pressed, ContextMenuPointerButton::Left,)
        .expect("dispatch Resources"));
    main_assert_eq!(app_lobby(&app).active_sheet => LobbySheet::Resources);
    main_assert!(app.context_menu.is_none());

    let exit = {
        let rect = app
            .network_lobby
            .as_ref()
            .and_then(|lobby| lobby.layout.as_ref())
            .test_value()
            .exit_button;
        GuiPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        )
    };
    app_lobby_mut(&mut app.network_lobby).handle_panel_pointer_move(exit);
    app.sound.ui_log.clear();
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string()]);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "Click".to_string()]);
    main_assert_eq!(app.startup.view => StartupView::MainMenu);
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());

    let mut escape = joined_app();
    escape.sound.ui_log.clear();
    escape.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(escape.startup.view => StartupView::MainMenu);
    main_assert!(escape.network_lobby.is_none());
    main_assert!(escape.network.is_none());
    main_assert!(escape.network_mode.is_none());
    main_assert!(escape.sound.ui_log.is_empty(), "Escape is silent");

    let mut hotkey = joined_app();
    hotkey.keyboard_modifiers = ModifiersState::ALT;
    hotkey.sound.ui_log.clear();
    hotkey.test_key(VirtualKeyCode::KeyX, ElementState::Pressed);
    main_assert_eq!(hotkey.startup.view => StartupView::MainMenu);
    main_assert!(hotkey.network_lobby.is_none());
    main_assert!(hotkey.network.is_none());
    main_assert!(hotkey.network_mode.is_none());
    main_assert!(hotkey.sound.ui_log.is_empty(), "the Exit hotkey is silent");
}

#[test]
fn joined_client_roster_context_reaches_mute_and_info_without_host_actions() {
    let (mut app, mut commands) = joined_client_app_with_commands(new_menu_app(640, 480));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Client")]);

    let entries = app.classic_lobby_client_context_entries(0).test_value();
    main_assert_eq!(
        entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        vec![
            AppContextMenuCommand::LobbyClientToggleMute(0),
            AppContextMenuCommand::LobbyClientInfo(0),
        ]
    );
    main_assert!(entries
        .iter()
        .all(|entry| entry.icon == ContextMenuIcon::None));
    let mut labels = entries
        .iter()
        .map(|entry| entry.text.clone())
        .collect::<Vec<_>>();
    for label in &mut labels {
        Markup::strip_markup(label);
    }
    main_assert_eq!(labels => vec!["Mute", "Info"]);

    let point = {
        let surface = app.graphics.surface();
        let lobby = app_lobby_mut(&mut app.network_lobby);
        let (mut controller, _) = lobby
            .classic_render_state(surface, app.assets.as_ref(), &app.scenario_game_options)
            .test_value();
        let fonts = app.assets.clonk_fonts.as_deref().test_value();
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
            .test_value();
        GuiPoint::new(
            (row.rect.x + row.rect.w / 2) as f32,
            (row.rect.y + row.rect.h / 2) as f32,
        )
    };
    app_lobby_mut(&mut app.network_lobby).handle_panel_pointer_move(point);
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .test_value();
    main_assert_eq!(some(&app.context_menu).layout().panels[0].rows.len() => 2);
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::KeyM, ElementState::Pressed)
        .expect("select Mute"));
    main_assert!(app.control_messages.is_muted(0));
    main_assert!(commands.take_submitted_client_updates().is_empty());
    main_assert!(commands.take_submitted_client_removes().is_empty());
    main_assert!(commands.take_submitted_votes().is_empty());

    let entries = app.classic_lobby_client_context_entries(0).test_value();
    let mut label = entries[0].text.clone();
    Markup::strip_markup(&mut label);
    main_assert_eq!(label => "Unmute");
}

#[test]
fn joined_lobby_roster_routes_and_retains_classic_interactions() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let (mut app, mut commands) = joined_client_app_with_commands(new_menu_app(640, 480));
    app.app_paths = Some(paths);
    app.network_max_players = 64;

    let chooser = lobby_fixture!(player {
        id: 31,
        name: LegacyCString::from_bytes(b"Chooser".to_vec()).test_value(),
        team: 1,
        color: 0x0012_3456,
        original_color: 0x0065_4321,
        league_rank_symbol: 5,
    });
    let companion = lobby_fixture!(player {
        id: 32,
        name: LegacyCString::from_bytes(b"Companion".to_vec()).test_value(),
        team: 3,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED,
    });
    let script = lobby_fixture!(player {
        id: 33,
        name: LegacyCString::from_bytes(b"Script".to_vec()).test_value(),
        team: 1,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
    });
    let foreign = lobby_fixture!(player {
        id: 41,
        name: LegacyCString::from_bytes(b"Foreign".to_vec()).test_value(),
        team: 2,
    });
    let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
    app.control_player_infos.replace_snapshot(
        50,
        [
            lobby_fixture!(player_data: 0, flags: 0, vec![foreign.clone()], by: 0),
            lobby_fixture!(player_data: 7, flags: packet_flags, vec![chooser.clone(), companion.clone(), script.clone()], by: 7),
        ],
    );

    let mut clients = vec![message_client(0, b"Host"), message_client(7, b"Client")];
    for id in 8..30 {
        clients.push(message_client(id, format!("Filler {id}").as_bytes()));
    }
    app.control_clients.replace_snapshot(clients.clone());

    let free_restore = lobby_fixture!(player {
        id: 50,
        name: LegacyCString::from_bytes(b"Free restore".to_vec()).test_value(),
        player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
        color: 0x0012_3456,
        original_color: 0x0012_3456,
    });
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.max_players = 64;
    snapshot.parameters.league_address =
        LegacyCString::from_bytes(b"https://league.example/".to_vec()).test_value();
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
    app.pending_network_join_data = Some(
        lobby_fixture!(join_data: 7, 0, host_config.initial_status, snapshot.dynamic, snapshot.parameters),
    );
    app.network_is_league = true;
    app.sync_classic_lobby_roster();
    app.joined_lobby_layouts().test_value();

    let lobby = app_lobby(&app);
    main_assert_eq!(lobby.controller.players_title() => "&Players (4/64)", "free restore rows do not inflate the authoritative player count");
    main_assert!(lobby.controller.league_mode());
    let chooser_index = lobby
        .controller
        .rows()
        .iter()
        .position(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 31))
        .test_value();
    main_assert!(
        matches!(&lobby.controller.rows()[chooser_index], LobbyRosterRow::Player(player) if player.league_rank == Some(5))
    );
    main_assert!(lobby.controller.rows().iter().any(|row| matches!(
        row,
        LobbyRosterRow::Header(LobbyHeaderRow {
            kind: LobbyRosterHeader::UnassignedSavegamePlayers,
            ..
        })
    )));
    main_assert!(lobby.controller.rows().iter().any(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 50 && player.client_id == -1)));

    let free_point = joined_lobby_row_point(&mut app, LobbyRosterId::Player(50));
    click_network_lobby(&mut app, free_point);
    let lobby = app_lobby(&app);
    main_assert_eq!(lobby.controller.selected_roster_id() => Some(&LobbyRosterId::Player(50)));
    main_assert_eq!(lobby.controller.focus() => LobbyControl::Roster);

    for modifiers in [ModifiersState::empty(), ModifiersState::SHIFT] {
        app_lobby_mut(&mut app.network_lobby).chat_edit = LobbyChatEditView::default();
        app.joined_lobby_layouts().test_value();
        app.keyboard_modifiers = modifiers;
        // Dialog::CharIn refocuses the default edit for unprocessed
        // characters EXCEPT space, which buttons consume on key-up
        // (src/C4GuiDialogs.cpp:552-567); the focused listbox binds no
        // confirm keys, so Space stays inert on the roster.
        app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
        app.test_text_input(' ');
        app.test_key(VirtualKeyCode::Space, ElementState::Released);
        let lobby = app_lobby(&app);
        main_assert_eq!(lobby.controller.focus() => LobbyControl::Roster);
        main_assert_eq!(lobby.chat_edit.text => "");
        main_assert_eq!(lobby.controller.selected_roster_id() => Some(&LobbyRosterId::Player(50)));
    }
    app.keyboard_modifiers = ModifiersState::empty();

    app_lobby_mut(&mut app.network_lobby).chat_edit = LobbyChatEditView {
        text: "draft".into(),
        caret: 5,
        cursor_visible: true,
        ..LobbyChatEditView::default()
    };
    app.joined_lobby_layouts().test_value();
    app.keyboard_modifiers = ModifiersState::CONTROL;
    tap_test_key(&mut app, VirtualKeyCode::KeyC);
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::Roster);
    app.keyboard_modifiers = ModifiersState::empty();
    tap_test_key(&mut app, VirtualKeyCode::Enter);
    main_assert!(commands.take_submitted_messages().is_empty());
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::Roster);

    tap_test_key(&mut app, VirtualKeyCode::ArrowLeft);
    let lobby = app_lobby(&app);
    main_assert_eq!(lobby.chat_edit.text => "draft");
    main_assert_eq!(lobby.chat_edit.caret => 5);
    main_assert_eq!(lobby.controller.focus() => LobbyControl::Roster);

    app_lobby_mut(&mut app.network_lobby).chat_edit = LobbyChatEditView::default();
    app.joined_lobby_layouts().test_value();
    app.test_text_input('x');
    let lobby = app_lobby(&app);
    main_assert_eq!(lobby.controller.focus() => LobbyControl::ChatInput);
    main_assert_eq!(lobby.chat_edit.text => "x");

    main_assert!(
        app.joined_lobby_layouts()
            .expect("scrollable joined roster")
            .1
            .max_scroll
            > 0
    );

    let selected_point = joined_lobby_row_point(&mut app, LobbyRosterId::Player(50));
    click_network_lobby(&mut app, selected_point);
    let wheel_hover = joined_lobby_row_point(
        &mut app,
        LobbyRosterId::Header(LobbyRosterHeader::UnassignedSavegamePlayers),
    );
    app.handle_network_lobby_pointer_move(wheel_hover)
        .test_value();
    main_assert!(app_lobby(&app)
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert!(
        app_lobby(&app)
            .controller
            .tooltip_state_at(Instant::now() + Duration::from_secs(1))
            .is_none(),
        "a joined ScrollWindow wheel releases tooltip hover ownership"
    );
    let retained_scroll = app_lobby(&app).controller.roster_scroll();
    main_assert!(retained_scroll > 0);
    app.joined_lobby_layouts().test_value();
    app.joined_lobby_layouts().test_value();
    clients.push(message_client(40, b"Late filler"));
    app.control_clients.replace_snapshot(clients);
    app.sync_classic_lobby_roster();
    app.joined_lobby_layouts().test_value();
    let lobby = app_lobby(&app);
    main_assert_eq!(lobby.controller.selected_roster_id() => Some(&LobbyRosterId::Player(50)), "row refresh retains semantic selection");
    main_assert_eq!(lobby.controller.focus() => LobbyControl::Roster);
    main_assert_eq!(lobby.controller.roster_scroll() => retained_scroll);

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 100.0), 1.0);
    main_assert_eq!(app_lobby(&app).controller.roster_scroll() => 0);

    let last_roster_id = app_lobby(&app).controller.rows().last().test_value().id();
    tap_test_key(&mut app, VirtualKeyCode::End);
    main_assert_eq!(app_lobby(&app).controller.selected_roster_id() => Some(&last_roster_id));
    main_assert!(app_lobby(&app).controller.roster_scroll() > 0);
    tap_test_key(&mut app, VirtualKeyCode::Home);
    main_assert_eq!(app_lobby(&app).controller.roster_scroll() => 0);
    tap_test_key(&mut app, VirtualKeyCode::PageDown);
    let first_page_selection = app_lobby(&app).controller.selected_roster_id().cloned();
    tap_test_key(&mut app, VirtualKeyCode::PageDown);
    let lobby = app_lobby(&app);
    main_assert_ne!(lobby.controller.selected_roster_id().cloned() => first_page_selection);
    main_assert!(lobby.controller.roster_scroll() > 0);
    tap_test_key(&mut app, VirtualKeyCode::Home);

    app_lobby_mut(&mut app.network_lobby).chat_edit = LobbyChatEditView {
        text: "focus me".to_string(),
        caret: 0,
        cursor_visible: false,
        ..LobbyChatEditView::default()
    };
    app.joined_lobby_layouts().test_value();
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    let lobby = app_lobby(&app);
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(lobby.controller.focus() => LobbyControl::Roster);
    main_assert_eq!(lobby.chat_edit.text => "focus me");
    main_assert_eq!(lobby.chat_edit.caret => 0);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Released);
    click_network_lobby(&mut app, free_point);

    app.keyboard_modifiers = ModifiersState::SHIFT;
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::ScenarioTab);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    app.keyboard_modifiers = ModifiersState::empty();

    let client_point = joined_lobby_row_point(&mut app, LobbyRosterId::Client(7));
    click_network_lobby(&mut app, client_point);
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("focus joined Add Player control"));
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::RosterAddPlayer);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::Exit);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    app.keyboard_modifiers = ModifiersState::SHIFT;
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::RosterAddPlayer);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    app.keyboard_modifiers = ModifiersState::empty();
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("latch joined Add Player key"));
    main_assert!(app.definition_selector.is_none());
    app.handle_focus_lost().test_value();
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Released)
        .expect("release canceled joined Add Player key"));
    main_assert!(app.definition_selector.is_none());
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("relatch joined Add Player key"));
    app.handle_gamepad_event(GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    })
    .test_value();
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Released)
        .expect("release controller-canceled joined Add Player key"));
    main_assert!(app.definition_selector.is_none());

    let chooser_point = joined_lobby_row_point(&mut app, LobbyRosterId::Player(31));
    click_network_lobby(&mut app, chooser_point);
    let (_, league_roster) = app.joined_lobby_layouts().test_value();
    let chooser_index = app_lobby(&app)
        .controller
        .rows()
        .iter()
        .position(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 31))
        .test_value();
    main_assert!(
        league_roster
            .rows
            .iter()
            .find(|row| row.index == chooser_index)
            .and_then(|row| row.rank)
            .is_some(),
        "expanded joined league rows reserve the native rank-symbol cell"
    );
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("focus local team control"));
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::RosterTeam);
    main_assert!(!app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Released)
        .expect("release local team focus key"));
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Space, ElementState::Pressed)
        .expect("open joined local team selector"));
    main_assert_eq!(some(&app.context_menu).layout().panels[0].rows.len() => 2);
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select current joined team"));
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select available joined team"));
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("activate joined team selection"));
    let mut team_selected = chooser.clone();
    team_selected.team = 3;
    main_assert_eq!(
        commands.take_player_info_updates() =>
        vec![
            lobby_fixture!(player_update: 7, packet_flags, vec![team_selected, companion.clone(), script.clone()])
        ],
        "joined team combo submits one full packet without optimistic mutation"
    );
    app.keyboard_modifiers = ModifiersState::SHIFT;
    main_assert!(app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Pressed)
        .expect("reverse focus to roster"));
    main_assert_eq!(app_lobby(&app).controller.focus() => LobbyControl::Roster);
    main_assert!(!app
        .handle_joined_lobby_roster_key(VirtualKeyCode::Tab, ElementState::Released)
        .expect("release reverse roster focus key"));
    app.keyboard_modifiers = ModifiersState::empty();
    some_mut(&mut app.pending_network_join_data)
        .parameters
        .teams
        .team_distribution = 1;
    app.sync_classic_lobby_roster();
    app.submit_classic_lobby_team_selection(31, 3);
    app.move_local_classic_lobby_players_into_team(3);
    main_assert!(
        commands.take_player_info_updates().is_empty(),
        "a joined client cannot choose teams under host-only distribution"
    );
    some_mut(&mut app.pending_network_join_data)
        .parameters
        .teams
        .team_distribution = 0;
    app.sync_classic_lobby_roster();
    let chooser_point = joined_lobby_row_point(&mut app, LobbyRosterId::Player(31));
    app.handle_network_lobby_pointer_move(chooser_point)
        .test_value();
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .test_value();
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::KeyR, ElementState::Pressed)
        .expect("activate joined local Remove"));
    main_assert_eq!(
        commands.take_player_info_updates() =>
        vec![
            lobby_fixture!(player_update: 7, packet_flags, vec![script.clone(), companion.clone()])
        ],
        "joined Remove submits the remaining full owner packet"
    );
    let chooser_point = joined_lobby_row_point(&mut app, LobbyRosterId::Player(31));
    app.handle_network_lobby_pointer_move(chooser_point)
        .test_value();
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .test_value();
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::KeyC, ElementState::Pressed)
        .expect("activate New Color"));
    let mut recolored = chooser.clone();
    recolored.color = recolored.original_color;
    main_assert_eq!(commands.take_player_info_updates() => vec![lobby_fixture!(player_update: 7, packet_flags, vec![recolored, companion.clone(), script.clone()])]);
    main_assert!(
        app.classic_lobby_player_context_entries(41)
            .expect("foreign joined roster player")
            .1
            .is_empty(),
        "joined clients cannot mutate a foreign player's context"
    );

    let free_point = joined_lobby_row_point(&mut app, LobbyRosterId::Player(50));
    app.handle_network_lobby_pointer_move(free_point)
        .test_value();
    app.handle_network_lobby_secondary_button(ElementState::Pressed)
        .test_value();
    let root = some(&app.context_menu).layout().panels[0].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((root.x + 1) as f32, (root.y + 1) as f32))
        .test_value();
    main_assert_eq!(some(&app.context_menu).layout().panels[1].rows.len() => 1);
    let child = some(&app.context_menu).layout().panels[1].rows[0].rect;
    app.handle_context_menu_pointer_move(GuiPoint::new((child.x + 1) as f32, (child.y + 1) as f32))
        .test_value();
    main_assert!(app
        .handle_context_menu_pointer_button(ElementState::Pressed, ContextMenuPointerButton::Left,)
        .expect("activate takeover player"));
    let mut associated = chooser.clone();
    associated.savegame_player = 50;
    main_assert_eq!(
        commands.take_player_info_updates() =>
        vec![
            lobby_fixture!(player_update: 7, packet_flags, vec![associated, companion.clone(), script.clone()])
        ]
    );
    main_assert_eq!(app.control_player_infos.client_update_request(7).unwrap().players[0].savegame_player => 0, "takeover waits for the authoritative echo");
    main_assert!(app
        .handle_context_menu_pointer_button(ElementState::Released, ContextMenuPointerButton::Left,)
        .expect("consume takeover activation release"));

    let teams_tab = joined_lobby_tab_point(&mut app, LobbySheet::Teams);
    click_network_lobby(&mut app, teams_tab);
    main_assert_eq!(app_lobby(&app).active_sheet => LobbySheet::Teams);
    for click in 0..2 {
        let team_point =
            joined_lobby_row_point(&mut app, LobbyRosterId::Header(LobbyRosterHeader::Team(2)));
        let (team_layout, team_roster) = app.joined_lobby_layouts().test_value();
        app.handle_network_lobby_pointer_move(team_point)
            .test_value();
        app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
            .test_value();
        main_assert_eq!(app_lobby(&app).controller.selected_roster_id() => Some(&LobbyRosterId::Header(LobbyRosterHeader::Team(2))));
        main_assert_eq!(
            app_lobby(&app).controller.accepted_roster_click_id(
                team_point,
                &team_layout,
                &team_roster
            ) =>
            Some(LobbyRosterId::Header(LobbyRosterHeader::Team(2)))
        );
        app.handle_network_lobby_pointer_button(ElementState::Released, false)
            .test_value();
        main_assert_eq!(app_lobby(&app).last_roster_click.as_ref().map(|(id, _)| id) => (click == 0).then_some(&LobbyRosterId::Header(LobbyRosterHeader::Team(2))));
    }
    let mut moved_chooser = chooser.clone();
    moved_chooser.team = 2;
    let mut moved_companion = companion.clone();
    moved_companion.team = 2;
    main_assert_eq!(
        commands.take_player_info_updates() =>
        vec![
            lobby_fixture!(player_update: 7, packet_flags, vec![moved_chooser, moved_companion, script.clone()])
        ],
        "joined team double-click mutates every local User exactly once"
    );

    let players_tab = joined_lobby_tab_point(&mut app, LobbySheet::Players);
    click_network_lobby(&mut app, players_tab);
    let chooser_point = joined_lobby_row_point(&mut app, LobbyRosterId::Player(31));
    click_network_lobby(&mut app, chooser_point);
    let chooser_index_before = app_lobby(&app)
        .controller
        .rows()
        .iter()
        .position(|row| row.id() == LobbyRosterId::Player(31))
        .test_value();
    app.control_player_infos.replace_snapshot(
        50,
        [
            lobby_fixture!(player_data: 0, flags: 0, vec![foreign.clone()], by: 0),
            lobby_fixture!(player_data: 7, flags: packet_flags, vec![companion.clone(), chooser.clone(), script.clone()], by: 7),
        ],
    );
    app.sync_classic_lobby_roster();
    let lobby = app_lobby(&app);
    let chooser_index_after = lobby
        .controller
        .rows()
        .iter()
        .position(|row| row.id() == LobbyRosterId::Player(31))
        .test_value();
    main_assert_ne!(chooser_index_after => chooser_index_before);
    main_assert_eq!(lobby.controller.selected_roster_id() => Some(&LobbyRosterId::Player(31)), "authoritative row reordering retains semantic joined selection");
    main_assert_eq!(lobby.controller.focus() => LobbyControl::Roster);
    app.control_player_infos.replace_snapshot(
        50,
        [
            lobby_fixture!(player_data: 0, flags: 0, vec![foreign], by: 0),
            lobby_fixture!(player_data: 7, flags: packet_flags, vec![chooser, companion, script], by: 7),
        ],
    );
    app.sync_classic_lobby_roster();
    let (_, roster) = app.joined_lobby_layouts().test_value();
    let lobby = app_lobby(&app);
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
        .test_value();
    let add_point = GuiPoint::new(
        (add_player.x + add_player.w / 2) as f32,
        (add_player.y + add_player.h / 2) as f32,
    );
    click_network_lobby(&mut app, add_point);
    main_assert!(app.definition_selector.is_some());
    main_assert_eq!(some(&app.pending_lobby_player_selection).client_id => 7);
}

#[test]
fn joined_roster_double_click_is_roster_scoped() {
    let (mut app, mut commands) = joined_client_app_with_commands(new_menu_app(640, 480));
    app.network_max_players = 8;

    let chooser = set_control_test_player(31, 1, 0);
    let companion = set_control_test_player(32, 3, clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED);
    let packet_flags = clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL;
    let clients = vec![message_client(0, b"Host"), message_client(7, b"Client")];
    app.control_player_infos.replace_snapshot(
        40,
        [lobby_fixture!(player_data: 7, flags: packet_flags, vec![chooser.clone(), companion.clone()], by: 7)],
    );
    app.control_clients.replace_snapshot(clients.clone());

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
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
    app.pending_network_join_data = Some(
        lobby_fixture!(join_data: 7, 0, host_config.initial_status, snapshot.dynamic, snapshot.parameters),
    );
    app.sync_classic_lobby_roster();

    let teams_tab = joined_lobby_tab_point(&mut app, LobbySheet::Teams);
    click_network_lobby(&mut app, teams_tab);
    main_assert_eq!(app_lobby(&app).active_sheet => LobbySheet::Teams);

    // Two completed clicks on DIFFERENT team headers inside the 400 ms
    // window stay single clicks: the synthesized LeftDouble is scoped to
    // the retained semantic row, exactly like the persistent host path.
    let other_point =
        joined_lobby_row_point(&mut app, LobbyRosterId::Header(LobbyRosterHeader::Team(3)));
    click_network_lobby(&mut app, other_point);
    let target_point =
        joined_lobby_row_point(&mut app, LobbyRosterId::Header(LobbyRosterHeader::Team(2)));
    click_network_lobby(&mut app, target_point);
    main_assert!(
        commands.take_player_info_updates().is_empty(),
        "fast clicks across two roster rows never classify as a double click"
    );

    // A second completed click on the same header fires one bulk move.
    app.handle_network_lobby_pointer_button(ElementState::Pressed, false)
        .test_value();
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .test_value();
    let mut moved_chooser = chooser.clone();
    moved_chooser.team = 2;
    let mut moved_companion = companion.clone();
    moved_companion.team = 2;
    main_assert_eq!(
        commands.take_player_info_updates() =>
        vec![
            lobby_fixture!(player_update: 7, packet_flags, vec![moved_chooser, moved_companion])
        ],
        "the roster-scoped double click clones one full local packet"
    );

    // A press-classified LeftDouble (the SDL/X11 global press clock;
    // C4FullScreen.cpp:327-350) reaches the hovered team header directly,
    // and its release never double-fires.
    app.handle_network_lobby_pointer_button(ElementState::Pressed, true)
        .test_value();
    main_assert_eq!(commands.take_player_info_updates().len() => 1, "LeftDouble runs the hovered header action exactly once");
    app.handle_network_lobby_pointer_button(ElementState::Released, false)
        .test_value();
    main_assert!(
        commands.take_player_info_updates().is_empty(),
        "the release after a LeftDouble neither activates nor re-fires"
    );
}

#[test]
fn unstaged_host_retained_roster_routes_script_player_add() {
    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(host_lobby_state());
    let (_events, mut commands) = install_client_network_commands(&mut app, 0);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    app.control_player_infos.replace_snapshot(
        0,
        [lobby_fixture!(player_data: 0, flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL, Vec::new(), by: 0)],
    );
    let mut metadata = set_control_test_metadata(false, Vec::new());
    metadata.max_script_players = 1;
    metadata.script_player_names = LegacyCString::from_bytes(b"Bot".to_vec()).test_value();
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.sync_classic_lobby_roster();

    let (_, roster) = app.joined_lobby_layouts().test_value();
    let lobby = app_lobby(&app);
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
        .test_value();
    let point = GuiPoint::new((add.x + add.w / 2) as f32, (add.y + add.h / 2) as f32);
    click_network_lobby(&mut app, point);

    let requests = commands.take_player_info_updates();
    let [request] = requests.as_slice() else {
        panic!("expected one retained-host script request, got {requests:?}");
    };
    main_assert_eq!(request.client_id => 0);
    main_assert_eq!(request.flags => clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS);
    let [player] = request.players.as_slice() else {
        panic!("expected one retained-host script player");
    };
    main_assert_eq!(player.name.as_bytes() => b"Bot");
    main_assert_eq!(player.player_type => clonk_engine::PLAYER_INFO_TYPE_SCRIPT);
    main_assert_eq!(player.original_color => player.color);
}

fn joined_option_strip_app() -> GameApp {
    let mut app = joined_client_app(new_menu_app(640, 480));
    app.sync_network_lobby_game_option_state();
    app
}

fn joined_option_center(app: &GameApp, button: GameOptionButton) -> PhysicalPosition<f64> {
    let rect = app.scenario_game_options.layout().rect(button).test_value();
    PhysicalPosition::new(
        f64::from(rect.x + rect.w / 2),
        f64::from(rect.y + rect.h / 2),
    )
}

fn joined_option_point(app: &GameApp, button: GameOptionButton) -> GuiPoint {
    let rect = app.scenario_game_options.layout().rect(button).test_value();
    GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
}

fn joined_option_controller_focus(app: &mut GameApp) -> LobbyControl {
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.sync_classic_controller();
    lobby.controller.focus()
}

fn tap_test_key(app: &mut GameApp, key: VirtualKeyCode) {
    app.test_key(key, ElementState::Pressed);
    app.test_key(key, ElementState::Released);
}

fn focus_joined_option(app: &mut GameApp, button: GameOptionButton) {
    for _ in 0..15 {
        if joined_option_controller_focus(app) == LobbyControl::GameOption(button) {
            return;
        }
        tap_test_key(app, VirtualKeyCode::Tab);
    }
    main_assert_eq!(joined_option_controller_focus(app) => LobbyControl::GameOption(button), "the dialog focus cycle reaches {button:?}");
}

#[test]
fn joined_lobby_game_option_strip_routes_input() {
    let mut app = joined_option_strip_app();

    // C4GameLobby.cpp:214 builds the client strip (fNetwork, !fHost,
    // fLobby): League and Fair Crew stay locked while Record is live.
    main_assert_eq!(app.scenario_game_options.context() => GameOptionContext::LobbyClient);
    main_assert!(
        !app.scenario_game_options
            .view(GameOptionButton::League)
            .expect("League is visible")
            .enabled
    );
    main_assert!(
        !app.scenario_game_options
            .view(GameOptionButton::FairCrew)
            .expect("Fair Crew is visible")
            .enabled
    );
    main_assert!(
        app.scenario_game_options
            .view(GameOptionButton::Record)
            .expect("Record is visible")
            .enabled
    );

    // Pointer: an enabled joined control presses with the native sounds
    // and toggles Config.General.Record on release. Buttons deliberately
    // keep the chat focus on mouse clicks.
    app.test_cursor(joined_option_center(&app, GameOptionButton::Record));
    main_assert_eq!(
        app.scenario_game_options.hovered_button() =>
        Some(GameOptionButton::Record),
        "the retained strip tracks controller-routed hover for its tooltip"
    );
    app.sound.ui_log.clear();
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string()]);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "Click".to_string()]);
    main_assert!(app.scenario_game_options.values().record);
    main_assert!(app.startup.view_flags.record);
    main_assert_eq!(app.scenario_game_options.focused_button() => None);
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::ChatInput);

    // Pointer: host-only/locked controls stay inert and visually exact.
    // Each press resets the native double-click clock so the classic
    // LeftDouble path (which never presses buttons) stays out of the way.
    let fair_crew_before = app.scenario_game_options.values().fair_crew;
    for locked in [GameOptionButton::League, GameOptionButton::FairCrew] {
        app.test_cursor(joined_option_center(&app, locked));
        app.sound.ui_log.clear();
        app.last_application_left_press = None;
        app.test_left_button(ElementState::Pressed);
        app.test_left_button(ElementState::Released);
        main_assert!(app.sound.ui_log.is_empty(), "{locked:?} is silent");
    }
    main_assert!(!app.scenario_game_options.values().lobby_is_league);
    main_assert_eq!(app.scenario_game_options.values().fair_crew => fair_crew_before);
    main_assert_eq!(
        app.scenario_game_options
            .view(GameOptionButton::League)
            .expect("League stays visible")
            .icon =>
        clonk_frontend::game_option_buttons::GameOptionIcon::LeagueOff
    );
    main_assert_eq!(
        app.scenario_game_options
            .view(GameOptionButton::FairCrew)
            .expect("Fair Crew stays visible")
            .icon =>
        if fair_crew_before {
            clonk_frontend::game_option_buttons::GameOptionIcon::FairCrewGray
        } else {
            clonk_frontend::game_option_buttons::GameOptionIcon::NormalCrewGray
        }
    );

    // A press that drags off the enabled button pops the visual with the
    // native ArrowHit and releases without an activation.
    app.test_cursor(joined_option_center(&app, GameOptionButton::Record));
    app.sound.ui_log.clear();
    app.last_application_left_press = None;
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(joined_option_center(&app, GameOptionButton::FairCrew));
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "ArrowHit".to_string()]);
    main_assert!(
        app.scenario_game_options.values().record,
        "an aborted click keeps the Record preference"
    );

    // Keyboard: Tab walks the C4GUI dialog focus order, in which every
    // strip button is a stop (Control::IsFocusElement holds while
    // disabled). Space activates only enabled buttons; an unhandled key
    // on a locked stop falls back to the chat default per
    // Dialog::KeyFocusDefault.
    let tab = |app: &mut GameApp| {
        tap_test_key(app, VirtualKeyCode::Tab);
    };
    focus_joined_option(&mut app, GameOptionButton::League);
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(GameOptionButton::League));
    app.sound.ui_log.clear();
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::ChatInput, "KeyFocusDefault returns an unhandled option key to the chat edit");
    main_assert_eq!(app.scenario_game_options.focused_button() => None);
    main_assert!(app.sound.ui_log.is_empty());
    app.test_key(VirtualKeyCode::Space, ElementState::Released);
    focus_joined_option(&mut app, GameOptionButton::League);
    tab(&mut app);
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::GameOption(GameOptionButton::FairCrew));
    tab(&mut app);
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::GameOption(GameOptionButton::Record));
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(GameOptionButton::Record));
    app.sound.ui_log.clear();
    tap_test_key(&mut app, VirtualKeyCode::Space);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "Click".to_string()]);
    main_assert!(!app.scenario_game_options.values().record);

    // A typed character on a focused option button refocuses the chat
    // edit and inserts (Dialog::KeyFocusDefault with CharIn).
    app.test_text_input('y');
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::ChatInput);
    main_assert_eq!(app.scenario_game_options.focused_button() => None);
    main_assert_eq!(app_lobby(&app).chat_edit.text => "y");

    // Alt hotkeys reach enabled controls silently and skip locked ones.
    app.keyboard_modifiers = ModifiersState::ALT;
    app.sound.ui_log.clear();
    tap_test_key(&mut app, VirtualKeyCode::KeyR);
    main_assert!(app.scenario_game_options.values().record);
    main_assert!(app.sound.ui_log.is_empty(), "dialog hotkeys are silent");
    tap_test_key(&mut app, VirtualKeyCode::KeyL);
    main_assert!(!app.scenario_game_options.values().lobby_is_league);
    app.keyboard_modifiers = ModifiersState::empty();

    // Touch mirrors the pointer path.
    let record_point = joined_option_point(&app, GameOptionButton::Record);
    app.sound.ui_log.clear();
    app.last_application_left_press = None;
    app.test_touch(TouchPhase::Started, record_point);
    app.test_touch(TouchPhase::Ended, record_point);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "Click".to_string()]);
    main_assert!(!app.scenario_game_options.values().record);
    let league_point = joined_option_point(&app, GameOptionButton::League);
    app.sound.ui_log.clear();
    app.last_application_left_press = None;
    app.test_touch(TouchPhase::Started, league_point);
    app.test_touch(TouchPhase::Ended, league_point);
    main_assert!(app.sound.ui_log.is_empty());

    // Gamepad: Left/Right traverse the strip stops, Select activates the
    // focused button, and Select on a locked stop falls back to the chat
    // default.
    focus_joined_option(&mut app, GameOptionButton::League);
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::GameOption(GameOptionButton::FairCrew));
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::GameOption(GameOptionButton::Record));
    app.sound.ui_log.clear();
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Pressed,
    )
    .test_value();
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Released,
    )
    .test_value();
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "Click".to_string()]);
    main_assert!(app.scenario_game_options.values().record);
    focus_joined_option(&mut app, GameOptionButton::League);
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(joined_option_controller_focus(&mut app) => LobbyControl::ChatInput, "Select on a locked option falls back to the chat default");

    // A league round grays Record exactly like the host rules; the
    // joined control then rejects pointer and hotkey input.
    app.network_is_league = true;
    app.sync_network_lobby_game_option_state();
    main_assert!(app.scenario_game_options.values().lobby_is_league);
    main_assert!(
        !app.scenario_game_options
            .view(GameOptionButton::Record)
            .expect("Record stays visible")
            .enabled
    );
    main_assert_eq!(
        app.scenario_game_options
            .view(GameOptionButton::Record)
            .expect("Record stays visible")
            .icon =>
        clonk_frontend::game_option_buttons::GameOptionIcon::RecordOn
    );
    let record_before = app.scenario_game_options.values().record;
    app.test_cursor(joined_option_center(&app, GameOptionButton::Record));
    app.sound.ui_log.clear();
    app.last_application_left_press = None;
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    app.keyboard_modifiers = ModifiersState::ALT;
    tap_test_key(&mut app, VirtualKeyCode::KeyR);
    app.keyboard_modifiers = ModifiersState::empty();
    main_assert_eq!(app.scenario_game_options.values().record => record_before);
    main_assert!(app.sound.ui_log.is_empty());
}

#[test]
fn network_lobby_game_option_state_matches_role_and_render_focus() {
    // The retained strip and joined controller keep the recursive focus
    // mirrored, so ClassicGameLobby::render's focus/context checks hold
    // on the projected render state.
    let mut app = joined_option_strip_app();
    let tab = |app: &mut GameApp| {
        tap_test_key(app, VirtualKeyCode::Tab);
    };
    let mut seen = Vec::new();
    loop {
        let focus = joined_option_controller_focus(&mut app);
        if focus == LobbyControl::GameOption(GameOptionButton::League) {
            break;
        }
        seen.push(focus);
        tab(&mut app);
        main_assert!(
            seen.len() < 16,
            "focus cycle never reaches the strip: {seen:?}"
        );
    }
    let surface = app.graphics.surface();
    let (controller, options) = app_lobby_mut(&mut app.network_lobby)
        .classic_render_state(surface, app.assets.as_ref(), &app.scenario_game_options)
        .test_value();
    main_assert_eq!(controller.focus() => LobbyControl::GameOption(GameOptionButton::League));
    main_assert_eq!(options.focused_button() => Some(GameOptionButton::League));
    main_assert_eq!(options.context() => GameOptionContext::LobbyClient);

    // A host-role generic lobby retains the LobbyHost context so the
    // render-time context checks and native enable rules hold.
    let mut host = new_menu_app(640, 480);
    host.startup.view = StartupView::NetworkLobby;
    host.network_lobby = Some(host_lobby_state());
    host.sync_network_lobby_game_option_state();
    main_assert_eq!(host.scenario_game_options.context() => GameOptionContext::LobbyHost);
    main_assert!(
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
    classic.network_lobby = Some(host_lobby_state());
    let before = classic.scenario_game_options.values().clone();
    classic.network_is_league = true;
    classic.sync_network_lobby_game_option_state();
    main_assert_eq!(*classic.scenario_game_options.values() => before);
}

// C4Network2ClientDlg is constructed from an id and resolves the client in
// UpdateText, so an id that no longer resolves opens on
// IDS_NET_CLIENT_INFO_UNKNOWNID instead of doing nothing
// (src/C4Network2Dialogs.cpp:42-59).
#[test]
fn client_info_dialog_shows_unknown_id_and_host_unacknowledged_marker() {
    let mut app = joined_client_app(new_real_menu_app(640, 480));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Client")]);

    main_assert!(
        app.open_classic_lobby_client_info(42)
            .expect("a stale client id still opens the native dialog"),
        "C++ never refuses to construct C4Network2ClientDlg"
    );
    let info = some(&app.runtime_client_list);
    main_assert_eq!(info.info_client_id() => Some(42));
    main_assert_eq!(info.info_lines() => ["Unknown client ID #42.".to_string()]);

    // A known id still resolves, and a joined client is never the network
    // host, so it cannot show the acknowledgement marker.
    main_assert!(app
        .open_classic_lobby_client_info(0)
        .expect("known client id opens"));
    let info = some(&app.runtime_client_list);
    main_assert_eq!(info.info_client_id() => Some(0));
    main_assert!(
        info.info_lines()
            .iter()
            .all(|line| !line.contains("(!ack)")),
        "only Game.Network.isHost() adds the marker (src/C4Network2Dialogs.cpp:71)"
    );

    // The host's own row has no C4Network2Client, so it never carries the
    // marker either (src/C4Network2Dialogs.cpp:62).
    let mut host = new_real_menu_app(640, 480);
    host.startup.view = StartupView::NetworkLobby;
    let _host_events = install_client_network_stub(&mut host, 0);
    host.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 0, "Host".to_string(), None),
    ));
    host.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    main_assert!(host
        .open_classic_lobby_client_info(0)
        .expect("host opens its own client information"));
    main_assert!(host
        .runtime_client_list
        .as_ref()
        .expect("host info dialog")
        .info_lines()
        .iter()
        .all(|line| !line.contains("(!ack)")));
}

#[test]
fn lobby_client_info_renders_modally_and_escape_release_cannot_exit_lobby() {
    let (mut app, mut commands) = joined_client_app_with_commands(new_real_menu_app(640, 480));
    app.sync_network_lobby_game_option_state();
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Client")]);

    let mut base = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut base);
    main_assert!(app
        .open_classic_lobby_client_info(0)
        .expect("open client information"));
    let info = some(&app.runtime_client_list);
    main_assert!(info.is_info_only());
    main_assert_eq!(info.info_client_id() => Some(0));

    let mut with_info = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut with_info);
    main_assert_ne!(with_info => base);

    app.test_text_input('x');
    main_assert!(app_lobby(&app).chat_edit.text.is_empty());
    app.handle_other_mouse_button(ElementState::Pressed)
        .test_value();
    app.handle_other_mouse_button(ElementState::Released)
        .test_value();
    main_assert!(app.runtime_client_list.is_some());
    main_assert!(app_lobby(&app).chat_edit.text.is_empty());
    app.running_pointer_position = Some(GuiPoint::new(0.0, 0.0));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(app.runtime_client_list.is_some());
    main_assert!(app.context_menu.is_none());

    tap_test_key(&mut app, VirtualKeyCode::Enter);
    main_assert!(app.runtime_client_list.is_some());
    main_assert!(commands.take_submitted_client_updates().is_empty());
    main_assert!(commands.take_submitted_client_removes().is_empty());

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.runtime_client_list.is_none());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(app.network_lobby.is_some());
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(app.network_lobby.is_some());

    main_assert!(app
        .open_classic_lobby_client_info(0)
        .expect("reopen client information for gamepad ownership"));
    let slot = GamepadSlot::new(0);
    app.test_gamepad_events([
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
    ]);
    main_assert!(app.runtime_client_list.is_none());
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(app.network_lobby.is_some());
}

#[test]
fn classic_lobby_add_player_picker_publishes_relative_file_and_projects_echo() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let user_data = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let player_dir = install.path().join("Players");
    fs::create_dir_all(&player_dir).test_value();
    let player_path = player_dir.join("Alice.c4p");
    let mut group = clonk_resources::MutableGroup::new("Alice.c4p");
    group
        .add_file_with_metadata(
            "Player.txt",
            b"[Player]\nName=Alice\n[Preferences]\nColorDw=1193046\n".to_vec(),
            1,
            false,
        )
        .test_value();
    fs::write(&player_path, group.pack().test_value()).test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=US\nPlayerPath=Players\n[Network]\nLocalName=Host\n",
    )
    .test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_test_classic_host_lobby(&mut app);
    app.control_clients
        .replace_snapshot([lobby_fixture!(client {
            client_id: 0,
            activated: true,
            name: LegacyCString::from_bytes(b"Host".to_vec()).test_value(),
        })]);
    let (event_tx, commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));

    process_classic_lobby_action(
        &mut app,
        ClassicLobbyAction::AddPlayerRequested { client_id: 0 },
    );
    let selected = some(&app.definition_selector).rows()[0]
        .full_path()
        .to_string();
    main_assert_eq!(some(&app.definition_selector).root_path() => player_dir.to_string_lossy(), "C4PlayerSelDlg snapshots the configured PlayerPath root");
    main_assert_eq!(
        some(&app.pending_lobby_player_selection).candidates[&selected].wire_filename =>
        "Players/Alice.c4p",
        "the physical picker path must not leak onto the wire"
    );
    main_assert_eq!(
        some(&app.pending_lobby_player_selection).candidates[&selected].source_path =>
        player_path,
        "the exact physical path must survive the string-keyed selector"
    );

    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=US\nPlayerPath=ChangedAfterOpen\n",
    )
    .test_value();
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::RefreshRequested,
    ])
    .test_value();
    main_assert_eq!(some(&app.definition_selector).rows()[0].full_path() => selected, "F5 must not re-read PlayerPath");

    let resource = lobby_fixture!(player_resource:
        17,
        LegacyCString::from_bytes(b"Players/Alice.c4p".to_vec()).test_value(),
    );
    let (direct_ready, direct_wait) = std::sync::mpsc::channel();
    let command_observer = thread::spawn(move || {
        commands.complete_runtime_host_join(resource, event_tx, direct_ready)
    });
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Accepted(vec![selected]),
    ])
    .test_value();
    direct_wait
        .recv_timeout(Duration::from_secs(1))
        .test_value();
    app.test_network_events();
    drop(app.network.take());
    let (_, _, infos, _) = command_observer.test_join();
    main_assert_eq!(infos.len() => 1);
    main_assert!(app_classic_lobby(&app)
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
    let (_events, mut commands) = install_client_network_commands(&mut app, 0);

    request_classic_lobby_team(&mut app, 7);

    let menu = some(&app.context_menu);
    let panel = &menu.layout().panels[0];
    main_assert_eq!(panel.bounds.x => team_rect.x);
    main_assert_eq!(panel.bounds.y => team_rect.y + team_rect.h);
    main_assert!(panel.bounds.w >= team_rect.w);
    main_assert_eq!(panel.rows.len() => 2, "the full current team stays visible; full and negative-limit alternatives are filtered");
    main_assert_eq!(app.context_menu_lobby_team_player => Some(7));
    main_assert_eq!(app_classic_lobby(&app).controller.open_team_combo_player() => Some(7));

    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select first team row"));
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::ArrowDown, ElementState::Pressed)
        .expect("select second team row"));
    main_assert!(app
        .handle_context_menu_key(VirtualKeyCode::Enter, ElementState::Pressed)
        .expect("activate selected team"));

    let updates = commands.take_player_info_updates();
    main_assert_eq!(updates.len() => 1);
    let mut changed = chooser;
    changed.team = 2;
    main_assert_eq!(
        updates[0] =>
        lobby_fixture!(player_update: 0, clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL, vec![changed, companion]),
        "OnTeamComboSelChange clones the complete client packet and mutates only Team"
    );
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.context_menu_lobby_team_player => None);
    main_assert_eq!(
        app_classic_lobby(&app)
            .controller
            .rows()
            .iter()
            .find_map(|row| match row {
                LobbyRosterRow::Player(player) if player.id == 7 => {
                    player.team.as_ref().map(|team| team.id)
                }
                _ => None,
            }) =>
        Some(1),
        "the combo text waits for the authoritative player-info echo"
    );

    app.network_lobby = Some(NetworkLobbyState::new(0, "Peer view".to_string(), false));
    app.apply_direct_player_info_control(
        lobby_fixture!(player_data: updates[0].client_id, flags: updates[0].flags, updates[0].players.clone(), by: 0),
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
    main_assert_eq!(projected_team(app_classic_lobby(&app).controller.rows()) => Some(2));
    main_assert_eq!(projected_team(&app_lobby(&app).roster_rows) => Some(2), "the same authoritative PlayerInfo converges every lobby projection");
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

    main_assert_eq!(pings.get(&7) => Some(&70));
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

    main_assert_eq!(pings.get(&7) => Some(&20));
    main_assert_eq!(pings.get(&8) => Some(&0), "zero is a visible ping");
    main_assert!(!pings.contains_key(&9));
    main_assert!(!pings.contains_key(&10));
    main_assert!(
        !pings.contains_key(&11),
        "a present data route replaces a nonpositive message value"
    );
    main_assert_eq!(pings.get(&12) => Some(&13));
    main_assert!(
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
    let lobby = app_classic_lobby_mut(&mut app.classic_host_lobby);
    let mut rows = lobby.controller.rows().to_vec();
    rows.push(client_row(7, "Downloading"));
    rows.push(client_row(8, "Disconnected"));
    lobby.controller.set_rows(rows);

    let connection = |connection_id, usage: &str, protocol, ping_ms, lag_ms| {
        clonk_network::RuntimeNetworkConnection {
            connection_id,
            client_id: 7,
            usage: usage.to_string(),
            protocol,
            peer_address: None,
            packet_loss: 0,
            ping_ms,
            lag_ms,
        }
    };

    let (network, _events, _commands) = NetworkManager::test_stub_with_commands();
    network.set_test_lobby_client_telemetry(clonk_network::RuntimeLobbyClientTelemetry {
        connections: vec![
            // An unanswered message-route ping whose getLag wait (70)
            // outgrew the cached round trip (33): the roster label shows
            // the live 70 (src/C4PlayerInfoListBox.cpp:894-905).
            connection(1, "Msg", clonk_network::NetworkProtocol::Udp, 33, 70),
            connection(2, "Data", clonk_network::NetworkProtocol::Tcp, 20, 20),
        ],
        resource_progress: vec![(7, 30), (8, 25)],
    });
    app.network = Some(network);

    main_assert!(app.sec1_timer().expect("refresh lobby telemetry"));
    let clients = app_classic_lobby(&app)
        .controller
        .rows()
        .iter()
        .filter_map(|row| match row {
            LobbyRosterRow::Client(client) => Some((client.id, client)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    main_assert_eq!(clients[&0].ping_ms => None);
    main_assert_eq!(clients[&0].resource_progress => None);
    main_assert_eq!(clients[&7].ping_ms => Some(70));
    main_assert_eq!(clients[&7].resource_progress => Some(30));
    main_assert!(clients[&7].connected);
    main_assert_eq!(clients[&8].ping_ms => None);
    main_assert_eq!(clients[&8].resource_progress => None);
    main_assert!(!clients[&8].connected);

    some(&app.network).set_test_lobby_client_telemetry(
        clonk_network::RuntimeLobbyClientTelemetry {
            connections: vec![connection(
                1,
                "Data/Msg",
                clonk_network::NetworkProtocol::Tcp,
                15,
                15,
            )],
            resource_progress: vec![(7, 100)],
        },
    );
    main_assert!(app.sec1_timer().expect("refresh completed resources"));
    let completed = app_classic_lobby(&app)
        .controller
        .rows()
        .iter()
        .find_map(|row| match row {
            LobbyRosterRow::Client(client) if client.id == 7 => Some(client),
            _ => None,
        })
        .test_value();
    main_assert_eq!(completed.ping_ms => Some(15));
    main_assert_eq!(completed.resource_progress => Some(100), "native keeps the (100%) prefix while the remote remains connected");
}

#[test]
fn client_roster_projection_hides_foreign_team_controls_and_random_assignments() {
    let team = |id, name: &[u8], player_ids, color| clonk_engine::InitialNetworkTeam {
        id,
        name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
        player_ids,
        color,
        ..set_control_test_team(id, Vec::new(), 0)
    };
    let mut metadata = set_control_test_metadata(
        false,
        vec![
            team(1, b"One", vec![1], 0x00f4_0000),
            team(2, b"Two", vec![2], 0x0000_c800),
        ],
    );
    metadata.custom = true;
    metadata.team_colors = true;
    metadata.max_script_players = 1;
    let project = |metadata: &clonk_engine::InitialNetworkTeamMetadata, local_client_id| {
        let mut clients = ControlClientRegistry::default();
        clients.replace_snapshot([
            lobby_fixture!(client {
                client_id: 0,
                activated: true,
            }),
            lobby_fixture!(client {
                client_id: 7,
                activated: true,
            }),
            lobby_fixture!(client {
                client_id: 8,
                activated: false,
            }),
            lobby_fixture!(client {
                client_id: 9,
                activated: true,
            }),
        ]);
        let mut infos = ControlPlayerInfoRegistry::default();
        let random_invisible = matches!(
            metadata.team_distribution,
            clonk_engine::InitialNetworkTeamDistribution::RandomInvisible
        );
        infos.replace_snapshot(
            3,
            [
                lobby_fixture!(player_data:
                    0,
                    vec![lobby_fixture!(player {
                        id: 1,
                        team: 1,
                        flags: if random_invisible {
                            clonk_engine::PLAYER_INFO_FLAG_JOINED
                        } else {
                            0
                        },
                        savegame_player: if random_invisible { 41 } else { 0 },
                        name: LegacyCString::from_bytes(b"Host player".to_vec()).unwrap(),
                    })],
                ),
                lobby_fixture!(player_data:
                    7,
                    vec![lobby_fixture!(player {
                        id: 2,
                        team: 2,
                        flags: if random_invisible {
                            clonk_engine::PLAYER_INFO_FLAG_JOINED
                        } else {
                            0
                        },
                        savegame_player: if random_invisible { 42 } else { 0 },
                        name: LegacyCString::from_bytes(b"Own player".to_vec()).unwrap(),
                    })],
                ),
                lobby_fixture!(player_data: 8, Vec::new()),
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
    main_assert_eq!(selectable(&client_rows, 1) => Some(false));
    main_assert_eq!(selectable(&client_rows, 2) => Some(true));
    main_assert!(client_rows.iter().any(|row| matches!(
        row,
        LobbyRosterRow::Header(LobbyHeaderRow {
            kind: LobbyRosterHeader::ScriptPlayers,
            can_add_player: false,
            ..
        })
    )));
    main_assert!(client_rows.iter().any(|row| matches!(
        row,
        LobbyRosterRow::Client(LobbyClientRow {
            id: 8,
            status: LobbyClientStatus::Observer,
            ..
        })
    )));
    main_assert!(client_rows.iter().any(|row| matches!(
        row,
        LobbyRosterRow::Client(LobbyClientRow {
            id: 9,
            status: LobbyClientStatus::Unknown,
            ..
        })
    )));

    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Host;
    main_assert_eq!(selectable(&project(&metadata, 7), 2) => Some(false));
    main_assert_eq!(selectable(&project(&metadata, 0), 1) => Some(true));
    main_assert_eq!(selectable(&project(&metadata, 0), 2) => Some(true));

    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::RandomInvisible;
    let random_rows = project(&metadata, 7);
    main_assert!(random_rows
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
        name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).test_value(),
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
    let mut script = lobby_fixture!(player {
        id: 7,
        player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
        team: 1,
    });

    main_assert!(classic_lobby_player_can_choose_team(&teams, &script, false));
    main_assert!(
        !classic_lobby_player_can_choose_team(&teams, &script, true),
        "an associated savegame script row has joined info"
    );
    teams.teams[1].player_ids.push(8);
    main_assert!(
        !classic_lobby_player_can_choose_team(&teams, &script, false),
        "the only other team is full"
    );
    teams.auto_generate_teams = 1;
    main_assert!(classic_lobby_player_can_choose_team(&teams, &script, false));
    script.flags |= clonk_engine::PLAYER_INFO_FLAG_JOINED;
    main_assert!(!classic_lobby_player_can_choose_team(
        &teams, &script, false
    ));
}

#[test]
fn clicking_an_open_lobby_team_combo_closes_without_reopening() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_team_lobby(&mut app);
    let team_rect = test_lobby_team_rect(&mut app);
    let (_events, mut commands) = install_client_network_commands(&mut app, 0);
    request_classic_lobby_team(&mut app, 7);

    let point = PhysicalPosition::new(f64::from(team_rect.x + 2), f64::from(team_rect.y + 2));
    app.test_cursor(point);
    app.test_left_button(ElementState::Pressed);

    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.context_menu_lobby_team_player => None);
    main_assert_eq!(app.context_menu_pointer_dismissed_lobby_team_player => None);
    main_assert_eq!(app_classic_lobby(&app).controller.open_team_combo_player() => None);
    main_assert!(commands.take_player_info_updates().is_empty());
    app.test_left_button(ElementState::Released);
    main_assert!(app.context_menu.is_none());

    app.context_menu_pointer_dismissed_lobby_team_player = Some(7);
    app.handle_gamepad_event(GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    })
    .test_value();
    main_assert_eq!(app.context_menu_pointer_dismissed_lobby_team_player => None);

    request_classic_lobby_team(&mut app, 7);
    main_assert!(app.context_menu.is_some());
    main_assert!(app.select_classic_lobby_sheet(LobbySheet::Resources));
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.context_menu_lobby_team_player => None);
}

#[test]
fn classic_lobby_team_combo_rechecks_cpp_team_permissions_before_opening() {
    let mut app = new_menu_app(640, 480);
    let (mut chooser, companion) = install_test_classic_host_team_lobby(&mut app);
    let metadata = some_mut(&mut app.network_team_assignment).teams_mut();
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    request_classic_lobby_team(&mut app, 7);
    main_assert!(app.context_menu.is_none());

    let metadata = some_mut(&mut app.network_team_assignment).teams_mut();
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Free;
    metadata.teams.retain(|team| team.id == 1);
    request_classic_lobby_team(&mut app, 7);
    main_assert!(app.context_menu.is_none());

    some_mut(&mut app.network_team_assignment)
        .teams_mut()
        .auto_generate_teams = true;
    request_classic_lobby_team(&mut app, 7);
    main_assert!(app.context_menu.is_some());
    app.close_context_menu_silently();

    chooser.savegame_player = 99;
    app.control_player_infos.replace_snapshot(
        8,
        [lobby_fixture!(player_data: 0, flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL, vec![chooser, companion], by: 0)],
    );
    request_classic_lobby_team(&mut app, 7);
    main_assert!(app.context_menu.is_none());
}

#[test]
fn focused_lobby_team_combo_opens_from_cpp_keyboard_bindings_and_escape_closes() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_team_lobby(&mut app);
    let (_, roster) = app.classic_host_lobby_layouts().test_value();
    let player_row = roster
        .rows
        .iter()
        .find(|row| row.index == 1)
        .test_value()
        .rect;
    app.test_cursor(PhysicalPosition::new(
        f64::from(player_row.x + player_row.w / 2),
        f64::from(player_row.y + 2),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app_classic_lobby(&app).controller.focus() => LobbyControl::Roster);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(app_classic_lobby(&app).controller.focus() => LobbyControl::RosterTeam);

    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.context_menu.is_none());

    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

    app.keyboard_modifiers = ModifiersState::ALT;
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    app.keyboard_modifiers = ModifiersState::empty();
}

#[test]
fn paste_scanner_preserves_edit_rules_and_skips_empty_lines() {
    let paste = |mut view: LobbyChatEditView, text: &str, keep_going| {
        let mut submissions = Vec::new();
        let outcome = lobby_chat_paste_text(
            &mut view,
            text,
            LobbyChatPasteMode::Lobby,
            |_| {},
            |submission| {
                submissions.push(submission);
                Ok::<bool, ()>(keep_going)
            },
        )
        .test_value();
        (view, submissions, outcome)
    };

    let (view, submissions, outcome) = paste(
        LobbyChatEditView {
            text: "abZZcd".into(),
            caret: 4,
            selection: Some((2, 4)),
            cursor_visible: true,
            ..LobbyChatEditView::default()
        },
        "x|y\t\u{1}",
        true,
    );
    main_assert_eq!(outcome.completed_lines => 0);
    main_assert!(submissions.is_empty());
    main_assert_eq!(view.text => "abx¦y\t\u{1}cd");
    main_assert_eq!(view.caret => 8);
    main_assert_eq!(view.selection => None);

    let mut typed = LobbyChatEditView::default();
    main_assert!(!lobby_chat_insert_text(&mut typed, "\t"));
    main_assert!(lobby_chat_insert_text(&mut typed, "\u{80}"));
    main_assert_eq!(typed.text => "\u{80}");

    let (view, submissions, outcome) = paste(
        LobbyChatEditView {
            text: "draft".into(),
            caret: 5,
            cursor_visible: true,
            ..LobbyChatEditView::default()
        },
        "\r\nmore",
        true,
    );
    main_assert_eq!(outcome.completed_lines => 0);
    main_assert!(submissions.is_empty());
    main_assert_eq!(view.text => "draftmore");

    let oversized = format!("{}\ntrailing", "a".repeat(300));
    let (view, submissions, outcome) = paste(LobbyChatEditView::default(), &oversized, true);
    main_assert_eq!(outcome.completed_lines => 1);
    main_assert_eq!(submissions.len() => 1);
    main_assert_eq!(clonk_script::c4_string_byte_len(&submissions[0]) => 254);
    main_assert_eq!(view.text => "trailing");
    main_assert_eq!(view.caret => view.text.len());

    let (view, submissions, outcome) = paste(LobbyChatEditView::default(), "one\ntwo\nthree", true);
    main_assert_eq!(outcome.completed_lines => 2);
    main_assert_eq!(submissions => ["one", "two"]);
    main_assert_eq!(view.text => "three");

    let (view, submissions, outcome) =
        paste(LobbyChatEditView::default(), "first\nnever-inserted", false);
    main_assert!(outcome.stopped);
    main_assert_eq!(submissions => ["first"]);
    main_assert!(view.text.is_empty());
}

#[test]
fn lobby_paste_submits_each_line_and_retains_the_tail() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (_events, mut commands) = install_client_network_commands(&mut app, 0);

    app.paste_classic_lobby_chat_text("hello|there\nsecond\nworld")
        .test_value();
    let submitted = commands.take_submitted_messages();
    main_assert_eq!(submitted.len() => 2);
    main_assert_eq!(submitted[0].message.as_bytes() => "hello¦there".as_bytes());
    main_assert_eq!(submitted[1].message.as_bytes() => b"second");
    main_assert_eq!(app_classic_lobby(&app).controller.chat_edit_view().text => "world");

    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(client_lobby_state());
    let (_events, mut commands) = install_client_network_commands(&mut app, 7);
    app.paste_network_lobby_chat_text("one\r\ntwo\nthree")
        .test_value();
    let submitted = commands.take_submitted_messages();
    main_assert_eq!(submitted.len() => 2);
    main_assert_eq!(submitted[0].message.as_bytes() => b"one");
    main_assert_eq!(submitted[1].message.as_bytes() => b"two");
    main_assert_eq!(app_lobby(&app).chat_edit.text => "three");
}

#[test]
fn running_paste_obeys_finish_result_and_crlf_more_flag() {
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_client_network_commands(&mut app, 0);
    let paste = |app: &mut GameApp, text: &str| {
        let layout = app.game_option_input_layout().test_value();
        let fonts = app.assets.clonk_fonts.clone().test_value();
        let actions = app
            .running_chat_controller_mut()
            .test_value()
            .apply_context_command(
                InputDialogContextCommand::Paste,
                Some(text),
                &layout,
                &fonts.text,
            );
        app.finish_game_option_input_dialog_actions(actions)
            .test_value();
    };

    app.start_running_chat(RunningChatMode::All);
    paste(&mut app, "hello\nsecond\nworld");
    let submitted = commands.take_submitted_messages();
    main_assert_eq!(submitted.len() => 2);
    main_assert_eq!(submitted[0].message.as_bytes() => b"hello");
    main_assert_eq!(submitted[1].message.as_bytes() => b"second");
    main_assert_eq!(app.running_chat_text() => Some("world"));

    app.close_running_chat().test_value();
    app.start_running_chat(RunningChatMode::All);
    paste(&mut app, "done\n");
    let submitted = commands.take_submitted_messages();
    main_assert_eq!(submitted.len() => 1);
    main_assert_eq!(submitted[0].message.as_bytes() => b"done");
    main_assert!(app.chat.running.is_none());

    app.start_running_chat(RunningChatMode::All);
    paste(&mut app, "stay\r\n");
    let submitted = commands.take_submitted_messages();
    main_assert_eq!(submitted.len() => 1);
    main_assert_eq!(submitted[0].message.as_bytes() => b"stay");
    main_assert_eq!(app.running_chat_text() => Some("stay"));
    main_assert_eq!(app.running_chat_controller().expect("CRLF reports more at the first delimiter").selection() => Some((0, 4)));
}

#[test]
fn classic_lobby_chat_edits_parses_and_submits_private_delivery_controls() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (_events, mut commands) = install_client_network_commands(&mut app, 0);

    process_classic_lobby_chat_action(
        &mut app,
        LobbyChatRequest::InsertText("/me hello".to_string()),
    );
    main_assert_eq!(app_classic_lobby(&app).controller.chat_edit_view().text => "/me hello");
    process_classic_lobby_chat_action(&mut app, LobbyChatRequest::Submit("/me hello".to_string()));

    main_assert_eq!(
        commands.take_submitted_messages() =>
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_ME,
            player: -1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"hello".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 0,
        }]
    );
    main_assert!(app_classic_lobby(&app)
        .controller
        .chat_edit_view()
        .text
        .is_empty());

    main_assert!(app.engine.set_team_distribution(4));
    process_classic_lobby_chat_action(&mut app, LobbyChatRequest::Submit("^surprise".to_string()));
    main_assert!(commands.take_submitted_messages().is_empty());
    main_assert_eq!(app_classic_lobby(&app).controller.logs().last().map(|line| line.text.as_str()) => Some("Can't send team message: Teams not known."));
    process_classic_lobby_chat_action(&mut app, LobbyChatRequest::Submit("^".to_string()));
    main_assert!(commands.take_submitted_messages().is_empty());
    main_assert_eq!(app_classic_lobby(&app).controller.logs().last().map(|line| line.text.as_str()) => Some("Can't send team message: Teams not known."));

    let alert = parse_lobby_message_control(" /alert ")
        .expect("parse")
        .test_value();
    main_assert_eq!(alert.message_type => MESSAGE_TYPE_NORMAL);
    let alert = parse_lobby_message_control("/alert")
        .expect("parse alert")
        .test_value();
    main_assert_eq!(alert.message_type => MESSAGE_TYPE_ALERT);
    main_assert!(alert.message.is_empty());

    app.control_clients
        .replace_snapshot([message_client(7, b"Remote")]);
    process_classic_lobby_chat_action(
        &mut app,
        LobbyChatRequest::Submit("/mute Remote".to_string()),
    );
    main_assert!(app.control_messages.is_muted(7));
    process_classic_lobby_chat_action(
        &mut app,
        LobbyChatRequest::Submit("/unmute Remote".to_string()),
    );
    main_assert!(!app.control_messages.is_muted(7));
    main_assert!(commands.take_submitted_messages().is_empty());
}

#[test]
fn classic_host_lobby_exit_directly_tears_down_and_returns_to_startup() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = prepare_tutorial_host_lobby(&app, repository);
    app.staged_network_host_scenario = Some(staged);
    install_test_classic_host_lobby(&mut app);
    let _ = app_classic_lobby_mut(&mut app.classic_host_lobby)
        .controller
        .apply_countdown_packet(clonk_frontend::game_lobby::LobbyCountdownPacket::Seconds(3));
    main_assert!(app_classic_lobby(&app).controller.countdown().is_any());

    let _events = install_network_stub(&mut app);
    app.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 11112, "Exact Host".to_string(), None),
    ));
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
        .apply(lobby_fixture!(player_data: 8, vec![lobby_fixture!(player { id: 9 })]));
    app.executing_ready_tick = Some(6);
    main_assert!(app.loader_screen.is_some());

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);

    main_assert_eq!(app.startup.view => StartupView::MainMenu);
    main_assert!(app.classic_host_lobby.is_none());
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.staged_network_host_scenario.is_none());
    // The return through PreInit re-initializes the loader screen for the
    // next game (src/C4Application.cpp:242-247,373-389).
    main_assert!(app.loader_screen.is_some());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.startup_network_connection.is_none());
    main_assert!(app.network_ticks.ready.is_empty());
    main_assert!(app.network_sync.scheduled.is_empty());
    main_assert!(app.sync_checks.local.is_empty() && app.sync_checks.remote.is_empty());
    main_assert!(app.admission_resources.resources.is_empty());
    main_assert!(app.executing_ready_tick.is_none());
    main_assert!(app.control_player_infos.client_info_ids(8).is_empty());
    main_assert!(app.network_control_running);
    main_assert_eq!(app.scenario_game_options.context() => GameOptionContext::LocalSelector);
    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.status_text.is_empty());
    reset_cached_app_paths();
}

#[test]
fn classic_host_lobby_chat_keyboard_routes_edit_locally() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_test_classic_host_lobby(&mut app);

    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.context_menu.is_none());

    for (key, modifiers) in [
        (VirtualKeyCode::KeyA, ModifiersState::CONTROL),
        (VirtualKeyCode::ArrowLeft, ModifiersState::CONTROL),
        (VirtualKeyCode::Delete, ModifiersState::empty()),
        (VirtualKeyCode::Home, ModifiersState::SHIFT),
    ] {
        app.test_modifiers(modifiers);
        app.test_key(key, ElementState::Pressed);
    }
    app.test_modifiers(ModifiersState::empty());
    app.test_text_input('x');
    main_assert_eq!(app_classic_lobby(&app).controller.chat_edit_view().text => "x");

    for _ in 0..10 {
        if app
            .classic_host_lobby
            .as_ref()
            .is_some_and(|lobby| lobby.controller.focus() == LobbyControl::Roster)
        {
            break;
        }
        app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    }
    main_assert_eq!(app_classic_lobby(&app).controller.focus() => LobbyControl::Roster);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    main_assert!(app.context_menu.is_none());
}

#[test]
fn generic_client_lobby_chat_submits_private_delivery_message_controls() {
    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(client_lobby_state());
    let (_events, mut commands) = install_client_network_commands(&mut app, 7);

    for character in "hello".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);

    main_assert_eq!(
        commands.take_submitted_messages() =>
        vec![MessageControlData {
            message_type: MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(b"hello".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 7,
        }]
    );
    main_assert!(app_lobby(&app).chat_edit.text.is_empty());

    for character in "ab".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Backspace, ElementState::Pressed);
    main_assert_eq!(app_lobby(&app).chat_edit.text => "a");
    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    main_assert_eq!(app_lobby(&app).chat_edit.text => "hello");
    app.mode = AppMode::Running;
    app.start_running_chat(RunningChatMode::All);
    app.browse_running_chat_history(true);
    main_assert_eq!(app.running_chat_text() => Some("hello"), "C4MessageInput history survives the lobby-to-game transition");
}

#[test]
fn classic_host_lobby_network_events_update_supported_live_state() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (events, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 11112, "Exact Host".to_string(), None),
    ));

    events
        .send(NetworkEvent::PeerConnected {
            client_id: 0,
            name: "Exact Host".to_string(),
            kind: ParticipantKind::Player,
        })
        .test_value();
    app.test_network_events();
    main_assert!(app.status_text.is_empty());
    main_assert!(app.network_lobby.is_none());

    app.control_player_infos.replace_snapshot(
        7,
        [lobby_fixture!(player_data:
            0,
            vec![lobby_fixture!(player {
                id: 7,
                color: 0x00ff_0000,
                original_color: 0x00ff_0000,
                league_projected_gain: -1,
            })],
        )],
    );
    events
        .send(NetworkEvent::LeagueUpdate(
            clonk_network::LeagueUpdateResponse {
                player_infos: clonk_network::ClientPlayerInfosSnapshot {
                    client_id: -1,
                    flags: 0,
                    players: vec![lobby_fixture!(player {
                        id: 7,
                        league_projected_gain: 4,
                    })],
                },
                ..Default::default()
            },
        ))
        .test_value();
    app.test_network_events();
    main_assert_eq!(app.control_player_infos.get(7).unwrap().league_projected_gain => 4);
    let league_broadcasts = commands.take_broadcast_player_infos();
    let [league_info] = league_broadcasts.as_slice() else {
        panic!("expected one projected-gain PlayerInfo broadcast");
    };
    main_assert_eq!(league_info.client_id => 0);
    main_assert_eq!(league_info.players.len() => 1);
    main_assert_eq!(league_info.players[0].id => 7);
    main_assert_eq!(league_info.players[0].league_projected_gain => 4);

    send_added_player_infos(
        &events,
        1,
        1,
        vec![lobby_fixture!(player {
            name: clonk_engine::LegacyCString::from_bytes(b"Remote player".to_vec())
                .expect("valid player name"),
        })],
    );
    app.test_network_events();
    let broadcasts = commands.take_broadcast_player_infos();
    let [gain_reset, admitted] = broadcasts.as_slice() else {
        panic!("expected gain reset then authoritative admission, got {broadcasts:?}");
    };
    main_assert_eq!(gain_reset.client_id => 0);
    main_assert_eq!(gain_reset.players[0].id => 7);
    main_assert_eq!(gain_reset.players[0].league_projected_gain => -1);
    main_assert_eq!(admitted.client_id => 1);
    for info in broadcasts {
        events
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                original: info.clone(),
                info,
                join_players_on_echo: Vec::new(),
            })
            .test_value();
    }
    app.test_network_events();
    main_assert!(app.control_player_infos.contains_client(1));

    events
        .send(NetworkEvent::PeerConnected {
            client_id: 1,
            name: "Remote".to_string(),
            kind: ParticipantKind::Player,
        })
        .test_value();
    app.test_network_events();
    events
        .send(NetworkEvent::DirectControl(NetworkControl::ClientJoin(
            clonk_engine::ClientJoinControlData {
                core: clonk_engine::ClientCoreControlData {
                    client_id: 1,
                    activated: true,
                    observer: false,
                    name: LegacyCString::from_bytes(b"Remote".to_vec()).test_value(),
                    nick: LegacyCString::from_bytes(b"Remote nick".to_vec()).test_value(),
                    lobby_ready: false,
                },
                by_client: 0,
            },
        )))
        .test_value();
    app.test_network_events();
    // Raw transport callbacks stay presentation-silent. The accepted
    // host-authored control owns C++'s localized lobby log
    // (src/C4GameLobby.cpp:669-675; src/C4Control.cpp:554-565;
    // src/C4Log.cpp:227-239).
    main_assert!(app.status_text.is_empty());
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.classic_host_lobby.is_some());
    main_assert_eq!(
        app_classic_lobby(&app).controller.logs().last() =>
        Some(&LobbyLogLine {
            text: "Client Remote connected.".to_string(),
            color: [0xaf, 0xaf, 0xaf, 0xff],
        })
    );
    main_assert!(app_classic_lobby(&app)
        .controller
        .rows()
        .iter()
        .any(|row| matches!(row, LobbyRosterRow::Client(client) if client.id == 1)));
}

#[test]
fn classic_host_lobby_cancel_paths_clear_pressed_activation_latches() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
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
        app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    }
    main_assert_eq!(app_classic_lobby(&app).controller.focus() => LobbyControl::Exit);

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.handle_focus_lost().test_value();
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.resize(650, 490).test_value();
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);

    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Pressed,
    )
    .test_value();
    app.handle_gamepad_event(GamepadEvent::Clear {
        slot: GamepadSlot::new(0),
    })
    .test_value();
    app.handle_gamepad_action(
        GamepadSlot::new(0),
        GamepadActionType::Select,
        ElementState::Released,
    )
    .test_value();

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.pointer_left().test_value();
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.startup.view => StartupView::MainMenu);
    main_assert!(app.classic_host_lobby.is_none());
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
        .test_value();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::Join,
    ));
    app.poll_startup_network_connection().test_value();
    main_assert!(app.network.is_some());
    main_assert!(matches!(app.network_mode, Some(NetworkMode::Client(_))));
    main_assert!(app.classic_host_lobby.is_none());
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    let lobby = app_lobby(&app);
    main_assert_eq!(lobby.local_client_id => 7);
    main_assert!(!lobby.is_host);
    main_assert!(app.status_text.is_empty());
    main_assert!(!app.network_control_running);
    main_assert!(app.control_clients.contains(7));
    main_assert!(!app.control_clients.is_activated(7));

    events
        .send(NetworkEvent::PeerConnected {
            client_id: 7,
            name: "Client".to_string(),
            kind: ParticipantKind::Player,
        })
        .test_value();
    app.test_network_events();
    main_assert!(
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
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
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
    .test_value();
    wait_for_menu(&mut app);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(client_lobby_state());
    app.sync_network_lobby_game_option_state();
    let mut frame = vec![0x5a; 640 * 480 * 4];

    app.test_render(&mut frame);

    let layout = app
        .network_lobby
        .as_ref()
        .and_then(|lobby| lobby.layout.as_ref())
        .test_value();
    main_assert_eq!(
        (
            layout.ready_button.origin.x as i32,
            layout.ready_button.origin.y as i32,
            layout.ready_button.size.width as i32,
            layout.ready_button.size.height as i32,
        ) =>
        (508, 400, 110, 32)
    );
    main_assert!(layout.start_button.is_none());
    main_assert!(frame.iter().any(|byte| *byte != 0x5a));

    let (classic_layout, roster) = app.joined_lobby_layouts().test_value();
    let exit = GuiPoint::new(
        (classic_layout.exit_button.x + 1) as f32,
        (classic_layout.exit_button.y + 1) as f32,
    );
    app_lobby_mut(&mut app.network_lobby)
        .controller
        .pointer_move(exit, &classic_layout, &roster);
    main_assert!(app_lobby(&app)
        .controller
        .tooltip_state_at(Instant::now() + Duration::from_secs(1))
        .is_some());
    let sentinel = vec![0x45; 640 * 480 * 4];
    let mut refreshed = sentinel.clone();
    main_assert!(app
        .render(&mut refreshed)
        .expect("joined lobby composes a fresh frame"));
    main_assert_ne!(refreshed => sentinel);

    // The classic renderer remains fail-closed when its required assets
    // are absent; NetworkLobby must not re-enable the old generic pane.
    let mut assetless = new_menu_app(320, 200);
    Arc::get_mut(&mut assetless.assets)
        .test_value()
        .startup_dialog_images
        .remove("GUIButtonDown.png")
        .test_value();
    assetless.startup.view = StartupView::NetworkLobby;
    assetless.network_lobby = Some(client_lobby_state());
    let mut untouched = vec![0x3c; 320 * 200 * 4];
    let error = assetless
        .render(&mut untouched)
        .expect_err("assetless lobby refuses generic fallback");
    main_assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::GlobalGuiBootstrapResources { issues })
            if issues.contains(&ClassicGuiBootstrapIssue::missing("GUIButtonDown"))
    ));
    main_assert!(untouched.iter().all(|byte| *byte == 0x3c));
    reset_cached_app_paths();
}

#[test]
fn saving_a_file_picture_preserves_an_unchecked_lobby_icon() {
    let _lock = env_lock().lock();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(repository)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    let player_root = user_data.path().join("Players");
    persist_config_value(
        &paths,
        "General",
        "PlayerPath",
        player_root.to_string_lossy().into_owned(),
    )
    .test_value();
    let selected_path = paths.user_data_dir().join("Selected.PNG");
    write_preview_image(&selected_path, [0, 0, 255, 255], image::ImageFormat::Png);

    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    app.open_new_startup_player_properties();
    let old_icon = ImageData::new(2, 1, vec![4, 5, 6, 255, 7, 8, 9, 255]);
    let pending = some_mut(&mut app.startup.player_properties_dialog);
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
    main_assert_eq!(
        some(&app.startup.player_properties_dialog)
            .controller
            .big_icon_update() =>
        &clonk_frontend::startup_plrproperties::PlayerImageUpdate::Replace(old_icon.clone())
    );
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    let saved = Group::open(player_root.join("UncheckedIcon.c4p")).test_value();
    let encoded_icon = saved.read_file("BigIcon.png").test_value();
    let decoded_icon = image::load_from_memory(&encoded_icon)
        .test_value()
        .into_rgba8();
    main_assert_eq!(decoded_icon.dimensions() => (2, 1));
    main_assert_eq!(decoded_icon.into_raw() => old_icon.pixels());
    reset_cached_app_paths();
}

#[test]
fn network_lobby_renders_live_without_a_deferred_native_text_pass() {
    let mut app = new_real_classic_menu_app(320, 200);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Host".to_string(), true));
    app.sync_network_lobby_game_option_state();
    main_assert!(app.status_text.is_empty());

    // A retained lobby advances tooltip clocks, held scrollbars and
    // transient status icons without input, so every presentation composes
    // the live frame.
    let sentinel = vec![0x73; 320 * 200 * 4];
    let mut frame = sentinel.clone();
    main_assert!(app.render(&mut frame).expect("live lobby renders"));
    main_assert_ne!(frame => sentinel, "the live lobby must reach the frame");

    let mut native_frame = vec![0x47; 960 * 600 * 4];
    app.render_native_main_menu_text(&mut native_frame, 960, 600)
        .test_value();
    main_assert!(native_frame.iter().all(|byte| *byte == 0x47));
}

#[test]
fn options_program_round_trips_bound_values_and_raw_fair_crew_strength() {
    use clonk_frontend::startup_options_dlg::{fair_crew_slider_to_strength, OptionsDlgAction};

    let _lock = env_lock().lock();
    let install_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let user_data = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nFontName=Endeavour\nFontSize=14\nUseWhiteIngameChat=1\nUseWhiteLobbyChat=1\nShowLogTimestamps=0\nPreloading=0\nDefCrewStrength=1000\nVendorProgramKey=keep\n").test_value();

    let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths)).test_value();
    wait_for_menu(&mut app);
    app.open_options_menu();

    let program = some(&app.startup.options_dialog).program();
    main_assert_eq!(program.font_face => "Endeavour");
    main_assert_eq!(program.font_size => "14");
    main_assert!(program.white_chat_ingame);
    main_assert!(program.white_chat_lobby);
    main_assert!(!program.preloading);
    main_assert_eq!(program.fair_crew_strength => 1_000);
    main_assert_eq!(program.fair_crew_slider => 9);

    let strength = fair_crew_slider_to_strength(10);
    {
        let program = some_mut(&mut app.startup.options_dialog).program_mut();
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
    .test_value();
    main_assert!(!app.display_flags.white_chat);
    main_assert!(!app.white_lobby_chat);
    app.process_options_dialog_actions(vec![OptionsDlgAction::Back])
        .test_value();

    let config = Config::load(paths.config_file()).test_value();
    main_assert_eq!(config.get_in(Some("General"), "FontName") => Some("Endeavour"));
    main_assert_eq!(config.get_in(Some("General"), "FontSize") => Some("14"));
    main_assert_eq!(config.get_in(Some("General"), "UseWhiteIngameChat") => Some("0"));
    main_assert_eq!(config.get_in(Some("General"), "UseWhiteLobbyChat") => Some("0"));
    main_assert_eq!(config.get_in(Some("General"), "Preloading") => Some("1"));
    let strength_string = strength.to_string();
    main_assert_eq!(config.get_in(Some("General"), "DefCrewStrength") => Some(strength_string.as_str()));
    main_assert_eq!(config.get_in(Some("General"), "VendorProgramKey") => Some("keep"));
}

#[test]
fn initial_network_game_join_fully_loads_the_client_lobby_within_500ms() {
    // C++ enters DoLobby only after network initialization, initial PlayerInfo
    // publication, and resource registration, then reaches and acknowledges
    // GS_Lobby with MainDlg alive (src/C4Game.cpp:361-409,3823-3844;
    // src/C4Network2.cpp:445-461,1574-1620,2017-2058;
    // src/C4Network2Players.cpp:38-49,78-136;
    // src/C4GameLobby.cpp:141-218,781-790).
    let _lock = env_lock().lock();
    let host_user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    fs::write(
        scenario
            .path
            .as_ref()
            .expect("minimal prepared host scenario has a path")
            .join("Scenario.txt"),
        "[Head]\nTitle=Fixture\nIcon=2\nMaxPlayer=2\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=GOOD=1\n",
    )
    .test_value();
    let (_host_guard, host_paths) =
        exact_loader_test_paths(host_user_data.path(), Some(content.path()));
    configure_test_startup_participant(&host_paths, host_user_data.path());
    persist_config_value(&host_paths, "Network", "PortUDP", "0").test_value();
    persist_config_value(&host_paths, "Network", "PortDiscovery", "0").test_value();
    persist_config_value(&host_paths, "Network", "EnableUPnP", "0").test_value();
    persist_config_value(&host_paths, "General", "Preloading", "0").test_value();
    let reference_port = std::net::TcpListener::bind("[::1]:0")
        .expect("reserve initial-join host reference port")
        .local_addr()
        .test_value()
        .port();
    persist_config_value(
        &host_paths,
        "Network",
        "PortRefServer",
        reference_port.to_string(),
    )
    .test_value();
    let mut host = new_menu_app_with_paths(640, 480, &host_paths);
    host.staged_network_host_scenario = Some(prepare_minimal_host_lobby(&host, scenario.clone()));

    let client_user_data = tempdir();
    let mut client = {
        let (_client_guard, client_paths) = exact_loader_test_paths(client_user_data.path(), None);
        persist_config_value(&client_paths, "Network", "LocalName", "Initial Client").test_value();
        persist_config_value(&client_paths, "Network", "Nick", "Initial Client").test_value();
        persist_config_value(&client_paths, "Network", "PortUDP", "0").test_value();
        persist_config_value(&client_paths, "Network", "PortDiscovery", "0").test_value();
        persist_config_value(&client_paths, "Network", "EnableUPnP", "0").test_value();
        persist_config_value(&client_paths, "Network", "MasterServerSignUp", "0").test_value();
        let client_tcp_port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("reserve initial-join client TCP port")
            .local_addr()
            .test_value()
            .port();
        persist_config_value(
            &client_paths,
            "Network",
            "PortTCP",
            client_tcp_port.to_string(),
        )
        .test_value();
        let client_player_path = client_user_data.path().join("Joining.c4p");
        let mut client_player = clonk_resources::MutableGroup::new("Joining.c4p");
        client_player
            .add_file_with_metadata(
                "Player.txt",
                b"[Player]\nName=Joining Player\n\n[Preferences]\nControl=0\nMouse=1\nAutoStopControl=0\nColorDw=65280\n"
                    .to_vec(),
                1,
                false,
            )
            .test_value();
        client_player
            .add_file_with_metadata(
                "BigIcon.png",
                encode_screenshot_png(1, 1, &[231, 72, 19, 255])
                    .expect("encode joining player icon"),
                1,
                false,
            )
            .test_value();
        fs::write(&client_player_path, client_player.pack().test_value()).test_value();
        persist_config_value(
            &client_paths,
            "General",
            "PlayerPath",
            client_user_data.path().to_string_lossy().into_owned(),
        )
        .test_value();
        persist_config_value(
            &client_paths,
            "General",
            "Participants",
            client_player_path.to_string_lossy().into_owned(),
        )
        .test_value();
        new_menu_app_with_paths(640, 480, &client_paths)
    };
    client.open_network_game_dialog();
    main_assert!(client.startup_game_search.is_some());
    let mut client_frame = vec![0x73; 640 * 480 * 4];

    host.activate_prepared_network_host(scenario.clone(), SocketAddr::from(([127, 0, 0, 1], 0)));
    let host_deadline = Instant::now() + Duration::from_secs(30);
    while host.startup_network_connection.is_some() {
        host.test_update();
        main_assert!(
            Instant::now() < host_deadline,
            "prepared host did not reach its lobby: {}",
            host.status_text
        );
        thread::yield_now();
    }
    main_assert!(host.classic_host_lobby_active(), "{}", host.status_text);
    let host_reference = some(&host.advertised_game_reference).summary().clone();
    main_assert!(host_reference.join_allowed);
    main_assert!(host_reference
        .addresses
        .iter()
        .any(|address| address.protocol == clonk_network::NetworkProtocol::Tcp));
    client.startup_game_references = vec![host_reference];
    client.sync_startup_network_game_rows();
    let selected_reference = client.startup_game_references[0].clone();
    client.focus_startup_game_reference(&selected_reference);
    main_assert_eq!(client.startup_network_dialog.test_ref().selected_game() => Some(0));

    let started = Instant::now();
    client
        .process_network_dialog_actions(vec![
            clonk_frontend::startup_netdlg::NetDlgAction::JoinGame { address: None },
        ])
        .test_value();
    let timeout = started + Duration::from_secs(10);
    let mut checkpoints = [None; 8];
    let mut lobby_rendered = false;
    loop {
        host.test_update();
        client.test_update();

        let local_client_id = client
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        let lobby_visible =
            client.joined_network_lobby_active() && client.message_dialogs.is_empty();
        let scenario_identity = client
            .pending_network_join_data
            .as_ref()
            .is_some_and(|join| {
                join.parameters.title.as_bytes() == b"Fixture"
                    && join.parameters.scenario.filename.as_bytes()
                        == scenario.identifier.as_bytes()
                    && client
                        .network_lobby
                        .as_ref()
                        .is_some_and(|lobby| lobby.scenario_label() == "Fixture")
            });
        let roster_propagated = local_client_id.is_some_and(|local_client_id| {
            let client_ids = |rows: &[LobbyRosterRow]| {
                rows.iter()
                    .filter_map(|row| match row {
                        LobbyRosterRow::Client(client) => Some(client.id),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            client.control_clients.contains(0)
                && client.control_clients.contains(local_client_id)
                && client.control_clients.is_activated(0)
                && client.control_clients.is_activated(local_client_id)
                && host.control_clients.contains(local_client_id)
                && host.control_clients.is_activated(local_client_id)
                && client.network_lobby.as_ref().is_some_and(|lobby| {
                    lobby.roster_rows_authoritative
                        && client_ids(lobby.controller.rows()) == [0, local_client_id]
                })
                && host.classic_host_lobby.as_ref().is_some_and(|lobby| {
                    client_ids(lobby.controller.rows()) == [0, local_client_id]
                })
        });
        let player_info_propagated = local_client_id.is_some_and(|local_client_id| {
            [0, local_client_id].into_iter().all(|client_id| {
                let player_resource_available = |app: &GameApp| {
                    let ids = app.control_player_infos.client_info_ids(client_id);
                    let expected_name = if client_id == 0 {
                        b"Exact Player".as_slice()
                    } else {
                        b"Joining Player".as_slice()
                    };
                    ids.len() == 1
                        && app.control_player_infos.get(ids[0]).is_some_and(|player| {
                            player.name.as_bytes() == expected_name
                                && player.resource.as_ref().is_some_and(|core| {
                                    app.admission_resources.complete_path(core.id).is_some()
                                })
                        })
                };
                player_resource_available(&client) && player_resource_available(&host)
            })
        });
        let resources_available = local_client_id.is_some_and(|local_client_id| {
            let every_loadable_resource_is_complete = |app: &GameApp| {
                !app.admission_resources.resource_cores.is_empty()
                    && app.admission_resources.resource_cores.values().all(|core| {
                        !core.loadable || app.admission_resources.complete_path(core.id).is_some()
                    })
            };
            let player_resources = [0, local_client_id]
                .into_iter()
                .filter_map(|client_id| {
                    let ids = client.control_player_infos.client_info_ids(client_id);
                    let [id] = ids.as_slice() else {
                        return None;
                    };
                    client
                        .control_player_infos
                        .get(*id)
                        .and_then(|player| player.resource.as_ref())
                        .map(|core| (client_id, core.id))
                })
                .collect::<Vec<_>>();
            client
                .pending_network_join_data
                .as_ref()
                .is_some_and(|join| {
                    std::iter::once(&join.parameters.scenario)
                        .chain(join.parameters.game_resources.iter())
                        .chain(std::iter::once(&join.dynamic))
                        .all(|core| {
                            !core.loadable
                                || client.admission_resources.complete_path(core.id).is_some()
                        })
                        && client.admission_resources.lobby_ready_available()
                })
                && every_loadable_resource_is_complete(&client)
                && every_loadable_resource_is_complete(&host)
                && player_resources.len() == 2
                && player_resources
                    .iter()
                    .all(|(client_id, resource_id)| resource_id >> 16 == *client_id)
                && player_resources[0].1 != player_resources[1].1
                && client.network_lobby.as_ref().is_some_and(|lobby| {
                    !lobby.resource_rows.is_empty()
                        && lobby
                            .resource_rows
                            .values()
                            .all(|row| row.present_percent == 100)
                        && player_resources.iter().all(|(_, resource_id)| {
                            lobby
                                .resource_rows
                                .get(resource_id)
                                .is_some_and(|row| row.present_percent == 100)
                        })
                })
                && host.classic_host_lobby.as_ref().is_some_and(|lobby| {
                    !lobby.resource_rows.is_empty()
                        && lobby
                            .resource_rows
                            .values()
                            .all(|row| row.present_percent == 100)
                        && player_resources.iter().all(|(_, resource_id)| {
                            lobby
                                .resource_rows
                                .get(resource_id)
                                .is_some_and(|row| row.present_percent == 100)
                        })
                })
        });
        let status_acknowledged =
            client.pending_network_join_data.is_some() && !client.initial_lobby_status_ack_pending;
        let startup_connection_finished =
            client.startup_network_connection.is_none() && client.network.is_some();
        let state_ready = [
            lobby_visible,
            scenario_identity,
            roster_propagated,
            player_info_propagated,
            resources_available,
            status_acknowledged,
            startup_connection_finished,
        ];
        if state_ready.into_iter().all(|ready| ready) && !lobby_rendered {
            lobby_rendered = client.test_render(&mut client_frame);
        }
        let ready = [
            lobby_visible,
            scenario_identity,
            roster_propagated,
            player_info_propagated,
            resources_available,
            status_acknowledged,
            startup_connection_finished,
            lobby_rendered,
        ];
        let elapsed = started.elapsed();
        for (checkpoint, ready) in checkpoints.iter_mut().zip(ready) {
            if ready && checkpoint.is_none() {
                *checkpoint = Some(elapsed);
            }
        }
        if ready.into_iter().all(|ready| ready) {
            break;
        }
        main_assert!(
            Instant::now() < timeout,
            "initial network game join did not fully load the lobby after {elapsed:?}; checkpoints [lobby, scenario, roster, PlayerInfo, resources, status ack, startup connection, render] = {checkpoints:?}; host status = {:?}; client status = {:?}",
            host.status_text,
            client.status_text,
        );
        thread::yield_now();
    }

    let elapsed = started.elapsed();
    eprintln!("initial full-lobby network game join completed in {elapsed:?}");
    main_assert!(
        elapsed <= Duration::from_millis(500),
        "initial network game join took {elapsed:?}, exceeding the inclusive 500ms lobby budget; checkpoints [lobby, scenario, roster, PlayerInfo, resources, status ack, startup connection, render] = {checkpoints:?}"
    );
}

#[test]
fn selected_network_scenario_installs_prepared_host_before_admission() {
    // OpenScenario and InitHost finish before Players.Init authors the
    // empty Initial PlayerInfo; AllowJoin follows that direct local
    // execution (src/C4Game.cpp:421-438,3847-3876;
    // src/C4Network2Players.cpp:38-49,78-123,160-239).
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content.path()));
    persist_config_value(&paths, "Network", "PortUDP", "0").test_value();
    persist_config_value(&paths, "Network", "PortDiscovery", "0").test_value();
    persist_config_value(&paths, "Network", "EnableUPnP", "0").test_value();
    // The enabled async path has its own preload-reuse regression; this test
    // isolates host preparation and admission ordering.
    persist_config_value(&paths, "General", "Preloading", "0").test_value();
    let reference_port = std::net::TcpListener::bind("[::1]:0")
        .expect("reserve selected-host reference port")
        .local_addr()
        .test_value()
        .port();
    persist_config_value(
        &paths,
        "Network",
        "PortRefServer",
        reference_port.to_string(),
    )
    .test_value();
    let mut app = new_menu_app_with_paths(1280, 720, &paths);
    let staged = prepare_minimal_host_lobby(&app, scenario.clone());
    app.staged_network_host_scenario = Some(staged);

    app.activate_prepared_network_host(scenario.clone(), SocketAddr::from(([127, 0, 0, 1], 0)));
    main_assert!(app.network.is_none(), "preparation must precede bind");
    main_assert!(app.startup_network_connection.is_some());
    // OpenScenario publishes 4 before InitNetworkHost begins, so the
    // loader installed around host preparation must retain that value
    // (src/C4Game.cpp:124-270,421-440).
    main_assert_eq!(some(&app.loader_screen).state().progress() => 4);

    for _ in 0..3_000 {
        app.poll_startup_network_connection().test_value();
        if app.network.is_some() && app.network_control_clock.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    main_assert!(app.network.is_some(), "{}", app.status_text);
    let NetworkMode::Host(settings) = some(&app.network_mode) else {
        panic!("prepared network selection must install a host");
    };
    let prepared = settings.prepared.test_ref();
    main_assert!(!prepared.host_config().allow_join);
    main_assert!(prepared.host_config().initial_join_snapshot.is_some());
    main_assert!(app.control_player_infos.contains_client(0));
    main_assert_eq!(app.control_player_infos.player_count() => 0);
    main_assert_eq!(prepared.scenario_wire_name().as_bytes() => scenario.identifier.as_bytes(), "the prepared host retains the selected scenario's wire identity");
    main_assert!(
        app.network_lobby.is_none(),
        "a staged host uses the exact C++ lobby instead of the generic projection"
    );
    main_assert!(app.classic_host_lobby.is_some());
    let local_addresses = some(&app.network).local_addresses();
    main_assert!(matches!(local_addresses.len(), 1 | 2));
    let tcp = local_addresses.first().test_value();
    main_assert_eq!(tcp.protocol => clonk_network::NetworkProtocol::Tcp);
    main_assert_ne!(tcp.endpoint.port() => 0);
    if let Some(udp) = local_addresses.get(1) {
        // C4Network2IO starts TCP and UDP independently and publishes only
        // live transports. A parallel test or process may own PortUDP;
        // that is a valid TCP-only host, not an admission failure.
        main_assert_eq!(udp.protocol => clonk_network::NetworkProtocol::Udp);
        main_assert_eq!(udp.endpoint.ip() => tcp.endpoint.ip());
        main_assert_eq!(udp.endpoint.port() => 11_113);
    }
    let advertised = some(&app.advertised_game_reference);
    main_assert!(app.network_game_advertiser.is_some());
    main_assert!(advertised.summary().join_allowed);
    main_assert_eq!(advertised.metadata().icon => 2);
    main_assert_eq!(advertised.metadata().addresses => local_addresses);
    main_assert_eq!(advertised.parameters() => &prepared.host_config().initial_join_snapshot.as_ref().expect("prepared JoinData").parameters);
    let prepared_parameters = &prepared
        .host_config()
        .initial_join_snapshot
        .test_ref()
        .parameters;
    let prepared_random_seed = u64::from(prepared_parameters.random_seed as u32);
    let prepared_fair_crew = (
        prepared_parameters.use_fair_crew,
        prepared_parameters.fair_crew_strength,
    );
    main_assert_eq!(
        app.network_control_clock =>
        Some(NetworkControlClock::new(
            i32::try_from(prepared.host_config().start_tick).expect("start tick fits i32"),
            prepared_parameters.control_rate,
        ))
    );

    let expected_go = lobby_fixture!(status:
        clonk_network::NETWORK_STATE_GO,
        prepared.host_config().initial_status.control_mode,
        0,
    );
    let (manager, events, mut commands) = NetworkManager::test_stub_with_commands();
    // The live manager owns the temporary published definition packs.
    // Keep it alive while the command stub observes the countdown; C++
    // likewise retains its resource list through game activation.
    let _prepared_resource_owner = app.network.replace(manager).test_value();
    install_test_classic_host_lobby(&mut app);
    request_classic_lobby_start(&mut app, DEFAULT_LOBBY_COUNTDOWN_SECONDS);
    main_assert!(app.select_classic_lobby_sheet(LobbySheet::Options));
    process_classic_lobby_action(
        &mut app,
        ClassicLobbyAction::OptionSelectionRequested {
            option: LobbyOptionKind::ControlRate,
            anchor: GuiPoint::new(400.0, 240.0),
            minimum_width: 120,
        },
    );
    main_assert!(app.context_menu.is_some());
    main_assert_eq!(app.context_menu_lobby_option => Some(LobbyOptionKind::ControlRate));
    let go_observer = thread::spawn(move || {
        let observed = commands.complete_lobby_start(Ok(()));
        (commands, observed)
    });
    for _ in 0..DEFAULT_LOBBY_COUNTDOWN_SECONDS {
        main_assert!(
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
    let (mut commands, observed_start) = go_observer.test_join();
    main_assert_eq!(
        observed_start =>
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
    main_assert!(
        app.host_lobby_countdown.is_none(),
        "natural zero releases C4Network2::pLobbyCountdown ownership before GO"
    );
    app.sec1_timer().test_value();
    main_assert!(
        commands.take_lobby_start_commands().is_empty(),
        "later second pulses cannot repeat zero or GO"
    );
    main_assert!(matches!(app.mode, AppMode::Loading));
    main_assert!(app.loading_state.is_some());
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.context_menu_lobby_option => None);
    // Init returns from InitNetworkHost/DoLobby at 7 before beginning
    // InitGame's script and definition phases
    // (src/C4Game.cpp:438-457,3872-3913).
    main_assert_eq!(some(&app.loading_state).last_progress => 7);
    main_assert_eq!(some(&app.loader_screen).state().progress() => 7);
    main_assert!(app
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| !wait.visible));
    // C4GameParameters chooses the host seed before InitNetworkHost and
    // the same Parameters.RandomSeed is serialized to every client. The
    // retained host scenario must therefore enter InitGame with that
    // exact bit pattern (pristine 9ffa0a5d
    // src/C4GameParameters.cpp:418-432,555;
    // src/C4Game.cpp:2617-2627).
    main_assert_eq!(
        app.loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|loading| loading.random_seed) =>
        Some(prepared_random_seed),
        "prepared host must retain Parameters.RandomSeed for scenario activation"
    );
    main_assert_eq!(
        app.loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|loading| (loading.use_fair_crew, loading.fair_crew_strength)) =>
        Some(prepared_fair_crew),
        "prepared host must retain synchronized fair-crew parameters"
    );

    // FinalInit reports the host's local arrival, but OnStatusAck is what
    // starts network control after every waited-for client has reached Go
    // (src/C4Network2.cpp:2017-2077,2091-2110). The initialized game must
    // therefore remain behind the loading screen until that exact commit.
    let loading_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll_loading().test_value();
        if app
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .is_some_and(|pending| pending.local_reached)
        {
            break;
        }
        main_assert!(
            Instant::now() < loading_deadline,
            "prepared host InitGame worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    main_assert_eq!(some(&app.loader_screen).state().progress() => 97);
    main_assert!(app.loading_state.as_ref().is_some_and(|loading| loading
        .log
        .iter()
        .any(|line| line == "Definition selection resolved")));
    main_assert_eq!(
        app.engine.random_seed() =>
        prepared_random_seed,
        "the network host seed remains authoritative over offline defaults (status: {:?}, mode: {:?}, loading: {})",
        app.status_text,
        app.mode,
        app.loading_state.is_some(),
    );
    main_assert_eq!((app.engine.use_fair_crew(), app.engine.fair_crew_strength(),) => prepared_fair_crew,);
    main_assert_eq!(commands.take_status_reached() => 1);
    main_assert!(matches!(app.mode, AppMode::Loading));
    main_assert!(app.loading_state.is_some());
    main_assert!(app
        .network_start_wait
        .as_ref()
        .is_some_and(|wait| wait.visible));
    main_assert!(
        app.message_dialogs.is_empty(),
        "the host uses its roster wait instead of the client message dialog"
    );
    send_network_event(&events, NetworkEvent::StatusRequested(expected_go));
    app.test_network_events();
    main_assert_eq!(app.loading_state.as_ref().and_then(|loading| loading.prepared_go.as_ref()).map(|pending| pending.local_reached) => Some(true));
    main_assert_eq!(commands.take_status_reached() => 0, "an identical host status echo must not report local reach twice");
    main_assert!(app.engine.snapshot().players.is_empty(), "network InitPlayers must not directly join the local player before host-issued JoinPlr controls");

    send_network_event(&events, NetworkEvent::StatusCommitted(expected_go));
    app.test_network_events();
    main_assert!(matches!(app.mode, AppMode::Running));
    main_assert!(app.loading_state.is_none());
    main_assert!(app.network_start_wait.is_none());
    main_assert!(
        app.network_game_advertiser.is_some(),
        "native keeps the reference listener alive during play"
    );
    let running_reference = some(&app.advertised_game_reference);
    main_assert_eq!(running_reference.summary().state => "Running");
    main_assert!(!running_reference.summary().join_allowed);
    app.control_player_infos.apply(lobby_fixture!(player_data {
        client_id: 0,
        flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
        players: vec![lobby_fixture!(player {
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
        })],
    }));
    main_assert!(app.control_player_infos.mark_joined(41, 3, 77));
    main_assert!(app
        .control_clients
        .apply_join(&clonk_engine::ClientJoinControlData {
            core: lobby_fixture!(client {
                client_id: 7,
                activated: true,
                name: clonk_engine::LegacyCString::from_bytes(b"Remote".to_vec()).unwrap(),
                nick: clonk_engine::LegacyCString::from_bytes(b"R".to_vec()).unwrap(),
                lobby_ready: true,
            }),
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
        some(&app.advertised_game_reference),
        some(&app.host_join_snapshot).parameters.clone(),
        &app.control_clients,
        &app.control_player_infos,
        app.engine.teams(),
        app.engine.max_players().expect("live maximum is set"),
        app.engine.startup_player_count(),
        &app.snapshot,
    )
    .test_value();
    main_assert_eq!(final_reference.summary().state => "Running");
    main_assert!(!final_reference.summary().join_allowed);
    main_assert_eq!(final_reference.metadata().time => 123);
    main_assert_eq!(final_reference.metadata().league_performance => -7);
    main_assert_eq!(final_reference.parameters().max_players => 13);
    main_assert!(final_reference
        .parameters()
        .clients
        .clients
        .iter()
        .any(|client| {
            client.client_id == 7 && client.name.as_bytes() == b"Remote" && client.lobby_ready
        }));
    main_assert_eq!(final_reference.parameters().teams.teams[0].player_ids => vec![41]);
    let player_packet = &final_reference.parameters().player_infos.clients[0];
    main_assert_eq!(player_packet.flags => clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL);
    let player = &player_packet.players[0];
    main_assert_eq!(player.league_performance => 19);
    main_assert_eq!((player.game_number, player.game_join_frame) => (3, 77));
    main_assert_ne!(player.flags & clonk_engine::PLAYER_INFO_FLAG_WON => 0);
    main_assert_eq!(player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE => 0);
    main_assert!(player.resource.is_none());
    app.advertised_game_reference = Some(final_reference);
    app.snapshot.game_over = true;
    main_assert!(app.control_clients.set_lobby_ready(7, false));
    app.publish_updated_host_join_snapshot();
    let republished_player = &some(&app.advertised_game_reference)
        .parameters()
        .player_infos
        .clients[0]
        .players[0];
    main_assert_eq!(republished_player.league_performance => 19);
    main_assert_ne!(republished_player.flags & clonk_engine::PLAYER_INFO_FLAG_WON => 0);
    let live_player = app.control_player_infos.get(41).test_value();
    main_assert_ne!(live_player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE => 0);
    main_assert!(live_player.resource.is_some());
}

#[test]
fn only_a_host_announces_its_compatibility_profile_in_the_lobby() {
    // The lobby line states a property of the *session*, and only the host's
    // setting decides that: `session_control_mode` resolves the host's
    // `initial_status.control_mode` (`game_app/network.rs:5612,7190`) and every
    // client adopts the received `status.control_mode`
    // (`game_app/network.rs:2208`). A joining client's own profile therefore
    // changes nothing about the session it is joining, so announcing it there
    // asserts a promise that session never made.
    // The wording depends on whether the contract can back the profile today
    // (clonk-org/clonk-rs#588): a claimable one states itself, a blocked one
    // reports why it is not being claimed. Either way only a host says it.
    let compat_line = |app: &GameApp| {
        app.network_lobby
            .as_ref()
            .expect("the lobby exists")
            .logs
            .iter()
            .any(|line| line.text.starts_with("Compatibility profile"))
    };

    let (mut client, _client_events) = networked_client_lobby(
        new_menu_app(640, 480),
        "Client",
        NetworkLobbyState::new(7, "Client".to_string(), false),
    );
    client.config.compat_profile = crate::settings::CompatProfile::LegacyClonk;
    client.open_network_lobby();
    main_assert!(
        !compat_line(&client),
        "a joining client must not announce its own profile as the session's"
    );

    let (mut host, _host_events, _host_commands) = networked_host_lobby_with_commands(
        new_menu_app(640, 480),
        NetworkLobbyState::new(0, "Host".to_string(), true),
    );
    host.config.compat_profile = crate::settings::CompatProfile::LegacyClonk;
    host.open_network_lobby();
    main_assert!(
        compat_line(&host),
        "the host decides the session profile, so it still states it"
    );
}

#[test]
fn a_joining_client_is_told_its_own_requested_profile_is_unavailable() {
    // clonk-org/clonk-rs#588 wants the contract explained before hosting *or*
    // joining, and forbids a silent downgrade of the requested profile. A
    // client still must not announce a profile as the session's -- the host
    // decides that, which
    // `only_a_host_announces_its_compatibility_profile_in_the_lobby` pins --
    // so this line is explicitly about the client's own request instead, and
    // appears only when the contract cannot back it.
    let compat_line = |app: &GameApp, needle: &str| {
        app.network_lobby
            .as_ref()
            .expect("the lobby exists")
            .logs
            .iter()
            .any(|line| line.text.contains(needle))
    };

    let (mut client, _client_events) = networked_client_lobby(
        new_menu_app(640, 480),
        "Client",
        NetworkLobbyState::new(7, "Client".to_string(), false),
    );
    client.config.compat_profile = crate::settings::CompatProfile::LegacyClonk;
    client.open_network_lobby();

    let blocked = !crate::compat_readiness::is_ready();
    main_assert_eq!(
        compat_line(&client, "cannot honour it") => blocked,
        "a client that asked for a profile it cannot have must be told"
    );
    // Still never phrased as a claim about the session the host owns.
    main_assert!(
        !client
            .network_lobby
            .as_ref()
            .test_value()
            .logs
            .iter()
            .any(|line| line.text.starts_with("Compatibility profile")),
        "the session's profile is the host's statement, not the client's"
    );
    // The request itself is never rewritten: a player who asked still sees it.
    main_assert_eq!(client.config.compat_profile => crate::settings::CompatProfile::LegacyClonk);

    // A normal-profile client says nothing at all.
    let (mut ordinary, _ordinary_events) = networked_client_lobby(
        new_menu_app(640, 480),
        "Ordinary",
        NetworkLobbyState::new(8, "Ordinary".to_string(), false),
    );
    ordinary.open_network_lobby();
    main_assert!(
        !compat_line(&ordinary, "cannot honour it"),
        "normal Rust-only play is unaffected"
    );
}

#[test]
fn a_blocked_compatibility_profile_is_reported_and_not_claimed_to_peers() {
    // clonk-org/clonk-rs#588: readiness is computed from the contract itself,
    // and a profile the contract cannot back must be refused *before* anyone
    // joins. The requested profile is never rewritten — a player who asked for
    // it still sees that they asked — but what the session may claim to a peer
    // is the honest answer, and the refusal is visible in the lobby.
    let (mut host, _host_events, _host_commands) = networked_host_lobby_with_commands(
        new_menu_app(640, 480),
        NetworkLobbyState::new(0, "Host".to_string(), true),
    );
    host.config.compat_profile = crate::settings::CompatProfile::LegacyClonk;
    host.open_network_lobby();

    let logs = &host.network_lobby.as_ref().test_value().logs;
    let reported = logs
        .iter()
        .any(|line| line.text.contains("NOT claimed") && line.text.contains("LegacyClonk"));
    let blocked = !crate::compat_readiness::is_ready();
    main_assert_eq!(
        reported => blocked,
        "a blocked profile must say so in the lobby, and a claimable one must not"
    );
    main_assert_eq!(
        host.config.compat_profile => crate::settings::CompatProfile::LegacyClonk,
        "the requested profile is never rewritten"
    );
    main_assert_eq!(
        host.claimed_compat_profile() => if blocked {
            crate::settings::CompatProfile::Normal
        } else {
            crate::settings::CompatProfile::LegacyClonk
        },
        "only a contract-backed profile may be claimed to a peer"
    );

    // Normal-profile play is untouched: no compatibility line at all.
    let (mut ordinary, _events, _commands) = networked_host_lobby_with_commands(
        new_menu_app(640, 480),
        NetworkLobbyState::new(0, "Host".to_string(), true),
    );
    ordinary.open_network_lobby();
    main_assert!(
        !ordinary
            .network_lobby
            .as_ref()
            .test_value()
            .logs
            .iter()
            .any(|line| line.text.starts_with("Compatibility profile")),
        "an ordinary session promises nothing and says nothing"
    );
    main_assert_eq!(
        ordinary.claimed_compat_profile() => crate::settings::CompatProfile::Normal
    );
}

#[test]
fn network_lobby_does_not_displace_join_or_host_startup_dialog() {
    let mut joined = new_menu_app(640, 480);
    joined.open_network_game_dialog();
    joined.open_network_lobby();
    joined
        .start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    confirm_abort_dialog(&mut joined);
    main_assert!(matches!(joined.mode, AppMode::Menu));
    main_assert_eq!(joined.startup.view => StartupView::NetworkGame);
    main_assert_eq!(joined.last_startup_dialog => StartupDialog::NetworkGame);

    let mut hosted = running_browser_sandbox(ScenarioSelectorMode::NetworkHost);
    hosted.open_network_lobby();
    confirm_abort_dialog(&mut hosted);
    assert_l038_browser_return(&hosted, ScenarioSelectorMode::NetworkHost);
    hosted.scensel_do_back().test_value();
    main_assert_eq!(hosted.startup.view => StartupView::MainMenu);
    main_assert_eq!(hosted.last_startup_dialog => StartupDialog::MainMenu);

    let mut reused = new_menu_app(640, 480);
    reused.open_network_game_dialog();
    reused.open_network_host_scenario_browser();
    reused.scensel_do_back().test_value();
    main_assert_eq!(reused.startup.view => StartupView::NetworkGame);
    main_assert_eq!(reused.last_startup_dialog => StartupDialog::ScenarioBrowser(ScenarioSelectorMode::NetworkHost));
    reused.open_network_lobby();
    reused
        .start_sandbox_scenario(FrontendScenario::fallback())
        .test_value();
    confirm_abort_dialog(&mut reused);
    assert_l038_browser_return(&reused, ScenarioSelectorMode::NetworkHost);
}

#[test]
fn finishing_recording_deletes_stale_final_infos_for_an_empty_roster() {
    let directory = tempdir();
    let output_path = directory.path().join("001-Empty.c4s");
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos = ControlPlayerInfoRegistry::default();
    install_test_recording_template(&mut app, output_path.clone());
    some_mut(&mut app.recording_template)
        .group
        .add_file("RecPlayerInfos.txt", b"stale final roster".to_vec())
        .test_value();

    main_assert!(app.start_recording(true).expect("start empty recording"));
    main_assert!(app.finish_recording().is_none());

    let group = Group::open(&output_path).test_value();
    main_assert!(!group.exists("RecPlayerInfos.txt"));
}

#[test]
fn runtime_client_drains_ready_target_before_retargeted_ack() {
    // Native's reach predicate also requires !CtrlReady(ControlTick). If
    // target control is complete already, execute it and acknowledge the
    // later actual tick at the next cadence boundary.
    let mut app = new_running_sandbox_app();
    let (events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let pause = lobby_fixture!(status: clonk_network::NETWORK_STATE_PAUSE, 1, 2);
    send_network_event(&events, NetworkEvent::StatusRequested(pause));
    for tick in 0..=2 {
        send_ready_tick(&events, tick, Vec::new());
    }
    for _ in 0..7 {
        app.test_update();
    }

    let reached = clonk_network::NetworkStatus {
        target_tick: 3,
        ..pause
    };
    main_assert_eq!((app.engine.frame(), app.expected_network_control_tick()) => (6, 3));
    main_assert!(!app.network_control_running);
    main_assert_eq!(commands.take_framed_status_acknowledgements() => vec![(reached, 6)]);
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
    let core = lobby_fixture!(player_resource:
        resource_id,
        clonk_engine::LegacyCString::from_bytes(b"Pending.c4p".to_vec()).test_value(),
    );
    send_ready_tick(
        &events,
        0,
        vec![NetworkControl::JoinPlayer(
            clonk_engine::JoinPlayerControlData {
                at_client: 0,
                info_id: 9,
                source: clonk_engine::JoinPlayerSource::Resource(core),
                by_client: 0,
                ..Default::default()
            },
        )],
    );
    let pause = lobby_fixture!(status: clonk_network::NETWORK_STATE_PAUSE, 1, 0);
    send_network_event(&events, NetworkEvent::StatusRequested(pause));

    app.test_network_events();

    main_assert!(!app.network_control_running);
    main_assert!(app.network_ticks.ready.contains_key(&0));
    main_assert_eq!(app.admission_resources.status(resource_id) => Some(&AdmissionResourceState::Loading { removed: false }));
    main_assert_eq!(commands.take_runtime_status_commands() => vec![network::TestRuntimeStatusCommand::Reached {status: pause, actual_control_tick: 0,}]);
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
    app.control_player_infos.apply(lobby_fixture!(player_data:
        7,
        vec![lobby_fixture!(player {
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            team: 4,
            color: 0x0000_0000,
            original_color: 0x0065_4321,
        })],
    ));

    main_assert!(
        app.execute_message_control(message_control(MESSAGE_TYPE_NORMAL, -1, -1, b"hello", 7,))
            .displayed
    );
    let line = &app_classic_lobby(&app).controller.logs()[0];
    main_assert_eq!(line.text => "<Remote> hello");
    main_assert_eq!(line.color => [0x65, 0x65, 0x65, 0xff]);

    main_assert!(app.engine.set_team_distribution(4));
    app.engine.set_team_colors(true);
    app.execute_message_control(message_control(
        MESSAGE_TYPE_NORMAL,
        -1,
        -1,
        b"hidden team color",
        7,
    ));
    let line = &app_classic_lobby(&app).controller.logs()[1];
    main_assert_eq!(line.color => [0x65, 0x65, 0x65, 0xff]);

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
    let line = &app_classic_lobby(&app).controller.logs()[2];
    main_assert_eq!(line.color => [0x82, 0x60, 0x3e, 0xff]);

    app.show_log_timestamps = true;
    app.execute_message_control(message_control(MESSAGE_TYPE_SYSTEM, -1, -1, b"notice", 0));
    let line = &app_classic_lobby(&app).controller.logs()[3];
    main_assert!(line.text.starts_with("<c 909090>["));
    main_assert!(line.text.ends_with("</c> Network: notice"));
    main_assert_eq!(line.color => [0xaf, 0xaf, 0xaf, 0xff]);

    app.show_log_timestamps = false;
    app.white_lobby_chat = true;
    app.execute_message_control(message_control(
        MESSAGE_TYPE_NORMAL,
        -1,
        -1,
        b"white body",
        7,
    ));
    let line = app_classic_lobby(&app)
        .controller
        .logs()
        .last()
        .test_value();
    main_assert_eq!(line.text => "<Remote> <c ffffff>white body");
    main_assert_eq!(line.color => [0x82, 0x60, 0x3e, 0xff]);
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
    app.control_player_infos.apply(lobby_fixture!(player_data:
        7,
        vec![lobby_fixture!(player {
            id: 41,
            player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            color: 0,
            original_color: 0,
        })],
    ));

    app.sync_classic_lobby_roster();

    let rows = app_classic_lobby(&app).controller.rows();
    main_assert!(rows.iter().any(|row| matches!(row, LobbyRosterRow::Client(client) if client.id == 7 && client.color == [0x65, 0x65, 0x65, 0xff])));
    main_assert!(rows.iter().any(|row| matches!(row, LobbyRosterRow::Player(player) if player.id == 41 && player.color == [0x65, 0x65, 0x65, 0xff])));
}

#[test]
fn direct_lobby_player_info_survives_network_game_initialization() {
    // Network PlayerInfo is applied directly in the lobby and the same
    // registry is reused when the game starts; the later synchronized
    // JoinPlayer resolves its InfoID there (src/C4Network2Players.cpp:245-269;
    // src/C4Game.cpp:2392-2423).
    let mut app = new_state_only_menu_app(320, 200);
    let event_tx = install_network_stub(&mut app);
    let info_id = 41;
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            lobby_fixture!(player_data {
                client_id: 3,
                players: vec![lobby_fixture!(player { id: info_id })],
                by_client: 0,
            }),
        )))
        .test_value();

    app.test_network_events();
    main_assert!(app.control_player_infos.get(info_id).is_some());

    app.configure_running_state("Network".to_string(), DEFAULT_GROUND_HEIGHT);

    main_assert!(
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
    let event_tx = install_client_network_stub(&mut app, 7);
    let mut lobby = NetworkLobbyState::new(7, "stale local".to_string(), false);
    lobby.register_peer(99, "stale peer".to_string(), ParticipantKind::Player);
    app.network_lobby = Some(lobby);

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.title =
        clonk_engine::LegacyCString::from_bytes(b"Caf\xe9 Arena".to_vec()).test_value();
    snapshot.parameters.clients = clonk_network::JoinClientRegistrySnapshot {
        clients: vec![
            lobby_fixture!(client {
                client_id: 0,
                activated: true,
                name: clonk_engine::LegacyCString::from_bytes(b"Exact Andr\xe9".to_vec())
                    .expect("valid host name"),
                lobby_ready: true,
            }),
            lobby_fixture!(client {
                client_id: 7,
                name: clonk_engine::LegacyCString::from_bytes(b"Exact local".to_vec())
                    .expect("valid local name"),
            }),
            lobby_fixture!(client {
                client_id: 9,
                observer: true,
                name: clonk_engine::LegacyCString::from_bytes(b"Exact observer".to_vec())
                    .expect("valid observer name"),
                lobby_ready: false,
            }),
        ],
        local_client_id: Some(7),
    };
    let join_data = lobby_fixture!(join_data: 7, snapshot.dynamic_tick, host_config.initial_status, snapshot.dynamic, snapshot.parameters);
    send_network_event(&event_tx, NetworkEvent::JoinData(join_data));

    app.test_network_events();

    let participants = &app_lobby(&app).participants;
    main_assert_eq!(participants.keys().copied().collect::<Vec<_>>() => [0, 7, 9]);
    main_assert_eq!(participants[&0].name => "Exact Andr\u{e9}");
    main_assert!(participants[&0].ready);
    main_assert_eq!(participants[&7].name => "Exact local");
    main_assert!(!participants[&7].ready);
    main_assert_eq!(participants[&9].name => "Exact observer");
    main_assert_eq!(participants[&9].kind => ParticipantKind::Observer);
    main_assert!(!participants[&9].ready);
    main_assert_eq!(app_lobby(&app).scenario_label() => "Caf\u{e9} Arena");
    main_assert_eq!(app.scenario_label => "Caf\u{e9} Arena");
    main_assert_eq!(app.control_clients.state(0).expect("raw host core remains registered").name.as_bytes() => b"Exact Andr\xe9");
    main_assert_eq!(some(&app.pending_network_join_data).parameters.title.as_bytes() => b"Caf\xe9 Arena");
}

#[test]
fn lobby_ready_gate_waits_for_registered_non_player_resource() {
    // MainDlg::UpdateResourceProgress keeps Ready disabled while any
    // registered non-player C4Network2Res is incomplete
    // (src/C4GameLobby.cpp:779-802).
    let mut resources = AdmissionResourceStore::default();
    let scenario = lobby_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 41,
        loadable: true,
    });

    resources.register_lobby_resource(&scenario);
    main_assert!(!resources.lobby_ready_available());

    resources.mark_complete(scenario.id, PathBuf::from("Scenario.c4s"));
    main_assert!(resources.lobby_ready_available());
}

#[test]
fn lobby_ready_gate_ignores_incomplete_player_resource() {
    // MainDlg::UpdateResourceProgress explicitly excludes NRT_Player from
    // the resource-completeness gate (src/C4GameLobby.cpp:781-790).
    let mut resources = AdmissionResourceStore::default();
    resources.register_lobby_resource(&lobby_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Player as u8,
        id: 42,
        loadable: true,
    }));

    main_assert!(resources.lobby_ready_available());
}

#[test]
fn lobby_preload_gate_uses_one_shared_eligibility_edge_and_success_is_one_shot() {
    let mut automatic = LobbyPreloadState::new(true);
    main_assert!(!automatic.synchronize(false, true));
    main_assert!(!automatic.synchronize(true, false));
    main_assert!(automatic.synchronize(true, true));
    main_assert!(!automatic.synchronize(true, true));
    automatic.record_result(false);
    main_assert!(
        automatic.eligible,
        "failure remains eligible but does not spin"
    );
    main_assert!(!automatic.synchronize(true, true));
    main_assert!(!automatic.synchronize(false, true));
    main_assert!(automatic.synchronize(true, true));

    let mut manual = LobbyPreloadState::new(false);
    main_assert!(!manual.synchronize(true, true));
    main_assert!(manual.manual_button_present);
    main_assert!(manual.eligible);
    manual.record_result(true);
    main_assert!(manual.spent);
    main_assert!(!manual.eligible);
    main_assert!(!manual.manual_button_present);
    main_assert!(!manual.synchronize(true, true));
    manual.reset_for_context();
    main_assert!(!manual.spent);
    main_assert!(manual.manual_button_present);
    main_assert!(!manual.synchronize(true, true));
    main_assert!(manual.eligible);
}

#[test]
fn queued_client_lobby_preload_cleanup_removes_uncommitted_staging_file() {
    let directory = tempdir();
    let staging_path = directory.path().join(".Combined7.c4s.preload.tmp");
    fs::write(&staging_path, b"staged scenario").test_value();
    let artifact = ClientLobbyPreloadArtifact {
        client_id: 7,
        dynamic_resource_id: 23,
        random_seed: 41,
        scenario: None,
        material_groups: Vec::new(),
        staging_path: Some(staging_path.clone()),
    };
    let (sender, receiver) = mpsc::channel();
    main_assert!(sender.send(artifact).is_ok(), "queue completed preload");

    drop(receiver);

    main_assert!(
        !staging_path.exists(),
        "dropping an unread completed result must retire its staging file"
    );
}

#[test]
fn clearing_client_lobby_preload_removes_only_its_committed_combined_file() {
    let directory = tempdir();
    let owned_path = directory.path().join("Combined7.c4s");
    fs::write(&owned_path, b"preload-owned scenario").test_value();
    let mut app = new_state_only_menu_app(320, 200);
    app.client_combined_scenario_path = Some(owned_path.clone());
    app.client_combined_preload_file.replace(owned_path.clone());
    app.network_material_resource_groups = Some(Vec::new());

    app.clear_lobby_preload();

    main_assert!(!owned_path.exists());
    main_assert!(app.client_combined_scenario_path.is_none());
    main_assert!(!app.client_combined_preload_file.is_owned());
    main_assert!(app.network_material_resource_groups.is_none());

    let existing_path = directory.path().join("Combined8.c4s");
    fs::write(&existing_path, b"pre-existing scenario").test_value();
    app.client_combined_scenario_path = Some(existing_path.clone());
    app.clear_lobby_preload();

    main_assert!(
        existing_path.exists(),
        "clearing preload state must not remove a pack it did not create"
    );
    main_assert!(app.client_combined_scenario_path.is_none());

    let dropped_path = directory.path().join("Combined9.c4s");
    fs::write(&dropped_path, b"drop-owned scenario").test_value();
    {
        let mut dropped_app = new_state_only_menu_app(320, 200);
        dropped_app.client_combined_scenario_path = Some(dropped_path.clone());
        dropped_app
            .client_combined_preload_file
            .replace(dropped_path.clone());
    }
    main_assert!(
        !dropped_path.exists(),
        "dropping the app must retire its preload-owned combined pack"
    );
}

#[test]
fn configured_automatic_lobby_preload_runs_off_thread_and_activation_reuses_it() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "Preloading", "1").test_value();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    let mut staged = prepare_tutorial_host_lobby(&app, repository);
    app.loader_screen = staged.loader_screen.take();
    app.staged_network_host_scenario = Some(staged);
    let (manager, _events) = NetworkManager::test_stub();
    let mode = NetworkMode::Host(lobby_fixture!(host: 11_112, "Exact Host".to_string(), None));
    let (lobby, options) = app.build_classic_host_lobby(&mode, &manager).test_value();
    main_assert!(lobby.preload.automatic);
    main_assert!(!lobby.preload.manual_button_present);
    app.classic_host_lobby = Some(lobby);
    app.scenario_game_options = options;
    app.network_mode = Some(mode);
    app.network = Some(manager);

    app.sync_classic_lobby_resource_ready();
    let preload = app
        .classic_host_lobby
        .as_ref()
        .map(|lobby| lobby.preload)
        .test_value();
    main_assert!(preload.spent, "successful worker launch is one-shot");
    main_assert!(app.lobby_preload_task.is_some());

    let deadline = Instant::now() + Duration::from_secs(180);
    while app.lobby_preload_task.is_some() {
        app.poll_lobby_preload().test_value();
        main_assert!(Instant::now() < deadline, "lobby preload did not finish");
        thread::yield_now();
    }
    let artifact = some(&app.lobby_preload_artifact);
    let expected_hud = Arc::clone(&artifact.game_graphics.hud_graphics);
    let expected_textures = Arc::clone(&artifact.material_texture_images);
    let expected_render_info = Arc::clone(&artifact.material_render_info);

    app.network = None;
    app.network_mode = None;
    app.classic_host_lobby = None;
    let staged = app.staged_network_host_scenario.take().test_value();
    app.activate_loaded_scenario(staged.frontend, &staged.scenario)
        .test_value();
    main_assert!(Arc::ptr_eq(
        &expected_hud,
        &some(&app.active_game_graphics).hud_graphics
    ));
    main_assert!(Arc::ptr_eq(
        &expected_textures,
        &app.material_texture_images
    ));
    main_assert!(Arc::ptr_eq(
        &expected_render_info,
        &app.material_render_info
    ));
}

#[test]
fn catalog_host_lobby_preload_is_eligible_and_caches_the_selected_scenario() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .test_value();
    let frontend = tutorial_frontend(repository);
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.scensel.catalog
        .insert(frontend.identifier.clone(), frontend.clone());
    let mut lobby = NetworkLobbyState::new(0, "Catalog Host".to_string(), true)
        .with_preloading(true, LobbyLabels::default());
    lobby.select_scenario(&frontend.identifier, &frontend.title);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(
        lobby_fixture!(host: 11_112, "Catalog Host".to_string(), None),
    ));
    let _events = install_network_stub(&mut app);

    app.sync_classic_lobby_resource_ready();
    main_assert!(app.lobby_preload_task.is_some());
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.lobby_preload_task.is_some() {
        app.poll_lobby_preload().test_value();
        main_assert!(Instant::now() < deadline, "catalog preload did not finish");
        thread::yield_now();
    }
    main_assert!(
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
        .test_value();
    let mut stale_key = catalog_host.key.clone();
    stale_key.languages.push("DE".to_string());
    main_assert!(catalog_host.take_matching_scenario(&stale_key).is_none());
    main_assert!(
        catalog_host.scenario.is_some(),
        "a changed raw load key must leave the cached scenario untouched"
    );
    let artifact = some(&app.lobby_preload_artifact);
    let expected_hud = Arc::clone(&artifact.game_graphics.hud_graphics);
    let expected_textures = Arc::clone(&artifact.material_texture_images);
    let expected_render_info = Arc::clone(&artifact.material_render_info);

    let definition_load = app.scenario_seed_definition_load();
    app.begin_loading_scenario(frontend, definition_load)
        .test_value();

    main_assert!(
        app.lobby_preload_artifact
            .as_ref()
            .and_then(|artifact| artifact.catalog_host.as_ref())
            .is_some_and(|catalog_host| catalog_host.scenario.is_none()),
        "the regular loading path consumes the cached scenario"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.loading_state.is_some() {
        app.poll_loading().test_value();
        main_assert!(
            Instant::now() < deadline,
            "preloaded catalog scenario did not activate"
        );
        thread::yield_now();
    }
    main_assert!(Arc::ptr_eq(
        &expected_hud,
        &some(&app.active_game_graphics).hud_graphics
    ));
    main_assert!(Arc::ptr_eq(
        &expected_textures,
        &app.material_texture_images
    ));
    main_assert!(Arc::ptr_eq(
        &expected_render_info,
        &app.material_render_info
    ));
}

#[test]
fn lobby_preload_launch_failure_logs_red_without_error_sound_and_stays_retryable() {
    let mut app = new_state_only_menu_app(320, 200);
    install_test_classic_host_lobby(&mut app);
    {
        let lobby = app_classic_lobby_mut(&mut app.classic_host_lobby);
        lobby.preload = LobbyPreloadState::new(false);
        main_assert!(!lobby.preload.synchronize(true, true));
        lobby.controller.set_preload_button_state(true, true);
    }
    let sounds_before = app.sound.ui_log.len();

    app.request_lobby_preload();

    let lobby = app_classic_lobby(&app);
    main_assert!(lobby.preload.eligible);
    main_assert!(lobby.preload.manual_button_present);
    main_assert_eq!(app.sound.ui_log.len() => sounds_before);
    main_assert_eq!(lobby.controller.logs().last() => Some(&LobbyLogLine {text: "Preloading error.".to_string(), color: [255, 32, 32, 255],}));
}

#[test]
fn lobby_ready_toggle_is_disabled_while_non_player_resource_loads() {
    // UpdatePreloadingGUIState disables the Ready checkbox until every
    // registered non-player resource is complete, so OnReadyCheck cannot
    // broadcast or mutate local readiness (src/C4GameLobby.cpp:779-824,
    // 329-343).
    let mut app = new_state_only_menu_app(320, 200);
    let (_events, mut commands) = install_client_network_commands(&mut app, 7);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local client".to_string(), false));
    app.admission_resources
        .register_lobby_resource(&lobby_fixture!(resource {
            resource_type: clonk_network::HostResourceType::Scenario as u8,
            id: 43,
            loadable: true,
        }));

    process_joined_lobby_action(&mut app, LobbyAction::ToggleReady);

    main_assert!(!app_lobby(&app).local_ready());
    main_assert!(commands.take_submitted_ready_checks().is_empty());
}

#[test]
fn join_data_resources_keep_lobby_ready_disabled_until_completion_events() {
    // InitClient registers GameRes and Dynamic from JoinData before DoLobby;
    // UpdateResourceProgress then observes those same resources until all
    // non-player loads finish (src/C4Network2.cpp:1612-1620;
    // src/C4GameLobby.cpp:779-802).
    let mut app = new_state_only_menu_app(320, 200);
    let event_tx = install_client_network_stub(&mut app, 7);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Observer",
    )));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Observer".to_string(), false));
    let resource = |resource_type, id| {
        lobby_fixture!(resource {
            resource_type,
            id,
            loadable: true,
        })
    };
    let scenario = resource(clonk_network::HostResourceType::Scenario as u8, 44);
    let dynamic = resource(clonk_network::HostResourceType::Dynamic as u8, 45);
    let definitions = resource(clonk_network::HostResourceType::Definitions as u8, 46);
    let player = resource(clonk_network::HostResourceType::Player as u8, 47);
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot.parameters.clients = clonk_network::JoinClientRegistrySnapshot {
        clients: vec![
            lobby_fixture!(client {
                client_id: 0,
                activated: true,
            }),
            lobby_fixture!(client {
                client_id: 7,
                observer: true,
            }),
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
            players: vec![lobby_fixture!(player {
                id: 1,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(player.clone()),
            })],
        }],
    };
    snapshot.dynamic = dynamic.clone();
    let join_data = lobby_fixture!(join_data:
        7,
        snapshot.dynamic_tick,
        host_config.initial_status,
        dynamic,
        snapshot.parameters,
    );
    send_network_event(&event_tx, NetworkEvent::JoinData(join_data));

    app.test_network_events();
    main_assert!(!app.admission_resources.lobby_ready_available());
    main_assert_eq!(app.admission_resources.status(player.id) => Some(&AdmissionResourceState::Loading { removed: false }));

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
            .test_value();
    }
    app.test_network_events();

    main_assert!(app.admission_resources.lobby_ready_available());
    main_assert_eq!(
        app.admission_resources.status(player.id) =>
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
    let mut lobby = host_lobby_state();
    lobby.register_peer(7, "Player".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Observer".to_string(), ParticipantKind::Observer);
    for participant in lobby.participants.values_mut() {
        participant.ready = true;
    }
    lobby.countdown = Some(5);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_lobby_countdown = Some(HostLobbyCountdown::new());
    for client_id in [0, 7, 9] {
        app.control_clients.register(client_id, true, false);
        main_assert!(app.control_clients.set_lobby_ready(client_id, true));
    }
    let (_events, mut commands) = install_network_commands(&mut app);

    main_assert!(app
        .request_lobby_ready_check_at(Instant::now())
        .expect("host ready check starts"));

    let lobby = app_lobby(&app);
    main_assert!(lobby.participants[&0].ready);
    main_assert!(!lobby.participants[&7].ready);
    main_assert!(!lobby.participants[&9].ready);
    main_assert!(app.control_clients.state(0).unwrap().lobby_ready);
    main_assert!(!app.control_clients.state(7).unwrap().lobby_ready);
    main_assert!(!app.control_clients.state(9).unwrap().lobby_ready);
    main_assert_eq!(lobby.countdown => None);
    main_assert!(app.host_lobby_countdown.is_none());
    assert_ready_checks(&mut commands, 0, clonk_network::ReadyCheckData::Request);
}

#[test]
fn host_ready_check_uses_cpp_ten_second_default_cooldown() {
    // /readycheck calls Config.Cooldowns.ReadyCheck.TryReset before it
    // mutates lobby state; the stock configured default is ten seconds
    // (src/C4GameLobby.cpp:614-627; src/C4Config.cpp:394-400;
    // src/C4Cooldown.h:54-64).
    let mut app = new_state_only_menu_app(320, 200);
    let mut lobby = host_lobby_state();
    lobby.register_peer(7, "Player".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).test_value().ready = true;
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    let (_events, mut commands) = install_network_commands(&mut app);
    let now = Instant::now();

    main_assert!(app.request_lobby_ready_check_at(now).unwrap());
    main_assert_eq!(commands.take_submitted_ready_checks().len() => 1);
    app_lobby_mut(&mut app.network_lobby)
        .participants
        .get_mut(&7)
        .test_value()
        .ready = true;
    app.host_lobby_countdown = Some(HostLobbyCountdown::new());

    main_assert!(!app
        .request_lobby_ready_check_at(now + Duration::from_secs(9))
        .unwrap());
    main_assert!(app_lobby(&app).participants[&7].ready);
    main_assert!(app.host_lobby_countdown.is_some());
    main_assert!(commands.take_submitted_ready_checks().is_empty());
    main_assert_eq!(app.status_text => "Too early! Please wait 1 seconds.");

    main_assert!(app
        .request_lobby_ready_check_at(now + Duration::from_secs(10))
        .unwrap());
    main_assert!(!app_lobby(&app).participants[&7].ready);
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert_eq!(commands.take_submitted_ready_checks().len() => 1);
}

#[test]
fn ready_check_config_clamps_below_cpp_five_second_minimum() {
    // mkParAdapt compiles Config.Cooldowns.ReadyCheck with a five-second
    // minimum, independently of its ten-second missing-value default
    // (src/C4Config.cpp:394-400; src/C4Cooldown.h:85-93).
    let cooldown = LobbyReadyCheckCooldown::from_config_seconds(2);

    main_assert_eq!(cooldown.duration => Duration::from_secs(5));
}

#[test]
fn ready_check_cooldown_reads_cpp_cooldowns_config_key() {
    // C4ConfigCooldowns compiles this value as [Cooldowns] ReadyCheck
    // (src/C4Config.cpp:394-400,865).
    let mut config = Config::new();
    config.set_in(Some("Cooldowns"), "ReadyCheck", "17");

    let cooldown = lobby_ready_check_cooldown_from_config(Some(&config));

    main_assert_eq!(cooldown.duration => Duration::from_secs(17));
}

#[test]
fn ready_check_toast_config_uses_cpp_boolean_grammar_and_default() {
    main_assert!(ready_check_toasts_enabled_from_config(b""));
    main_assert!(ready_check_toasts_enabled_from_config(
        b"[Toasts]\nReadyCheck=true\n"
    ));
    main_assert!(!ready_check_toasts_enabled_from_config(
        b"[Toasts]\nReadyCheck=false\n"
    ));
    main_assert!(
        ready_check_toasts_enabled_from_config(b"[Toasts]\nReadyCheck=invalid\n"),
        "malformed values retain C++'s enabled default"
    );
}

/// `Config.Toasts.ReadyCheck` is the only condition on the toast.
///
/// `ReadyCheckDialog::UpdateText` builds it under
/// `Config.Toasts.ReadyCheck && !toast && Application.ToastSystem`
/// (`src/C4Network2.cpp:152-171`) — window focus is not part of that test.
/// C++ keeps the two mechanisms apart: raising the window is the separate
/// `Application.NotifyUserIfInactive()` call the same handler already makes
/// (`src/C4Network2.cpp:1670`), which the port models as its own
/// `ClassicLobbyAction`. Gating the toast on focus as well would mean a
/// focused client never gets one, and so never reaches the notification's
/// answer actions at all.
#[test]
fn ready_check_toast_follows_the_config_flag_rather_than_window_focus() {
    fn client_app(window_active: bool, toasts_enabled: bool) -> GameApp {
        let mut app = new_menu_app(320, 200);
        app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
            SocketAddr::from(([127, 0, 0, 1], 11_112)),
            "Client",
        )));
        app.network_lobby = Some(client_lobby_state());
        app.window_active = window_active;
        app.ready_check_toasts_enabled = toasts_enabled;
        app
    }

    let packet = clonk_network::ReadyCheckPacket::new(0, clonk_network::ReadyCheckData::Request);
    let mut app = client_app(false, true);
    app.handle_lobby_ready_check_request(packet).test_value();

    main_assert_eq!(
        app.take_desktop_notification().map(|(_, notification)| notification) =>
        Some(DesktopNotification::new(
            "Are you ready?",
            "The host wants to know whether you're ready.\n15 seconds remaining.",
            Duration::from_secs(15),
        ))
    );
    app.handle_lobby_ready_check_request(packet).test_value();
    main_assert!(app.take_desktop_notification().is_none());

    // A focused client is toasted too: C++ tests the config flag alone.
    let mut focused = client_app(true, true);
    focused.handle_lobby_ready_check_request(packet).test_value();
    main_assert!(
        focused.take_desktop_notification().is_some(),
        "window focus is not one of ReadyCheckDialog::UpdateText's conditions"
    );

    // Turning the toast off is the one thing that suppresses it.
    for window_active in [true, false] {
        let mut app = client_app(window_active, false);
        app.handle_lobby_ready_check_request(packet).test_value();
        main_assert!(app.take_desktop_notification().is_none());
    }
}

#[test]
fn desktop_notification_delivery_failure_is_nonfatal() {
    let mut app = new_state_only_menu_app(320, 200);
    let shown = app.queue_desktop_notification(DesktopNotification::new(
        "Ready check",
        "Synthetic failure",
        Duration::from_secs(15),
    ));
    app.pending_desktop_notification_dismissals.push_back(shown);
    let mut shows = 0;
    let mut hides = 0;

    deliver_desktop_notifications(
        &mut app,
        |_, _| {
            shows += 1;
            Err(anyhow!("synthetic notification backend failure"))
        },
        |_| {
            hides += 1;
            Err(anyhow!("synthetic notification close failure"))
        },
    );

    main_assert_eq!(shows => 1);
    main_assert_eq!(hides => 1, "a failed show still lets the hide be attempted");
    main_assert!(app.take_desktop_notification().is_none());
    main_assert!(app.take_dismissed_desktop_notification().is_none());
}

/// The ready check's toast dies with its dialog, whichever side closed it.
///
/// `ReadyCheckDialog::OnClosed` detaches the toast's handler and hides it
/// (`src/C4Network2.cpp:176-183`), and it runs for every way the modal can
/// end. C++ cannot leave one on screen: the toast is a member of the dialog
/// being destroyed.
#[test]
fn answering_a_ready_check_dismisses_its_notification_exactly_once() {
    let mut app = new_menu_app(320, 200);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_lobby = Some(client_lobby_state());
    app.window_active = false;
    app.ready_check_toasts_enabled = true;

    let packet = clonk_network::ReadyCheckPacket::new(0, clonk_network::ReadyCheckData::Request);
    app.handle_lobby_ready_check_request(packet).test_value();

    let (shown, _) = app
        .take_desktop_notification()
        .expect("the request queues one notification");
    main_assert!(
        app.take_dismissed_desktop_notification().is_none(),
        "an open check dismisses nothing"
    );

    app.complete_lobby_ready_check_response(true).test_value();

    main_assert_eq!(
        app.take_dismissed_desktop_notification() => Some(shown),
        "the answer hides the toast it was shown for",
    );
    main_assert!(
        app.take_dismissed_desktop_notification().is_none(),
        "and hides it once, not once per drain"
    );
}

/// The countdown ends the dialog too, so it takes the toast with it.
///
/// `ReadyCheckDialog` is a `TimedDialog{15}` whose last callback closes it
/// false (`src/C4Network2.cpp:129-146`), and closing is what runs `OnClosed`.
/// A toast left behind here is the visible form of the bug: it outlives the
/// check by whatever remains of its own expiry.
#[test]
fn a_timed_out_ready_check_dismisses_its_notification() {
    let (mut app, event_tx, _commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Client",
        client_lobby_state(),
    );
    app.window_active = false;
    app.ready_check_toasts_enabled = true;
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();

    let (shown, _) = app
        .take_desktop_notification()
        .expect("the request queues one notification");

    for _ in 0..LOBBY_READY_CHECK_PROMPT_SECONDS {
        app.sec1_timer().test_value();
    }

    main_assert!(app.message_dialogs.is_empty(), "the countdown closed it");
    main_assert_eq!(
        app.take_dismissed_desktop_notification() => Some(shown),
        "and the toast went with the dialog",
    );
    main_assert!(app.take_dismissed_desktop_notification().is_none());
}

/// Lobby teardown resolves a live check, so it dismisses the toast as well.
///
/// `C4Network2::LobbyDestroyed` clears every remaining dialog without running
/// their callbacks (`src/C4Network2.cpp:493-512`); the toast is not a callback
/// but a member, so it dies regardless.
#[test]
fn lobby_teardown_dismisses_a_live_ready_check_notification() {
    let mut app = new_menu_app(320, 200);
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.network_lobby = Some(client_lobby_state());
    app.window_active = false;
    app.ready_check_toasts_enabled = true;

    let packet = clonk_network::ReadyCheckPacket::new(0, clonk_network::ReadyCheckData::Request);
    app.handle_lobby_ready_check_request(packet).test_value();
    let (shown, _) = app
        .take_desktop_notification()
        .expect("the request queues one notification");

    app.close_lobby_ready_check_continuation();

    main_assert_eq!(app.take_dismissed_desktop_notification() => Some(shown));
    // Teardown runs the same close again on its way out; the toast is already
    // gone and must not be named a second time.
    app.close_lobby_ready_check_continuation();
    main_assert!(app.take_dismissed_desktop_notification().is_none());
}

#[test]
fn client_ready_check_request_replies_not_ready_while_resources_load() {
    // HandleReadyCheck clears every non-host readiness flag, and when
    // MainDlg::CanBeReady is false it skips the dialog and immediately
    // broadcasts NotReady for the local client
    // (src/C4Network2.cpp:1635-1688).
    let mut lobby = client_lobby_state();
    lobby.register_peer(9, "Peer".to_string(), ParticipantKind::Player);
    for participant in lobby.participants.values_mut() {
        participant.ready = true;
    }
    let (mut app, event_tx, mut commands) =
        networked_client_lobby_with_commands(new_menu_app(320, 200), "Client", lobby);
    app.admission_resources
        .register_lobby_resource(&lobby_fixture!(resource {
            resource_type: clonk_network::HostResourceType::Scenario as u8,
            id: 51,
            loadable: true,
        }));
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);

    app.test_network_events();

    let lobby = app_lobby(&app);
    main_assert!(lobby.participants[&0].ready);
    main_assert!(!lobby.participants[&7].ready);
    main_assert!(!lobby.participants[&9].ready);
    main_assert!(app.message_dialogs.is_empty());
    assert_ready_checks(&mut commands, 7, clonk_network::ReadyCheckData::NotReady);
}

#[test]
fn complete_client_ready_check_opens_one_exact_fifteen_second_prompt() {
    // A resource-complete client creates one ReadyCheckDialog for fifteen
    // seconds. While its nested modal loop handles packets, another Request
    // is ignored before readiness is cleared again
    // (src/C4Network2.cpp:129-173,1635-1643,1657-1688).
    let mut lobby = client_lobby_state();
    lobby.register_peer(9, "Peer".to_string(), ParticipantKind::Player);
    for participant in lobby.participants.values_mut() {
        participant.ready = true;
    }
    let (mut app, event_tx, mut commands) =
        networked_client_lobby_with_commands(new_menu_app(320, 200), "Client", lobby);
    let request = NetworkEvent::ReadyCheck(clonk_network::ReadyCheckPacket::new(
        0,
        clonk_network::ReadyCheckData::Request,
    ));
    event_tx.send(request).test_value();

    app.test_network_events();

    main_assert_eq!(app.message_dialogs.len() => 1);
    let prompt = &app.message_dialogs[0].state;
    main_assert_eq!(prompt.caption() => "Are you ready?");
    main_assert_eq!(prompt.message() => "The host wants to know whether you're ready.|15 seconds remaining.");
    main_assert_eq!(prompt.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::YES_NO);
    main_assert_eq!(prompt.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Standard(30));
    main_assert_eq!(prompt.focused_button() => None);
    main_assert!(commands.take_submitted_ready_checks().is_empty());

    app_lobby_mut(&mut app.network_lobby)
        .participants
        .get_mut(&9)
        .test_value()
        .ready = true;
    event_tx
        .send(NetworkEvent::ReadyCheck(
            clonk_network::ReadyCheckPacket::new(0, clonk_network::ReadyCheckData::Request),
        ))
        .test_value();
    app.test_network_events();

    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert!(app_lobby(&app).participants[&9].ready);
    main_assert!(commands.take_submitted_ready_checks().is_empty());
}

#[test]
fn accepting_ready_check_sets_local_ready_and_submits_cpp_ready_reply() {
    // ShowModalDlg(true) broadcasts Ready for the local client, checks the
    // Ready checkbox, and then applies that local C4Client transition
    // (src/C4Network2.cpp:1673-1695,1721-1729).
    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Client",
        client_lobby_state(),
    );
    app.startup.view = StartupView::NetworkLobby;
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();

    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app_lobby(&app).local_ready());
    assert_ready_checks(&mut commands, 7, clonk_network::ReadyCheckData::Ready);
}

#[test]
fn declining_ready_check_keeps_local_unready_and_submits_cpp_reply() {
    // ShowModalDlg(false), including the explicit No button, broadcasts
    // NotReady for the local client and leaves the checkbox clear
    // (src/C4Network2.cpp:1673-1695).
    let mut lobby = client_lobby_state();
    lobby.participants.get_mut(&7).test_value().ready = true;
    let (mut app, event_tx, mut commands) =
        networked_client_lobby_with_commands(new_menu_app(320, 200), "Client", lobby);
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();

    main_assert!(app.message_dialogs.is_empty());
    main_assert!(!app_lobby(&app).local_ready());
    assert_ready_checks(&mut commands, 7, clonk_network::ReadyCheckData::NotReady);
}

/// `ReadyCheckDialog::OnAction` closes the modal with the answer the toast
/// button carries, and `ShowModalDlg`'s single return value is what gets
/// broadcast (src/C4Network2.cpp:190-193,1673-1695). So a notification answer
/// is the *same* answer the dialog would have given — one packet, and the
/// prompt is gone.
#[test]
fn ready_check_notification_actions_submit_the_answer_and_close_the_prompt() {
    use crate::ready_check_notification::{NotificationAction, NotificationActivation};

    for (action, expected) in [
        (NotificationAction::Yes, clonk_network::ReadyCheckData::Ready),
        (NotificationAction::No, clonk_network::ReadyCheckData::NotReady),
    ] {
        let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
            new_menu_app(320, 200),
            "Client",
            client_lobby_state(),
        );
        send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
        app.test_network_events();
        main_assert!(!app.message_dialogs.is_empty());

        let continuation = app.lobby_ready_check_continuation.clone().test_value();
        main_assert!(continuation.activate(
            NotificationActivation::Chosen(action),
            app.lobby_ready_check_sink.as_ref()
        ));
        app.poll_lobby_ready_check_notification().test_value();

        main_assert!(app.message_dialogs.is_empty());
        main_assert_eq!(
            app_lobby(&app).local_ready() => matches!(action, NotificationAction::Yes)
        );
        assert_ready_checks(&mut commands, 7, expected);

        // A second activation, a late dialog answer and a repeated poll are
        // all inert: the continuation is claimed exactly once.
        main_assert!(!continuation.activate(
            NotificationActivation::Chosen(NotificationAction::No),
            app.lobby_ready_check_sink.as_ref()
        ));
        app.poll_lobby_ready_check_notification().test_value();
        app.complete_lobby_ready_check_response(false).test_value();
        main_assert!(commands.take_submitted_ready_checks().is_empty());
    }
}

/// Clicking the toast body asks for the window and leaves the question open.
///
/// C++ disagrees with itself across platforms — libnotify's `default` action
/// reaches `Activated()` and closes the dialog *true*, while a WinRT body
/// click still carries `IToastActivatedEventArgs` and so takes `OnAction("")`,
/// closing it *false* (src/C4ToastLibNotify.cpp:45,107-128;
/// src/C4ToastWinRT.cpp:110-143; src/C4Network2.cpp:185-193). The same gesture
/// therefore broadcasts Ready on one platform and NotReady on the other, so
/// there is no single behaviour to mirror and the port picks neither: it takes
/// "the user came back to the game" literally, raises the window, and leaves
/// the prompt answerable.
#[test]
fn a_ready_check_notification_body_click_raises_without_answering() {
    use crate::ready_check_notification::NotificationActivation;

    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Client",
        client_lobby_state(),
    );
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();
    main_assert!(!app.message_dialogs.is_empty(), "the prompt is up");

    let continuation = app.lobby_ready_check_continuation.clone().test_value();
    main_assert!(
        !continuation.activate(NotificationActivation::Default, app.lobby_ready_check_sink.as_ref()),
        "a body click resolves nothing"
    );
    app.poll_lobby_ready_check_notification().test_value();

    main_assert!(
        app.lobby_ready_check_continuation.is_some(),
        "the continuation outlives a body click"
    );
    main_assert!(
        !app.message_dialogs.is_empty(),
        "and so does the dialog it raised"
    );
    main_assert!(app.pending_window_attention, "the window was asked for");
    main_assert!(commands.take_submitted_ready_checks().is_empty());

    // The dialog it raised is still the answer path.
    app.complete_lobby_ready_check_response(true).test_value();
    main_assert_eq!(
        commands
            .take_submitted_ready_checks()
            .into_iter()
            .map(|packet| packet.data)
            .collect::<Vec<_>>() =>
        vec![clonk_network::ReadyCheckData::Ready]
    );
}

/// The countdown and the toast race every second. `TimedDialog::OnSec1Timer`
/// closes the prompt false on the fifteenth tick
/// (src/C4GuiDialogs.cpp:1279-1299), so whichever of the two resolves first
/// owns the answer and the other becomes inert.
#[test]
fn a_ready_check_timeout_and_a_notification_answer_cannot_both_submit() {
    use crate::ready_check_notification::{NotificationAction, NotificationActivation};

    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Client",
        client_lobby_state(),
    );
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();

    let continuation = app.lobby_ready_check_continuation.clone().test_value();
    main_assert!(continuation.activate(
        NotificationActivation::Chosen(NotificationAction::Yes),
        app.lobby_ready_check_sink.as_ref()
    ));

    // The timer fires before the loop has polled the notification.
    for _ in 0..LOBBY_READY_CHECK_PROMPT_SECONDS {
        app.tick_lobby_ready_check_prompt();
    }
    app.poll_lobby_ready_check_notification().test_value();

    main_assert!(app.message_dialogs.is_empty());
    assert_ready_checks(&mut commands, 7, clonk_network::ReadyCheckData::Ready);
}

/// `close_lobby_child_dialogs_silently` drops the prompt without running its
/// continuation, which is how C++'s `DoLobby` deletes the lobby on GS_Go. The
/// toast must go with it — `ReadyCheckDialog::OnClosed` detaches the handler
/// and hides it (src/C4Network2.cpp:176-178) — and a later activation must not
/// answer a check that no longer exists.
#[test]
fn lobby_teardown_closes_the_ready_check_notification_and_makes_it_inert() {
    use crate::ready_check_notification::{NotificationAction, NotificationActivation};

    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Client",
        client_lobby_state(),
    );
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();
    let continuation = app.lobby_ready_check_continuation.clone().test_value();

    app.close_lobby_child_dialogs_silently();
    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.lobby_ready_check_continuation.is_none());
    main_assert!(continuation.resolved());

    main_assert!(!continuation.activate(
        NotificationActivation::Chosen(NotificationAction::Yes),
        app.lobby_ready_check_sink.as_ref()
    ));
    app.poll_lobby_ready_check_notification().test_value();
    main_assert!(commands.take_submitted_ready_checks().is_empty());
}

#[test]
fn ready_check_prompt_sends_no_reply_after_lobby_ends() {
    // The modal loop may outlive the lobby. C++ rechecks
    // C4Network2::isLobbyActive after the dialog closes and returns before
    // broadcasting or applying the local ready state when the game has
    // already started (src/C4Network2.cpp:1673-1695).
    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Client",
        client_lobby_state(),
    );
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();

    app.mode = AppMode::Running;
    app.network_lobby = None;
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();

    main_assert!(app.message_dialogs.is_empty());
    main_assert!(commands.take_submitted_ready_checks().is_empty());
}

#[test]
fn go_status_request_deletes_client_lobby_and_suppresses_stale_ready_reply() {
    // HandleStatus installs GS_Go before resource preparation finishes.
    // DoLobby therefore closes and deletes pLobby immediately, and a
    // modal ready-check callback that unwinds afterward cannot broadcast
    // a reply (src/C4Network2.cpp:475-515,1673-1695,2010-2029).
    let mut app = new_menu_app(320, 200);
    let fonts = app.assets.clonk_fonts.clone().test_value();
    app.loader_screen = Some(
        LoaderScreen::new(
            LoaderSelection::startup("LoaderClientGo.png").expect("valid client loader selection"),
            ImageData::new(1, 1, vec![7, 8, 9, 255]),
            LoaderResources::new(fonts, ImageData::new(3, 1, vec![255; 12]))
                .expect("valid client loader resources"),
            LoaderState::initial("Loading"),
        )
        .test_value(),
    );
    app.loader_error = None;
    app.loader_render_error = None;
    let (mut app, event_tx, mut commands) =
        networked_client_lobby_with_commands(app, "Client", client_lobby_state());
    app.startup.view = StartupView::NetworkLobby;
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Stale lobby child",
            "Lobby",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    app.open_game_option_input_dialog(GameOptionInputDialogRequest {
        kind: GameOptionInputKind::Password,
        message: "Password",
        caption: "Password",
        icon: clonk_frontend::game_option_buttons::GameOptionIcon::Locked,
        max_text: 31,
        initial_text: String::new(),
        chat_layout: false,
    })
    .test_value();
    let go = lobby_fixture!(status: clonk_network::NETWORK_STATE_GO, 2, 23);
    send_network_event(&event_tx, NetworkEvent::StatusRequested(go));
    app.test_network_events();
    main_assert!(matches!(app.mode, AppMode::Loading));
    main_assert_eq!(app.pending_client_start_status => Some(go));
    main_assert!(
        app.network_lobby.is_none(),
        "native pLobby is deleted as soon as GS_Go is installed"
    );
    main_assert!(
        app.message_dialogs.is_empty(),
        "DoLobby closes lobby-owned dialogs while entering the loader"
    );
    main_assert!(
        app.game_option_input_dialog.is_none(),
        "CloseAllDialogs also removes lobby option input"
    );
    main_assert!(commands.take_submitted_ready_checks().is_empty());
    let mut frame = vec![0x4c; 320 * 200 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn ready_check_prompt_counts_down_and_times_out_not_ready_at_fifteen_seconds() {
    // ReadyCheckDialog is a TimedDialog{15}; each one-second callback
    // updates the remaining text and the fifteenth closes false, producing
    // NotReady (src/C4Network2.cpp:129-146,1673-1695;
    // src/C4GuiDialogs.cpp:1279-1299).
    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Client",
        client_lobby_state(),
    );
    send_ready_check(&event_tx, 0, clonk_network::ReadyCheckData::Request);
    app.test_network_events();

    for remaining in (1..LOBBY_READY_CHECK_PROMPT_SECONDS).rev() {
        app.sec1_timer().test_value();
        main_assert_eq!(app.message_dialogs.len() => 1);
        main_assert_eq!(app.message_dialogs[0].state.message() => lobby_ready_check_message(remaining));
    }
    main_assert!(commands.take_submitted_ready_checks().is_empty());

    app.sec1_timer().test_value();

    main_assert!(app.message_dialogs.is_empty());
    main_assert!(!app_lobby(&app).local_ready());
    assert_ready_checks(&mut commands, 7, clonk_network::ReadyCheckData::NotReady);
}

#[test]
fn lobby_ready_toggle_broadcasts_cpp_ready_packet() {
    // MainDlg::OnReadyCheck broadcasts the local Client ID and new
    // Ready/NotReady state, then updates the local lobby row
    // (src/C4GameLobby.cpp:329-343).
    let mut app = new_state_only_menu_app(320, 200);
    let (_events, mut commands) = install_client_network_commands(&mut app, 7);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local client".to_string(), false));

    process_joined_lobby_action(&mut app, LobbyAction::ToggleReady);
    main_assert!(app_lobby(&app).local_ready());
    assert_ready_checks(&mut commands, 7, clonk_network::ReadyCheckData::Ready);

    process_joined_lobby_action(&mut app, LobbyAction::ToggleReady);
    main_assert!(!app_lobby(&app).local_ready());
    assert_ready_checks(&mut commands, 7, clonk_network::ReadyCheckData::NotReady);
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
    app.startup.view = StartupView::NetworkLobby;
    let event_tx = install_client_network_stub(&mut app, 7);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local client".to_string(), false));
    app.sync_network_lobby_game_option_state();

    send_lobby_countdown(&event_tx, 12);
    app.test_network_events();
    main_assert_eq!(app_lobby(&app).controller.countdown() => clonk_frontend::game_lobby::LobbyCountdownState::Long { seconds: 12 });
    main_assert!(!app.scenario_game_options.values().countdown);

    app.sync_network_lobby_game_option_state();
    main_assert!(
        !app.scenario_game_options.values().countdown,
        "ordinary lobby synchronization must preserve LongCountdown's unlocked strip"
    );
    let assets = Arc::clone(&app.assets);
    let (_, options) = app_lobby_mut(&mut app.network_lobby)
        .classic_render_state(
            app.graphics.surface(),
            assets.as_ref(),
            &app.scenario_game_options,
        )
        .test_value();
    main_assert!(!options.values().countdown);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let presentation = retained_test_presentation(&app);
    let retained = app.render_retained_gpu_frame(presentation).test_value();
    assert_retained_frame_has_commands("joined lobby long countdown", &retained);

    send_lobby_countdown(&event_tx, 10);
    app.test_network_events();
    main_assert_eq!(app_lobby(&app).controller.countdown() => clonk_frontend::game_lobby::LobbyCountdownState::Final { seconds: 10 });
    main_assert!(app.scenario_game_options.values().countdown);
    let final_frame = app.render_retained_gpu_frame(presentation).test_value();
    assert_retained_frame_has_commands("joined lobby final countdown", &final_frame);
}

#[test]
fn inbound_lobby_countdown_updates_cpp_countdown_start_and_abort_states() {
    // MainDlg maps -1 to no countdown, zero to the start transition, and
    // values through ten to the active countdown state
    // (src/C4GameLobby.cpp:392-418).
    let mut app = new_menu_app(320, 200);
    app.startup.view = StartupView::NetworkLobby;
    let event_tx = install_client_network_stub(&mut app, 7);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.startup.view = StartupView::NetworkLobby;
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
        send_lobby_countdown(&event_tx, countdown);
        app.test_network_events();
        let retained_logs = {
            let lobby = app_lobby(&app);
            main_assert_eq!(lobby.countdown => expected);
            main_assert_eq!(lobby.controller.countdown() => expected_controller);
            main_assert_eq!(lobby.logs.iter().map(|line| line.text.as_str()).collect::<Vec<_>>() => expected_logs,);
            main_assert_eq!(lobby.logs => lobby.controller.logs());
            lobby.logs.clone()
        };
        let assets = Arc::clone(&app.assets);
        let (projection, _) = app_lobby_mut(&mut app.network_lobby)
            .classic_render_state(
                app.graphics.surface(),
                assets.as_ref(),
                &app.scenario_game_options,
            )
            .test_value();
        main_assert_eq!(projection.countdown() => expected_controller);
        main_assert_eq!(projection.logs() => retained_logs);
        main_assert_eq!(app_lobby(&app).controller.logs() => retained_logs,);
        main_assert!(matches!(app.mode, AppMode::Menu));
        main_assert!(app.host_lobby_countdown.is_none());
        main_assert!(
            !app.sec1_timer().expect("pulse client second timer"),
            "client packet state never installs a one-second callback"
        );
        main_assert_eq!(app_lobby(&app).countdown => expected);
    }
}

#[test]
fn host_start_begins_default_cpp_lobby_countdown_without_leaving_lobby() {
    // MainDlg::OnRunBtn starts Config.Lobby.CountdownTime, whose stock
    // value is five; Countdown broadcasts and locally applies that initial
    // value before installing its one-second callback
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:442-472,1111-1131;
    // src/C4Config.cpp:276).
    let (mut app, _events, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));

    process_joined_lobby_action(&mut app, LobbyAction::StartGame);

    assert_lobby_countdowns(&mut commands, &[5]);
    main_assert_eq!(app_lobby(&app).countdown => Some(5));
    main_assert_eq!(app.host_lobby_countdown => Some(HostLobbyCountdown {remaining: DEFAULT_LOBBY_COUNTDOWN_SECONDS,}));
    main_assert!(matches!(app.mode, AppMode::Menu));
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
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(host_lobby_state());
    let (_events, mut commands) = install_network_commands(&mut app);

    process_joined_lobby_action(&mut app, LobbyAction::StartGame);
    main_assert_eq!(app.status_text => "Select a scenario before starting");
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert!(commands.take_lobby_start_commands().is_empty());

    app_lobby_mut(&mut app.network_lobby).select_scenario("missing.c4s", "Missing");
    process_joined_lobby_action(&mut app, LobbyAction::StartGame);
    main_assert_eq!(app.status_text => "Scenario `missing.c4s` is not available in the catalog");
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert!(commands.take_lobby_start_commands().is_empty());
    main_assert!(matches!(app.mode, AppMode::Menu));
}

#[test]
fn host_start_cancels_an_active_cpp_lobby_countdown() {
    // OnRunBtn checks the active countdown before attempting another
    // start. Abort broadcasts -1, locally applies it, and deletes the
    // timer without entering Network.Start
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:442-450,1176-1193;
    // src/C4Network2.cpp:3046-3051).
    let (mut app, _events, mut commands) =
        selected_host_lobby_with_commands(new_menu_app(320, 200));
    process_joined_lobby_action(&mut app, LobbyAction::StartGame);
    assert_lobby_countdowns(&mut commands, &[5]);

    process_joined_lobby_action(&mut app, LobbyAction::StartGame);

    assert_lobby_countdowns(&mut commands, &[-1]);
    main_assert_eq!(app_lobby(&app).countdown => None);
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert!(matches!(app.mode, AppMode::Menu));
    main_assert!(
        !app.sec1_timer().expect("pulse aborted countdown timer"),
        "abort releases the one-second callback"
    );
    main_assert!(commands.take_lobby_start_commands().is_empty());
}

#[test]
fn host_sec1_timer_counts_cpp_lobby_down_through_one() {
    // Countdown::OnSec1Timer decrements first and broadcasts every value
    // in the final ten seconds. The callback is driven by the process-wide
    // second timer, not by a private interval
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:1140-1160;
    // src/C4Application.cpp:495-506; src/StdAppUnix.cpp:261-291).
    let (mut app, _events, mut commands) =
        selected_host_lobby_with_commands(new_menu_app(320, 200));
    process_joined_lobby_action(&mut app, LobbyAction::StartGame);
    commands.take_submitted_lobby_countdowns();

    let mut observed = Vec::new();
    for expected in (1..DEFAULT_LOBBY_COUNTDOWN_SECONDS).rev() {
        main_assert!(
            app.sec1_timer().expect("advance host countdown"),
            "countdown changes visible lobby state"
        );
        observed.extend(commands.take_submitted_lobby_countdowns());
        main_assert_eq!(app_lobby(&app).countdown => Some(expected));
        main_assert!(matches!(app.mode, AppMode::Menu));
    }

    main_assert_eq!(observed => [4, 3, 2, 1].map(clonk_network::LobbyCountdownPacket::new).to_vec());
}

#[test]
fn inbound_countdown_packet_cannot_arm_the_host_owned_timer() {
    // C4Network2::pLobbyCountdown is created only by the host's
    // StartLobbyCountdown. MainDlg::OnCountdownPacket updates presentation
    // state only, so a received or late packet cannot install a callback
    // that eventually enters Network.Start
    // (pristine 9ffa0a5d src/C4Network2.cpp:3038-3051;
    // src/C4GameLobby.cpp:392-418,1111-1131).
    let (mut app, event_tx, mut commands) =
        networked_host_lobby_with_commands(new_menu_app(320, 200), host_lobby_state());
    send_lobby_countdown(&event_tx, 2);
    app.test_network_events();

    main_assert!(
        !app.sec1_timer().expect("pulse packet-only countdown"),
        "no host timer callback was installed"
    );
    main_assert!(commands.take_lobby_start_commands().is_empty());
    main_assert_eq!(app_lobby(&app).countdown => Some(2));
    main_assert!(matches!(app.mode, AppMode::Menu));
}

#[test]
fn connected_observer_starts_not_ready_and_retains_explicit_ready_state() {
    // C4ClientCore initializes LobbyReady=false independently of Observer;
    // only C4PacketReadyCheck changes that field
    // (src/C4Client.cpp:32-36; src/C4Network2.cpp:1625-1635,1703-1731).
    let mut app = new_state_only_menu_app(320, 200);
    let event_tx = install_client_network_stub(&mut app, 7);
    app.network_lobby = Some(NetworkLobbyState::new(7, "Local".to_string(), false));

    event_tx
        .send(NetworkEvent::PeerConnected {
            client_id: 9,
            name: "Observer".to_string(),
            kind: ParticipantKind::Observer,
        })
        .test_value();
    app.test_network_events();
    main_assert!(!app_lobby(&app).participants[&9].ready);

    event_tx
        .send(NetworkEvent::ReadyCheck(
            clonk_network::ReadyCheckPacket::new(9, clonk_network::ReadyCheckData::Ready),
        ))
        .test_value();
    app.test_network_events();
    app_lobby_mut(&mut app.network_lobby).register_peer(
        9,
        "Renamed observer".to_string(),
        ParticipantKind::Observer,
    );

    let observer = &app_lobby(&app).participants[&9];
    main_assert!(observer.ready);
    main_assert_eq!(observer.name => "Renamed observer");
}

#[test]
fn inbound_ready_check_updates_the_claimed_lobby_participant() {
    // HandleReadyCheck looks up packet.Client and applies IsReady to that
    // exact C4Client; it does not substitute the transport sender
    // (src/C4Network2.cpp:1625-1635,1703-1731).
    let mut app = new_state_only_menu_app(320, 200);
    let event_tx = install_client_network_stub(&mut app, 7);
    let mut lobby = NetworkLobbyState::new(7, "Local".to_string(), false);
    lobby.register_peer(9, "Remote".to_string(), ParticipantKind::Player);
    app.network_lobby = Some(lobby);
    app.control_clients.register(9, true, false);

    event_tx
        .send(NetworkEvent::ReadyCheck(
            clonk_network::ReadyCheckPacket::new(9, clonk_network::ReadyCheckData::Ready),
        ))
        .test_value();
    app.test_network_events();

    let lobby = app_lobby(&app);
    main_assert!(lobby.participants[&9].ready);
    main_assert!(!lobby.participants[&7].ready);
    main_assert!(app.control_clients.state(9).unwrap().lobby_ready);
}

#[test]
fn final_remote_ready_transition_starts_cpp_default_countdown() {
    // MainDlg::OnClientReadyStateChange starts Config.Lobby.CountdownTime
    // after the changed client leaves every relevant client ready
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:868-893;
    // src/C4Network2.cpp:1721-1729).
    let (mut app, event_tx, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.participants.get_mut(&0).test_value().ready = true;
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    app.control_player_infos.apply(lobby_player_infos(7, &[1]));
    send_ready_check(&event_tx, 7, clonk_network::ReadyCheckData::Ready);

    app.test_network_events();

    assert_lobby_countdowns(&mut commands, &[5]);
    main_assert_eq!(app_lobby(&app).countdown => Some(5));
    main_assert_eq!(app.host_lobby_countdown => Some(HostLobbyCountdown {remaining: DEFAULT_LOBBY_COUNTDOWN_SECONDS,}));
}

#[test]
fn empty_nonhost_does_not_block_ready_autostart() {
    // MainDlg::OnClientReadyStateChange skips a non-host client when
    // GetInfoByClientID has no players for it
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-888).
    let (mut app, event_tx, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.participants.get_mut(&0).test_value().ready = true;
    lobby.register_peer(7, "Player client".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Empty client".to_string(), ParticipantKind::Observer);
    app.control_player_infos.apply(lobby_player_infos(7, &[1]));
    send_ready_check(&event_tx, 7, clonk_network::ReadyCheckData::Ready);

    app.test_network_events();

    assert_lobby_countdowns(&mut commands, &[5]);
    main_assert!(!app_lobby(&app).participants[&9].ready);
}

#[test]
fn host_without_players_still_blocks_ready_autostart() {
    // MainDlg::OnClientReadyStateChange always includes the host in its
    // readiness scan, even when the host has no C4ClientPlayerInfos entry
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-887).
    let (mut app, event_tx, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    app.control_player_infos.apply(lobby_player_infos(7, &[1]));
    send_ready_check(&event_tx, 7, clonk_network::ReadyCheckData::Ready);

    app.test_network_events();

    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert!(!app_lobby(&app).participants[&0].ready);
}

#[test]
fn unready_nonhost_with_player_blocks_ready_autostart() {
    // MainDlg::OnClientReadyStateChange includes a non-host client when
    // its C4ClientPlayerInfos contains at least one player
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-887).
    let (mut app, event_tx, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.participants.get_mut(&0).test_value().ready = true;
    lobby.register_peer(7, "Unready".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Changed".to_string(), ParticipantKind::Player);
    for (client_id, info_id) in [(7, 1), (9, 2)] {
        app.control_player_infos
            .apply(lobby_player_infos(client_id, &[info_id]));
    }
    send_ready_check(&event_tx, 9, clonk_network::ReadyCheckData::Ready);

    app.test_network_events();

    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert!(!app_lobby(&app).participants[&7].ready);
}

#[test]
fn final_local_host_ready_transition_starts_cpp_default_countdown() {
    // MainDlg::OnReadyCheck applies the host's own ready packet through
    // HandleReadyCheck, which invokes OnClientReadyStateChange for the
    // actual state transition
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:329-344,868-893;
    // src/C4Network2.cpp:1721-1729).
    let (mut app, _events, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).test_value().ready = true;
    app.control_player_infos.apply(lobby_player_infos(7, &[1]));

    process_joined_lobby_action(&mut app, LobbyAction::ToggleReady);

    assert_lobby_countdowns(&mut commands, &[5]);
    main_assert!(app_lobby(&app).local_ready());
    main_assert_eq!(app_lobby(&app).countdown => Some(5));
}

#[test]
fn changed_relevant_client_becoming_unready_aborts_active_countdown() {
    // The first relevant unready client aborts an active host countdown
    // only when it is the client whose ready state actually changed
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-885;
    // src/C4Network2.cpp:1721-1729).
    let (mut app, event_tx, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.participants.get_mut(&0).test_value().ready = true;
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).test_value().ready = true;
    app.control_player_infos.apply(lobby_player_infos(7, &[1]));
    process_joined_lobby_action(&mut app, LobbyAction::StartGame);
    assert_lobby_countdowns(&mut commands, &[5]);
    send_ready_check(&event_tx, 7, clonk_network::ReadyCheckData::NotReady);

    app.test_network_events();

    assert_lobby_countdowns(&mut commands, &[-1]);
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert_eq!(app_lobby(&app).countdown => None);
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
    let mut lobby = host_lobby_state();
    lobby.register_peer(client_id, "Remote".to_string(), kind);
    lobby.countdown = Some(remaining);
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_lobby_countdown = Some(HostLobbyCountdown { remaining });
    app.control_player_infos.apply(lobby_player_infos(
        i32::try_from(client_id).test_value(),
        player_ids,
    ));
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
    send_peer_disconnected(&event_tx, client_id, Some("connection lost"));

    app.test_network_events();

    // C4Network2's raw disconnect callback only mutates transport/control
    // state. Presentation belongs to the later authoritative ClientRemove
    // control (src/C4Network2.cpp:1774-1833; src/C4Control.cpp:637-670).
    main_assert!(
        app.status_text.is_empty(),
        "raw disconnect must not poison the exact lobby renderer"
    );
    main_assert!(!app_lobby(&app).participants.contains_key(&client_id));
    main_assert!(app.host_lobby_countdown.is_none());
    main_assert_eq!(app_lobby(&app).countdown => None);
    assert_lobby_countdowns(&mut commands, &[clonk_network::LobbyCountdownPacket::ABORT]);

    send_peer_disconnected(&event_tx, client_id, None);
    app.test_network_events();
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
}

#[test]
fn host_disconnect_of_playerless_observer_keeps_lobby_countdown() {
    let client_id = 9;
    let (mut app, event_tx, mut commands) =
        host_countdown_disconnect_fixture(client_id, ParticipantKind::Observer, 5, &[]);
    main_assert!(app.control_player_infos.client_info_ids(9).is_empty());
    send_peer_disconnected(&event_tx, client_id, None);

    app.test_network_events();

    main_assert!(!app_lobby(&app).participants.contains_key(&client_id));
    main_assert_eq!(app.host_lobby_countdown => Some(HostLobbyCountdown::new()));
    main_assert_eq!(app_lobby(&app).countdown => Some(5));
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
}

#[test]
fn host_disconnect_during_long_countdown_keeps_native_timer() {
    // MainDlg::IsCountdown excludes CDS_LongCountdown (>10 seconds)
    // (src/C4GameLobby.h:43,92-94; src/C4GameLobby.cpp:392-425).
    let (mut app, event_tx, mut commands) =
        host_countdown_disconnect_fixture(7, ParticipantKind::Player, 11, &[1]);
    send_peer_disconnected(&event_tx, 7, None);

    app.test_network_events();

    main_assert_eq!(app.host_lobby_countdown => Some(HostLobbyCountdown { remaining: 11 }));
    main_assert_eq!(app_lobby(&app).countdown => Some(11));
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
}

#[test]
fn client_host_disconnect_aborts_lobby_and_restores_network_dialog() {
    // A client host loss clears C4Network2. While DoLobby is active that
    // makes DoLobby return false, so C4Game::Init aborts back through the
    // remembered startup dialog instead of continuing locally
    // (src/C4Network2.cpp:477-515,1809-1833;
    // src/C4Game.cpp:405-411).
    let mut app = new_real_classic_menu_app(320, 200);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(client_lobby_state());
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.control_clients.replace_snapshot([
        lobby_fixture!(client {
            client_id: 0,
            name: clonk_engine::LegacyCString::from_bytes(b"Oracle Host".to_vec()).test_value(),
        }),
        lobby_fixture!(client {
            client_id: 7,
            name: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec()).test_value(),
        }),
    ]);
    let (manager, event_tx) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(manager);
    send_peer_disconnected(&event_tx, 0, Some("all client transport routes closed"));

    app.test_network_events();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert!(app.startup_network_dialog.is_some());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.network_lobby.is_none());
    assert_startup_error_log(&app, "Network: host Oracle Host disconnected!");
    let engine_results = app.engine.snapshot().round_results;
    main_assert_eq!(engine_results.network_result => Some(clonk_engine::RoundResultsNetworkResult::NetworkError));
    main_assert_eq!(engine_results.network_result_message => b"Network: host Oracle Host disconnected!");
    main_assert_eq!(app.snapshot.round_results => engine_results);
    let mut frame = vec![0x4c; 320 * 200 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn repeated_or_irrelevant_unready_does_not_abort_manual_countdown() {
    // HandleReadyCheck invokes the lobby callback only for an actual
    // state change. OnClientReadyStateChange returns at the first relevant
    // unready client and aborts only when that exact client changed
    // (pristine 9ffa0a5d src/C4GameLobby.cpp:872-885;
    // src/C4Network2.cpp:1721-1729).
    let (mut app, event_tx, mut commands) =
        selected_host_lobby_with_commands(new_state_only_menu_app(320, 200));
    let lobby = app_lobby_mut(&mut app.network_lobby);
    lobby.participants.get_mut(&0).test_value().ready = true;
    lobby.register_peer(7, "Unready player".to_string(), ParticipantKind::Player);
    lobby.register_peer(9, "Empty client".to_string(), ParticipantKind::Observer);
    lobby.participants.get_mut(&9).test_value().ready = true;
    app.control_player_infos.apply(lobby_player_infos(7, &[1]));
    process_joined_lobby_action(&mut app, LobbyAction::StartGame);
    assert_lobby_countdowns(&mut commands, &[5]);

    for packet in [
        clonk_network::ReadyCheckPacket::new(7, clonk_network::ReadyCheckData::NotReady),
        clonk_network::ReadyCheckPacket::new(7, clonk_network::ReadyCheckData::Ready),
        clonk_network::ReadyCheckPacket::new(9, clonk_network::ReadyCheckData::NotReady),
    ] {
        send_network_event(&event_tx, NetworkEvent::ReadyCheck(packet));
        app.test_network_events();
        main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
        main_assert!(app.host_lobby_countdown.is_some());
        main_assert_eq!(app_lobby(&app).countdown => Some(5));
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
    app.scensel.catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    let mut lobby = host_lobby_state();
    lobby.select_scenario(&scenario.identifier, &scenario.title);
    lobby.participants.get_mut(&0).test_value().ready = true;
    lobby.register_peer(7, "Remote".to_string(), ParticipantKind::Player);
    lobby.participants.get_mut(&7).test_value().ready = true;
    app.network_lobby = Some(lobby);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    let (event_tx, mut commands) = install_network_commands(&mut app);
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            lobby_fixture!(player_data: 7, vec![lobby_fixture!(player { id: 1 })]),
        )))
        .test_value();

    app.test_network_events();

    main_assert_eq!(app.control_player_infos.client_info_ids(7) => vec![1]);
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(app.host_lobby_countdown.is_none());

    app_lobby_mut(&mut app.network_lobby)
        .participants
        .get_mut(&7)
        .test_value()
        .ready = false;
    event_tx
        .send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
            lobby_fixture!(player_data: 7, Vec::new()),
        )))
        .test_value();

    app.test_network_events();

    main_assert!(app.control_player_infos.client_info_ids(7).is_empty());
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(app.host_lobby_countdown.is_none());
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
        .test_value()
        .set_team(Some(1));
    let chooser = join_team_selection_player(&mut app, "Chooser", 0, 0x0000_c800);
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
    main_assert!(app.ingame_menu.is_some(), "selection menu starts open");
    let (events, mut commands) = install_network_commands(&mut app);
    let tick = app.local_control_submission_tick();
    send_ready_tick(&events, tick, Vec::new());

    app.test_update();

    main_assert_eq!(app.engine.player(chooser).map(clonk_engine::Player::status) => Some(PlayerStatus::TeamSelectionPending));
    main_assert!(
        app.ingame_menu.is_none(),
        "forced selection closes the menu"
    );
    main_assert_eq!(
        commands.take_submitted_init_scenario_players() =>
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
    let chooser = join_team_selection_player(&mut app, "Chooser", 41, 0x0012_3456);
    app.control_player_infos.apply(lobby_fixture!(player_data:
        0,
        vec![lobby_fixture!(player {
            id: 41,
            color: 0x0012_3456,
            original_color: 0x0012_3456,
        })],
    ));
    let (host_snapshot, reference) = default_exact_host_reference();
    app.control_clients
        .replace_snapshot(host_snapshot.parameters.clients.clients.clone());
    app.host_join_snapshot = Some(host_snapshot);
    app.advertised_game_reference = Some(reference);
    let _events = install_client_network_stub(&mut app, 0);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));

    app.execute_init_scenario_player_control(chooser, 2)
        .test_value();

    let info = app.control_player_infos.get(41).test_value();
    main_assert_eq!((info.team, info.color, info.original_color) => (2, 0x0000_c800, 0x0012_3456));
    main_assert_eq!(app.engine.teams().iter().find(|team| team.id == 2).expect("green team").player_ids => vec![41]);
    let parameters = &some(&app.host_join_snapshot).parameters;
    main_assert_eq!(parameters.player_infos.clients[0].players[0].team => 2);
    main_assert_eq!(parameters.teams.teams[1].player_ids => vec![41]);

    app.engine.set_player_team(chooser, Some(1)).test_value();
    app.handle_script_player_info_updates().test_value();

    main_assert_eq!(app.control_player_infos.get(41).unwrap().team => 1);
    let parameters = &some(&app.host_join_snapshot).parameters;
    main_assert_eq!(parameters.player_infos.clients[0].players[0].team => 1);
    main_assert_eq!(parameters.teams.teams[0].player_ids => vec![41]);
    main_assert!(parameters.teams.teams[1].player_ids.is_empty());
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
        .test_value()
        .set_name(clonk_script::c4_string_from_bytes(b"Chooser"));
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Alpha", 0x0011_2233),
        clonk_engine::TeamInfo::new(2, "Beta", 0x0044_5566),
    ]);
    let mut configuration = app.engine.team_configuration();
    configuration.allow_team_switch = true;
    app.engine.set_team_configuration(configuration);

    app.apply_ingame_menu_action_for_player(owner, MenuAction::ActivateTeamSelection)
        .test_value();
    {
        let menu = app.ingame_menu.get(owner).test_value();
        main_assert_eq!(menu.page() => ingame_menu::MenuPage::TeamSelection);
        main_assert!(menu.is_team_switch());
        main_assert_eq!(menu.items().iter().map(|item| item.caption.clone()).collect::<Vec<_>>() => ["Alpha", "Beta"]);
    }
    app.ingame_menu.get_mut(owner).test_value().set_selection(1);

    // Membership changes without any menu control executing.
    app.engine.player_mut(owner).test_value().set_team(Some(2));

    main_assert_eq!(app.ingame_menu.get(owner).expect("team switch page").items()[1].caption => "Beta", "native waits for the periodic refill");
    app.refresh_team_menus();
    let menu = app.ingame_menu.get(owner).test_value();
    main_assert_eq!(menu.items().iter().map(|item| item.caption.clone()).collect::<Vec<_>>() => ["Alpha", "Beta (Chooser)"]);
    main_assert!(
        menu.is_team_switch(),
        "a refill keeps dispatching TeamSwitch:<id>"
    );
    main_assert_eq!(menu.selection() => 1, "ClearItems(false) keeps the row index");

    // Auto-generated teams add the New Team row only while no configured
    // team is empty (C4MainMenu.cpp:182-197), so the row appears and
    // disappears across refills.
    let mut configuration = app.engine.team_configuration();
    configuration.auto_generate_teams = true;
    app.engine.set_team_configuration(configuration);
    app.refresh_team_menus();
    main_assert_eq!(app.ingame_menu.get(owner).expect("refilled page").items().len() => 2, "team Alpha is still empty, so no New Team row is offered");

    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Alpha", 0x0011_2233).with_player_ids(vec![41]),
        clonk_engine::TeamInfo::new(2, "Beta", 0x0044_5566).with_player_ids(vec![42]),
    ]);
    app.refresh_team_menus();
    let menu = app.ingame_menu.get(owner).test_value();
    main_assert_eq!(
        menu.items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>() =>
        [
            MenuAction::SwitchTeam(1),
            MenuAction::SwitchTeam(2),
            MenuAction::SwitchTeam(-1),
        ]
    );
    main_assert_eq!(menu.selection() => 1);

    // A shrinking refill clamps an out-of-range selection exactly like
    // AdjustSelection.
    app.ingame_menu.get_mut(owner).test_value().set_selection(2);
    app.engine
        .set_teams(vec![clonk_engine::TeamInfo::new(1, "Alpha", 0x0011_2233)]);
    app.refresh_team_menus();
    let menu = app.ingame_menu.get(owner).test_value();
    main_assert_eq!(menu.items().len() => 1);
    main_assert_eq!(menu.selection() => 0);
}

#[test]
fn team_selection_entries_cache_icon_specs_and_player_info_occupancy() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let mut definition = Definition::from_script("TICO", "Team icon", "").test_value();
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
    app.engine.register_definition(definition).test_value();
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Declared", 0x0011_2233)
            .with_player_ids(vec![999])
            .with_icon_spec("TICO"),
        clonk_engine::TeamInfo::new(2, "Missing", 0x0044_5566).with_icon_spec("MISS"),
    ]);

    let entries = app.team_selection_entries();
    main_assert_eq!(
        (
            entries[0].icon_spec.as_deref(),
            entries[0].color,
            entries[0].has_participants,
        ) =>
        (Some("TICO"), 0x0011_2233, true),
        "C4Team::GetPlayerCount includes retained, not-yet-joined PlayerInfo IDs"
    );
    main_assert_eq!((entries[1].icon_spec.as_deref(), entries[1].color, entries[1].has_participants,) => (Some("MISS"), 0x0044_5566, false));

    app.engine
        .set_player_status(owner, PlayerStatus::TeamSelection)
        .test_value();
    app.open_initial_team_selection(owner);
    let team_icons = &some(&app.ingame_menu_gfx).team_icons;
    main_assert_eq!(team_icons.get(&1).map(ImageData::pixels) => Some([0x11, 0x22, 0x33, 0xff].as_slice()));
    main_assert!(
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
        .test_value()
        .set_name(clonk_script::c4_string_from_bytes(b"Andr\xe9"));
    app.engine
        .player_mut(occupant)
        .test_value()
        .set_team(Some(1));
    app.engine.set_teams(vec![clonk_engine::TeamInfo::new(
        1,
        clonk_script::c4_string_from_bytes(b"Bl\xe5"),
        0x0000_00ff,
    )]);
    let chooser = join_team_selection_player(&mut app, "Chooser", 0, 0x0000_00ff);

    app.open_initial_team_selection(chooser);
    let item = app
        .ingame_menu
        .get(chooser)
        .and_then(|menu| menu.items().first())
        .test_value();
    main_assert_eq!(item.caption => "Bl\u{e5} (Andr\u{e9})");
    main_assert_eq!(item.info_caption.as_deref() => Some("Join team Bl\u{e5} (Andr\u{e9})"));
    main_assert_eq!(clonk_script::c4_string_bytes(app.engine.player(occupant).expect("occupant remains").name()) => b"Andr\xe9");
    main_assert_eq!(clonk_script::c4_string_bytes(&app.engine.teams()[0].name) => b"Bl\xe5");
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
    let chooser = join_team_selection_player(&mut app, "Chooser", 0, 0x0000_c800);
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
        .get_mut(chooser)
        .test_value()
        .set_selection(1);
    let (events, mut commands) = install_network_commands(&mut app);
    let tick = app.local_control_submission_tick();
    send_ready_tick(&events, tick, Vec::new());

    app.test_update();

    main_assert_eq!(app.engine.player(chooser).map(clonk_engine::Player::status) => Some(PlayerStatus::TeamSelection));
    let menu = app.ingame_menu.get(chooser).test_value();
    main_assert_eq!(menu.selection() => 1, "the local choice is not reset");
    main_assert!(commands.take_submitted_init_scenario_players().is_empty());

    app.engine.set_teams(vec![clonk_engine::TeamInfo::new(
        1,
        "Existing",
        0x00f4_0000,
    )]);
    app.engine.set_auto_generate_teams(true);
    let local_owner = app.local_owner;
    app.engine
        .set_player_team(local_owner, Some(1))
        .test_value();
    let tick = app.local_control_submission_tick();
    send_ready_tick(&events, tick, Vec::new());

    app.test_update();

    main_assert_eq!(
        app.ingame_menu
            .as_ref()
            .expect("generated alternative keeps menu open")
            .items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>() =>
        [MenuAction::SelectTeam(1), MenuAction::SelectTeam(-1)]
    );
    main_assert!(commands.take_submitted_init_scenario_players().is_empty());
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
    let first = join_team_selection_player(&mut app, "First chooser", 31, 0xff0000);
    let second = join_team_selection_player(&mut app, "Second chooser", 32, 0x0000ff);
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

    main_assert!(
        [first, second]
            .into_iter()
            .all(|owner| app.ingame_menu_belongs_to(owner)),
        "both local players retain their own initial team menu"
    );
    // The team page is a five-column C4MN_Style_Normal grid, so COM_MenuDown
    // on a two-item menu computes iData = 0 and MoveSelection returns
    // without moving; COM_MenuRight is the within-row step
    // (C4Menu.cpp:444-473).
    main_assert!(app
        .handle_menu_command(first, ControlCommand::MenuDown, CommandKind::Press,)
        .expect("first player consumes the vertical command"));
    main_assert!(app
        .handle_menu_command(first, ControlCommand::MenuRight, CommandKind::Press,)
        .expect("first player navigates own menu"));
    main_assert!(app
        .handle_menu_command(first, ControlCommand::MenuEnter, CommandKind::Press,)
        .expect("first player selects own team"));
    main_assert_eq!(app.engine.player(first).map(|player| (player.status(), player.team())) => Some((PlayerStatus::Active, Some(2))));
    main_assert!(
        app.engine.crew_cursor(first).is_some(),
        "first team activation spawns its native crew"
    );
    main_assert_eq!(
        app.engine
            .player(second)
            .map(|player| (player.status(), player.team())) =>
        Some((PlayerStatus::TeamSelection, None)),
        "first player's selection must not mutate the second player"
    );
    main_assert!(
        app.ingame_menu_belongs_to(second),
        "second player's menu survives the first player's selection"
    );

    main_assert!(app
        .handle_menu_command(second, ControlCommand::MenuEnter, CommandKind::Press,)
        .expect("second player selects own team"));
    main_assert_eq!(app.engine.player(second).map(|player| (player.status(), player.team())) => Some((PlayerStatus::Active, Some(1))));
    main_assert!(
        app.engine.crew_cursor(second).is_some(),
        "second team activation spawns its native crew"
    );
    main_assert!(!app.ingame_menu_belongs_to(first));
    main_assert!(!app.ingame_menu_belongs_to(second));
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
        .test_value();
    let user_data = tempdir();
    let players = tempdir();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install_root)),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
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
            .test_value();
        fs::write(&path, group.pack().test_value()).test_value();
        path
    };
    let bravo = write_player("Bravo.c4p", "Bravo", 0x11_22_33);
    let alpha = write_player("Alpha.c4p", "Alpha", 0x44_55_66);
    let mut config = b"[General]\nName=\"Maker\"\nParticipants=\"".to_vec();
    config.extend_from_slice(bravo.as_os_str().as_encoded_bytes());
    config.push(b';');
    config.extend_from_slice(alpha.as_os_str().as_encoded_bytes());
    config.extend_from_slice(b"\"\n");
    fs::write(paths.config_file(), config).test_value();

    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    wait_for_menu(&mut app);
    app.freeze_configured_client_players_for_game().test_value();
    fs::write(
        paths.config_file(),
        b"[General]\nName=Changed\nParticipants=\"\"\n",
    )
    .test_value();
    let (manager, event_tx, commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.network_lobby = Some(client_lobby_state());
    app.startup.view = StartupView::NetworkLobby;
    // The visible startup list is deliberately stale: production joining
    // must use Config.General.Participants directly, not this UI model.
    app.startup.player_files.clear();
    app.startup.player_models.clear();
    app.selected_player_file = None;

    let configured_paths = [bravo, alpha];
    let cores = configured_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            lobby_fixture!(player_resource:
                (7 << 16) + index as i32,
                clonk_engine::LegacyCString::from_bytes(
                    path.as_os_str().as_encoded_bytes().to_vec(),
                )
                .test_value(),
            )
        })
        .collect::<Vec<_>>();
    let expected_cores = cores.clone();
    let command_observer = thread::spawn(move || commands.complete_initial_client_join(cores));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot
        .parameters
        .clients
        .clients
        .push(lobby_fixture!(client {
            client_id: 7,
            name: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec()).test_value(),
        }));
    snapshot.parameters.clients.local_client_id = Some(7);
    let join_data = lobby_fixture!(join_data: 7, 23, host_config.initial_status, snapshot.dynamic, snapshot.parameters);
    send_network_event(&event_tx, NetworkEvent::JoinData(join_data));

    app.test_network_events();
    for (core, path) in expected_cores.iter().zip(&configured_paths) {
        main_assert_eq!(app.admission_resources.complete_path(core.id) => Some(path.as_path()), "the publishing client keeps each initial player resource complete");
    }

    let (order, publications, player_infos, acknowledgements) = command_observer.test_join();
    main_assert_eq!(order => vec!["publish", "publish", "player-info", "status-ack"]);
    main_assert_eq!(
        publications
            .iter()
            .map(|request| request.wire_name.as_bytes())
            .collect::<Vec<_>>() =>
        configured_paths
            .iter()
            .map(|path| path.as_os_str().as_encoded_bytes())
            .collect::<Vec<_>>()
    );
    main_assert!(publications
        .iter()
        .all(|request| request.group_maker.as_bytes() == b"Maker"));
    main_assert_eq!(player_infos.len() => 1);
    main_assert_eq!(
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
            .collect::<Vec<_>>() =>
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
    main_assert_eq!(acknowledgements.len() => 1);
    main_assert_eq!(acknowledgements[0].target_tick => 23);
}

#[test]
fn startup_network_client_enters_and_acknowledges_lobby_when_boot_completes() {
    // A command-line direct join completes network initialization before
    // C4Game::Init enters C4Network2::DoLobby. DoLobby then marks the lobby
    // running so the initial GS_Lobby can be acknowledged
    // (src/C4Game.cpp:366-409; src/C4Network2.cpp:445-461,2017-2052).
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app_with_paths(320, 200, &paths),
        "Observer",
        NetworkLobbyState::new(7, "Observer".to_string(), false),
    );
    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot
        .parameters
        .clients
        .clients
        .push(lobby_fixture!(client {
            client_id: 7,
            observer: true,
            name: clonk_engine::LegacyCString::from_bytes(b"Observer".to_vec()).test_value(),
        }));
    snapshot.parameters.clients.local_client_id = Some(7);
    event_tx
        .send(NetworkEvent::JoinData(lobby_fixture!(join_data: 7, 23, host_config.initial_status, snapshot.dynamic, snapshot.parameters)))
        .test_value();
    app.test_network_events();
    main_assert!(commands.take_status_acknowledgements().is_empty());

    app.mode = AppMode::Loading;
    let (boot_tx, boot_rx) = mpsc::channel();
    app.boot_loading = Some(BootLoadingState::new(boot_rx));
    boot_tx.send(BootLoadingEvent::Finished(None)).test_value();

    app.poll_boot_loading();

    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(app.network.is_some());
    main_assert!(app.network_lobby.is_some());
    main_assert_eq!(commands.take_framed_status_acknowledgements() => vec![(clonk_network::NetworkStatus {target_tick: 23,..host_config.initial_status}, 0,)]);
}

#[test]
fn client_lobby_acknowledges_join_status_at_the_initialized_control_tick_once() {
    // DoLobby marks GS_Lobby reached only after the lobby is running, then
    // rewrites the reference status target to the initialized ControlTick
    // and sends one PID_StatusAck after initial PlayerInfo submission
    // (src/C4Network2.cpp:445-461,2041-2058;
    // src/C4Network2Players.cpp:124-136).
    let (mut app, event_tx, mut commands) = networked_client_lobby_with_commands(
        new_menu_app(320, 200),
        "Observer",
        NetworkLobbyState::new(7, "Observer".to_string(), false),
    );
    app.startup.view = StartupView::NetworkLobby;
    app.selected_player_file = None;
    for _ in 0..3 {
        app.engine.tick().test_value();
    }

    let host_config = clonk_network::HostConfig::default();
    let mut reference_status = host_config.initial_status;
    reference_status.target_tick = -1;
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    snapshot
        .parameters
        .clients
        .clients
        .push(lobby_fixture!(client {
            client_id: 7,
            name: clonk_engine::LegacyCString::from_bytes(b"Observer".to_vec()).test_value(),
        }));
    snapshot.parameters.clients.local_client_id = Some(7);
    let join_data =
        lobby_fixture!(join_data: 7, 23, reference_status, snapshot.dynamic, snapshot.parameters);
    send_network_event(&event_tx, NetworkEvent::JoinData(join_data));

    app.test_network_events();

    main_assert_eq!(app.network_control_clock.map(NetworkControlClock::current_tick) => Some(23), "PID_StatusAck uses Game.Control.ControlTick");
    main_assert_eq!(app.engine.frame() => 3, "the distinct Game.FrameCounter is reserved for ClientActReq");

    main_assert_eq!(commands.take_framed_status_acknowledgements() => vec![(clonk_network::NetworkStatus {target_tick: 23,..reference_status}, 3,)]);
    app.test_network_events();
    main_assert!(commands.take_status_acknowledgements().is_empty());
}

#[test]
fn later_admission_reuses_shifted_initial_hosts_persisted_alternate_color() {
    // `ResolvePlayerAttributeConflicts` revisits every retained packet on
    // each admission. The synchronized row omits AlternateColorDw, but
    // the host's original C4PlayerInfo keeps it for the whole session
    // (src/C4PlayerInfo.cpp:82-90,177-230;
    // src/C4PlayerInfoConflicts.cpp:249-296).
    let legacy =
        |bytes: &[u8]| clonk_engine::LegacyCString::from_bytes(bytes.to_vec()).test_value();
    let resource = |id| lobby_fixture!(resource { id });
    let mut app = new_state_only_menu_app(320, 200);
    app.network_max_players = 8;
    app.control_player_infos.replace_snapshot(
        2,
        [lobby_fixture!(player_data {
            client_id: 0,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![
                lobby_fixture!(player_named: legacy(b"Blocker"), color: 0x00f4_0000,
                    id: 1,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(resource(11)),
                ),
                lobby_fixture!(player {
                    id: 2,
                    name: legacy(b"Shifted host"),
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    color: 0x0000_00e8,
                    original_color: 0x00f4_0000,
                    resource: Some(resource(22)),
                }),
            ],
        })],
    );
    app.host_local_alternate_colors_by_resource = HashMap::from([(11, 0), (22, 0x0000_00e8)]);
    app.host_local_player_info_ids = HashSet::from([1, 2]);
    let aliased_remote = lobby_fixture!(player {
        id: 99,
        resource: Some(resource(22)),
    });
    main_assert_eq!(
        host_runtime_alternate_color(
            &app.host_local_alternate_colors_by_resource,
            &app.host_local_player_info_ids,
            &aliased_remote,
        ) =>
        Some(0),
        "a remote row sharing the resource ID still has native's wire default"
    );
    let (event_tx, mut commands) = install_network_commands(&mut app);
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_named:
            legacy(b"Later remote"),
            color: 0x0000_c800,
        )],
    );

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [admitted] = broadcasts.as_slice() else {
        panic!("expected one admitted packet, got {broadcasts:?}");
    };
    main_assert_eq!(admitted.client_id => 3);
    main_assert_eq!(admitted.players[0].id => 3);
    main_assert_eq!(app.control_player_infos.get(2).expect("shifted initial host remains retained").color => 0x0000_00e8);
}

#[test]
fn host_player_info_conflict_update_precedes_admission_and_converges_join_data() {
    // ResolvePlayerAttributeConflicts may change an older, lower-priority
    // packet while admitting a higher-priority player. SendUpdatedPlayers
    // submits those retained packets before the new packet's CID_PlrInfo;
    // each direct control updates the live JoinData projection in the same
    // order (src/C4Network2Players.cpp:203-239;
    // src/C4PlayerInfoConflicts.cpp:193-344).
    let same_name = clonk_engine::LegacyCString::from_bytes(b"Same".to_vec()).test_value();
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos.replace_snapshot(
        1,
        [lobby_fixture!(player_data:
            2,
            vec![lobby_fixture!(player_named:
                same_name.clone(),
                color: 0x00f4_0000,
                id: 1,
            )],
        )],
    );
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_named:
            same_name,
            color: 0x0000_00f4,
            league_score: 10,
        )],
    );

    app.test_network_events();

    let (broadcasts, published) = commands.take_team_control_updates();
    let [updated_existing, admitted] = broadcasts.as_slice() else {
        panic!("expected existing update followed by admitted PlayerInfo");
    };
    main_assert_eq!((updated_existing.client_id, admitted.client_id) => (2, 3));
    main_assert_eq!(updated_existing.players[0].forced_name.as_bytes() => b"Same (2)");
    main_assert!(admitted.players[0].forced_name.is_empty());
    main_assert_eq!(admitted.players[0].id => 2);
    main_assert!(app.control_player_infos.get(2).is_some());

    for info in broadcasts.clone() {
        event_tx
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                join_players_on_echo: Vec::new(),
                original: info.clone(),
                info,
            })
            .test_value();
    }
    app.test_network_events();

    main_assert_eq!(app.control_player_infos.get(1).expect("existing player remains retained").forced_name.as_bytes() => b"Same (2)");
    main_assert_eq!(app.control_player_infos.get(2).expect("new player control is applied").name.as_bytes() => b"Same");
    main_assert_eq!(published.len() => 2);
    let latest = published.last().test_value();
    main_assert_eq!(latest.parameters.player_infos.last_player_id => 2);
    let existing = latest
        .parameters
        .player_infos
        .clients
        .iter()
        .find(|client| client.client_id == 2)
        .test_value();
    main_assert_eq!(existing.players[0].forced_name.as_bytes() => b"Same (2)");
    let admitted = latest
        .parameters
        .player_infos
        .clients
        .iter()
        .find(|client| client.client_id == 3)
        .test_value();
    main_assert_eq!(admitted.players[0].id => 2);
    main_assert_eq!(published.last() => app.host_join_snapshot.as_ref());
}

#[test]
fn updated_existing_echo_cannot_issue_new_admission_before_its_echo() {
    let same_name = clonk_engine::LegacyCString::from_bytes(b"Same".to_vec()).test_value();
    let resource = lobby_fixture!(player_resource:
        17,
        clonk_engine::LegacyCString::from_bytes(b"New.c4p".to_vec()).test_value(),
    );
    let mut app = new_state_only_running_sandbox_app();
    app.control_clients.register(3, true, false);
    app.control_player_infos.replace_snapshot(
        1,
        [lobby_fixture!(player_data:
            3,
            vec![lobby_fixture!(player_named:
                same_name.clone(),
                color: 0x00f4_0000,
                id: 1,
            )],
        )],
    );
    app.admission_resources.register_lobby_resource(&resource);
    app.admission_resources
        .mark_complete(resource.id, PathBuf::from("New.c4p"));
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_named:
            same_name,
            color: 0x0000_00f4,
            league_score: 10,
            resource: Some(resource),
        )],
    );

    app.test_network_events();
    let controls = commands.take_preexecuted_player_infos();
    let [(updated_existing, updated_ids), (admitted, admitted_ids)] = controls.as_slice() else {
        panic!("expected non-joining update echo before joining admission echo");
    };
    main_assert!(updated_ids.is_empty());
    main_assert_eq!(admitted_ids.iter().map(|player| player.id).collect::<Vec<_>>() => vec![1, 2]);

    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: updated_existing.clone(),
            info: updated_existing.clone(),
            join_players_on_echo: updated_ids.clone(),
        })
        .test_value();
    app.test_network_events();
    main_assert!(commands.take_submitted_join_players().is_empty());

    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: admitted.clone(),
            info: admitted.clone(),
            join_players_on_echo: admitted_ids.clone(),
        })
        .test_value();
    app.test_network_events();
    let joins = commands.take_submitted_join_players();
    main_assert_eq!(joins.len() => 1);
    main_assert_eq!(joins[0].1.info_id => admitted.players[0].id);
}

#[test]
fn delayed_admission_joins_parent_before_other_client_rebalance_follow_up() {
    let resource = |id, filename: &[u8]| lobby_fixture!(player_resource: id, clonk_engine::LegacyCString::from_bytes(filename.to_vec()).test_value());
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
        [lobby_fixture!(player_data: 4, vec![other_unjoined, joined_twenty, joined_thirty])],
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
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_bytes: b"Parent", color: 0x00f4_f400,
            flags: fixed,
            resource: Some(parent_resource),
        )],
    );

    app.test_network_events();
    let controls = commands.take_preexecuted_player_infos();
    main_assert_eq!(controls.iter().map(|(info, _)| info.client_id).collect::<Vec<_>>() => vec![3, 4]);
    main_assert_eq!(controls.iter().map(|(_, players)| players.iter().map(|player| player.id).collect::<Vec<_>>()).collect::<Vec<_>>() => vec![vec![31], vec![10]]);
    for (info, join_players_on_echo) in controls {
        event_tx
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                original: info.clone(),
                info,
                join_players_on_echo,
            })
            .test_value();
    }

    app.test_network_events();
    main_assert_eq!(
        commands
            .take_submitted_join_players()
            .into_iter()
            .map(|(_, join)| (join.at_client, join.info_id))
            .collect::<Vec<_>>() =>
        vec![(3, 31), (4, 10)]
    );
}

#[test]
fn host_updated_admission_emits_clean_full_follow_up_after_direct_apply() {
    let same_name = clonk_engine::LegacyCString::from_bytes(b"Same".to_vec()).test_value();
    let mut app = new_state_only_running_sandbox_app();
    app.control_player_infos.replace_snapshot(
        1,
        [lobby_fixture!(player_data:
            2,
            vec![lobby_fixture!(player_named:
                same_name.clone(),
                color: 0x00f4_0000,
                id: 1,
                league_score: 10,
            )],
        )],
    );
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.host_join_snapshot = clonk_network::HostConfig::default().initial_join_snapshot;
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_named:
            same_name,
            color: 0x0000_00f4,
        )],
    );
    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [admitted, clean_follow_up] = broadcasts.as_slice() else {
        panic!("expected updated admission and clean full follow-up");
    };
    main_assert_ne!(admitted.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED => 0);
    main_assert_eq!(admitted.players[0].forced_name.as_bytes() => b"Same (2)");
    main_assert_eq!(clean_follow_up.client_id => 3);
    main_assert_eq!(clean_follow_up.flags & (clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED) => 0);
    main_assert_eq!(clean_follow_up.players => admitted.players);
    main_assert!(app.control_player_infos.get(2).is_some());
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
        name: clonk_engine::LegacyCString::from_bytes(format!("Team {id}").into_bytes())
            .test_value(),
        player_start_index: 0,
        player_ids,
        color,
        icon_spec: clonk_engine::LegacyCString::default(),
        max_players: 0,
    };
    let mut app = new_state_only_menu_app(320, 200);
    app.control_player_infos.replace_snapshot(
        1,
        [lobby_fixture!(player_data:
            0,
            vec![lobby_fixture!(player {
                id: 1,
                name: clonk_engine::LegacyCString::from_bytes(b"Existing".to_vec())
                    .expect("valid existing player name"),
                team: 1,
            })],
        )],
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
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(host_lobby_state());
    let original_color = 0x0012_3456;
    send_added_player_infos(
        &event_tx,
        9,
        3,
        vec![lobby_fixture!(player_named:
            clonk_engine::LegacyCString::from_bytes(b"New".to_vec())
                .expect("valid new player name"),
            color: original_color,
        )],
    );

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [info] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo broadcast");
    };
    let [player] = info.players.as_slice() else {
        panic!("expected one admitted player");
    };
    main_assert_eq!((player.id, player.team) => (2, 2));
    main_assert_eq!((player.color, player.original_color) => (0x0000_c800, original_color));
    let teams = some_mut(&mut app.network_team_assignment).teams_mut();
    main_assert_eq!(teams.teams[0].player_ids => vec![1]);
    main_assert_eq!(teams.teams[1].player_ids => vec![2]);
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
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(host_lobby_state());
    app.mode = AppMode::Loading;
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_bytes:
            b"Loading",
            color: 0x0012_3456,
        )],
    );

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [info] = broadcasts.as_slice() else {
        panic!("expected one authoritative PlayerInfo");
    };
    main_assert_eq!(info.players[0].team => 0);
    let teams = some(&app.network_team_assignment).teams();
    main_assert!(teams.teams.iter().all(|team| team.player_ids.is_empty()));
}

#[test]
fn lobby_admission_does_not_gain_join_eligibility_before_delayed_echo() {
    let mut app = new_state_only_menu_app(320, 200);
    app.control_clients.register(3, true, false);
    let resource = lobby_fixture!(player_resource:
        17,
        clonk_engine::LegacyCString::from_bytes(b"Lobby.c4p".to_vec()).test_value(),
    );
    app.admission_resources.register_lobby_resource(&resource);
    app.admission_resources
        .mark_complete(resource.id, PathBuf::from("Lobby.c4p"));
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(host_lobby_state());
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_bytes: b"Lobby", color: 0x0012_3456,
            resource: Some(resource),
        )],
    );

    app.test_network_events();
    let controls = commands.take_preexecuted_player_infos();
    let [(info, join_players)] = controls.as_slice() else {
        panic!("expected one preexecuted lobby admission");
    };
    main_assert!(join_players.is_empty());

    app.mode = AppMode::Running;
    event_tx
        .send(NetworkEvent::PreexecutedPlayerInfoEcho {
            original: info.clone(),
            info: info.clone(),
            join_players_on_echo: join_players.clone(),
        })
        .test_value();
    app.test_network_events();
    main_assert!(commands.take_submitted_join_players().is_empty());
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
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(host_lobby_state());
    for (client_id, name, color) in [
        (3, b"First".as_slice(), 0x00f4_0000),
        (4, b"Second".as_slice(), 0x0000_00f4),
    ] {
        send_added_player_infos(
            &event_tx,
            client_id as u32,
            client_id,
            vec![lobby_fixture!(player_bytes: name, color: color, team: 2)],
        );
    }

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    main_assert_eq!(broadcasts.iter().map(|info| info.players[0].team).collect::<Vec<_>>() => vec![2, 1]);
    for info in &broadcasts {
        event_tx
            .send(NetworkEvent::PreexecutedPlayerInfoEcho {
                original: info.clone(),
                info: info.clone(),
                join_players_on_echo: Vec::new(),
            })
            .test_value();
    }
    app.test_network_events();
    let (_, clients) = app.control_player_infos.retained_rows_snapshot();
    main_assert_eq!(clients.iter().map(|(_, _, players)| players.len()).sum::<usize>() => 2, "preexecuted AddPlayers echoes must not duplicate retained rows");
}

#[test]
fn admission_that_requires_generated_team_generates_and_broadcasts() {
    let mut app = new_state_only_menu_app(320, 200);
    app.control_player_infos.replace_snapshot(
        1,
        [lobby_fixture!(player_data:
            0,
            vec![lobby_fixture!(player_bytes: b"Existing", color: 0x00f4_0000,
                id: 1,
                team: 1,
            )],
        )],
    );
    let metadata = set_control_test_metadata(true, vec![set_control_test_team(1, vec![1], 0)]);
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(host_lobby_state());
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_bytes:
            b"Generated",
            color: 0x0000_00f4,
        )],
    );

    app.test_network_events();

    let broadcasts = commands.take_broadcast_player_infos();
    let [admitted] = broadcasts.as_slice() else {
        panic!("expected one generated-team admission, got {broadcasts:?}");
    };
    main_assert_eq!((admitted.players[0].id, admitted.players[0].team) => (2, 2));
    let teams = some(&app.network_team_assignment).teams();
    main_assert_eq!(teams.teams.len() => 2);
    main_assert_eq!(teams.teams[1].id => 2);
    main_assert_eq!(teams.teams[1].name.as_bytes() => b"Team 2");
    main_assert_eq!(teams.teams[1].player_ids => vec![2]);
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
        .test_value();
    app.network_team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    send_added_player_infos(
        &event_tx,
        3,
        3,
        vec![lobby_fixture!(player_bytes: b"Joined", color: 0x00f4_0000,
            id: 41,
            flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            team: 0,
        )],
    );

    app.test_network_events();

    let live = app.engine.player(17).test_value();
    main_assert_eq!(live.team() => Some(2));
    main_assert_eq!(live.color() => Some(RgbColor::new(0, 0, 0xf4)));
    let broadcasts = commands.take_broadcast_player_infos();
    main_assert_eq!(broadcasts[0].players[0].team => 2);
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
        .test_value();

    let before = app.engine.snapshot().players.len();
    app.engine
        .call_scenario_script_function("RaiseLimitAndSpawn", Vec::new())
        .test_value();

    let globals = app.engine.snapshot().script_globals.named;
    main_assert_eq!(globals.get("set_result") => Some(&Value::Int(1)), "FnSetMaxPlayer has the C4ValueInt success result");
    main_assert_eq!(app.engine.max_players() => Some(2));
    main_assert_eq!(app.engine.snapshot().players.len() => before, "CreateScriptPlayer remains deferred until app admission");

    app.handle_script_player_info_updates().test_value();

    main_assert_eq!(app.network_max_players => 2, "the app admission cap follows Game.Parameters.MaxPlayers");
    let admitted = app
        .control_player_infos
        .client_info_ids(0)
        .into_iter()
        .filter_map(|id| app.control_player_infos.get(id))
        .find(|info| info.name.as_bytes() == b"Admitted Bot")
        .test_value();
    main_assert!(
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
    main_assert_ne!(app.engine.physics().gravity => 77);

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
    .test_value();

    main_assert_eq!(app.engine.physics().gravity => 77);
    main_assert_eq!(app.executing_ready_tick => None);
}

#[test]
fn synchronized_em_draw_tool_executes_at_the_ready_tick() {
    let mut app = new_state_only_running_sandbox_app();
    main_assert_ne!(app.engine.landscape().expect("sandbox landscape exists").mode() => clonk_engine::LANDSCAPE_MODE_EXACT);

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
    .test_value();

    main_assert_eq!(app.engine.landscape().expect("sandbox landscape remains").mode() => clonk_engine::LANDSCAPE_MODE_EXACT);
    main_assert_eq!(app.executing_ready_tick => None);
}

#[test]
fn synchronized_em_drop_def_executes_at_the_ready_tick() {
    let mut app = new_state_only_running_sandbox_app();
    let mut definition = Definition::from_script("DROP", "Drop", "#strict\n").test_value();
    definition.set_category(clonk_engine::CATEGORY_OBJECT);
    app.engine.register_definition(definition).test_value();
    main_assert!(app
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
    .test_value();

    let object = app
        .engine
        .first_active_object_for_definition("DROP")
        .and_then(|id| app.engine.object_snapshot(id))
        .test_value();
    main_assert_eq!(object.position => Vector2::new(23, 17));
    main_assert_eq!(object.owner => clonk_engine::OWNER_NONE);
    main_assert_eq!(app.executing_ready_tick => None);
}

#[test]
fn message_board_query_opens_on_tick35_and_routes_ui_answer_at_ready_tick() {
    let mut app = new_synthetic_running_sandbox_app();
    let player = app.local_owner;
    app.engine
        .player_mut(player)
        .test_value()
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
        .test_value();
    let target = app
        .engine
        .spawn_object(SpawnConfig::new("MBUI"))
        .test_value();
    let target_index = app.engine.find_object_index(target).test_value();
    main_assert_eq!(app.engine.call_object_function(target_index, "Open", vec![Value::Int(player)]).expect("query opens") => Value::Bool(true));

    let updates_to_tick35 = 35 - app.engine.frame() % 35;
    for update in 1..updates_to_tick35 {
        app.update()
            .unwrap_or_else(|error| panic!("pre-activation update {update} succeeds: {error}"));
        main_assert!(app.engine.active_message_board_input().is_none());
        main_assert!(app.running_chat_controller().is_none());
    }
    app.test_update();
    main_assert_eq!(app.engine.frame() % 35 => 0);
    main_assert_eq!(app.engine.active_message_board_input().expect("engine query activates").prompt => "Exact prompt");
    let controller = app.running_chat_controller().test_value();
    main_assert_eq!(controller.message() => "Exact prompt");
    main_assert_eq!(controller.text() => "");

    let (_events, mut commands) = install_client_network_commands(&mut app, 0);
    for character in "mixed".chars() {
        app.test_text_input(character);
    }
    let submission_tick = app.local_control_submission_tick();
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.running_chat_controller().is_none());
    main_assert!(app.engine.active_message_board_input().is_none());
    let target_before_ready = app.engine.object_snapshot(target).test_value();
    main_assert_ne!(target_before_ready.local_vars.get("callback_count") => Some(&Value::Int(1)), "submission closes the local UI without running the callback");
    let mut submitted = commands.take_submitted_message_board_answers();
    main_assert_eq!(submitted.len() => 1);
    let (queued_tick, answer) = submitted.pop().test_value();
    main_assert_eq!(queued_tick => submission_tick);
    main_assert_eq!(answer.answer.as_bytes() => b"MIXED");
    main_assert_eq!(answer.by_client => 0);

    app.apply_ready_controls(
        queued_tick,
        vec![NetworkControl::MessageBoardAnswer(answer)],
    )
    .test_value();
    let target_after_ready = app.engine.object_snapshot(target).test_value();
    main_assert_eq!(target_after_ready.local_vars.get("callback_answer") => Some(&Value::String("MIXED".to_string().into())));
    main_assert_eq!(target_after_ready.local_vars.get("callback_count") => Some(&Value::Int(1)));

    app.network = None;
    main_assert_eq!(app.engine.call_object_function(target_index, "Open", vec![Value::Int(player)]).expect("second query opens") => Value::Bool(true));
    let updates_to_tick35 = 35 - app.engine.frame() % 35;
    for _ in 0..updates_to_tick35 {
        app.test_update();
    }
    main_assert!(app.running_chat_controller().is_some());
    let (_events, mut commands) = install_client_network_commands(&mut app, 0);
    let cancel_tick = app.local_control_submission_tick();
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    let mut submitted = commands.take_submitted_message_board_answers();
    main_assert_eq!(submitted.len() => 1);
    let (queued_tick, answer) = submitted.pop().test_value();
    main_assert_eq!(queued_tick => cancel_tick);
    main_assert!(answer.answer.is_empty());
    app.apply_ready_controls(
        queued_tick,
        vec![NetworkControl::MessageBoardAnswer(answer)],
    )
    .test_value();
    main_assert!(app.running_chat_controller().is_none());
    main_assert!(app.engine.active_message_board_input().is_none());
    main_assert!(app
        .engine
        .player(player)
        .expect("local player remains after cancellation")
        .message_board_queries()
        .is_empty());
    let target_after_cancel = app.engine.object_snapshot(target).test_value();
    main_assert_eq!(
        target_after_cancel.local_vars.get("callback_count") =>
        Some(&Value::Int(1)),
        "an empty answer removes the query without invoking InputCallback"
    );
}

// One `C4MainMenu::MenuCommand` branch serves `Player:Goal:` and
// `Player:Rule:` alike: it closes the menu, looks the object up, and queues
// `CID_ActivateGameGoalRule` only when `FindInternal` answers — otherwise it
// is a plain `return false` (src/C4MainMenu.cpp:885-896), not a diagnostic
// (clonk-org/clonk-rs#1200, clonk-org/clonk-rs#1201).
#[test]
fn a_goal_or_rule_activation_runs_its_object_and_a_vanished_one_does_nothing() {
    let mut app = new_running_sandbox_app();
    let player = app.local_owner;
    for (id, name) in [("IGOL", "Integrated Goal"), ("IRUL", "Integrated Rule")] {
        let mut definition = test_definition(
            id,
            name,
            "#strict 3\nfunc Activate(int plr) { SetWealth(plr, GetWealth(plr) + 5); }",
        );
        definition.set_category(C4D_GOAL);
        app.engine.register_test_definition(definition);
        app.engine
            .spawn_test_object(clonk_engine::SpawnConfig::new(id));
    }

    main_assert!(
        app.engine
            .first_active_object_for_definition("IGOL")
            .is_some(),
        "the goal object is live"
    );
    main_assert_eq!(app.engine.test_player(player).wealth() => 0, "baseline");
    // The live path is unchanged: the object's `Activate` runs.
    for (action, expected) in [
        (MenuAction::GoalInfo("IGOL".to_string()), 5),
        (MenuAction::RuleInfo("IRUL".to_string()), 10),
    ] {
        app.apply_ingame_menu_action_for_player(player, action)
            .test_value();
        main_assert_eq!(app.engine.test_player(player).wealth() => expected);
    }

    // A row whose object disappeared between drawing and selecting is not a
    // failure, and queues nothing.
    for action in [
        MenuAction::GoalInfo("MISS".to_string()),
        MenuAction::RuleInfo("MISS".to_string()),
    ] {
        let status = app.status_text.clone();
        app.apply_ingame_menu_action_for_player(player, action.clone())
            .unwrap_or_else(|error| {
                panic!("a vanished {action:?} object is not a failure: {error:?}")
            });
        main_assert_eq!(
            app.engine.test_player(player).wealth() => 10,
            "{action:?} activated nothing"
        );
        main_assert_eq!(app.status_text => status, "{action:?} wrote no status");
    }
}

#[test]
fn ready_ticks_follow_plus_and_minus_one_control_rate_changes() {
    let mut app = new_running_sandbox_app();
    let event_tx = install_network_stub(&mut app);
    app.network_control_clock = Some(NetworkControlClock::new(9, 2));
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(9, 2).test_value(),
    );

    send_ready_tick(
        &event_tx,
        9,
        vec![NetworkControl::Set(lobby_fixture!(control_set: 0, 1, 0))],
    );
    app.test_update();
    main_assert_eq!((app.engine.frame(), app.engine.control_rate()) => (1, 3));

    app.test_update();
    app.test_update();
    app.test_update();
    main_assert_eq!(app.engine.frame() => 3);
    main_assert_eq!(app.network_control_clock.map(NetworkControlClock::current_tick) => Some(10));

    send_ready_tick(
        &event_tx,
        10,
        vec![NetworkControl::Set(lobby_fixture!(control_set: 0, -1, 0))],
    );
    app.test_update();
    main_assert_eq!((app.engine.frame(), app.engine.control_rate()) => (4, 2));

    app.test_update();
    main_assert_eq!(app.engine.frame() => 4);
    main_assert_eq!(app.network_control_clock.map(NetworkControlClock::current_tick) => Some(11));
}

#[test]
fn host_lobby_sets_update_published_join_data_and_obey_fair_crew_lock() {
    let mut app = new_state_only_running_sandbox_app();
    let (_events, mut commands) = install_network_commands(&mut app);
    let mut snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    snapshot.parameters.fair_crew_forced = true;
    app.host_join_snapshot = Some(snapshot);
    app.engine.set_fair_crew_forced(false);
    let before = (app.engine.use_fair_crew(), app.engine.fair_crew_strength());

    app.execute_control_set(lobby_fixture!(control_set: 5, 777, 0));
    main_assert_eq!((app.engine.use_fair_crew(), app.engine.fair_crew_strength()) => before, "the synchronized FairCrewForced parameter owns the lobby gate");
    main_assert!(commands.take_published_join_snapshots().is_empty());

    some_mut(&mut app.host_join_snapshot)
        .parameters
        .fair_crew_forced = false;
    for (value_type, data) in [(0, 1), (1, 0), (2, 37), (3, 4), (4, 1), (5, 777)] {
        app.execute_control_set(clonk_network::LegacyControlSet {
            value_type,
            data,
            by_client: 0,
        });
    }

    let parameters = &some(&app.host_join_snapshot).parameters;
    main_assert_eq!(parameters.control_rate => 2);
    main_assert!(!parameters.allow_debug);
    main_assert_eq!(parameters.max_players => 37);
    main_assert_eq!(parameters.teams.team_distribution => 4);
    main_assert_eq!(parameters.teams.team_colors => 1);
    main_assert!(parameters.use_fair_crew);
    main_assert_eq!(parameters.fair_crew_strength => 777);
    let published = commands.take_published_join_snapshots();
    main_assert_eq!(published.len() => 6);
    main_assert_eq!(published.last() => app.host_join_snapshot.as_ref());
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
    .test_value();
    definition.set_crew_member(true);
    definition.set_physical(clonk_engine::PhysicalInfo {
        energy: 50_000,
        scale: 30_000,
        hangle: 30_000,
        swim: 60_000,
        fight: 50_000,
        ..clonk_engine::PhysicalInfo::default()
    });
    app.engine.register_definition(definition).test_value();
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
        .test_value();
    let crew = app
        .engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "FCRW")
        .map(|object| object.id)
        .test_value();
    let crew_index = app.engine.find_object_index(crew).test_value();
    let before = app.engine.object_physical(crew_index);
    main_assert_eq!(before.energy => 55_001);
    main_assert_eq!(before.scale => 33_500);

    app.apply_ready_controls(
        14,
        vec![NetworkControl::Set(
            lobby_fixture!(control_set: 5, 1_000, 0),
        )],
    )
    .test_value();

    let after = app.engine.object_physical(crew_index);
    main_assert_eq!(after.energy => 55_002);
    main_assert_eq!(after.scale => 33_500);
    main_assert!(app.engine.use_fair_crew());
    main_assert_eq!(app.engine.fair_crew_strength() => 1_000);
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
        app.engine.tick().test_value();
    }
    let (_events, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_control_running = false;
    app.control_clients.register(3, true, false);

    app.test_update();
    main_assert!(commands.take_submitted_client_updates().is_empty());

    app.engine.tick().test_value();
    app.test_update();
    let expected = lobby_fixture!(client_update: clonk_engine::CLIENT_UPDATE_ACTIVATE, 3, 0, 0);
    main_assert_eq!(commands.take_submitted_client_updates() => vec![expected.clone()]);
    main_assert!(
        app.control_clients.is_activated(3),
        "submission must not bypass synchronized execution"
    );

    app.sec1_timer().test_value();
    main_assert_eq!(commands.take_submitted_client_updates() => vec![expected]);
}

#[test]
fn host_elimination_deactivates_remote_client_after_last_player() {
    // C4Player::Eliminate queues a synchronized CUT_Activate(false) only
    // after its last non-eliminated player at a remote client is gone
    // (oracle 7d43b47b7d789b533f32d005e64596e0a07019cd
    // src/C4Player.cpp:2015-2037).
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients.register(3, true, false);
    app.engine
        .register_player(PlayerConfig::new(17, "Remote").with_player_info_id(41))
        .test_value();
    app.engine
        .test_player_mut(17)
        .set_at_client(clonk_engine::PlayerAtClient::new(3));

    let tick = app.local_control_submission_tick();
    app.apply_ready_controls(
        tick,
        vec![NetworkControl::EliminatePlayer(
            clonk_engine::EliminatePlayerControlData {
                player: 17,
                by_client: 0,
            },
        )],
    )
    .test_value();

    main_assert_eq!(commands.take_submitted_client_updates() => vec![lobby_fixture!(client_update: clonk_engine::CLIENT_UPDATE_ACTIVATE, 3, 0, 0)]);
    main_assert_eq!(app.engine.player(17).map(clonk_engine::Player::status) => Some(clonk_engine::PlayerStatus::Eliminated));
    main_assert_eq!(app.engine.snapshot().eliminated_crew_owners => vec![17]);
    main_assert!(
        app.control_clients.is_activated(3),
        "the synchronized update must not execute locally at submission time"
    );
}

#[test]
fn host_automatic_elimination_deactivates_remote_client_after_last_player() {
    // C4Player::Execute reaches C4Player::Eliminate on the Tick35 boundary;
    // its synchronized early deactivation must not wait for player retirement
    // (oracle 7d43b47b7d789b533f32d005e64596e0a07019cd
    // src/C4Player.cpp:2015-2037).
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients.register(3, true, false);
    app.engine
        .register_player(PlayerConfig::new(17, "Remote").with_player_info_id(41))
        .test_value();
    app.engine
        .test_player_mut(17)
        .set_at_client(clonk_engine::PlayerAtClient::new(3));
    main_assert!(app.engine.is_control_host());
    main_assert!(!app.engine.player(17).test_value().no_elimination_check());
    main_assert_eq!(app.engine.player(17).test_value().status() => clonk_engine::PlayerStatus::Active);

    for _ in 0..35 {
        app.engine.tick().test_value();
    }
    main_assert_eq!(app.engine.player(17).map(clonk_engine::Player::status) => Some(clonk_engine::PlayerStatus::Eliminated));
    app.flush_pending_client_updates();

    main_assert_eq!(commands.take_submitted_client_updates() => vec![lobby_fixture!(client_update: clonk_engine::CLIENT_UPDATE_ACTIVATE, 3, 0, 0)]);
}

#[test]
fn host_elimination_does_not_deactivate_host_client() {
    // C4Player::Eliminate gates this update on AtClient > C4ClientIDHost, so
    // eliminating the host's last player leaves the host active
    // (oracle 7d43b47b7d789b533f32d005e64596e0a07019cd
    // src/C4Player.cpp:2023-2035).
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients.register(0, true, false);
    app.engine
        .register_player(PlayerConfig::new(17, "Host").with_player_info_id(41))
        .test_value();
    app.engine
        .test_player_mut(17)
        .set_at_client(clonk_engine::PlayerAtClient::HOST);

    app.apply_ready_controls(
        app.local_control_submission_tick(),
        vec![NetworkControl::EliminatePlayer(
            clonk_engine::EliminatePlayerControlData {
                player: 17,
                by_client: 0,
            },
        )],
    )
    .test_value();

    main_assert!(commands.take_submitted_client_updates().is_empty());
    main_assert!(app.control_clients.is_activated(0));
}

#[test]
fn host_elimination_keeps_remote_client_active_with_another_player() {
    // C4Player::Eliminate scans every player at AtClient and suppresses the
    // update when another player is still not eliminated
    // (oracle 7d43b47b7d789b533f32d005e64596e0a07019cd
    // src/C4Player.cpp:2026-2035).
    let mut app = new_running_sandbox_app();
    let (_events, mut commands) = install_network_commands(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients.register(3, true, false);
    for player in [17, 18] {
        app.engine
            .register_player(PlayerConfig::new(player, format!("Remote {player}")))
            .test_value();
        app.engine
            .test_player_mut(player)
            .set_at_client(clonk_engine::PlayerAtClient::new(3));
    }

    app.apply_ready_controls(
        app.local_control_submission_tick(),
        vec![NetworkControl::EliminatePlayer(
            clonk_engine::EliminatePlayerControlData {
                player: 17,
                by_client: 0,
            },
        )],
    )
    .test_value();

    main_assert!(commands.take_submitted_client_updates().is_empty());
    main_assert_eq!(app.engine.player(18).map(clonk_engine::Player::status) => Some(clonk_engine::PlayerStatus::Active));
    main_assert!(app.control_clients.is_activated(3));
}

#[test]
fn frozen_lobby_executes_synchronized_activation_immediately() {
    // HandleControlPkt executes synchronized controls immediately while
    // the network is frozen in GS_Lobby, rather than waiting for a game
    // simulation tick (pristine 9ffa0a5d
    // src/C4GameControlNetwork.cpp:558-588).
    let directory = tempdir();
    let mut app = new_state_only_menu_app(320, 200);
    install_test_recording_template(&mut app, directory.path().join("001-FrozenLobbySync.c4s"));
    let event_tx = install_network_stub(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.network_lobby = Some(NetworkLobbyState::new(0, "Host".to_string(), false));
    app.startup.view = StartupView::NetworkLobby;
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
                NetworkControl::ClientUpdate(lobby_fixture!(client_update:
                    clonk_engine::CLIENT_UPDATE_ACTIVATE,
                    3,
                    1,
                    0,
                )),
            ],
        })
        .test_value();

    app.test_network_events();

    main_assert!(app.control_clients.is_activated(3));
    main_assert!(app.network_sync.scheduled.is_empty());
    main_assert!(app.recording.is_none());
    main_assert!(app.recording_template.is_some());
}

#[test]
fn frozen_classic_host_lobby_executes_synchronized_activation_immediately() {
    // The exact classic-host projection is the same frozen GS_Lobby
    // control state as the client lobby (src/C4GameControlNetwork.cpp:558-588).
    let mut app = new_menu_app(320, 200);
    let event_tx = install_network_stub(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    install_test_classic_host_lobby(&mut app);
    app.control_clients.register(3, false, false);
    event_tx
        .send(NetworkEvent::ScheduledSync {
            tick: 0,
            controls: vec![NetworkControl::ClientUpdate(
                lobby_fixture!(client_update: clonk_engine::CLIENT_UPDATE_ACTIVATE, 3, 1, 0),
            )],
        })
        .test_value();

    app.test_network_events();

    main_assert!(app.control_clients.is_activated(3));
    main_assert!(app.network_sync.scheduled.is_empty());
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
        .test_value();
    app.engine
        .player_mut(app.local_owner)
        .test_value()
        .control
        .control_style = true;
    app.engine
        .player_mut(remote_owner)
        .test_value()
        .control
        .control_style = true;
    let event_tx = install_network_stub(&mut app);

    let tick = u32::try_from(app.engine.frame()).test_value();
    let initial_frame = app.engine.frame();
    app.dispatch_control_event(ControlEvent::Press(ControlButton::Right))
        .test_value();
    main_assert_eq!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_RIGHT) =>
        0,
        "network input is submitted but not dispatched immediately"
    );

    app.test_update();
    main_assert_eq!(app.engine.frame() => initial_frame, "simulation cannot advance before its exact ready tick");

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
    send_ready_tick(&event_tx, tick, controls.clone());
    app.test_update();
    main_assert_eq!(app.engine.frame() => initial_frame + 1);
    main_assert_ne!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_RIGHT) =>
        0,
        "local aggregate control is applied"
    );
    main_assert_ne!(
        app.engine
            .player(remote_owner)
            .expect("remote player")
            .control
            .pressed_coms
            & (1 << clonk_engine::COM_LEFT) =>
        0,
        "remote aggregate control is applied"
    );
    let local_last_com = app
        .engine
        .player(app.local_owner)
        .test_value()
        .control
        .last_com;

    send_ready_tick(&event_tx, tick, controls);
    app.test_update();
    main_assert_eq!(app.engine.frame() => initial_frame + 1, "a stale duplicate cannot advance simulation twice");
    main_assert_eq!(
        app.engine
            .player(app.local_owner)
            .expect("local player")
            .control
            .last_com =>
        local_last_com,
        "a stale duplicate cannot dispatch its controls twice"
    );
}

#[test]
fn synchronized_player_select_executes_at_the_ready_tick() {
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let first = app.engine.crew_cursor(owner).test_value();
    let definition = app.engine.object_snapshot(first).test_value().definition_id;
    let second = app
        .engine
        .spawn_object(
            SpawnConfig::new(definition)
                .with_owner(owner)
                .with_crew_member(true),
        )
        .test_value();
    app.engine.select_crew(owner, [first, second]).test_value();
    let (event_tx, _commands) = install_client_network_commands(&mut app, 7);
    let tick = app.local_control_submission_tick();
    send_ready_tick(
        &event_tx,
        tick,
        vec![NetworkControl::PlayerSelect(PlayerSelectControlData {
            player: owner,
            objects: vec![second.as_u64() as i32],
            by_client: 7,
        })],
    );

    app.test_update();

    main_assert_eq!(app.engine.selected_crew(owner) => vec![second]);
    main_assert_eq!(app.engine.crew_cursor(owner) => Some(second));
    let stats = some(&app.network_stats);
    let controls = stats.player_control_graph(owner).test_value();
    let actions = stats.player_action_graph(owner).test_value();
    main_assert_eq!(controls.raw_value(controls.end_time() - 1) => 1.0);
    main_assert_eq!(actions.raw_value(actions.end_time() - 1) => 1.0);
    let player = app.engine.player(owner).test_value();
    main_assert_eq!((player.control_count(), player.action_count()) => (0, 0), "the running statistics sample drains native per-control-frame counters");
}

#[test]
fn ready_tick_follow_on_uses_next_open_tick_and_clears_marker() {
    // Input generated while C4Control::Execute is running goes back into
    // Game.Input for the next unsent control tick
    // (src/C4GameControl.cpp:314-318; src/C4GameControlNetwork.cpp:145-176).
    let mut app = new_running_sandbox_app();
    let (event_tx, mut commands) = install_network_commands(&mut app);
    app.open_ingame_menu().test_value();
    main_assert!(
        commands.take_submitted_local().is_empty(),
        "opening the main menu does not clear controls"
    );
    let tick = u32::try_from(app.engine.frame()).test_value();
    send_ready_tick(
        &event_tx,
        tick,
        vec![NetworkControl::Player {
            owner: app.local_owner,
            event: ControlEvent::Command {
                command: ControlCommand::MenuClose,
                kind: CommandKind::Press,
            },
        }],
    );

    app.test_update();

    main_assert_eq!(
        commands.take_submitted_local() =>
        vec![(
            app.local_owner,
            ControlEvent::ClearPressed,
            tick.saturating_add(1),
        )],
        "a reentrant follow-on targets the next open tick"
    );
    main_assert_eq!(app.executing_ready_tick => None, "the ready marker is cleared after application");
}
