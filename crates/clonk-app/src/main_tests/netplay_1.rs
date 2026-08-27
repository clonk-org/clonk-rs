// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! n1_fixture {
    (player { $($field:ident $(: $value:expr)?),* $(,)? }) => {
        clonk_engine::ControlPlayerInfoEntry {
            $($field $(: $value)?,)*
            ..Default::default()
        }
    };
    (player_data { $($field:ident $(: $value:expr)?),* $(,)? }) => {
        clonk_engine::PlayerInfoControlData {
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
}

fn n1_picture_snapshot(
    definition_id: &str,
    blit_mode: u32,
    color: u32,
    color_modulation: u32,
) -> clonk_engine::ObjectMenuPictureSnapshot {
    clonk_engine::ObjectMenuPictureSnapshot {
        definition_id: definition_id.to_string(),
        symbol_size: 1,
        base_graphics: None,
        graphics_overlays: Vec::new(),
        blit_mode,
        color,
        color_modulation,
        picture_rect: clonk_engine::DefinitionRect::default(),
        rank: None,
    }
}

fn n1_register_picture(
    engine: &mut Engine,
    id: &str,
    name: &str,
    pixels: [u8; 4],
    color_mask: Option<[u8; 4]>,
) -> clonk_engine::DefinitionPictureImage {
    let mut definition = test_definition(id, name, "");
    definition.set_picture(Some(clonk_engine::DefinitionPicture {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }));
    definition.set_sprite_image(Some(clonk_engine::DefinitionSpriteImage {
        width: 1,
        height: 1,
        pixels: Arc::from(pixels),
        color_mask: color_mask.map(Arc::from),
    }));
    engine.register_test_definition(definition);
    engine.definition_picture_phase_image(id, 0).test_value()
}

fn n1_test_paths(content_dir: Option<&Path>) -> (tempfile::TempDir, EnvGuard, AppPaths) {
    let user_data = tempdir();
    let (guard, paths) = exact_loader_test_paths(user_data.path(), content_dir);
    (user_data, guard, paths)
}

fn n1_repository_paths() -> (tempfile::TempDir, EnvGuard, AppPaths) {
    let user_data = tempdir();
    let guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(test_repository_root())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
    ]);
    let paths = test_app_paths();
    (user_data, guard, paths)
}

fn n1_joined_client_app() -> GameApp {
    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.sync_network_lobby_game_option_state();
    app
}

fn n1_joined_client_app_with_commands() -> (GameApp, network::TestNetworkCommands) {
    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    let (network, _events, commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    (app, commands)
}

fn n1_install_loading_state(app: &mut GameApp) {
    let resources = app.loader_screen.test_ref().resources().clone();
    let (_sender, receiver) = mpsc::channel();
    app.loading_state = Some(ScenarioLoadingState::new(
        FrontendScenario::fallback(),
        resources,
        HashMap::new(),
        Vec::new(),
        receiver,
    ));
    app.mode = AppMode::Loading;
}

fn n1_text_command<'a>(
    plan: &'a NativePresentationPlan,
    needle: &str,
) -> (usize, &'a clonk_graphics::clonk_font::CapturedClonkText) {
    plan.batches
        .iter()
        .enumerate()
        .find_map(|(index, batch)| {
            batch
                .text
                .iter()
                .find(|command| command.text == needle)
                .map(|command| (index, command))
        })
        .unwrap_or_else(|| panic!("captured native text `{needle}`"))
}

fn n1_text_batch(plan: &NativePresentationPlan, needle: &str) -> usize {
    n1_text_command(plan, needle).0
}

fn n1_loader_batch(plan: &NativePresentationPlan) -> usize {
    plan.batches
        .iter()
        .position(|batch| batch.native_loader_text)
        .test_value()
}

fn n1_press_and_release_key(app: &mut GameApp, key: VirtualKeyCode) {
    app.test_key(key, ElementState::Pressed);
    app.test_key(key, ElementState::Released);
}

fn n1_restore_player_infos(id: i32) -> clonk_network::PlayerInfoListSnapshot {
    clonk_network::PlayerInfoListSnapshot {
        last_player_id: id,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: vec![n1_fixture!(player {
                id,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
            })],
        }],
    }
}

fn n1_lobby_start_request(confirm_unassociated_savegame_players: bool) -> Vec<ClassicLobbyAction> {
    vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: 5,
        check_league_rules: true,
        confirm_unassociated_savegame_players,
    }]
}

fn n1_lobby_option(
    app: &GameApp,
    kind: LobbyOptionKind,
) -> Option<&clonk_frontend::game_lobby::LobbyOptionRow> {
    app.classic_host_lobby
        .test_ref()
        .controller
        .option_rows()
        .iter()
        .find(|row| row.kind == kind)
}

fn n1_wait_client(
    app: &GameApp,
    client_id: i32,
) -> &clonk_frontend::network_start_wait::NetworkStartWaitClient {
    app.network_start_wait
        .test_ref()
        .controller
        .clients()
        .iter()
        .find(|client| client.client_id == client_id)
        .test_value()
}

fn n1_gamepad_direction(button: ControlButton, state: ElementState) -> GamepadEvent {
    GamepadEvent::Direction {
        slot: GamepadSlot::new(0),
        button,
        state,
    }
}

fn n1_gamepad_button(class: GuiButtonClass, state: ElementState) -> GamepadEvent {
    GamepadEvent::GuiButton {
        slot: GamepadSlot::new(0),
        class,
        state,
    }
}

fn n1_gamepad_action(action: GamepadActionType, state: ElementState) -> GamepadEvent {
    GamepadEvent::Action {
        slot: GamepadSlot::new(0),
        action,
        state,
    }
}

fn n1_select_empty_startup_view(app: &mut GameApp, view: StartupView) {
    // Exhaustive arms make a future StartupView addition update the all-root
    // regressions rather than silently escaping the audit.
    match view {
        StartupView::MainMenu => app.startup.view = StartupView::MainMenu,
        StartupView::ScenarioBrowser => app.startup.view = StartupView::ScenarioBrowser,
        StartupView::NetworkLobby => {
            app.startup.view = StartupView::NetworkLobby;
            app.classic_host_lobby = None;
        }
        StartupView::NetworkGame => {
            app.startup.view = StartupView::NetworkGame;
            app.startup_network_dialog = None;
        }
        StartupView::Options => {
            app.startup.view = StartupView::Options;
            app.startup.options_dialog = None;
        }
        StartupView::About => {
            app.startup.view = StartupView::About;
            app.startup.about_dialog = None;
        }
        StartupView::PlayerSelection => {
            app.startup.view = StartupView::PlayerSelection;
            app.startup.player_dialog = None;
        }
    }
}

fn n1_expect<'a, T>(value: &'a Option<T>, message: &str) -> &'a T {
    value.as_ref().expect(message)
}

#[cfg(unix)]
#[test]
fn definition_paths_round_trip_native_bytes_at_string_boundaries() {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let native_path = PathBuf::from(OsString::from_vec(b"Defs-\xe4-\xff.c4d".to_vec()));
    let projected = path_as_legacy_text(&native_path);
    main_assert_eq!(clonk_script::c4_string_bytes(&projected) => native_path.as_os_str().as_bytes());
    main_assert_eq!(path_from_group_name_bytes(&clonk_script::c4_string_bytes(&projected)) => native_path);

    let mut config = b"[General]\nDefinitionPath=Custom-".to_vec();
    config.push(0xe4);
    config.extend_from_slice(b"/\n");
    let (_, definition_path) = game_save_definition_paths(None, &config);
    main_assert_eq!(clonk_script::c4_string_bytes(&definition_path) => b"Custom-\xe4/");
}

#[test]
fn initial_record_cleanup_matches_native_component_patterns() {
    let mut group = MutableGroup::new("Record.c4s");
    for name in [
        "Alice.C4P",
        "Title.bmp",
        "ICON.BMP",
        "Info.txt",
        "TitleUS.txt",
        "DescDE.rtf",
        "Title.png",
        "Icon.png",
        "Keep.bin",
    ] {
        group.add_file(name, name.as_bytes().to_vec()).test_value();
    }

    clean_initial_record_group(&mut group);

    let names = group.entry_names();
    main_assert_eq!(names.len() => 3);
    main_assert!(names.contains(&"Title.png"));
    main_assert!(names.contains(&"Icon.png"));
    main_assert!(names.contains(&"Keep.bin"));
}

#[test]
fn runtime_join_dynamic_requests_coalesce_until_a_newer_tick() {
    let mut pending = PendingRuntimeDynamicRequest::new(7, 23);
    main_assert!(pending.needs_synchronize());

    pending.synchronize_queued = true;
    pending.include(8, 23);
    main_assert_eq!(pending.client_ids => HashSet::from([7, 8]));
    main_assert!(!pending.needs_synchronize());

    pending.synchronize_queued = false;
    pending.synchronized_control_tick = Some(23);
    pending.include(9, 22);
    main_assert_eq!(pending.synchronized_control_tick => Some(23));
    main_assert!(!pending.needs_synchronize());

    pending.include(10, 24);
    main_assert_eq!(pending.requested_control_tick => 24);
    main_assert_eq!(pending.synchronized_control_tick => None);
    main_assert!(pending.needs_synchronize());
}

#[test]
fn delayed_join_data_needed_after_fanout_does_not_request_a_second_capture() {
    let null_dynamic = clonk_engine::NetworkResourceCore::default();
    let published_dynamic = n1_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Dynamic as u8,
        id: 17,
    });
    let mut capture_count = 0;

    // Client A has no usable dynamic, so one synchronized capture is queued.
    if !published_runtime_dynamic_covers_request(&null_dynamic, -1, 23) {
        capture_count += 1;
    }

    // SyncScheduled publishes tick 24 to A and waiting B before B's queued event.
    main_assert!(published_runtime_dynamic_covers_request(
        &published_dynamic,
        24,
        23,
    ));
    if !published_runtime_dynamic_covers_request(&published_dynamic, 24, 23) {
        capture_count += 1;
    }
    main_assert_eq!(capture_count => 1);

    // A newer request and a null resource still schedule regeneration.
    main_assert!(!published_runtime_dynamic_covers_request(
        &published_dynamic,
        24,
        25,
    ));
    main_assert!(!published_runtime_dynamic_covers_request(
        &null_dynamic,
        24,
        23,
    ));
}

#[test]
fn runtime_join_restore_infos_come_from_dynamic_not_join_data_parameters() {
    let restore_infos = |id: i32, name: &[u8]| clonk_network::PlayerInfoListSnapshot {
        last_player_id: id,
        clients: vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 3,
            flags: 0,
            players: vec![n1_fixture!(player {
                id,
                name: LegacyCString::from_bytes(name.to_vec()).expect("fixture player name"),
                player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                league_progress_data_is_null: false,
            })],
        }],
    };
    let packet_restore_infos = restore_infos(7, b"Packet restore");
    let dynamic_restore_infos = restore_infos(42, b"Dynamic restore");

    let directory = tempdir();
    let combined_path = directory.path().join("Combined9.c4s");
    let mut combined = MutableGroup::new("Combined9.c4s");
    combined
        .add_file(
            "Scenario.txt",
            b"[Head]\r\nNetworkGame=1\r\nNetworkRuntimeJoin=1\r\n".to_vec(),
        )
        .test_value();
    combined
        .add_file(
            "SavePlayerInfos.txt",
            clonk_network::encode_player_info_list_ini(&dynamic_restore_infos)
                .expect("encode dynamic restore infos"),
        )
        .test_value();
    fs::write(&combined_path, combined.pack().test_value()).test_value();
    let combined = Group::open(&combined_path).test_value();

    let selected = client_network_restore_player_infos(
        true,
        &combined,
        &packet_restore_infos,
        &[],
        &LanguagePacks::default(),
    );
    main_assert_eq!(selected.last_player_id => 42);
    main_assert_eq!(selected.clients[0].players[0].id => 42);
    main_assert_eq!(packet_restore_infos.last_player_id => 7);

    let ordinary = client_network_restore_player_infos(
        false,
        &combined,
        &packet_restore_infos,
        &[],
        &LanguagePacks::default(),
    );
    main_assert_eq!(ordinary.last_player_id => 7);
    main_assert_eq!(ordinary.clients[0].players[0].id => 7);

    let missing_path = directory.path().join("Combined10.c4s");
    let mut missing = MutableGroup::new("Combined10.c4s");
    missing
        .add_file(
            "Scenario.txt",
            b"[Head]\r\nNetworkGame=1\r\nNetworkRuntimeJoin=1\r\n".to_vec(),
        )
        .test_value();
    fs::write(&missing_path, missing.pack().test_value()).test_value();
    let missing = Group::open(&missing_path).test_value();
    let selected = client_network_restore_player_infos(
        true,
        &missing,
        &packet_restore_infos,
        &[],
        &LanguagePacks::default(),
    );
    main_assert_eq!(selected.last_player_id => 0);
    main_assert!(selected.clients.is_empty());
}

#[test]
fn console_input_and_open_parameters_follow_native_framing() {
    main_assert_eq!(
        parse_classic_console_parameters(
            "\"Missions/My Round/Scenario.txt\" /network /lobby:17 \"/comment:console game\"",
        ) =>
        [
            OsString::from("Missions/My Round/Scenario.txt"),
            OsString::from("/network"),
            OsString::from("/lobby:17"),
            OsString::from("/comment:console game"),
        ]
    );

    let (sender, receiver) = mpsc::channel();
    forward_console_input(
        std::io::Cursor::new(b"\n   \r\n/quit\r\nhello\tworld\nunterminated"),
        sender,
    )
    .test_value();
    let mut events = receiver.into_iter();
    main_assert!(
        matches!(events.next(), Some(ConsoleInputEvent::Command(command)) if command == "   ")
    );
    main_assert!(
        matches!(events.next(), Some(ConsoleInputEvent::Command(command)) if command == "/quit")
    );
    main_assert!(
        matches!(events.next(), Some(ConsoleInputEvent::Command(command)) if command == "helloworld")
    );
    main_assert!(matches!(events.next(), Some(ConsoleInputEvent::Eof)));
    main_assert!(events.next().is_none());
}

#[test]
fn classic_recdump_c4r_suffix_retains_both_native_interpretations() {
    let classic = parse_classic_command_line(&[OsString::from("/recdump:dump.c4r")]);
    main_assert_eq!(classic.record_dump.as_deref() => Some("dump.c4r"));
    main_assert_eq!(classic.record_stream => Some(PathBuf::from("/recdump:dump.c4r")));
}

#[test]
fn classic_command_line_direct_join_queries_the_first_reference() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").test_value();
    let reference_server = listener.local_addr().test_value();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().test_value();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .test_value();
        let mut request = [0_u8; 4096];
        let size = stream.read(&mut request).test_value();
        let request = String::from_utf8_lossy(&request[..size]).to_ascii_lowercase();
        main_assert!(request.starts_with("get / http/1.1"));
        main_assert!(request.contains("accept-language: de"));
        let body = "[Reference]\nTitle=Direct fixture\nState=Lobby\nJoinAllowed=1\nAddress=TCP:\"127.0.0.1:41234\"\nGame=LegacyClonk\nVersion=4,9,11,0\nBuild=362\n";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        )
        .test_value();
    });
    let config = clonk_network::ReferenceQueryConfig {
        language_sequence: "DE".to_string(),
        ..clonk_network::ReferenceQueryConfig::default()
    };

    let reference = query_first_classic_reference(
        clonk_network::ReferenceEndpoint::Address(reference_server),
        &config,
    )
    .test_value();

    server.test_join();
    main_assert_eq!(reference.title => "Direct fixture");
    main_assert_eq!(
        reference.addresses =>
        vec![clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Tcp,
            SocketAddr::from(([127, 0, 0, 1], 41_234)),
        )]
    );
}

#[test]
fn classic_command_line_passworded_reference_prompts_before_connecting() {
    let mut app = new_classic_menu_app(800, 600);
    app.classic_command_line = ClassicCommandLine {
        direct_join: Some("games.example.test".to_string()),
        ..ClassicCommandLine::default()
    };
    let attempts = vec![
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Tcp,
            "127.0.0.1:30111".parse().unwrap(),
        ),
        clonk_network::NetworkAddress::new(
            clonk_network::NetworkProtocol::Udp,
            "127.0.0.1:30112".parse().unwrap(),
        ),
    ];
    let settings =
        ClientSettings::new(attempts[0].endpoint, "Player").with_join_attempts(attempts.clone());
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(ClassicDirectReferenceQueryResult {
            settings,
            password_needed: true,
        }))
        .test_value();
    app.classic_direct_reference_query = Some(ClassicDirectReferenceQuery { receiver });
    app.mode = AppMode::Loading;
    let (boot_sender, boot_receiver) = mpsc::channel();
    app.boot_loading = Some(BootLoadingState::new(boot_receiver));

    app.poll_classic_direct_reference_query().test_value();
    main_assert!(app.classic_direct_reference_query.is_some());
    main_assert!(app.pending_network_join.is_none());
    main_assert!(app.game_option_input_dialog.is_none());

    boot_sender
        .send(BootLoadingEvent::Finished(None))
        .test_value();
    app.poll_boot_loading();
    main_assert!(app.boot_loading.is_none());
    main_assert_eq!(app.mode => AppMode::Loading);
    main_assert!(app.classic_direct_reference_query.is_some());

    app.poll_classic_direct_reference_query().test_value();

    main_assert!(app.classic_direct_reference_query.is_none());
    main_assert!(app.startup_network_connection.is_none());
    main_assert_eq!(app.pending_network_join.as_ref().expect("resolved join remains pending").server_addresses => attempts);
    main_assert_eq!(app.game_option_input_dialog.as_ref().expect("password prompt").purpose => PendingInputDialogPurpose::NetworkJoinPassword);

    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
        .test_value();
    main_assert!(app.pending_network_join.is_none());
}

#[cfg(unix)]
#[test]
fn save_description_preserves_native_definition_path_bytes() {
    use std::os::unix::ffi::OsStringExt;

    let native_path_bytes = b"Defs-\xff.c4f\\Pack.c4d".to_vec();
    let native_path = PathBuf::from(OsString::from_vec(native_path_bytes.clone()));
    main_assert_eq!(raw_definition_description_modules(&[native_path]).as_slice() => std::slice::from_ref(&native_path_bytes));
    let relative = developer_console_definition_description_path(&native_path_bytes, None);
    main_assert_eq!(relative.as_slice() => native_path_bytes.as_slice());

    let app = new_state_only_menu_app(320, 200);
    let (_, description) = app.classic_save_description(
        b"Raw definition path",
        &[native_path_bytes],
        ClassicSaveDescriptionKind::Savegame,
    );
    let mut escaped_path = b"Defs-\xff.c4f".to_vec();
    escaped_path.extend_from_slice(&[b'\\'; 4]);
    escaped_path.extend_from_slice(b"Pack.c4d");
    main_assert!(description
        .windows(escaped_path.len())
        .any(|window| window == escaped_path.as_slice()));
}

#[test]
fn optional_initial_network_game_source_distinguishes_missing_and_unreadable_entries() {
    let directory = tempdir();
    let group = Group::open(directory.path()).test_value();
    main_assert_eq!(read_optional_initial_network_game_source(&group).expect("missing is optional") => None);

    let game_path = directory.path().join("Game.txt");
    fs::write(&game_path, []).test_value();
    main_assert_eq!(read_optional_initial_network_game_source(&group).expect("empty is optional") => None);

    fs::write(&game_path, b"[Script]\nGlobals=1;42;").test_value();
    main_assert_eq!(read_optional_initial_network_game_source(&group).expect("read Game.txt") => Some(b"[Script]\nGlobals=1;42;".to_vec()));

    fs::remove_file(&game_path).test_value();
    fs::create_dir(&game_path).test_value();
    main_assert!(read_optional_initial_network_game_source(&group).is_err());
}

#[test]
fn context_command_coordinates_include_letterbox_and_ignore_camera_zoom() {
    let viewport = ActiveViewportProjection {
        index: 0,
        identity: None,
        owner: 1,
        is_no_owner_viewport: false,
        rect: Rect::new(10, 20, 160, 100),
        content_rect: Rect::new(30, 40, 120, 60),
        target_x: 0,
        target_y: 0,
        logical_width: 80,
        logical_height: 50,
        content_origin_x: 100.0,
        content_origin_y: -20.0,
        zoom: 2.0,
    };
    let pointer = ViewportPointer {
        owner: 1,
        screen: GuiPoint::new(71.0, 65.0),
        world: FloatVector2::new(120.5, -7.5),
    };

    main_assert_eq!(
        ingame_pointer_viewport_pixel(pointer, viewport) =>
        (61, 45),
        "C++ sends VpX/VpY relative to the full viewport output, including letterbox bars"
    );
}

#[test]
fn help_regions_share_one_native_caption_slot() {
    let (mut app, owner, _crew, _first, target, inventory_point) = inventory_region_fixture();
    app.live_input.ingame_mouse_help = true;

    app.test_cursor(PhysicalPosition::new(
        f64::from(inventory_point.x),
        f64::from(inventory_point.y),
    ));
    main_assert_eq!(
        app.ingame_mouse_help_caption
            .as_ref()
            .map(|caption| caption.text.as_str()) =>
        app.engine.object_help_caption(target).as_deref(),
        "a target-bearing region uses the Help tooltip caption"
    );
    main_assert!(
        app.live_input.ingame_mouse_caption.caption.is_none(),
        "the red object-name caption cannot coexist with the Help tooltip"
    );

    let help_button = viewport_button_point(&app, owner, clonk_frontend::hud::ViewportButton::Help);
    app.test_cursor(PhysicalPosition::new(
        f64::from(help_button.x),
        f64::from(help_button.y),
    ));
    main_assert!(
        app.ingame_mouse_help_caption.is_none(),
        "a targetless region replaces the prior Help tooltip"
    );
    let caption = app.live_input.ingame_mouse_caption.caption.test_ref();
    let expected = app.localized_ingame_mouse_caption("IDS_CON_HELP", "Help", &[], false);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let button_rect = clonk_frontend::hud::viewport_button_rect(
        viewport,
        clonk_frontend::hud::ViewportButton::Help,
    );
    main_assert_eq!(caption.text => expected);
    main_assert_eq!(caption.caption_bottom_y => Some(button_rect.y - viewport.y));
    main_assert_eq!(app.live_input.ingame_mouse_caption.cursor => IngameMouseCursorKind::Region);
}

#[test]
fn network_control_catch_up_latches_and_releases_render_skip() {
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(31, 1);

    let catch_up = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();
    main_assert!(catch_up.skipped_render_frames > 0);
    main_assert!(
        !catch_up.skip_redraw,
        "the recovered final frame is rendered"
    );
    main_assert_eq!(app.network_control_pacing().behind => 3);
    main_assert!(!app.network_control_pacing().skip_render);

    accumulator += schedule.simulation_interval;
    let paced = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();
    main_assert_eq!(paced.executed_frames => 1);
    main_assert!(!paced.skip_redraw);
}

#[test]
fn network_control_within_overflow_limit_keeps_normal_pacing() {
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(4, 1);

    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();

    main_assert_eq!(outcome.executed_frames => 1);
    main_assert_eq!(app.network_control_pacing().behind => 3);
    main_assert!(!outcome.skip_redraw);
    main_assert_eq!(accumulator => Duration::ZERO);
}

#[test]
fn control_buffered_inside_the_presend_horizon_is_not_a_catch_up_backlog() {
    // `CtrlNeeded` submits through `getCtrlTick(FrameCounter + PreSend)`
    // (src/C4GameControlNetwork.cpp:147-155); `CtrlOverflow` measures against
    // the executing tick (src/C4GameControlNetwork.h:124), including that horizon.
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(7, 2);
    let clock = app.network_control_clock.test_mut();
    clock.observe_control_send_time_ms(300);
    main_assert_eq!(clock.calculate_performance().map(|change| change.control_presend) => Some(12), "a 300 ms link sizes the horizon at twelve frames");

    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();

    main_assert_eq!(outcome.executed_frames => 1);
    main_assert!(!app.network_control_pacing().overflow);
}

#[test]
fn a_shallow_presend_horizon_keeps_the_cpp_overflow_limit() {
    // A 110 ms link sizes PreSend at 5: two future ticks remain inside C++'s
    // `C4ControlOverflowLimit` of 3.
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(4, 2);
    let clock = app.network_control_clock.test_mut();
    clock.observe_control_send_time_ms(110);
    main_assert_eq!(clock.calculate_performance().map(|change| change.control_presend) => Some(5));

    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();

    main_assert!(outcome.executed_frames > 1);
}

#[test]
fn a_backlog_beyond_the_presend_horizon_still_catches_up() {
    // Async packing can leave ready control beyond the client's own horizon.
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(12, 2);
    let clock = app.network_control_clock.test_mut();
    clock.observe_control_send_time_ms(300);
    clock.calculate_performance();

    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();

    main_assert!(outcome.executed_frames > 1);
    main_assert!(!app.network_control_pacing().overflow);
}

#[test]
fn completed_control_wakes_and_retries_only_the_blocked_boundary() {
    let mut app = new_running_sandbox_app();
    let (manager, events) = NetworkManager::test_stub();
    let start_tick = 41_u32;
    app.network = Some(manager);
    app.network_control_clock = Some(NetworkControlClock::new(
        i32::try_from(start_tick).test_value(),
        1,
    ));
    app.engine.initialize_network_control_timing(
        clonk_engine::NetworkControlTiming::new(i32::try_from(start_tick).test_value(), 1)
            .test_value(),
    );
    let (wake_tx, wake_rx) = mpsc::channel();
    app.install_network_event_waker(Arc::new(move |wake| {
        wake_tx.send(wake).test_value();
    }));
    let mut schedule = frame_schedule_for_mode(
        app.mode,
        app.engine.game_tick_delay_ms(),
        app.engine.game_tick_delay_revision(),
        app.max_refresh_delay_ms,
    );
    let mut accumulator = schedule.simulation_interval;

    let blocked = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();
    main_assert_eq!(blocked.executed_frames => 0);
    main_assert_eq!(accumulator => Duration::ZERO);
    main_assert_eq!(app.waiting_network_control => Some(NetworkControlWait::ReadyTick(start_tick)));

    events
        .send(NetworkEvent::ReadyTick {
            tick: start_tick,
            controls: Vec::new(),
        })
        .test_value();
    app.note_network_event_wake(wake_rx.recv_timeout(Duration::from_secs(1)).test_value());
    accumulator = Duration::from_millis(9);
    let retried = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();
    main_assert_eq!(retried.executed_frames => 1);
    main_assert!(retried.immediate_network_retry);
    main_assert_eq!(accumulator => Duration::ZERO);

    events
        .send(NetworkEvent::ReadyTick {
            tick: start_tick + 1,
            controls: Vec::new(),
        })
        .test_value();
    app.note_network_event_wake(wake_rx.recv_timeout(Duration::from_secs(1)).test_value());
    let future = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();
    main_assert_eq!(future.executed_frames => 0);
    main_assert!(!future.immediate_network_retry);
}

#[test]
fn deep_sea_gpu_presentation_meets_native_tick_budget() {
    let passing = PresentationBenchmarkReport {
        elapsed: Duration::from_secs(20),
        submissions: 1_200,
        retained_gpu_submissions: 1_200,
        cpu_submissions: 0,
        refreshed_frames: 1_200,
        simulation_frames: 714,
        runtime_stippels_at_start: 0,
        runtime_stippels_at_end: 0,
        automatic_graphics_skips: 0,
        graphics_average: INGAME_FRAME_INTERVAL,
        graphics_max: Duration::from_millis(32),
        graphics_p50: Duration::ZERO,
        graphics_p95: Duration::ZERO,
        graphics_p99: Duration::ZERO,
        graphics_samples: Vec::new(),
        present_max: Duration::ZERO,
        present_p50: Duration::ZERO,
        present_p95: Duration::ZERO,
        present_p99: Duration::ZERO,
        present_samples: Vec::new(),
        raster_max: Duration::ZERO,
        raster_p50: Duration::ZERO,
        raster_p95: Duration::ZERO,
        raster_p99: Duration::ZERO,
        raster_samples: Vec::new(),
        simulation_max: Duration::ZERO,
        simulation_p50: Duration::ZERO,
        simulation_p95: Duration::ZERO,
        simulation_p99: Duration::ZERO,
        simulation_samples: Vec::new(),
        frame_max: Duration::ZERO,
        frame_p50: Duration::ZERO,
        frame_p95: Duration::ZERO,
        frame_p99: Duration::ZERO,
        frame_samples: Vec::new(),
        surface_reallocations: 0,
        reallocation_max: Duration::ZERO,
        reallocation_p50: Duration::ZERO,
        reallocation_p95: Duration::ZERO,
        reallocation_p99: Duration::ZERO,
        reallocation_samples: Vec::new(),
        retained_gpu_profiles: Vec::new(),
        gpu_timestamp_frames: Vec::new(),
    };
    main_assert_eq!(validate_native_tick_presentation_budget(&passing) => Ok(()));

    let mut missing_submission = passing.clone();
    missing_submission.submissions = 713;
    missing_submission.retained_gpu_submissions = 713;
    main_assert!(
        validate_native_tick_presentation_budget(&missing_submission)
            .unwrap_err()
            .contains("successful presentation submissions 713 below native cadence 714")
    );

    let mut missing_refresh = passing.clone();
    missing_refresh.refreshed_frames = 713;
    main_assert!(validate_native_tick_presentation_budget(&missing_refresh)
        .unwrap_err()
        .contains("refreshed frames 713 below native cadence 714"));

    let mut missing_simulation = passing.clone();
    missing_simulation.simulation_frames = 713;
    main_assert!(
        validate_native_tick_presentation_budget(&missing_simulation)
            .unwrap_err()
            .contains("simulation frames 713 below native cadence 714")
    );

    let mut too_slow = passing.clone();
    too_slow.graphics_average = Duration::from_micros(28_001);
    main_assert!(validate_native_tick_presentation_budget(&too_slow)
        .unwrap_err()
        .contains("exceeds the native 28ms game tick"));

    let mut skipped = passing.clone();
    skipped.automatic_graphics_skips = 1;
    main_assert!(validate_native_tick_presentation_budget(&skipped)
        .unwrap_err()
        .contains("must be zero"));

    let mut cpu_fallback = passing.clone();
    cpu_fallback.retained_gpu_submissions -= 1;
    cpu_fallback.cpu_submissions = 1;
    main_assert!(validate_native_tick_presentation_budget(&cpu_fallback)
        .unwrap_err()
        .contains("CPU presentation submissions must be zero"));

    let mut missing_retained = passing.clone();
    missing_retained.retained_gpu_submissions -= 1;
    main_assert!(validate_native_tick_presentation_budget(&missing_retained)
        .unwrap_err()
        .contains("retained GPU submissions 1199 do not match total submissions 1200"));

    let mut not_refreshed = passing;
    not_refreshed.refreshed_frames = 0;
    main_assert!(validate_native_tick_presentation_budget(&not_refreshed)
        .unwrap_err()
        .contains("no refreshed presentation"));
}

#[test]
fn network_control_catch_up_advances_non_control_frames() {
    let (mut app, mut schedule, mut accumulator) = network_catch_up_fixture(11, 2);
    main_assert_eq!(app.engine.frame() % 2 => 0, "fixture starts on control cadence");

    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();

    main_assert_eq!(outcome.executed_frames => 16);
    main_assert_eq!(app.network_control_pacing().behind => 3);
    main_assert!(!app.network_control_pacing().overflow);
}

#[test]
fn activating_a_scenario_joins_the_local_player_with_crew() {
    // Match C4Game::InitPlayers -> C4PlayerList::Join so Crew= materialises
    // crew objects (C4PlayerList.cpp:271-318).
    let dir = tempdir();
    let (user_data, _guard, paths) = n1_test_paths(None);
    configure_test_startup_participant(&paths, user_data.path());
    let scenario_dir = dir.path().join("JoinTest.c4s");
    let def_dir = scenario_dir.join("GOOD.c4d");
    fs::create_dir_all(&def_dir).test_value();
    fs::write(
        def_dir.join("DefCore.txt"),
        "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=1\n",
    )
    .test_value();
    fs::write(def_dir.join("Script.c"), "// crew def\n").test_value();
    write_test_definition_graphics(&def_dir);
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=JoinTest\n\n[Player1]\nCrew=GOOD=2\nPosition=10,10\n",
    )
    .test_value();

    let scenario_data =
        clonk_engine::Scenario::load_from_path_with_languages_and_definition_modules(
            &scenario_dir,
            &InstallDefinitionResolver::new(None),
            &["US"],
            &[def_dir.to_string_lossy()],
        )
        .test_value();

    let mut app = GameApp::new(
        320,
        200,
        AudioOptions::default(),
        Some(&paths),
        RuntimeConfig {
            player_owner: 1,
            player_name: "Twonky".to_string(),
            network: None,
            record_enabled: false,
        },
    )
    .test_value();
    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::LocalSelector,
        GameOptionValues {
            fair_crew: true,
            fair_crew_strength: 4_321,
            ..GameOptionValues::default()
        },
    );

    let frontend = FrontendScenario {
        identifier: "JoinTest.c4s".to_string(),
        title: "JoinTest".to_string(),
        description: None,
        kind: ScenarioKind::Scenario,
        is_editable: false,
        is_playable: true,
        mission_access: None,
        path: Some(scenario_dir.clone()),
        source_paths: Vec::new(),
        root_label: None,
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
    app.activate_loaded_scenario(frontend.clone(), &scenario_data)
        .test_value();
    main_assert!(app.engine.use_fair_crew());
    main_assert_eq!(app.engine.fair_crew_strength() => 4_321);

    let expected_definition = def_dir.to_string_lossy();
    main_assert!(matches!(
        app.active_definition_load.as_ref(),
        Some(ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root: None,
        }) if modules == &[expected_definition.as_ref()]
    ));

    main_assert_eq!(app.players.local_owner => 0, "local owner adopts the joined number");
    let crew: Vec<_> = app
        .snapshot
        .objects
        .iter()
        .filter(|object| object.crew_member && object.owner == app.players.local_owner)
        .collect();
    main_assert_eq!(crew.len() => 2, "Crew=GOOD=2 joins with two crew members");
    let selection = app
        .snapshot
        .crew_selection
        .get(&app.players.local_owner)
        .test_value();
    let cursor = selection.cursor.test_value();
    // Cursor->DoSelect selects only the cursor (C4Player.cpp:1255-1257).
    main_assert_eq!(selection.selected.as_slice() => &[cursor], "only the cursor is selected at join");
    main_assert_eq!(app.focus_id => Some(cursor), "the app focus adopts the join cursor");

    app.restart_current_scenario().test_value();
    wait_for_running(&mut app);
    main_assert_eq!(app.active_scenario.as_ref().map(|scenario| (&scenario.identifier, &scenario.path)) => Some((&frontend.identifier, &frontend.path)));
    main_assert!(matches!(
        app.active_definition_load.as_ref(),
        Some(ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root: None,
        }) if modules == &[expected_definition.as_ref()]
    ));
}

#[test]
fn client_network_scenario_install_retains_authoritative_join_data_rules_and_goals() {
    // InitRules/InitGoals consume post-lobby parameters, including ConvertGoals
    // additions such as ENRG (C4Game.cpp:4056-4076).
    let dir = tempdir();

    let scenario_dir = dir.path().join("NetworkRuleGoalSync.c4s");
    for (folder, id, category) in [
        ("Revivals.c4d", "RVLR", 8192),
        ("Energy.c4d", "ENRG", 8192),
        ("RawRace.c4d", "RACE", 4096),
        ("SynchronizedGoal.c4d", "GOAL", 4096),
    ] {
        let definition = scenario_dir.join(folder);
        fs::create_dir_all(&definition).test_value();
        fs::write(
            definition.join("DefCore.txt"),
            format!("[DefCore]\nid={id}\nName={id}\nCategory={category}\n"),
        )
        .test_value();
        write_test_definition_graphics(&definition);
    }
    fs::create_dir_all(&scenario_dir).test_value();
    fs::write(
        scenario_dir.join("Scenario.txt"),
        "[Head]\nTitle=NetworkRuleGoalSync\nNetworkGame=1\n\n\
             [Game]\nGoals=RACE=1;\nRules=RVLR=1;\n\n\
             [Landscape]\nMapZoom=10\n",
    )
    .test_value();

    let mut authoritative = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    let id_entry = |id: [u8; 4]| clonk_network::JoinDataIdListEntry {
        id: clonk_network::JoinDataC4Id::from_bytes(id).test_value(),
        count: 1,
    };
    authoritative.parameters.rules = vec![id_entry(*b"RVLR"), id_entry(*b"ENRG")];
    authoritative.parameters.goals = vec![id_entry(*b"GOAL")];

    let status = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        ..clonk_network::HostConfig::default().initial_status
    };
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 3,
        start_control_tick: authoritative.dynamic_tick,
        status,
        dynamic: authoritative.dynamic,
        parameters: authoritative.parameters,
    };
    let mut app = new_menu_app(320, 200);
    configure_runtime_network_role(&mut app, RuntimeNetworkRole::Client);
    main_assert!(
        app.host_join_snapshot.is_none(),
        "a real client does not pretend to own the host's mutable snapshot"
    );
    app.pending_network_join_data = Some(join_data.clone());
    app.pending_client_start_status = Some(status);
    app.client_combined_scenario_path = Some(scenario_dir.clone());
    app.try_prepare_client_network_scenario().test_value();
    main_assert!(
        app.pending_network_join_data.is_none(),
        "the full pending JoinData packet is consumed after installation"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while app
        .loading_state
        .as_ref()
        .is_some_and(|loading| !loading.finished)
    {
        app.poll_loading().test_value();
        main_assert!(
            Instant::now() < deadline,
            "client scenario worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    main_assert!(app.host_join_snapshot.is_none());

    let count = |id: &str| {
        app.snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == id)
            .count()
    };
    main_assert_eq!(count("RVLR") => 1, "client places the authored rule");
    main_assert_eq!(count("ENRG") => 1, "client places the converted JoinData rule after packet consumption");
    main_assert_eq!(count("GOAL") => 1, "client places the authoritative JoinData goal");
    main_assert_eq!(count("RACE") => 0, "client does not place the superseded raw scenario goal");
}

#[test]
fn control_script_errors_are_non_fatal_like_cpp() {
    // ErrorOrWarning shows only the offending call failure and keeps running
    // (C4AulExec.cpp:1345-1361).
    let script_error = EngineError::Script {
        definition: "CLNK".into(),
        function: "Control".to_string(),
        source: ScriptError::parse("boom", 1, 1),
        recovery: None,
    };
    let status = control_script_error_to_status(script_error).test_value();
    main_assert!(
        status.contains("CLNK"),
        "status names the definition: {status}"
    );

    let fatal = EngineError::CrewSelection {
        owner: 0,
        detail: "broken".into(),
    };
    control_script_error_to_status(fatal).expect_err("engine-model errors stay fatal");
}

#[test]
fn startup_numbers_follow_native_long_narrowing() {
    main_assert_eq!(parse_startup_config_integer(b"7junk") => Some(7));
    main_assert_eq!(parse_startup_config_integer(b"0x2ajunk") => Some(42));
    main_assert_eq!(parse_startup_config_integer(b"0x") => Some(0));

    #[cfg(all(not(windows), target_pointer_width = "64"))]
    {
        main_assert_eq!(parse_startup_config_integer(b"4294967296") => Some(0));
        main_assert_eq!(parse_startup_config_integer(b"9223372036854775808") => Some(-1), "strtol overflow clamps to native LONG_MAX before int32 narrowing");
    }
    #[cfg(any(windows, target_pointer_width = "32"))]
    {
        main_assert_eq!(parse_startup_config_integer(b"4294967296") => Some(i32::MAX));
        main_assert_eq!(parse_startup_config_integer(b"-4294967296") => Some(i32::MIN));
    }
}

#[test]
fn unsigned_renderer_mask_follows_native_ulong_narrowing() {
    main_assert_eq!(parse_startup_config_unsigned(b"0xfffffffftail") => Some(u32::MAX));
    main_assert_eq!(parse_startup_config_unsigned(b"-1") => Some(u32::MAX));
    main_assert_eq!(parse_startup_config_unsigned(b"-0x10") => Some(0));
    main_assert_eq!(parse_startup_config_unsigned(b"0x") => Some(0));

    #[cfg(all(not(windows), target_pointer_width = "64"))]
    {
        main_assert_eq!(parse_startup_config_unsigned(b"9223372036854775808") => Some(0), "Unix-64 strtoul retains the value before uint32 narrowing");
        main_assert_eq!(parse_startup_config_unsigned(b"18446744073709551616") => Some(u32::MAX), "strtoul overflow saturates to native ULONG_MAX");
    }
    #[cfg(any(windows, target_pointer_width = "32"))]
    main_assert_eq!(parse_startup_config_unsigned(b"4294967296") => Some(u32::MAX), "32-bit strtoul overflow saturates before narrowing");
}

#[test]
fn advanced_renderer_config_loads_native_device_snapshot() {
    main_assert_eq!(load_advanced_renderer_config(b"") => clonk_frontend::AdvancedRendererConfig::DEFAULT);
    let loaded = load_advanced_renderer_config(
                b"[Graphics]\nNoAlphaAdd=true\nNoBoxFades=1\nTexIndent=-250junk\nBlitOffset=0x32tail\nAllowedBlitModes=0x5\nShader=true\nUseShaderGamma=false\nDisableGamma=true\n",
            );
    main_assert_eq!(
        loaded =>
        clonk_frontend::AdvancedRendererConfig {
            no_alpha_add: true,
            no_box_fades: true,
            tex_indent: -250,
            blit_offset: 50,
            allowed_blit_modes: 5,
            shader: true,
            use_shader_gamma: false,
            disable_gamma: true,
        }
    );
    main_assert_eq!(load_advanced_renderer_config(b"[Graphics]\nAllowedBlitModes=4294967295\n").allowed_blit_modes => u32::MAX);
}

#[test]
fn integrity_numbers_keep_native_scalar_grammar() {
    let quoted = b"[General]\nConfigResetSafety=\"7\"\n\n[Graphics]\nResolutionX=\"0\"\n";
    main_assert!(
        !startup_config_is_corrupted(quoted),
        "quoted strings are not native DWord values and retain typed defaults"
    );

    let bare_hex_prefix = b"[General]\nConfigResetSafety=42\n\n[Graphics]\nResolutionX=0x\n";
    main_assert!(startup_config_is_corrupted(bare_hex_prefix));

    let dir = tempdir();
    let path = dir.path().join("clonk-rust.config");
    fs::write(
        &path,
        "[General]\nConfigResetSafety=\"7\"\nVendor=keep\n\n[Graphics]\nResolutionX=1234junk\n",
    )
    .test_value();
    main_assert!(!validate_or_repair_startup_config(&path, false)
        .expect("canonicalize healthy native prefix config"));
    let canonical = Config::load(&path).test_value();
    main_assert_eq!(canonical.get_in(Some("General"), "ConfigResetSafety") => Some("42"));
    main_assert_eq!(canonical.get_in(Some("General"), "Vendor") => Some("keep"));
    main_assert_eq!(canonical.get_in(Some("Graphics"), "ResolutionX") => Some("1234"));

    fs::write(
        &path,
        "[General]\nConfigResetSafety=42\n\n[Graphics]\nResolutionX=\"1234\"\n",
    )
    .test_value();
    main_assert!(!validate_or_repair_startup_config(&path, false)
        .expect("canonicalize quoted resolution default"));
    main_assert_eq!(Config::load(&path).expect("reload quoted resolution config").get_in(Some("Graphics"), "ResolutionX") => Some("800"));
}

#[test]
fn global_sound_discovery_uses_only_native_sound_c4g() {
    let dir = tempdir();
    let native = dir.path().join("Sound.c4g");
    let nested = native.join("Nested");
    let probable = dir.path().join("SoundExtra.c4g");
    let alternate = dir.path().join("Sound.ocg");
    for path in [&nested, &probable, &alternate] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(native.join("Native.wav"), b"native").test_value();
    fs::write(nested.join("Nested.wav"), b"nested").test_value();
    fs::write(probable.join("Probable.wav"), b"probable").test_value();
    fs::write(alternate.join("Alternate.wav"), b"alternate").test_value();

    let (global, base_sample_loads) = discover_global_sound_libraries_at(dir.path());
    let mut resolver = SoundResolver::empty();
    resolver.global = global;
    resolver.base_sample_loads = base_sample_loads;
    resolver.rebuild_sample_ranks();

    main_assert_eq!(resolver.sample_names() => ["native.wav"]);
    main_assert_eq!(resolver.resolve_entry("Native").expect("native global sample").load_audio().expect("read native global sample") => b"native");
    for rejected in ["Nested", "Probable", "Alternate"] {
        main_assert!(resolver.resolve_entry(rejected).is_none());
    }
}

#[test]
fn global_portrait_relative_right_center_geometry_responds_at_321px() {
    // Percentages and halves truncate independently
    // (C4GameMessage.cpp:109-111,145-155).
    let viewport = Rect::new(17, 23, 321, 241);
    let offset = Vector2::new(33, 67);
    let flags = FLAG_X_REL | FLAG_Y_REL | FLAG_WIDTH_REL | FLAG_RIGHT | FLAG_VCENTER;

    main_assert_eq!(global_message_viewport_geometry(viewport, offset, 35, flags) => GlobalMessageViewportGeometry {x: 122, y: 184, width: 112,});
    main_assert_eq!(global_portrait_frame_rect(viewport, offset, flags, (101, 65)) => Rect::new(342, 272, 101, 65));
}

#[test]
fn global_portrait_position_flags_choose_cpp_frame_references() {
    // Position flags select the frame reference before its full/half offset
    // (src/C4GameMessage.cpp:136-155).
    let viewport = Rect::new(10, 20, 801, 601);
    let offset = Vector2::new(7, -11);
    let size = (101, 65);
    for (flags, expected) in [
        (FLAG_LEFT | FLAG_TOP, Rect::new(17, 9, 101, 65)),
        (FLAG_RIGHT | FLAG_TOP, Rect::new(717, 9, 101, 65)),
        (FLAG_HCENTER | FLAG_TOP, Rect::new(367, 9, 101, 65)),
        (FLAG_LEFT | FLAG_BOTTOM, Rect::new(17, 545, 101, 65)),
        (FLAG_LEFT | FLAG_VCENTER, Rect::new(17, 277, 101, 65)),
    ] {
        main_assert_eq!(global_portrait_frame_rect(viewport, offset, flags, size) => expected, "flags={flags:#x}");
    }
}

#[test]
fn global_portrait_centering_truncates_each_odd_half_like_cpp() {
    // C++ evaluates viewport/2 and frame/2 separately
    // (src/C4GameMessage.cpp:145,147,153,155).
    let rect = global_portrait_frame_rect(
        Rect::new(0, 0, 800, 600),
        Vector2::ZERO,
        FLAG_HCENTER | FLAG_VCENTER,
        (101, 65),
    );

    main_assert_eq!(rect => Rect::new(350, 268, 101, 65));
}

fn assert_one_pixel_native_edge(
    chrome: &[u8],
    rendered: &[u8],
    frame_width: u32,
    frame_height: u32,
    command: &clonk_graphics::clonk_font::CapturedClonkText,
    scale: f32,
) {
    let anchor_x = (command.x as f32 * scale).round() as i32;
    let anchor_y = (command.y as f32 * scale).round() as i32;
    let (left, right) = match command.align {
        clonk_graphics::clonk_font::TextAlign::Left => (anchor_x - 4, anchor_x + 100),
        clonk_graphics::clonk_font::TextAlign::Center => (anchor_x - 60, anchor_x + 60),
        clonk_graphics::clonk_font::TextAlign::Right => (anchor_x - 100, anchor_x + 4),
    };
    let top = anchor_y - 4;
    let bottom = anchor_y + 120;
    let delta = |x: i32, y: i32| -> u8 {
        if x < 0 || y < 0 || x >= frame_width as i32 || y >= frame_height as i32 {
            return 0;
        }
        let index = (y as usize * frame_width as usize + x as usize) * 4;
        (0..3)
            .map(|channel| rendered[index + channel].abs_diff(chrome[index + channel]))
            .max()
            .unwrap_or(0)
    };

    let mut transition_widths = Vec::new();
    for y in top..bottom {
        let samples = (left..right).map(|x| delta(x, y)).collect::<Vec<_>>();
        let Some(first_solid) = samples.iter().position(|value| *value >= 160) else {
            continue;
        };
        let mut start = first_solid;
        while start > 0 && (4..160).contains(&samples[start - 1]) {
            start -= 1;
        }
        transition_widths.push(first_solid - start);
    }
    main_assert!(
        !transition_widths.is_empty(),
        "captured {:?} glyph `{}` contributed no solid foreground pixels",
        command.role,
        command.text
    );
    transition_widths.sort_unstable();
    let median = transition_widths[transition_widths.len() / 2];
    main_assert!(
        median <= 1,
        "native `{}` edge spans {median} intermediate physical pixels: {transition_widths:?}",
        command.text
    );
}

#[test]
fn ordered_overlay_retains_additive_rgb_with_zero_alpha() {
    let mut app = new_running_sandbox_app();
    app.pending_native_presentation = Some(NativePresentationPlan::default());
    app.begin_native_text_capture(true);
    app.graphics.surface_mut().pixels_mut()[..4].copy_from_slice(&[37, 11, 5, 0]);

    app.commit_pending_native_overlay();

    let plan = app.pending_native_presentation.test_ref();
    let layer = plan
        .batches
        .first()
        .and_then(|batch| batch.logical_layer.as_ref())
        .test_value();
    main_assert_eq!(&layer[..4] => &[37, 11, 5, 0]);
}

#[test]
fn ordered_overlay_does_not_infer_clipper_from_shared_text_clip() {
    let mut app = new_running_sandbox_app();
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let clip = Rect::new(7, 11, 101, 79);
    app.pending_native_presentation = Some(NativePresentationPlan::default());
    app.begin_native_text_capture(true);
    {
        let surface = app.graphics.surface_mut();
        surface.set_clip(clip);
        surface.pixels_mut()[..4].copy_from_slice(&[9, 17, 25, 255]);
        fonts.text.draw(
            surface,
            12,
            16,
            "Shared clip",
            [255, 255, 255, 255],
            clonk_graphics::clonk_font::TextAlign::Left,
            false,
        );
    }

    app.commit_pending_native_overlay();

    let batch = app
        .pending_native_presentation
        .as_ref()
        .and_then(|plan| plan.batches.first())
        .test_value();
    main_assert!(batch.logical_layer.is_some());
    main_assert!(batch.text.iter().all(|command| command.clip == Some(clip)));
    main_assert_eq!(batch.clip => None, "shared text clipping alone cannot prove the raster layer is isolated");
}

#[test]
fn scale_three_target_message_commits_through_native_viewport_projection() {
    let mut app = new_running_sandbox_app();
    let target = app
        .snapshot
        .objects
        .first()
        .map(|object| object.id)
        .test_value();
    let shape_height = app
        .snapshot
        .object(target)
        .and_then(|object| app.engine.definition_shape_rect(&object.definition_id))
        .map(|shape| shape.height)
        .unwrap_or(0);
    app.snapshot.hud.messages = vec![clonk_engine::MessageSnapshot {
        id: 1,
        kind: MessageKind::TargetPlayer,
        lines: vec!["A".to_string()],
        target: Some(target),
        player: Some(app.players.local_owner),
        offset: Vector2::new(0, shape_height / 2 + 5),
        color: 0xffff_ffff,
        flags: FLAG_NO_BREAK,
        width: None,
        decoration: None,
        frame_decoration: None,
        portrait: None,
    }];
    install_native_test_fonts(&mut app, 3.0);
    main_assert!(app.can_defer_native_game_messages(3.0));

    let gamma = app
        .graphics
        .active_gamma_ramp(&app.snapshot.environment.gamma);
    let mut presenter = clonk_scaling::FramePresenter::new(3.0, 960, 600);
    let mut output = vec![0_u8; 960 * 600 * 4];
    main_assert!(presenter
        .present(&mut output, |frame| {
            app.render_for_presentation(frame, false, false, true)
        })
        .expect("render filtered base with deferred target message"));
    let filtered_base = output.clone();

    app.render_native_game_messages(&mut output, presenter.presentation_geometry(), &gamma)
        .test_value();
    let viewport = app
        .graphics
        .active_viewport_projections()
        .into_iter()
        .find(|viewport| viewport.owner == app.players.local_owner)
        .test_value()
        .rect;
    let physical_viewport = Rect::new(
        viewport.x * 3,
        viewport.y * 3,
        viewport.width * 3,
        viewport.height * 3,
    );
    let changed = output
        .chunks_exact(4)
        .zip(filtered_base.chunks_exact(4))
        .enumerate()
        .filter_map(|(index, (native, base))| (native != base).then_some(index))
        .collect::<Vec<_>>();
    main_assert!(
        !changed.is_empty(),
        "native target glyphs contribute pixels"
    );
    main_assert!(changed.iter().all(|index| {
        let x = (*index % 960) as i32;
        let y = (*index / 960) as i32;
        x >= physical_viewport.x
            && x < physical_viewport.x + physical_viewport.width as i32
            && y >= physical_viewport.y
            && y < physical_viewport.y + physical_viewport.height as i32
    }));
}

#[test]
fn scale_three_hud_caption_uses_one_pixel_native_edge() {
    let mut app = new_running_sandbox_app();
    let assets = Arc::make_mut(&mut app.assets);
    let mut hud = (*assets.hud_graphics).clone();
    hud.upper_board = Some(ImageData::new(1, 1, vec![24, 32, 40, 255]));
    hud.logo = None;
    assets.hud_graphics = Arc::new(hud);
    app.configure_running_state("H".to_string(), DEFAULT_GROUND_HEIGHT);
    install_native_test_fonts(&mut app, 3.0);

    let (chrome, rendered, plan) = render_ordered_test_frame(&mut app, 3.0, 960, 600);
    let (batch_index, command) = plan
        .batches
        .iter()
        .enumerate()
        .flat_map(|(batch, batch_data)| batch_data.text.iter().map(move |command| (batch, command)))
        .find(|(_, command)| command.text == "H")
        .test_value();
    main_assert!(batch_index > 0, "HUD text follows the world/base batch");
    main_assert!(
        plan.batches[batch_index].logical_layer.is_some(),
        "HUD chrome is committed immediately before its native text"
    );
    main_assert_eq!(command.role => clonk_graphics::clonk_font::ClonkFontRole::GuiText);
    assert_one_pixel_native_edge(&chrome, &rendered, 960, 600, command, 3.0);
}

#[test]
fn scale_one_point_five_hud_uses_fractional_native_font_bundle() {
    let mut app = new_classic_running_sandbox_app();
    app.configure_running_state("H".to_string(), DEFAULT_GROUND_HEIGHT);
    install_native_test_fonts(&mut app, 1.5);
    let fonts = app.native_startup_fonts.as_deref().test_value();
    main_assert_eq!(fonts.text.raster_height() => 21);
    main_assert_eq!(fonts.main_small.raster_height() => 19);

    let (chrome, rendered, plan) = render_ordered_test_frame(&mut app, 1.5, 480, 300);
    main_assert!(plan
        .batches
        .iter()
        .flat_map(|batch| &batch.text)
        .any(|command| command.text == "H"));
    main_assert_ne!(rendered => chrome, "fractional native HUD glyphs must draw");
}

#[test]
fn inventory_and_owned_menu_mod2_follow_their_native_draw_targets() {
    let mut engine = Engine::new();
    let image = n1_register_picture(
        &mut engine,
        "M2PX",
        "Mod2 Picture",
        [64, 128, 192, 128],
        None,
    );
    let transparent_image = n1_register_picture(
        &mut engine,
        "M2BG",
        "Transparent Picture",
        [0, 0, 0, 0],
        None,
    );
    let owner_image = n1_register_picture(
        &mut engine,
        "M2OW",
        "Owner Overlay",
        [0, 255, 0, 255],
        Some([128, 128, 128, 128]),
    );
    let modulation = 0x007f_7f7f;

    let inventory =
        compose_inventory_picture(image.clone(), Vec::new(), 0, modulation, 2).test_value();
    main_assert_eq!(inventory.pixels() => &[127, 255, 255, 128], "direct HUD inventory uses live GL clamp(2*S + 2*M - 255)",);

    let menu = compose_owned_menu_picture(
        image.clone(),
        Vec::new(),
        &n1_picture_snapshot("M2PX", 2, 0, modulation),
    )
    .test_value();
    main_assert_eq!(menu.pixels() => &[128, 255, 255, 128], "Picture2Facet uses packed software MOD2 in its temporary surface",);

    let masked_inventory = compose_inventory_picture_with_allowed_modes(
        image.clone(),
        Vec::new(),
        0,
        modulation,
        2,
        0,
    )
    .test_value();
    main_assert_eq!(masked_inventory.pixels() => &[31, 63, 95, 128], "AllowedBlitModes masks MOD2 while retaining active ColorMod",);
    let masked_menu = compose_owned_menu_picture_with_allowed_modes(
        image.clone(),
        Vec::new(),
        &n1_picture_snapshot("M2PX", 2, 0, modulation),
        0,
    )
    .test_value();
    main_assert_eq!(masked_menu.pixels() => &[31, 63, 95, 128]);

    let prepared_owner =
        prepare_inventory_picture(owner_image.clone(), Vec::new(), 0x00ff_0000, modulation, 2)
            .test_value();
    main_assert_eq!(prepared_owner.base.pixels() => &[0, 255, 0, 255]);
    main_assert_eq!(prepared_owner.overlays.len() => 1);
    main_assert_eq!(prepared_owner.overlays[0].picture.pixels() => &[63, 0, 0, 128], "owner tint and global modulation are prepared on the owner pass",);
    let (default_mod2_base, default_mod2_owner) = prepare_inventory_definition_layers(
        &owner_image,
        0x00ff_0000,
        None,
        2,
        clonk_frontend::AdvancedRendererConfig::DEFAULT,
    )
    .test_value();
    main_assert_eq!(default_mod2_base => [255, 255, 255, 255]);
    main_assert_eq!(default_mod2_owner.expect("owner pass") => [128, 0, 0, 128], "implicit white drives base MOD2 without dimming the owner pass",);
    let owner_inventory =
        compose_inventory_picture(owner_image.clone(), Vec::new(), 0x00ff_0000, modulation, 2)
            .test_value();
    main_assert_eq!(owner_inventory.pixels() => &[32, 127, 0, 255]);
    let fully_faded_owner =
        prepare_inventory_picture(owner_image.clone(), Vec::new(), 0x00ff_0000, 0xffff_ffff, 0)
            .test_value();
    main_assert_eq!(fully_faded_owner.base.pixels() => &[0, 255, 0, 0]);
    main_assert_eq!(fully_faded_owner.overlays[0].picture.pixels() => &[127, 0, 0, 0], "packed 0xffffffff is active full transparency, not a neutral sentinel",);
    for (mode, expected) in [
        (0, [16, 16, 12, 128]),
        (4, [32, 64, 96, 128]),
        (8, [65, 65, 49, 128]),
        (12, [129, 255, 255, 128]),
    ] {
        let prepared = prepare_inventory_picture(
            owner_image.clone(),
            Vec::new(),
            0x0040_80c0,
            0x0080_4020,
            mode,
        )
        .test_value();
        main_assert_eq!(prepared.overlays[0].picture.pixels() => &expected);
    }
    let owner_menu = compose_owned_menu_picture(
        owner_image,
        Vec::new(),
        &n1_picture_snapshot("M2OW", 2, 0x00ff_0000, modulation),
    )
    .test_value();
    main_assert_eq!(owner_menu.pixels() => &[31, 126, 0, 255]);

    let render_cached_picture = |picture: &ImageData| {
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(10, 20, 30));
        clonk_frontend::draw_image_bilinear(
            &mut surface,
            &clonk_gui::Rect::new(0.0, 0.0, 1.0, 1.0),
            picture,
            None,
        );
        surface.get_pixel(0, 0)
    };
    main_assert_eq!(render_cached_picture(&inventory) => Some(Color::opaque(69, 138, 143)), "the cached straight-alpha inventory pixel is blended exactly once",);
    main_assert_eq!(render_cached_picture(&menu) => Some(Color::opaque(69, 138, 143)), "the cached straight-alpha menu pixel is blended exactly once",);

    let black_reset = compose_inventory_picture(image.clone(), Vec::new(), 0, 0, 2).test_value();
    main_assert_eq!(black_reset.pixels() => &[0, 0, 0, 128]);
    let software_zero = compose_owned_menu_picture(
        image.clone(),
        Vec::new(),
        &n1_picture_snapshot("M2PX", 2, 0, 0),
    )
    .test_value();
    main_assert_eq!(software_zero.pixels() => &[0, 2, 130, 128]);

    let transparent_overlay =
        clonk_engine::ObjectGraphicsOverlay::new(3, clonk_engine::GraphicsOverlayMode::Picture);
    let software_transparent_overlay = compose_owned_menu_picture(
        image.clone(),
        vec![(transparent_overlay, transparent_image.clone())],
        &n1_picture_snapshot("M2PX", 0, 0, 0),
    )
    .test_value();
    main_assert_eq!(software_transparent_overlay.pixels() => &[63, 127, 191, 128], "software BltAlpha retains its transparent-source /256 quirk",);

    let mut transformed_overlay =
        clonk_engine::ObjectGraphicsOverlay::new(1, clonk_engine::GraphicsOverlayMode::Picture);
    transformed_overlay.blit_mode = 2;
    transformed_overlay.color_modulation = modulation;
    transformed_overlay.transform = Some(clonk_engine::DrawTransform::identity());
    let overlay_inventory = compose_inventory_picture(
        transparent_image.clone(),
        vec![(transformed_overlay, image.clone())],
        0,
        0,
        0,
    )
    .test_value();
    main_assert_eq!(overlay_inventory.pixels() => &[127, 255, 255, 128]);
    main_assert_eq!(
        render_cached_picture(&overlay_inventory) =>
        Some(Color::opaque(69, 138, 143)),
        "a translucent overlay over a transparent base is not alpha-squared",
    );

    let additive_picture =
        compose_inventory_picture(image.clone(), Vec::new(), 0, modulation, 3).test_value();
    let mut additive_target = Surface::new(45, 45, PixelFormat::Rgba8888);
    additive_target.fill(Color::opaque(10, 20, 30));
    let fallback_font = test_font();
    clonk_frontend::hud::draw_inventory(
        &mut additive_target,
        &clonk_frontend::hud::HudFont::Fallback(fallback_font.as_ref()),
        Rect::new(0, 0, 45, 45),
        &[InventoryOverlay {
            object_id: ObjectId::new(99),
            definition_id: "M2PX".to_string(),
            picture: Some(additive_picture),
            additive: true,
            picture_overlays: Vec::new(),
            count: 1,
        }],
    );
    main_assert_eq!(additive_target.get_pixel(22, 22) => Some(Color::opaque(74, 148, 158)), "direct inventory retains MOD2+ADDITIVE for the final HUD blend",);

    let mut additive_overlay =
        clonk_engine::ObjectGraphicsOverlay::new(4, clonk_engine::GraphicsOverlayMode::Picture);
    additive_overlay.blit_mode = 3;
    additive_overlay.color_modulation = modulation;
    let prepared_mixed = prepare_inventory_picture(
        transparent_image.clone(),
        vec![(additive_overlay, image.clone())],
        0,
        0,
        0,
    )
    .test_value();
    main_assert!(prepared_mixed.overlays[0].additive);
    let mut mixed_target = Surface::new(45, 45, PixelFormat::Rgba8888);
    mixed_target.fill(Color::opaque(10, 20, 30));
    clonk_frontend::hud::draw_inventory(
        &mut mixed_target,
        &clonk_frontend::hud::HudFont::Fallback(fallback_font.as_ref()),
        Rect::new(0, 0, 45, 45),
        &[InventoryOverlay {
            object_id: ObjectId::new(100),
            definition_id: "M2BG".to_string(),
            picture: Some(prepared_mixed.base),
            additive: false,
            picture_overlays: prepared_mixed.overlays,
            count: 1,
        }],
    );
    main_assert_eq!(mixed_target.get_pixel(22, 22) => Some(Color::opaque(74, 148, 158)), "an additive MOD2 overlay retains its own final HUD blend mode",);

    let mut inherited_overlay =
        clonk_engine::ObjectGraphicsOverlay::new(2, clonk_engine::GraphicsOverlayMode::Picture);
    inherited_overlay.blit_mode = 256;
    let overlay_menu = compose_owned_menu_picture(
        transparent_image,
        vec![(inherited_overlay, image)],
        &n1_picture_snapshot("M2BG", 2, 0, 0),
    )
    .test_value();
    main_assert_eq!(overlay_menu.pixels() => &[0, 2, 130, 128], "owned-menu overlays use packed software MOD2 without the GL zero reset",);
    main_assert_eq!(inventory_blit_mode(3) => BlitMode::Mod2Additive);
}

#[test]
fn network_row_colors_disable_errors_but_not_too_few_warning() {
    let (user_data, _guard, paths) = n1_test_paths(None);
    configure_test_startup_participant(&paths, user_data.path());
    let scenario_root = paths.scenario_dir().to_path_buf();
    fs::create_dir_all(&scenario_root).test_value();
    for (name, core) in [
        (
            "Replay.c4s",
            "[Head]\nTitle=Replay row\nReplay=1\nMinPlayer=1\nMaxPlayer=4\n",
        ),
        (
            "TooMany.c4s",
            "[Head]\nTitle=Too many row\nMinPlayer=1\nMaxPlayer=0\n",
        ),
        (
            "TooFew.c4s",
            "[Head]\nTitle=Too few row\nMinPlayer=2\nMaxPlayer=4\n",
        ),
    ] {
        let path = scenario_root.join(name);
        fs::create_dir(&path).test_value();
        fs::write(path.join("Scenario.txt"), core).test_value();
    }

    let scenarios = resource_scenario::discover(&scenario_root)
        .test_value()
        .into_iter()
        .map(|entry| FrontendScenario::from_resource(entry, "Test scenarios"))
        .collect::<Vec<_>>();
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);

    let render_row_alphas = |app: &mut GameApp| {
        let assets = app.assets.scensel_assets().test_value();
        let button_down = app.assets.dialog_image("GUIButtonDown.png").test_value();
        let fonts = app.assets.clonk_fonts.clone().test_value();
        let book = app.assets.book_fonts.clone().test_value();
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        draw_scensel_dynamic(
            &mut surface,
            &mut app.menu_state,
            &app.scensel.entry_enabled,
            &assets,
            &button_down,
            &fonts,
            &book,
            None,
            startup_gamma(),
            true,
        )
        .test_value();
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(640, 480, &fonts);
        let item_h = clonk_frontend::startup_scensel::scen_list_item_height(&book.text);
        let label_x = layout.list.x + 3 + item_h + 2;
        let label_top = layout.list.y + 3;
        app.menu_state
            .visible_entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let row_y = label_top + index as i32 * (item_h + 1);
                let max_alpha = (label_x..label_x + 120)
                    .flat_map(|x| (row_y..row_y + item_h).map(move |y| (x, y)))
                    .filter_map(|(x, y)| surface.get_pixel(x as u32, y as u32))
                    .map(|color| color.a)
                    .max()
                    .unwrap_or(0);
                (entry.identifier.clone(), max_alpha)
            })
            .collect::<HashMap<_, _>>()
    };

    app.open_network_host_scenario_browser();
    for (identifier, enabled) in [
        ("Replay.c4s", false),
        ("TooMany.c4s", false),
        ("TooFew.c4s", true),
    ] {
        main_assert_eq!(app.scensel.entry_enabled.get(identifier) => Some(&enabled), "network CanOpen state for {identifier}");
    }
    let network_alpha = render_row_alphas(&mut app);
    for identifier in ["Replay.c4s", "TooMany.c4s"] {
        main_assert!(
            network_alpha[identifier] > 0 && network_alpha[identifier] < 200,
            "{identifier} uses the disabled network row color: {:?}",
            network_alpha[identifier]
        );
    }
    main_assert!(
        network_alpha["TooFew.c4s"] > 200,
        "network too-few is a warning, so its row stays enabled"
    );

    app.open_scenario_browser();
    main_assert_eq!(app.scensel.entry_enabled.get("Replay.c4s") => Some(&true), "local replay bypasses regular player-count checks");
    main_assert_eq!(app.scensel.entry_enabled.get("TooMany.c4s") => Some(&false));
    main_assert_eq!(app.scensel.entry_enabled.get("TooFew.c4s") => Some(&false), "the same too-few row is fatal in the local selector");
    let local_alpha = render_row_alphas(&mut app);
    main_assert!(
        local_alpha["TooFew.c4s"] > 0 && local_alpha["TooFew.c4s"] < 200,
        "local too-few uses the disabled row color"
    );
    reset_cached_app_paths();
}

#[test]
fn network_replay_start_shows_cpp_error_and_never_opens_a_child() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let scenario_group = tempdir();
    fs::write(scenario_group.path().join("Scenario.txt"), "[Head]\nTitle=Replay\nReplay=1\nMinPlayer=9\nMaxPlayer=0\n\n[Definitions]\nAllowUserChange=true\n").test_value();
    persist_config_value(&paths, "General", "Participants", "").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "Replay.c4s".to_string();
    scenario.title = "Replay".to_string();
    scenario.path = Some(scenario_group.path().to_path_buf());
    scenario.allow_user_change = Some(true);
    let scenarios = vec![scenario.clone()];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);
    app.open_network_host_scenario_browser();
    app.menu_state.definition_checkbox_checked = true;

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

    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert!(app.definition_selector.is_none());
    main_assert!(app.staged_network_host_scenario.is_none());
    main_assert!(app.startup_network_connection.is_none());
    main_assert!(app.network.is_none());
    let dialog = &app.dialogs.messages[0].state;
    main_assert_eq!(dialog.caption() => "Cannot start scenario.");
    main_assert_eq!(dialog.message() => "Cannot play back records while in network mode.");
    main_assert_eq!(dialog.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::OK);
    main_assert_eq!(dialog.icon() => clonk_frontend::message_dialog::MessageDialogIcon::ERROR);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert!(app.definition_selector.is_none());
}

enum N1MissionAccessGrant {
    Persisted,
    MemoryOnly,
}

fn n1_assert_mission_access_grant(grant: N1MissionAccessGrant) {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let scenario_group = tempdir();
    fs::write(
        scenario_group.path().join("Scenario.txt"),
        "[Head]\nTitle=Locked replay\nMissionAccess=LOCK\nReplay=1\n",
    )
    .test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(scenario_group.path().to_path_buf());

    main_assert!(matches!(
        app.network_scenario_open_decision(&scenario)
            .expect("locked CanOpen decision"),
        NetworkScenarioOpenDecision::Error { message, .. }
            if message == "Access to this mission not yet granted."
    ));
    match grant {
        N1MissionAccessGrant::Persisted => {
            // C4Config only reads the file at load, so the granted list has to reach
            // the live config the way it does natively: through startup.
            persist_config_value(&paths, "General", "MissionAccess", "other;lock").test_value();
            app = new_menu_app_with_paths(640, 480, &paths);
        }
        N1MissionAccessGrant::MemoryOnly => {
            app.config
                .mission_access
                .update_modules("other;lock", false);
            main_assert_eq!(
                load_configured_mission_access(&paths).expect("read stale config access") =>
                "",
                "the grant is memory-only, exactly as the native sites leave it"
            );
        }
    }
    main_assert!(matches!(
        app.network_scenario_open_decision(&scenario)
            .expect("granted replay decision"),
        NetworkScenarioOpenDecision::Error { message, .. }
            if message == "Cannot play back records while in network mode."
    ));
}

#[test]
fn network_mission_access_gate_precedes_replay_rejection() {
    n1_assert_mission_access_grant(N1MissionAccessGrant::Persisted);
}

// `C4ScenarioListLoader::Scenario::CanOpen` tests the *in-memory*
// `Config.General.MissionAccess` (C4StartupScenSelDlg.cpp:743), and both
// native grant sites grow that string without writing the config file
// (`FnGainMissionAccess`, C4Script.cpp:2466-2471; the Alt+M dialog,
// C4StartupScenSelDlg.cpp:1838-1856). A password earned this session must
// therefore unlock the network selector exactly as it unlocks the local one.
#[test]
fn network_mission_access_gate_honours_memory_only_grant() {
    n1_assert_mission_access_grant(N1MissionAccessGrant::MemoryOnly);
}

#[test]
fn network_too_few_warning_persists_hide_on_cancel_and_then_continues() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let scenario_group = tempdir();
    fs::write(scenario_group.path().join("Scenario.txt"), "[Head]\nTitle=Needs players\nMinPlayer=2\nMaxPlayer=4\n\n[Definitions]\nAllowUserChange=true\n").test_value();
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
    app.menu_state.definition_checkbox_checked = true;
    let start = || {
        vec![StartupMenuAction::StartScenario(
            clonk_frontend::ScenarioSummary {
                identifier: scenario.identifier.clone(),
                title: scenario.title.clone(),
                kind: ScenarioKind::Scenario,
            },
        )]
    };

    app.handle_menu_input(|_| start()).test_value();
    let dialog = &app.dialogs.messages[0].state;
    main_assert_eq!(dialog.caption() => "Start Game");
    main_assert_eq!(
        dialog.message() =>
        "This scenario is designed for a minimum of 2 players. On start, you will have to wait for additional players to join from the network."
    );
    main_assert_eq!(dialog.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL);
    main_assert_eq!(dialog.icon() => clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY);
    main_assert_eq!(dialog.checkbox_checked() => Some(false));
    main_assert!(dialog
        .focused_button()
        .is_some_and(|button| button == clonk_frontend::message_dialog::MessageDialogButton::Ok));

    app.dialogs.messages[0].state.handle_hotkey('D');
    app.persist_top_message_dialog_checkbox_changes();
    // ShowMessageModal's by-pointer flag stays memory-only (C4ChatDlg.cpp:624).
    app.flush_deferred_config();
    main_assert_eq!(Config::load(paths.config_file()).unwrap().get_in(Some("Startup"), "HideMsgStartDedicated") => Some("1"));
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    main_assert!(app.definition_selector.is_none());
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);

    app.handle_menu_input(|_| start()).test_value();
    main_assert!(app.dialogs.messages.is_empty());
    main_assert!(app.definition_selector.is_some());
    main_assert_eq!(app.pending_definition_selection.as_ref().map(|pending| pending.selector_mode) => Some(ScenarioSelectorMode::NetworkHost));
    main_assert!(app.startup_network_connection.is_none());
}

#[test]
fn network_maximum_error_overrides_the_too_few_warning_text() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let scenario_group = tempdir();
    fs::write(
        scenario_group.path().join("Scenario.txt"),
        "[Head]\nTitle=Overconstrained\nMinPlayer=2\nMaxPlayer=0\n",
    )
    .test_value();
    let app = new_menu_app_with_paths(640, 480, &paths);
    persist_config_value(&paths, "General", "Participants", "One").test_value();
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(scenario_group.path().to_path_buf());
    main_assert!(matches!(
        app.network_scenario_open_decision(&scenario)
            .expect("overconstrained CanOpen decision"),
        NetworkScenarioOpenDecision::Error { message, .. }
            if message == "This scenario is designed for a maximum of 0 players."
    ));
}

#[test]
fn network_savegame_zero_max_lifts_to_minimum_for_selector_check() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let scenario_group = tempdir();
    fs::write(
        scenario_group.path().join("Scenario.txt"),
        "[Head]\nTitle=Saved round\nSaveGame=1\nMinPlayer=3\nMaxPlayer=0\n",
    )
    .test_value();
    let app = new_menu_app_with_paths(640, 480, &paths);
    persist_config_value(&paths, "General", "Participants", "One;Two;Three").test_value();
    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(scenario_group.path().to_path_buf());

    main_assert_eq!(app.network_scenario_open_decision(&scenario).expect("savegame CanOpen decision") => NetworkScenarioOpenDecision::Proceed);
}

#[test]
fn definition_file_scan_is_flat_case_insensitive_and_raw_ordered() {
    let root = tempdir();
    fs::create_dir(root.path().join("Folder.C4D")).test_value();
    fs::write(root.path().join("Packed.c4d"), b"pack").test_value();
    fs::create_dir(root.path().join("Ignore.c4f")).test_value();
    fs::create_dir_all(root.path().join("Nested/Hidden.c4d")).test_value();

    let expected = fs::read_dir(root.path())
        .test_value()
        .map(|entry| entry.test_value().path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("c4d"))
        })
        .collect::<Vec<_>>();
    let actual = enumerate_startup_definition_files(root.path()).test_value();
    main_assert_eq!(actual => expected, "the selector must not sort readdir order");
    main_assert_eq!(actual.len() => 2);
}

#[test]
fn definition_selector_app_route_keeps_recursive_error_refresh_and_cancel_modal() {
    let executable_data = tempdir();
    let (user_data, _guard, paths) = n1_test_paths(Some(executable_data.path()));
    let definition_root = executable_data.path().join("Definitions");
    fs::create_dir_all(definition_root.join("Alpha.c4d")).test_value();
    fs::create_dir(definition_root.join("Beta.C4D")).test_value();
    configure_test_startup_participant(&paths, user_data.path());
    persist_config_value(&paths, "General", "DefinitionPath", "Definitions/").test_value();
    let configured_definition_paths = startup_definition_paths(&paths).test_value();
    main_assert_eq!(
        configured_definition_paths =>
        StartupDefinitionPaths {
            selector_root: definition_root.clone(),
            active_custom_root: Some(definition_root.clone()),
        }
    );
    main_assert_eq!(
        path_to_legacy_bytes(
            configured_definition_paths
                .active_custom_root
                .as_deref()
                .expect("active definition prefix")
        )
        .last() =>
        Some(&(std::path::MAIN_SEPARATOR as u8)),
        "the active prefix retains DefinitionPath's trailing separator"
    );
    let external = tempdir();
    let external_value = format!(
        "{}{sep}",
        external.path().display(),
        sep = std::path::MAIN_SEPARATOR
    );
    persist_config_value(&paths, "General", "DefinitionPath", &external_value).test_value();
    let absolute_paths = startup_definition_paths(&paths).test_value();
    main_assert_eq!(
        absolute_paths.selector_root =>
        concatenate_legacy_path(
            &path_with_trailing_native_separator(executable_data.path()),
            &clonk_script::c4_string_bytes(&external_value),
        ),
        "the selector uses raw ExePath + DefinitionPath concatenation"
    );
    main_assert_eq!(
        path_to_legacy_bytes(
            absolute_paths
                .active_custom_root
                .as_deref()
                .expect("absolute prefix is active")
        ) =>
        normalize_legacy_path_bytes(clonk_script::c4_string_bytes(&external_value)),
        "game loading honors an absolute DefinitionPath independently"
    );
    let missing_definition_root = executable_data.path().join("missing-definitions");
    persist_config_value(&paths, "General", "DefinitionPath", "/missing-definitions/").test_value();
    main_assert_eq!(
        startup_definition_paths(&paths).expect("read absent definition paths") =>
        StartupDefinitionPaths {
            selector_root: missing_definition_root,
            active_custom_root: None,
        },
        "the selector displays an absent configured path, but C4Game does not expand it"
    );
    persist_config_value(&paths, "General", "DefinitionPath", "Definitions/").test_value();
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
            player_name: "Definition Tester".to_string(),
            network: None,
            record_enabled: false,
        },
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_scenario_browser();

    let mut scenario = FrontendScenario::fallback();
    scenario.path = Some(user_data.path().join("Start.c4s"));
    scenario.local_only = Some(false);
    scenario.allow_user_change = Some(true);
    scenario.definition_modules = vec!["Alpha.c4d".to_string()];
    app.menu_state.definition_checkbox_checked = false;
    app.open_definition_selector(scenario.clone()).test_value();

    let controller = app.definition_selector.test_ref();
    main_assert_eq!(app.pending_definition_selection.as_ref().and_then(|pending| pending.custom_definition_root.as_deref()) => Some(definition_root.as_path()));
    main_assert_eq!(controller.root_path() => format!("{}{sep}", definition_root.display(), sep = std::path::MAIN_SEPARATOR));
    main_assert!(controller
        .rows()
        .iter()
        .any(|row| { row.filename() == "Alpha.c4d" && row.is_fixed() && row.is_checked() }));
    main_assert!(controller
        .rows()
        .iter()
        .any(|row| row.filename() == "Beta.C4D"));
    main_assert_eq!(controller.selected_index() => None);

    let mut frame = vec![0_u8; 1280 * 720 * 4];
    app.test_render(&mut frame);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.definition_selector.is_some());
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert_eq!(app.dialogs.messages[0].state.caption() => "Error");
    main_assert_eq!(app.dialogs.messages[0].state.message() => "Please select a file first!");
    app.test_render(&mut frame);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);

    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    let controller = app.definition_selector.test_ref();
    main_assert_eq!(controller.selected_index() => None);
    main_assert!(
        controller.rows().iter().all(|row| !row.is_checked()),
        "C4DefinitionSelDlg does not reapply even fixed checks after F5"
    );
    app.test_key(VirtualKeyCode::F5, ElementState::Released);

    app.test_gamepad_events([
        n1_gamepad_direction(ControlButton::Right, ElementState::Pressed),
        n1_gamepad_button(GuiButtonClass::Low, ElementState::Pressed),
        n1_gamepad_button(GuiButtonClass::Low, ElementState::Released),
    ]);
    main_assert_eq!(app.dialogs.messages.len() => 1);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    app.test_gamepad_events([
        n1_gamepad_direction(ControlButton::Left, ElementState::Pressed),
        n1_gamepad_direction(ControlButton::Down, ElementState::Pressed),
    ]);
    main_assert_eq!(
        app.definition_selector
            .as_ref()
            .and_then(|controller| controller.selected_index()) =>
        Some(1),
        "focus return selects the first row and the following Down reaches the second"
    );

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.definition_selector.is_none());
    main_assert!(app.pending_definition_selection.is_none());
    main_assert!(
        app.definition_selector_consumed_keys
            .contains(&VirtualKeyCode::Escape),
        "close-on-key-down retains the matching physical release"
    );
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert!(app.definition_selector_consumed_keys.is_empty());
    main_assert!(
        !app.menu_state.definition_checkbox_checked,
        "cancel must retain the user's current checkbox toggle"
    );

    app.open_definition_selector(scenario.clone()).test_value();
    app.test_gamepad_events([
        n1_gamepad_button(GuiButtonClass::High, ElementState::Pressed),
        n1_gamepad_action(GamepadActionType::Cancel, ElementState::Pressed),
        n1_gamepad_button(GuiButtonClass::High, ElementState::Released),
    ]);
    main_assert!(app.definition_selector.is_none());
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);

    app.open_definition_selector(scenario.clone()).test_value();
    app.test_cursor(PhysicalPosition::new(10.0, 10.0));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.definition_selector_pointer_capture);
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Cancelled,
    ])
    .test_value();
    main_assert!(app.definition_selector_pointer_capture);
    app.test_left_button(ElementState::Released);
    main_assert!(!app.definition_selector_pointer_capture);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);

    app.open_definition_selector(scenario.clone()).test_value();
    app.test_touch(TouchPhase::Started, GuiPoint::new(10.0, 10.0));
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Cancelled,
    ])
    .test_value();
    main_assert!(app.definition_selector_pointer_capture);
    app.test_touch(TouchPhase::Ended, GuiPoint::new(10.0, 10.0));
    main_assert!(!app.definition_selector_pointer_capture);

    // Low activates OK on release; the remaining batch must not reach the game.
    let mut sandbox = scenario;
    sandbox.path = None;
    app.open_definition_selector(sandbox).test_value();
    let optional_index = app
        .definition_selector
        .test_ref()
        .rows()
        .iter()
        .position(|row| row.filename() == "Beta.C4D")
        .test_value();
    // DirectoryIterator order varies by filesystem (src/C4FileSelDlg.cpp:251-268),
    // so drive to the named optional row.
    let select_optional = (0..=optional_index)
        .map(|_| n1_gamepad_direction(ControlButton::Down, ElementState::Pressed));
    app.test_gamepad_events(select_optional.chain([
        n1_gamepad_direction(ControlButton::Right, ElementState::Pressed),
        n1_gamepad_direction(ControlButton::Right, ElementState::Pressed),
        n1_gamepad_button(GuiButtonClass::Low, ElementState::Pressed),
        n1_gamepad_button(GuiButtonClass::Low, ElementState::Released),
        n1_gamepad_action(GamepadActionType::MenuToggle, ElementState::Pressed),
    ]));
    main_assert!(matches!(app.mode, AppMode::Running));
    main_assert!(app.definition_selector.is_none());
    main_assert!(app.ingame_menu.is_none());

    // Pin the async handoff, rooted precedence, restart, and exact save/load.
    app.return_to_menu();
    let scenario_path = user_data.path().join("Start.c4s");
    let rooted_objects = definition_root.join("Objects.c4d");
    let original_objects = executable_data.path().join("Objects.c4d");
    fs::create_dir_all(&scenario_path).test_value();
    fs::create_dir_all(&rooted_objects).test_value();
    fs::create_dir_all(&original_objects).test_value();
    fs::write(
        rooted_objects.join("DefCore.txt"),
        "[DefCore]\nid=ROOT\nName=Rooted\nCategory=1\n",
    )
    .test_value();
    fs::write(
        original_objects.join("DefCore.txt"),
        "[DefCore]\nid=ORIG\nName=Original\nCategory=1\n",
    )
    .test_value();
    write_test_definition_graphics(&rooted_objects);
    write_test_definition_graphics(&original_objects);
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Definition Root Start\n",
    )
    .test_value();
    let mut rooted_scenario = FrontendScenario::fallback();
    rooted_scenario.identifier = "rooted-start".to_string();
    rooted_scenario.title = "Definition Root Start".to_string();
    rooted_scenario.path = Some(scenario_path.clone());
    app.start_scenario_with_definition_modules(
        rooted_scenario.clone(),
        vec!["Objects.c4d".to_string()],
        Some(definition_root.clone()),
    )
    .test_value();
    wait_for_running(&mut app);
    let expected_effective = vec![
        rooted_objects.to_string_lossy().into_owned(),
        original_objects.to_string_lossy().into_owned(),
    ];
    main_assert!(matches!(
        app.active_definition_load.as_ref(),
        Some(ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root: None,
        }) if modules == &expected_effective
    ));
    app.restart_current_scenario().test_value();
    wait_for_running(&mut app);
    main_assert!(matches!(
        app.active_definition_load.as_ref(),
        Some(ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root: None,
        }) if modules == &expected_effective
    ));
    app.quick_save().test_value();
    app.active_definition_load = None;
    app.quick_load().test_value();
    main_assert!(matches!(
        app.active_definition_load.as_ref(),
        Some(ScenarioDefinitionLoad::Fixed {
            modules,
            definition_root: None,
        }) if modules == &expected_effective
    ));
    cleanup_quicksave_file();
}

#[test]
fn network_create_navigates_nested_selector_and_retains_netdlg_without_binding() {
    let mut target = FrontendScenario::fallback();
    target.identifier = "outer/inner/target.c4s".to_string();
    target.title = "Deep Target".to_string();
    target.path = None;
    target.allow_user_change = Some(true);

    let mut inner = FrontendScenario::fallback();
    inner.identifier = "outer/inner".to_string();
    inner.title = "Inner Target Folder".to_string();
    inner.kind = ScenarioKind::Folder;
    inner.is_playable = false;
    inner.path = None;
    inner.children = vec![target.clone()];

    let mut outer = FrontendScenario::fallback();
    outer.identifier = "outer".to_string();
    outer.title = "Outer Folder".to_string();
    outer.kind = ScenarioKind::Folder;
    outer.is_playable = false;
    outer.path = None;
    outer.children = vec![inner.clone()];

    let scenarios = vec![outer];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app(800, 600);
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);
    app.open_network_game_dialog();
    app.startup_network_dialog
        .test_mut()
        .set_join_address("remembered.example:11112");

    app.process_network_dialog_actions(vec![
        clonk_frontend::startup_netdlg::NetDlgAction::CreateGame,
    ])
    .test_value();
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert_eq!(app.scenario_game_options.context() => GameOptionContext::NetworkHostSelector);
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.startup_network_connection.is_none());

    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("outer"));
    app.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Enter))
        .test_value();
    app.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))
        .test_value();
    main_assert_eq!(app.menu_state.stack.iter().map(|layer| layer.title.as_str()).collect::<Vec<_>>() => vec!["Scenarios", "Outer Folder"]);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("outer/inner"));
    app.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Enter))
        .test_value();
    app.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))
        .test_value();
    main_assert_eq!(app.menu_state.stack.iter().map(|layer| layer.title.as_str()).collect::<Vec<_>>() => vec!["Scenarios", "Outer Folder", "Inner Target Folder"]);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);

    app.scensel_do_back().test_value();
    main_assert_eq!(app.menu_state.stack.len() => 2);
    main_assert_eq!(app.menu_state.book_caption() => "Outer Folder");
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);

    app.open_definition_selector(target.clone()).test_value();
    main_assert_eq!(app.pending_definition_selection.as_ref().map(|pending| pending.selector_mode) => Some(ScenarioSelectorMode::NetworkHost));
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::RefreshRequested,
    ])
    .test_value();
    main_assert_eq!(app.pending_definition_selection.as_ref().map(|pending| pending.selector_mode) => Some(ScenarioSelectorMode::NetworkHost));
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Cancelled,
    ])
    .test_value();
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);

    app.scensel_do_back().test_value();
    main_assert_eq!(app.menu_state.stack.len() => 1);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    app.scensel_do_back().test_value();
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert_eq!(n1_expect(&app.startup_network_dialog, "same retained NetDlg").join_address() => "remembered.example:11112");
    main_assert!(app.startup_network_connection.is_none());

    // A pathless stage failure returns through QuitGame to the host selector,
    // never a bound socket (src/C4Application.cpp:373-405,438-450).
    app.process_network_dialog_actions(vec![
        clonk_frontend::startup_netdlg::NetDlgAction::CreateGame,
    ])
    .test_value();
    app.open_definition_selector(target).test_value();
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Accepted(Vec::new()),
    ])
    .test_value();
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert!(app.startup_network_connection.is_none());
    main_assert!(app.network.is_none());
    main_assert!(app.network_lobby.is_none());
}

#[test]
fn retained_netdlg_refreshes_internet_and_staged_host_keeps_options_noninteractive() {
    let (_user_data, _guard, paths) = n1_repository_paths();
    let repository = test_repository_root();
    persist_config_value(&paths, "General", "LanguageEx", "US".to_string()).test_value();
    persist_config_value(&paths, "Network", "LocalName", "Host Tester".to_string()).test_value();
    persist_config_value(&paths, "Network", "MasterServerSignUp", "0".to_string()).test_value();
    persist_config_value(&paths, "General", "Record", "0".to_string()).test_value();
    let mut app = GameApp::new(
        800,
        600,
        AudioOptions::default(),
        Some(&paths),
        RuntimeConfig {
            player_owner: 1,
            player_name: "Host Tester".to_string(),
            network: None,
            record_enabled: false,
        },
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_network_game_dialog();
    app.startup_network_dialog
        .test_mut()
        .set_join_address("remembered.example:11112");
    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(
        app.assets.clonk_fonts.as_deref().test_value(),
    );
    let net_layout = clonk_frontend::startup_netdlg::net_dlg_layout(800, 600, &metrics);
    let mut internet_off = vec![0_u8; 800 * 600 * 4];
    app.test_render(&mut internet_off);
    let internet_point = PhysicalPosition::new(
        f64::from(net_layout.btn_internet.x + net_layout.btn_internet.w / 2),
        f64::from(net_layout.btn_internet.y + net_layout.btn_internet.h / 2),
    );
    app.test_cursor(internet_point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(
        n1_expect(&app.startup_network_dialog, "live NetDlg")
            .config()
            .masterserver_signup
    );
    let mut internet_on = vec![0_u8; 800 * 600 * 4];
    app.test_render(&mut internet_on);
    main_assert!(
        (net_layout.list_entry.y..net_layout.list_entry.y + net_layout.list_entry.h).any(|y| {
            (net_layout.list_entry.x..net_layout.list_entry.x + net_layout.list_entry.w).any(|x| {
                let offset = ((y * 800 + x) * 4) as usize;
                internet_off[offset..offset + 4] != internet_on[offset..offset + 4]
            })
        })
    );
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert!(
        !n1_expect(&app.startup_network_dialog, "live NetDlg")
            .config()
            .masterserver_signup
    );

    let chat_point = GuiPoint::new(
        (net_layout.btn_chat.x + net_layout.btn_chat.w / 2) as f32,
        (net_layout.btn_chat.y + net_layout.btn_chat.h / 2) as f32,
    );
    let fonts = app.assets.clonk_fonts.clone().test_value();
    {
        let dialog = app.startup_network_dialog.test_mut();
        let _ = dialog.handle_pointer_down(chat_point, &fonts.text);
        let _ = dialog.handle_pointer_up(chat_point, &fonts.text);
        let _ = dialog.handle_key_down(KeyCode::Tab);
        let _ = dialog.handle_key_down(KeyCode::Tab);
        main_assert_eq!(dialog.mode() => clonk_frontend::startup_netdlg::NetDlgMode::Chat);
        main_assert_eq!(dialog.focused_control() => clonk_frontend::startup_netdlg::NetDlgControl::ChatInput);
        main_assert_eq!(dialog.chat_login_field() => clonk_frontend::startup_netdlg::NetDlgChatLoginField::RealName);
    }
    app.open_network_host_scenario_browser();
    app.process_game_option_actions(vec![
        GameOptionAction::InternetSignupChanged {
            enabled: true,
            live_lobby: false,
        },
        GameOptionAction::RecordPreferenceChanged(true),
    ])
    .test_value();
    app.close_scenario_browser();
    let retained = app.startup_network_dialog.test_ref();
    main_assert!(retained.config().masterserver_signup);
    main_assert!(
        !retained.config().record,
        "Record staleness is oracle-faithful"
    );
    main_assert_eq!(retained.join_address() => "remembered.example:11112");
    main_assert_eq!(retained.mode() => clonk_frontend::startup_netdlg::NetDlgMode::Chat);
    main_assert_eq!(retained.focused_control() => clonk_frontend::startup_netdlg::NetDlgControl::ChatInput);
    main_assert_eq!(retained.chat_login_field() => clonk_frontend::startup_netdlg::NetDlgChatLoginField::RealName);

    app.open_network_host_scenario_browser();
    let accepted_options = GameOptionValues {
        master_server_signup: true,
        league_server_signup: false,
        password: "round secret".to_string(),
        last_password: "round secret".to_string(),
        comment: "recursive host".to_string(),
        fair_crew: true,
        fair_crew_strength: 777,
        record: true,
        ..GameOptionValues::default()
    };
    app.scenario_game_options = GameOptionButtons::new(
        GameOptionContext::NetworkHostSelector,
        accepted_options.clone(),
    );
    let mut frontend = FrontendScenario::fallback();
    frontend.identifier = "Tutorial.c4f/Tutorial01.c4s".to_string();
    frontend.title = "A Clonk".to_string();
    frontend.path = Some(repository.join("content/Tutorial.c4f/Tutorial01.c4s"));
    let definition_load = ScenarioDefinitionLoad::Seed {
        modules: vec!["Objects.c4d".to_string()],
        definition_root: None,
    };
    let staged = app
        .prepare_network_host_scenario(frontend, definition_load)
        .test_value();
    main_assert_eq!(staged.options => accepted_options);
    main_assert_eq!(staged.options.password => "round secret");
    app.staged_network_host_scenario = Some(staged);

    let (sender, receiver) = mpsc::channel();
    app.begin_startup_network_connection(receiver, StartupNetworkPurpose::StagedHost, None, None)
        .test_value();
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert!(app.startup_network_transition_active());
    let join_before = app
        .startup_network_dialog
        .test_ref()
        .join_address()
        .to_string();
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.process_network_dialog_actions(vec![
        clonk_frontend::startup_netdlg::NetDlgAction::Back,
        clonk_frontend::startup_netdlg::NetDlgAction::CreateGame,
    ])
    .test_value();
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert_eq!(n1_expect(&app.startup_network_dialog, "same transition NetDlg").join_address() => join_before);
    let reported_host_error = "host preparation failed: initial host resources could not be published: failed to publish System resource /missing/planet/System.c4g: host C4Group could not be read: No such file or directory (os error 2)";
    sender
        .send(Err(NetworkStartError::Other(
            reported_host_error.to_string(),
        )))
        .test_value();
    app.poll_startup_network_connection().test_value();
    main_assert!(!app.startup_network_transition_active());
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert_eq!(app.last_startup_dialog => StartupDialog::ScenarioBrowser(ScenarioSelectorMode::NetworkHost));
    main_assert_eq!(app.startup.scenario_back_dialog => None);
    assert_startup_error_log(
        &app,
        &format!("Unable to start network session: {reported_host_error}"),
    );
    main_assert!(app.staged_network_host_scenario.is_none());
    // PreInit rebuilds the loader screen (src/C4Application.cpp:242-247,373-389).
    main_assert!(app.loader_screen.is_some());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    let mut frame = vec![0x4c; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    let stale = prepare_tutorial_host_lobby(&app, repository);
    app.staged_network_host_scenario = Some(stale);
    app.stage_network_host_scenario(
        FrontendScenario::fallback(),
        ScenarioDefinitionLoad::Seed {
            modules: vec!["Objects.c4d".to_string()],
            definition_root: None,
        },
    )
    .test_value();
    main_assert!(app.staged_network_host_scenario.is_none());
    main_assert!(app.startup_network_connection.is_none());
}

#[test]
fn unstaged_host_connection_returns_to_host_selector_with_error_log() {
    let mut app = new_real_classic_menu_app(800, 600);
    app.open_network_game_dialog();
    let (manager, _events) = NetworkManager::test_stub();
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok((
            NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
                player_name: "Host".to_string(),
                prepared: None,
            }),
            manager,
        )))
        .test_value();
    app.startup_network_connection = Some(StartupNetworkConnection::new(
        receiver,
        None,
        StartupNetworkPurpose::StagedHost,
    ));
    app.poll_startup_network_connection().test_value();
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert_ne!(app.startup.view => StartupView::NetworkLobby);
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.network.is_none(), "headless listener must be dropped");
    main_assert!(app.network_mode.is_none());
    assert_startup_error_log(
                &app,
                "Network lobby unavailable: classic game-lobby model is unavailable: host connection completed without a staged scenario; refusing guessed lobby state",
            );
    let mut frame = vec![0x4c; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn failed_host_staging_returns_to_host_selector_with_error_log() {
    let mut app = new_real_classic_menu_app(800, 600);
    app.open_network_game_dialog();
    let scenario = FrontendScenario::fallback();
    let title = scenario.title.clone();
    let Err(staging_error) = app.prepare_network_host_scenario(
        scenario.clone(),
        ScenarioDefinitionLoad::Seed {
            modules: vec!["Objects.c4d".to_string()],
            definition_root: None,
        },
    ) else {
        panic!("a pathless fixture cannot be staged for hosting");
    };

    app.stage_network_host_scenario(
        scenario,
        ScenarioDefinitionLoad::Seed {
            modules: vec!["Objects.c4d".to_string()],
            definition_root: None,
        },
    )
    .test_value();

    // Failed OpenGame reconstructs the startup dialog and Error Log without
    // terminating (src/C4Application.cpp:373-405,438-450; src/C4Startup.cpp:274-307).
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scensel.mode => ScenarioSelectorMode::NetworkHost);
    main_assert_eq!(app.last_startup_dialog => StartupDialog::ScenarioBrowser(ScenarioSelectorMode::NetworkHost));
    main_assert!(app.staged_network_host_scenario.is_none());
    main_assert!(app.startup_network_connection.is_none());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    assert_startup_error_log(&app, &format!("Cannot host {title}: {staging_error}"));
    // A retained fatal status would terminate rendering (clonk-org/clonk-rs#196).
    app.preflight_startup_presentation().test_value();
    let mut frame = vec![0x4c; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x4c));
}

#[test]
fn staged_host_uses_startup_gui_but_pending_failure_beats_start_and_pixels() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let repository = test_repository_root();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let mut staged = prepare_tutorial_host_lobby(&app, repository);
    let source =
        "Tutorial01.c4s/Graphics.c4g:GUISpinBoxArrow.bmp: not a decodable image".to_string();
    staged
        .pending_global_gui_failures
        .insert("GUISpinBoxArrow", source.clone());
    app.staged_network_host_scenario = Some(staged);
    install_test_classic_host_lobby(&mut app);

    let mut visible = vec![0_u8; 640 * 480 * 4];
    main_assert!(app
        .render(&mut visible)
        .expect("visible lobby keeps the accepted startup GUI bundle"));

    let error = app
        .process_classic_lobby_actions(n1_lobby_start_request(false))
        .expect_err("pending scenario GUI failure must beat the Start child");
    assert_engine_parity_boundary(
        error,
        ClassicParityBoundary::GlobalGuiBootstrapResources {
            issues: vec![ClassicGuiBootstrapIssue::malformed(
                "GUISpinBoxArrow",
                "a readable selected bmp/jpeg/jpg/png RGBA surface",
                source,
            )],
        },
    );
    let mut after_rejected_start = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut after_rejected_start);
    main_assert_eq!(after_rejected_start => visible, "a rejected Start child must not change what the lobby shows");
    main_assert!(app.classic_host_lobby.is_some());
    main_assert!(app.active_global_gui_failures.is_empty());

    remove_global_gui_sheet(&mut app, "GUIBigArrows.png");
    let mut frame = vec![0x97; 640 * 480 * 4];
    let error = app
        .render(&mut frame)
        .expect_err("lobby must preflight startup GUI before pixels");
    assert_global_gui_boundary(
        &error,
        vec![ClassicGuiBootstrapIssue::missing("GUIBigArrows")],
    );
    main_assert!(frame.iter().all(|byte| *byte == 0x97));
}

#[test]
fn staged_host_prebind_sanitizes_identity_and_keeps_other_gates() {
    let content = tempdir();
    let (user_data, _guard, paths) = n1_test_paths(Some(content.path()));
    let scenario = install_minimal_prepared_host_fixture(content.path());
    configure_test_startup_participant(&paths, user_data.path());
    let undiscovered_player_root = user_data.path().join("undiscovered-players");
    fs::create_dir(&undiscovered_player_root).test_value();
    persist_config_value(
        &paths,
        "General",
        "PlayerPath",
        undiscovered_player_root.to_string_lossy(),
    )
    .test_value();
    persist_config_value(&paths, "General", "LanguageEx", "DE").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    main_assert!(
        !app.startup
            .player_models
            .iter()
            .any(|player| player.activated),
        "regression requires a raw participant omitted by discovery"
    );
    let make_frontend = || scenario.clone();
    let definition_load = minimal_prepared_host_definition_load;

    let staged = app
        .prepare_network_host_scenario(make_frontend(), definition_load())
        .test_value();
    main_assert_eq!(staged.lobby.max_players => 1);

    persist_config_value(&paths, "General", "Participants", "").test_value();
    persist_config_value(&paths, "Network", "LocalName", "<i<i<i<i>>>>").test_value();
    persist_config_value(&paths, "Network", "Nick", "").test_value();
    let staged = app
        .prepare_network_host_scenario(make_frontend(), definition_load())
        .test_value();
    main_assert_eq!(staged.lobby.local_name => "<i<i>>");
    main_assert_eq!(staged.lobby.nick => "Unknown");
    main_assert!(app.network.is_none());
    main_assert!(app.startup_network_connection.is_none());

    persist_config_value(&paths, "Network", "LocalName", "Changed Host").test_value();
    persist_config_value(&paths, "Network", "Nick", "Changed Nick").test_value();
    let preparation = build_network_host_preparation(
        &app,
        &staged.frontend,
        &staged.definition_load,
        &staged.effective_definition_modules,
        &staged.definition_resources,
        Some((&staged.definition_executable_path, &staged.definition_path)),
        Some((&staged.lobby.local_name, &staged.lobby.nick)),
    )
    .test_value();
    main_assert_eq!(preparation.host_name => staged.lobby.local_name);
    main_assert_eq!(preparation.host_nick => staged.lobby.nick);
    let prepared = preparation.prepare().test_value();
    main_assert_eq!(prepared.host_config().local_core.name.as_bytes() => b"<i<i>>");
    main_assert_eq!(prepared.host_config().local_core.nick.as_bytes() => b"Unknown");

    persist_config_value(&paths, "Network", "LocalName", "Exact Host").test_value();
    let error = app
        .prepare_network_host_scenario(FrontendScenario::fallback(), definition_load())
        .err()
        .test_value();
    main_assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Model { detail }))
            if detail.contains("transferable scenario")
    ));

    app.loader_render_config = Some(LoaderRenderConfig::new(2.0, false).test_value());
    app.prepare_network_host_scenario(make_frontend(), definition_load())
        .test_value();
    main_assert!(app.network.is_none());
    main_assert!(app.startup_network_connection.is_none());
}

#[test]
fn control_rate_submits_relative_set_and_waits_for_echo() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let mut app = new_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    app.engine.set_control_rate(4);
    app.network_control_clock = Some(NetworkControlClock::new(0, 4));
    main_assert!(app.select_classic_lobby_sheet(LobbySheet::Options));

    app.submit_classic_lobby_control_rate(7);
    let sets = commands.take_submitted_control_sets();
    main_assert_eq!(sets => [n1_fixture!(control_set: 0, 3, 0)]);
    main_assert_eq!(app.engine.control_rate() => 4);
    main_assert_eq!(app.network_control_clock.unwrap().control_rate() => 4, "the menu selection is not an optimistic clock mutation");
    main_assert_eq!(n1_lobby_option(&app, LobbyOptionKind::ControlRate).map(|row| row.value.as_str()) => Some("4"));

    app.execute_control_set(sets[0]);
    main_assert_eq!(app.engine.control_rate() => 7);
    main_assert_eq!(app.network_control_clock.unwrap().control_rate() => 7);
    main_assert_eq!(app.config.deferred.get("Network", "ControlRate") => Some("7"), "only the authoritative host echo records the next-session setting");
    // C4ControlSet updates memory; shutdown saves it (C4Control.cpp:141).
    app.flush_deferred_config();
    main_assert_eq!(Config::load(paths.config_file()).expect("reload echoed control rate").get_in(Some("Network"), "ControlRate") => Some("7"));
    main_assert_eq!(n1_lobby_option(&app, LobbyOptionKind::ControlRate).map(|row| row.value.as_str()) => Some("7"));

    app.submit_classic_lobby_control_rate(7);
    app.submit_classic_lobby_control_rate(10);
    main_assert!(commands.take_submitted_control_sets().is_empty());
}

#[test]
fn runtime_join_persists_inverse_policy_and_refreshes_the_host_row() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    main_assert!(app.select_classic_lobby_sheet(LobbySheet::Options));

    app.set_classic_lobby_runtime_join(true);
    main_assert!(
        commands.take_lobby_start_commands().is_empty(),
        "changing the future policy must not close current lobby admission"
    );
    main_assert!(app
        .classic_host_lobby
        .as_ref()
        .is_some_and(|lobby| lobby.runtime_join_allowed));
    main_assert_eq!(n1_lobby_option(&app, LobbyOptionKind::RuntimeJoin).map(|row| row.value.as_str()) => Some("Runtime join allowed"));
    // C4GameOptions only updates NoRuntimeJoin in memory (C4GameOptions.cpp:169).
    main_assert_eq!(app.config.deferred.get("Network", "NoRuntimeJoin") => Some("0"));
    app.flush_deferred_config();
    main_assert_eq!(Config::load(paths.config_file()).expect("reload enabled runtime-join policy").get_in(Some("Network"), "NoRuntimeJoin") => Some("0"));

    app.set_classic_lobby_runtime_join(false);
    main_assert!(
        commands.take_lobby_start_commands().is_empty(),
        "the prohibited policy is applied only when the lobby exits"
    );
    main_assert!(!app
        .classic_host_lobby
        .as_ref()
        .is_some_and(|lobby| lobby.runtime_join_allowed));
    app.flush_deferred_config();
    main_assert_eq!(Config::load(paths.config_file()).expect("reload prohibited runtime-join policy").get_in(Some("Network"), "NoRuntimeJoin") => Some("1"));
}

#[test]
fn random_team_count_mutates_host_directly_and_tracks_distribution() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let mut metadata = set_control_test_metadata(
        true,
        vec![
            set_control_test_team(1, vec![1], 0),
            set_control_test_team(2, vec![2], 0),
        ],
    );
    metadata.team_distribution = clonk_engine::InitialNetworkTeamDistribution::Random;
    let mut snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    snapshot.parameters.teams = clonk_network::join_team_list_snapshot(metadata.clone());
    app.host_join_snapshot = Some(snapshot);
    app.players.team_assignment = Some(NetworkTeamAssignmentState::from_prepared_host(metadata));
    app.control_player_infos.replace_snapshot(
        3,
        [n1_fixture!(player_data {
            client_id: 0,
            players: vec![
                set_control_test_player(1, 1, 0),
                set_control_test_player(2, 2, clonk_engine::PLAYER_INFO_FLAG_INVISIBLE),
                set_control_test_player(3, 1, clonk_engine::PLAYER_INFO_FLAG_REMOVED),
            ],
        })],
    );
    main_assert!(app.select_classic_lobby_sheet(LobbySheet::Options));
    let random = n1_lobby_option(&app, LobbyOptionKind::RandomTeamCount).test_value();
    main_assert_eq!(
        random
            .choices
            .iter()
            .map(|choice| choice.id)
            .collect::<Vec<_>>() =>
        [0, 2],
        "the maximum includes invisible players and excludes removed players"
    );

    app.set_classic_lobby_random_team_count(2);
    let (player_infos, snapshots) = commands.take_team_control_updates();
    main_assert_eq!(player_infos.len() => 1);
    main_assert_eq!(snapshots.len() => 1);
    let teams = app.players.team_assignment.as_ref().test_value().teams();
    main_assert_eq!(teams.random_team_count => 2);
    main_assert_eq!(teams.teams.len() => 2);
    main_assert_eq!(app.host_join_snapshot.as_ref().unwrap().parameters.teams.random_team_count => 2);
    main_assert_eq!(n1_lobby_option(&app, LobbyOptionKind::RandomTeamCount).map(|row| row.value.as_str()) => Some("2"));

    app.execute_control_set(n1_fixture!(control_set: 3, 0, 0));
    main_assert!(n1_lobby_option(&app, LobbyOptionKind::RandomTeamCount).is_none());
    app.execute_control_set(n1_fixture!(control_set: 3, 4, 0));
    main_assert!(n1_lobby_option(&app, LobbyOptionKind::RandomTeamCount).is_some());
}

#[test]
fn classic_host_start_persists_and_honors_unassociated_savegame_warning() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);

    app.process_classic_lobby_actions(n1_lobby_start_request(true))
        .test_value();

    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(app.host_lobby_countdown.is_none());
    let warning = app.dialogs.messages.last().test_value();
    main_assert_eq!(warning.state.caption() => "Player assignment");
    main_assert_eq!(warning.state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Standard(12));
    main_assert!(matches!(
        warning.continuation,
        MessageDialogContinuation::ClassicLobbyStart {
            countdown_seconds: 5
        }
    ));
    main_assert_eq!(warning.state.checkbox_checked() => Some(false));

    app.dialogs
        .messages
        .last_mut()
        .test_value()
        .state
        .handle_hotkey('D');
    app.persist_top_message_dialog_checkbox_changes();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();
    main_assert!(commands.take_submitted_lobby_countdowns().is_empty());
    main_assert!(app.host_lobby_countdown.is_none());
    // ShowMessageModal updates the in-memory flag (C4GameLobby.cpp:462).
    main_assert_eq!(app.config.deferred.get("Startup", "HideMsgPlrNoTakeOver") => Some("1"));
    app.flush_deferred_config();
    main_assert_eq!(Config::load(paths.config_file()).expect("load persisted warning preference").get_in(Some("Startup"), "HideMsgPlrNoTakeOver") => Some("1"));

    app.process_classic_lobby_actions(n1_lobby_start_request(true))
        .test_value();
    main_assert!(app.dialogs.messages.is_empty());
    main_assert_eq!(commands.take_submitted_lobby_countdowns() => vec![clonk_network::LobbyCountdownPacket::new(5)]);
    main_assert_eq!(app.host_lobby_countdown => Some(HostLobbyCountdown::new()));
}

#[test]
fn classic_host_savegame_warning_ignores_an_assigned_restore_player() {
    // FindUnassociatedRestoreInfo scans associations, not the section caption
    // (C4PlayerInfoListBox.cpp:1372-1397; C4PlayerInfo.cpp:1125-1139).
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    install_test_free_savegame_player_row(&mut app, 50);
    let lobby = app.classic_host_lobby.test_mut();
    let rows = lobby
        .controller
        .rows()
        .iter()
        .filter(|row| !matches!(row, LobbyRosterRow::Player(player) if player.client_id == -1))
        .cloned()
        .collect();
    lobby.controller.set_rows(rows);

    app.control_player_infos.replace_snapshot(
        91,
        [n1_fixture!(player_data {
            client_id: 0,
            players: vec![n1_fixture!(player {
                id: 91,
                savegame_player: 50,
            })],
        })],
    );
    let mut snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    snapshot.parameters.restore_player_infos = n1_restore_player_infos(50);
    app.host_join_snapshot = Some(snapshot);

    main_assert!(app
        .classic_host_lobby
        .test_ref()
        .controller
        .rows()
        .iter()
        .any(|row| matches!(row, LobbyRosterRow::Header(header)
            if header.kind == LobbyRosterHeader::UnassignedSavegamePlayers)));
    app.process_classic_lobby_actions(n1_lobby_start_request(true))
        .test_value();
    main_assert!(app.dialogs.messages.is_empty());
    main_assert_eq!(commands.take_submitted_lobby_countdowns() => vec![clonk_network::LobbyCountdownPacket::new(5)]);
}

#[test]
fn classic_host_regular_scenario_never_warns_about_restore_rows() {
    // Only savegames call FindUnassociatedRestoreInfo (C4GameLobby.cpp:452-464).
    let content = tempdir();
    let (_user_data, _guard, paths) = n1_test_paths(Some(content.path()));
    let scenario = install_minimal_prepared_host_fixture(content.path());
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.staged_network_host_scenario = Some(prepare_minimal_host_lobby(&app, scenario));
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let mut snapshot = clonk_network::HostConfig::default()
        .initial_join_snapshot
        .test_value();
    snapshot.parameters.restore_player_infos = n1_restore_player_infos(50);
    app.host_join_snapshot = Some(snapshot);

    app.process_classic_lobby_actions(n1_lobby_start_request(true))
        .test_value();

    main_assert!(app.dialogs.messages.is_empty());
    main_assert_eq!(commands.take_submitted_lobby_countdowns() => vec![clonk_network::LobbyCountdownPacket::new(5)]);
}

#[test]
fn client_start_and_abort_report_the_cpp_host_only_error() {
    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));

    if let Some(lobby) = app.network_lobby.as_mut() {
        lobby.chat_history_index = 3;
        lobby.chat_edit.text = "stale".to_string();
    }
    app.process_lobby_action(LobbyAction::SubmitMessage(String::new()))
        .test_value();
    main_assert_eq!(app.sound.ui_log => ["Error"]);
    let lobby = app.network_lobby.as_ref().test_value();
    main_assert_eq!(lobby.chat_history_index => -1);
    main_assert!(lobby.chat_edit.text.is_empty());

    app.process_lobby_action(LobbyAction::SubmitMessage("/start 10".to_string()))
        .test_value();
    app.process_lobby_action(LobbyAction::SubmitMessage("/abort".to_string()))
        .test_value();

    main_assert_eq!(
        app.network_lobby
            .test_ref()
            .logs
            .iter()
            .map(|line| (&*line.text, line.color))
            .collect::<Vec<_>>() =>
        [
            ("Host only!", [255, 32, 32, 255]),
            ("Host only!", [255, 32, 32, 255]),
        ]
    );
}

/// A running host must advertise the runtime-join admission it is actually
/// enforcing, not the one its reference template was built with.
///
/// C++ never consults `fAllowJoin` on the accept path. It publishes it -
/// `C4Network2Reference::InitLocal` copies `Network.isJoinAllowed()`
/// (`src/C4Network2Reference.cpp:75`) - and the *client's* browser is what
/// gates the join, showing the runtime-join affordance only for a reference
/// that is both join-allowed and past the lobby
/// (`src/C4StartupNetDlg.cpp:480`) and refusing one that is not (`:495`). So a
/// stale advertised flag is not cosmetic: it is the whole mechanism, and a
/// host that advertises the wrong value cannot be runtime-joined at all
/// (clonk-org/clonk-rs#948).
///
/// `HostSettings::prepared` is `None` for a host that did not come through the
/// prepared-lobby flow, which is exactly when the published value used to fall
/// back to the template instead of the live admission.
/// clonk-org/clonk-rs#583: the profile has to reach the reference a peer reads,
/// and clonk-org/clonk-rs#588's fail-closed rule decides *which* value that is.
/// `compat_profile` is what the player asked for and is never rewritten;
/// `claimed_compat_profile` is what the contract can currently back. Publishing
/// the request would invite a peer into a session that does not behave that
/// way, which is the desync the profile exists to prevent.
#[test]
fn a_running_host_advertises_only_the_profile_it_can_claim() {
    let mut app = new_menu_app(640, 480);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11113)),
        player_name: "Profile Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, _commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    app.network = Some(manager);
    let (snapshot, reference) = default_exact_host_reference();
    main_assert_eq!(
        reference.summary().compat_profile.clone() => None,
        "the template names no profile, so a published one came from the host"
    );
    app.control_clients = ControlClientRegistry::default();
    app.control_clients
        .replace_snapshot(snapshot.parameters.clients.clients.clone());
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);

    // An ordinary session advertises nothing, so its reference stays exactly
    // what a host that never heard of the profile publishes.
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    app.publish_running_host_reference();
    main_assert_eq!(
        app.advertised_game_reference
            .test_ref()
            .summary()
            .compat_profile
            .clone() => None,
        "an ordinary host must not name a profile"
    );

    // Requesting a profile the contract cannot back publishes nothing either —
    // and the request itself is left alone.
    app.config.compat_profile = crate::settings::CompatProfile::LegacyClonk;
    app.publish_running_host_reference();
    let advertised = app
        .advertised_game_reference
        .test_ref()
        .summary()
        .compat_profile
        .clone();
    let expected = match app.claimed_compat_profile() {
        crate::settings::CompatProfile::Normal => None,
        profile => Some(profile.display_name().to_string()),
    };
    main_assert_eq!(
        advertised => expected,
        "the advertised profile follows what may be claimed, not what was asked"
    );
    main_assert_eq!(
        app.config.compat_profile => crate::settings::CompatProfile::LegacyClonk,
        "and the request is never rewritten by publishing"
    );
    if !crate::compat_readiness::is_ready() {
        main_assert_eq!(
            advertised => None,
            "a blocked profile is not advertised at all"
        );
    }
}

/// clonk-org/clonk-rs#583: mixed incompatible Rust profiles must be rejected
/// before lobby or game state diverges, and C++'s model is that the *browser*
/// refuses rather than the host (`src/C4StartupNetDlg.cpp:480,495`).
///
/// The silent case is the one worth stating: a reference that names no profile
/// is a stock C++ host — `StdCompilerINIRead` reads by name, so C++ never
/// writes the key — and playing with exactly that peer is the whole purpose of
/// the compatibility profile, so it must stay joinable from either side.
#[test]
fn a_reference_naming_a_profile_this_client_cannot_match_is_refused() {
    let mut app = new_menu_app(640, 480);
    let silent = clonk_network::NetworkGameReference::default();
    let legacy = clonk_network::NetworkGameReference {
        compat_profile: Some("LegacyClonk".to_string()),
        ..Default::default()
    };

    // Ordinary session: the silent legacy host is joinable, a profile-bearing
    // one is not.
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    main_assert_eq!(
        app.network_reference_profile_refusal(&silent) => None,
        "a stock C++ host names no profile and must stay joinable"
    );
    let refusal = app.network_reference_profile_refusal(&legacy).test_value();
    main_assert!(
        refusal.contains("LegacyClonk") && refusal.contains("desync"),
        "the refusal names the profile and why: {refusal}"
    );

    // Asking for the host's profile does not by itself make the join legal —
    // what matters is whether this session can *claim* it.
    app.config.compat_profile = crate::settings::CompatProfile::LegacyClonk;
    let claimed = app.claimed_compat_profile();
    let matched = app.network_reference_profile_refusal(&legacy);
    if claimed == crate::settings::CompatProfile::LegacyClonk {
        main_assert_eq!(
            matched => None,
            "a session that can claim the advertised profile joins"
        );
    } else {
        let refusal = matched.test_value();
        main_assert!(
            refusal.contains("cannot currently claim"),
            "an unclaimable profile must say the mismatch is ours: {refusal}"
        );
    }

    // The silent host stays joinable whatever this session asked for; that is
    // the compatibility profile's entire point.
    main_assert_eq!(
        app.network_reference_profile_refusal(&silent) => None,
        "a compatibility-profile client exists in order to join a stock host"
    );

    // Case is not the discriminator: a peer spelling the profile differently
    // names the same one. The refusal text quotes the peer's own spelling, so
    // compare the decision rather than the message.
    let lowercase = clonk_network::NetworkGameReference {
        compat_profile: Some("legacyclonk".to_string()),
        ..Default::default()
    };
    main_assert_eq!(
        app.network_reference_profile_refusal(&lowercase).is_some()
            => app.network_reference_profile_refusal(&legacy).is_some(),
        "profile names compare case-insensitively"
    );

    // And the match itself is case-insensitive: an ordinary session joins a
    // host that spells the ordinary profile in any case.
    app.config.compat_profile = crate::settings::CompatProfile::Normal;
    let spelled = clonk_network::NetworkGameReference {
        compat_profile: Some(
            crate::settings::CompatProfile::Normal
                .display_name()
                .to_lowercase(),
        ),
        ..Default::default()
    };
    main_assert_eq!(
        app.network_reference_profile_refusal(&spelled) => None,
        "the same profile in another case is the same profile"
    );
}

#[test]
fn a_running_host_advertises_its_live_runtime_join_admission() {
    let mut app = new_menu_app(640, 480);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 11112)),
        player_name: "Exact Host".to_string(),
        prepared: None,
    }));
    let (manager, _events, _commands) =
        NetworkManager::test_stub_with_league_commands_for_client_id(0);
    app.network = Some(manager);
    let (snapshot, reference) = default_exact_host_reference();
    main_assert!(
        reference.summary().join_allowed,
        "the template starts join-allowed, so a stale read is visible"
    );
    app.control_clients = ControlClientRegistry::default();
    app.control_clients
        .replace_snapshot(snapshot.parameters.clients.clients.clone());
    app.host_join_snapshot = Some(snapshot);
    app.advertised_game_reference = Some(reference);

    // The host barred runtime joins at runtime; nothing about that reaches
    // `prepared`, because there is no prepared host here.
    app.runtime_network_join_allowed = Some(false);
    app.publish_running_host_reference();

    main_assert!(
        !app.advertised_game_reference
            .as_ref()
            .expect("the running host republishes its reference")
            .summary()
            .join_allowed,
        "the advertised reference must follow the live admission, not the template"
    );

    // And back again, so this pins "follows the live value" rather than
    // "always false".
    app.runtime_network_join_allowed = Some(true);
    app.publish_running_host_reference();

    main_assert!(
        app.advertised_game_reference
            .as_ref()
            .expect("the running host republishes its reference")
            .summary()
            .join_allowed
    );
}

#[test]
fn set_comment_updates_state_reference_and_invalidation() {
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
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);

    let oversized = "x".repeat(clonk_frontend::game_option_buttons::COMMENT_MAX_TEXT + 1);
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(format!(
        "/set comment {oversized}"
    )))
    .test_value();

    let expected = "x".repeat(clonk_frontend::game_option_buttons::COMMENT_MAX_TEXT);
    main_assert_eq!(app.scenario_game_options.values().comment => expected);
    main_assert_eq!(app.advertised_game_reference.as_ref().expect("updated advertised reference").metadata().comment.as_bytes() => expected.as_bytes());
    main_assert_eq!(commands.take_league_update_effects().1 => 1);
    main_assert_eq!(
        n1_expect(&app.classic_host_lobby, "classic lobby")
            .controller
            .logs()
            .last()
            .map(|line| line.text.as_str()) =>
        Some(clonk_frontend::game_option_buttons::COMMENT_CHANGED_LOG)
    );
}

#[test]
fn set_password_sets_and_bare_command_clears_live_password() {
    let mut app = new_menu_app(640, 480);
    let (_events, mut commands) = install_classic_host_network_stub(&mut app);
    let (_snapshot, reference) = default_exact_host_reference();
    app.advertised_game_reference = Some(reference);
    let observer = thread::spawn(move || {
        let mut passwords = Vec::new();
        for _ in 0..2 {
            let (password, completion) = commands.receive_host_password();
            passwords.push(password.as_bytes().to_vec());
            completion.send(Ok(())).test_value();
        }
        passwords
    });

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit(
        "/set password secret".to_string(),
    ))
    .test_value();
    main_assert_eq!(app.scenario_game_options.values().password => "secret");
    main_assert!(
        app.advertised_game_reference
            .as_ref()
            .expect("password-protected reference")
            .summary()
            .password_needed
    );

    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/set password".to_string()))
        .test_value();
    main_assert!(app.scenario_game_options.values().password.is_empty());
    main_assert!(
        !app.advertised_game_reference
            .as_ref()
            .expect("unprotected reference")
            .summary()
            .password_needed
    );
    main_assert_eq!(observer.join().expect("password observer") => [b"secret".to_vec(), Vec::new()]);
}

#[test]
fn client_start_wait_escape_and_abort_clear_network_and_return_to_main() {
    use clonk_frontend::message_dialog::MessageDialogResult;

    for result in [MessageDialogResult::Dismissed, MessageDialogResult::Cancel] {
        let mut app = new_menu_app(640, 480);
        let (network, _events) = NetworkManager::test_stub_for_client_id(7);
        app.network = Some(network);
        app.network_mode = Some(NetworkMode::Client(client_network_settings()));
        app.startup.view = StartupView::NetworkLobby;
        app.last_startup_dialog = StartupDialog::NetworkGame;
        app.mode = AppMode::Loading;
        app.show_reached_network_start_wait().test_value();

        if result == MessageDialogResult::Dismissed {
            app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
        } else {
            app.finish_message_dialog(result).test_value();
        }

        main_assert!(matches!(app.mode, AppMode::Menu));
        main_assert_eq!(app.startup.view => StartupView::NetworkGame);
        main_assert!(app.network.is_none());
        main_assert!(app.network_mode.is_none());
        main_assert!(app.network_start_wait.is_none());
        main_assert!(app.dialogs.messages.is_empty());
    }
}

#[test]
fn client_host_timeout_during_final_init_aborts_startup() {
    // Losing C4Network2 releases FinalInit, whose isEnabled check aborts startup
    // (src/C4Network2.cpp:558-616,1809-1817; src/C4Game.cpp:459-466).
    let mut app = new_real_classic_menu_app(640, 480);
    let (network, events) = NetworkManager::test_stub_for_client_id(7);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.control_clients.replace_snapshot([
        message_client(0, b"Oracle Host"),
        message_client(7, b"Client"),
    ]);
    app.startup.view = StartupView::NetworkLobby;
    app.last_startup_dialog = StartupDialog::NetworkGame;
    app.mode = AppMode::Loading;
    let (_sender, receiver) = mpsc::channel();
    let mut loading = ScenarioLoadingState::from_network_receiver(
        FrontendScenario::fallback(),
        receiver,
        n1_fixture!(status: clonk_network::NETWORK_STATE_GO, 0, 0),
        Vec::new(),
        None,
        0,
        false,
        0,
        false,
        true,
        true,
        clonk_engine::GameParameterRuleGoalLists::new(Vec::new(), Vec::new()),
        TeamConfiguration::default(),
        Vec::new(),
    );
    loading.prepared_go.test_mut().local_reached = true;
    app.loading_state = Some(loading);
    app.show_reached_network_start_wait().test_value();
    events
        .send(NetworkEvent::PeerDisconnected {
            client_id: 0,
            reason: Some("connection Ping timeout".to_string()),
        })
        .test_value();

    app.test_network_events();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);
    main_assert!(app.startup_network_dialog.is_some());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());
    main_assert!(app.network_start_wait.is_none());
    let engine_results = app.engine.snapshot().round_results;
    main_assert_eq!(engine_results.network_result => Some(clonk_engine::RoundResultsNetworkResult::NetworkError));
    main_assert_eq!(engine_results.network_result_message => b"Network: host Oracle Host disconnected!");
    main_assert_eq!(app.snapshot.round_results => engine_results);
    assert_startup_error_log(&app, "Error on final network init.");
}

#[test]
fn network_start_wait_tracks_only_matching_accepted_status_acknowledgements() {
    let mut app = new_menu_app(640, 480);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    app.control_clients.replace_snapshot([
        message_client(0, b"Exact Host"),
        message_client(7, b"Remote"),
        message_client(8, b"Other"),
    ]);
    let expected = n1_fixture!(status: clonk_network::NETWORK_STATE_GO, 1, 4);
    app.begin_network_start_wait(expected);
    app.update_network_start_wait_ack(
        7,
        clonk_network::NetworkStatus {
            target_tick: 3,
            ..expected
        },
    );
    main_assert!(app
        .network_start_wait
        .test_ref()
        .controller
        .clients()
        .iter()
        .all(|client| client.status
            == clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Loading));

    app.update_network_start_wait_ack(7, expected);
    main_assert_eq!(n1_wait_client(&app, 7).status => clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Ready);
    main_assert_eq!(n1_wait_client(&app, 8).status => clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Loading);

    let retargeted = clonk_network::NetworkStatus {
        target_tick: 9,
        ..expected
    };
    app.update_network_start_wait_ack(8, retargeted);
    let wait = app.network_start_wait.as_ref().test_value();
    main_assert_eq!(wait.expected_status => retargeted);
    main_assert_eq!(n1_wait_client(&app, 7).status => clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Loading);
    main_assert_eq!(n1_wait_client(&app, 8).status => clonk_frontend::network_start_wait::NetworkStartWaitClientStatus::Ready);

    app.apply_synchronized_controls(
        0,
        vec![NetworkControl::ClientRemove(
            clonk_engine::ClientRemoveControlData {
                client_id: 7,
                reason: LegacyCString::default(),
                by_client: 0,
            },
        )],
    )
    .test_value();
    main_assert!(app
        .network_start_wait
        .test_ref()
        .controller
        .clients()
        .iter()
        .all(|client| client.client_id != 7));
}

#[test]
fn client_scenario_description_refreshes_only_while_active_until_terminal() {
    let mut app = new_menu_app(640, 480);
    app.startup.view = StartupView::NetworkLobby;
    app.network_lobby = Some(NetworkLobbyState::new(7, "Client".to_string(), false));
    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));

    let host_config = clonk_network::HostConfig::default();
    let mut snapshot = host_config.initial_join_snapshot.test_value();
    let scenario_core = n1_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Scenario as u8,
        id: 9,
        loadable: true,
        filename: LegacyCString::from_bytes(b"Scenarios/Remote.c4s".to_vec()).test_value(),
    });
    snapshot.parameters.scenario = scenario_core.clone();
    snapshot.parameters.title = LegacyCString::from_bytes(b"Remote title".to_vec()).test_value();
    app.pending_network_join_data = Some(clonk_network::JoinDataEnvelope {
        client_id: 7,
        start_control_tick: 0,
        status: host_config.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    });
    app.admission_resources
        .register_lobby_resource(&scenario_core);
    app.admission_resources.mark_progress(9, 42);

    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Scenario))
        .test_value();
    fn scenario_state(app: &GameApp) -> &LobbyScenarioDescriptionState {
        &app.network_lobby.test_ref().scenario_description
    }
    main_assert_eq!(scenario_state(&app).text => LobbyScenarioText::Message("Loading... (42%)".to_string()));
    main_assert!(!scenario_state(&app).finished);

    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Players))
        .test_value();
    app.admission_resources.mark_progress(9, 73);
    app.sec1_timer().test_value();
    main_assert_eq!(scenario_state(&app).text => LobbyScenarioText::Message("Loading... (42%)".to_string()), "inactive ScenDesc retains its last presentation");

    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Scenario))
        .test_value();
    main_assert_eq!(scenario_state(&app).text => LobbyScenarioText::Message("Loading... (73%)".to_string()));
    app.admission_resources.mark_progress(9, 100);
    app.sec1_timer().test_value();
    main_assert_eq!(scenario_state(&app).text => LobbyScenarioText::Message("Loading... (100%)".to_string()));
    main_assert!(
        !scenario_state(&app).finished,
        "present percent does not make an incomplete resource terminal"
    );

    let directory = tempdir();
    let scenario = directory.path().join("Remote.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(
        scenario.join("DescUS.rtf"),
        br"{\rtf1 Gold Mine\par Mine some gold.\par}",
    )
    .test_value();
    app.admission_resources.mark_complete(9, scenario);
    app.sec1_timer().test_value();
    main_assert_eq!(scenario_state(&app).text => LobbyScenarioText::Description("Gold Mine\nMine some gold.\n".to_string()));
    main_assert!(scenario_state(&app).finished);

    app.admission_resources
        .resources
        .insert(9, AdmissionResourceState::Loading { removed: false });
    app.admission_resources.present_percent.insert(9, 7);
    app.sec1_timer().test_value();
    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Players))
        .test_value();
    app.process_lobby_action(LobbyAction::SelectSheet(LobbySheet::Scenario))
        .test_value();
    main_assert_eq!(scenario_state(&app).text => LobbyScenarioText::Description("Gold Mine\nMine some gold.\n".to_string()));
    main_assert!(scenario_state(&app).finished);
}

#[test]
fn joined_chrome_focused_button_activates_on_confirm_keys() {
    fn tab(app: &mut GameApp) {
        n1_press_and_release_key(app, VirtualKeyCode::Tab);
    }

    fn controller_focus(app: &mut GameApp) -> LobbyControl {
        let lobby = app.network_lobby.test_mut();
        lobby.sync_classic_controller();
        lobby.controller.focus()
    }

    fn tab_to(app: &mut GameApp, control: LobbyControl) {
        let mut guard = 0;
        while controller_focus(app) != control {
            tab(app);
            guard += 1;
            main_assert!(guard < 16, "the dialog focus cycle reaches {control:?}");
        }
    }

    // Return mirrors Button::KeyButtonDown/Up (src/C4GuiButton.cpp:112-128).
    let mut app = n1_joined_client_app();
    tab_to(&mut app, LobbyControl::Exit);

    // Unhandled mapped keys reroute to the chat default.
    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    main_assert_eq!(controller_focus(&mut app) => LobbyControl::ChatInput);
    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Released);

    tab_to(&mut app, LobbyControl::Exit);
    app.sound.ui_log.clear();
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string()]);
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby, "KeyButtonDown only downs the button");
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "Click".to_string()]);
    main_assert_eq!(app.startup.view => StartupView::MainMenu);
    main_assert!(app.network_lobby.is_none());
    main_assert!(app.network.is_none());
    main_assert!(app.network_mode.is_none());

    // Dialog::KeyEscape aborts silently from any focus
    // (src/C4GuiDialogs.cpp:371-378).
    for stop in [LobbyControl::Exit, LobbyControl::Roster] {
        let mut app = n1_joined_client_app();
        tab_to(&mut app, stop);
        app.sound.ui_log.clear();
        app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
        main_assert_eq!(app.startup.view => StartupView::MainMenu, "{stop:?}");
        main_assert!(app.network_lobby.is_none());
        main_assert!(app.sound.ui_log.is_empty(), "Escape stays silent");
    }

    // Space activates other chrome stops through the shared sheet switch.
    let mut app = n1_joined_client_app();
    tab_to(&mut app, LobbyControl::ResourcesTab);
    app.sound.ui_log.clear();
    n1_press_and_release_key(&mut app, VirtualKeyCode::Space);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "Click".to_string(), "Command".to_string()]);
    main_assert_eq!(n1_expect(&app.network_lobby, "joined lobby").active_sheet => LobbySheet::Resources);

    // Ready binds Space on key-down; Return reroutes to chat
    // (src/C4GuiCheckBox.cpp:43-52).
    let (mut app, mut commands) = n1_joined_client_app_with_commands();
    app.sync_network_lobby_game_option_state();
    app.network_lobby.test_mut().resources_loaded = true;

    tab_to(&mut app, LobbyControl::Ready);
    app.sound.ui_log.clear();
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert_eq!(controller_focus(&mut app) => LobbyControl::ChatInput);
    main_assert!(app.sound.ui_log.is_empty());
    main_assert!(!n1_expect(&app.network_lobby, "joined lobby").local_ready());
    main_assert!(commands.take_submitted_ready_checks().is_empty());
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);

    tab_to(&mut app, LobbyControl::Ready);
    app.sound.ui_log.clear();
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string()]);
    main_assert!(n1_expect(&app.network_lobby, "joined lobby").local_ready());
    // OnReadyCheck publishes without a status overlay (src/C4GameLobby.cpp:329-344).
    main_assert!(
        app.status_text.is_empty(),
        "accepted Ready must leave the exact lobby renderer usable"
    );
    let checks = commands.take_submitted_ready_checks();
    main_assert_eq!(checks.len() => 1, "the accepted toggle submits exactly once");
    main_assert!(checks[0].data.is_ready());
    app.test_key(VirtualKeyCode::Space, ElementState::Released);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string()]);
    main_assert!(commands.take_submitted_ready_checks().is_empty());

    // The cooldown sounds but rejects the toggle (src/C4GameLobby.cpp:334-338).
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string(), "ArrowHit".to_string()]);
    main_assert!(
        n1_expect(&app.network_lobby, "joined lobby").local_ready(),
        "the cooldown keeps the accepted value"
    );
    main_assert!(commands.take_submitted_ready_checks().is_empty());
    app.test_key(VirtualKeyCode::Space, ElementState::Released);

    // Roster focus consumes confirm keys.
    let mut app = n1_joined_client_app();
    tab_to(&mut app, LobbyControl::Roster);
    app.sound.ui_log.clear();
    n1_press_and_release_key(&mut app, VirtualKeyCode::Enter);
    main_assert_eq!(controller_focus(&mut app) => LobbyControl::Roster);
    main_assert!(app.sound.ui_log.is_empty());
    main_assert_eq!(app.startup.view => StartupView::NetworkLobby);

    // Focus traversal skips Ready while resources load.
    let mut app = n1_joined_client_app();
    let mut cycle = Vec::new();
    loop {
        tab(&mut app);
        let control = controller_focus(&mut app);
        if control == LobbyControl::ChatInput {
            break;
        }
        cycle.push(control);
        main_assert!(cycle.len() < 16, "the focus cycle terminates");
    }
    main_assert!(cycle.contains(&LobbyControl::Exit));
    main_assert!(
        !cycle.contains(&LobbyControl::Ready),
        "the still-loading Ready checkbox is skipped"
    );

    // Pointer Ready emits one ArrowHit and one submission.
    let (mut app, mut commands) = n1_joined_client_app_with_commands();
    app.network_lobby.test_mut().resources_loaded = true;
    {
        let lobby = app.network_lobby.test_mut();
        let rect = lobby.update_layout(640.0, 480.0).ready_button;
        // CheckBox toggles only over its square (C4GuiCheckBox.cpp:82-97).
        lobby.handle_panel_pointer_move(GuiPoint::new(
            rect.origin.x + rect.size.height / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        ));
    }
    app.sound.ui_log.clear();
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.sound.ui_log => ["ArrowHit".to_string()]);
    main_assert!(n1_expect(&app.network_lobby, "joined lobby").local_ready());
    main_assert_eq!(commands.take_submitted_ready_checks().len() => 1, "pointer Ready emits through the routed controller exactly once");
}

#[test]
fn host_client_context_mutes_locally_and_submits_activation_without_optimism() {
    let mut app = new_menu_app(640, 480);
    install_test_classic_host_lobby(&mut app);
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(0);
    app.network = Some(network);
    app.network_mode = Some(NetworkMode::Host(HostSettings {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        player_name: "Host".to_string(),
        prepared: None,
    }));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    app.sync_classic_lobby_roster();

    let entries = app.classic_lobby_client_context_entries(7).test_value();
    main_assert_eq!(
        entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        vec![
            AppContextMenuCommand::LobbyClientToggleMute(7),
            AppContextMenuCommand::LobbyKick(7),
            AppContextMenuCommand::LobbyClientToggleActivate(7),
            AppContextMenuCommand::LobbyClientInfo(7),
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
    main_assert_eq!(labels => vec!["Mute", "Kick", "Deactivate", "Info"]);

    app.toggle_classic_lobby_client_mute(7);
    main_assert!(app.control_messages.is_muted(7));
    main_assert!(commands.take_submitted_client_updates().is_empty());
    main_assert!(commands.take_submitted_client_removes().is_empty());
    main_assert!(commands.take_submitted_votes().is_empty());

    app.toggle_classic_lobby_client_activation(7);
    let update =
        clonk_engine::ClientUpdateControlData::new(clonk_engine::CLIENT_UPDATE_ACTIVATE, 7, 0, 0);
    main_assert_eq!(commands.take_submitted_client_updates() => vec![update.clone()]);
    main_assert!(
        app.control_clients.is_activated(7),
        "the lobby waits for the synchronized client update"
    );

    app.control_clients.apply_update(&update);
    let entries = app.classic_lobby_client_context_entries(7).test_value();
    let mut activation_label = entries[2].text.clone();
    Markup::strip_markup(&mut activation_label);
    main_assert_eq!(activation_label => "Activate");

    let local_entries = app.classic_lobby_client_context_entries(0).test_value();
    main_assert_eq!(
        local_entries
            .iter()
            .filter_map(|entry| entry.action.clone())
            .collect::<Vec<_>>() =>
        vec![
            AppContextMenuCommand::LobbyKick(0),
            AppContextMenuCommand::LobbyClientToggleActivate(0),
            AppContextMenuCommand::LobbyClientInfo(0),
        ]
    );
    app.kick_classic_lobby_client(0);
    main_assert!(commands.take_submitted_client_removes().is_empty());
    app.toggle_classic_lobby_client_activation(0);
    main_assert_eq!(
        commands.take_submitted_client_updates() =>
        vec![clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 0,
            data: 0,
            by_client: 0,
        }]
    );
}

#[test]
fn packed_scenario_rename_missing_source_does_not_touch_destination() {
    let directory = tempdir();
    let outer_path = directory.path().join("Campaign.c4f");
    let mut destination = MutableGroup::new("Destination.c4s");
    destination
        .add_file("Scenario.txt", b"[Head]\nTitle=Keep\n".to_vec())
        .test_value();
    destination
        .add_file("Title.txt", b"US:Keep".to_vec())
        .test_value();
    let mut campaign = MutableGroup::new("Campaign.c4f");
    campaign
        .add_child("Destination.c4s", destination)
        .test_value();
    let before = campaign.pack().test_value();
    fs::write(&outer_path, &before).test_value();

    let error = rename_scenario_storage(
        &outer_path.join("Missing.c4s"),
        ScenarioKind::Scenario,
        "Destination",
        "US",
    )
    .expect_err("missing source must not rewrite an existing destination");
    main_assert!(error.to_string().contains("does not exist"));
    main_assert_eq!(fs::read(&outer_path).expect("read untouched campaign") => before);
    let group = Group::open(&outer_path).test_value();
    main_assert_eq!(group.open_child("Destination.c4s").expect("open untouched destination").read_file("Title.txt").expect("read untouched title") => b"US:Keep");
}

#[test]
fn scale_native_scensel_rows_retain_clipped_book_text() {
    let scenarios = [
        ("tutorial", "Tutorial"),
        ("missions", "Missions"),
        ("melees", "Melees"),
    ]
    .into_iter()
    .map(|(identifier, title)| {
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = identifier.to_string();
        scenario.title = title.to_string();
        scenario
    })
    .collect::<Vec<_>>();
    let mut app = new_menu_app_with_frontend_scenarios(640, 480, Some(scenarios));
    install_classic_test_assets(&mut app);
    app.open_scenario_browser();

    let assets = app.assets.scensel_assets().test_value();
    let button_down = app.assets.dialog_image("GUIButtonDown.png").test_value();
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let book = app.assets.book_fonts.clone().test_value();
    let expected_titles = app
        .menu_state
        .visible_entries()
        .iter()
        .map(|entry| entry.title.clone())
        .collect::<Vec<_>>();
    main_assert_eq!(expected_titles.len() => 3);

    // Scale-native row commands retain their ListBox clip.
    let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
    surface.begin_clonk_text_capture();
    draw_scensel_dynamic(
        &mut surface,
        &mut app.menu_state,
        &app.scensel.entry_enabled,
        &assets,
        &button_down,
        &fonts,
        &book,
        None,
        startup_gamma(),
        true,
    )
    .test_value();
    let commands = surface.take_clonk_text_capture();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(640, 480, &fonts);
    let item_h = clonk_frontend::startup_scensel::scen_list_item_height(&book.text);
    let list_x = layout.list.x + 3;
    let list_top = layout.list.y + 3;
    let list_clip = Rect::new(
        list_x,
        list_top,
        (layout.list.w - 6 - 16) as u32,
        (layout.list.h - 6) as u32,
    );

    for (index, title) in expected_titles.iter().enumerate() {
        let matches = commands
            .iter()
            .filter(|command| {
                command.role == clonk_graphics::clonk_font::ClonkFontRole::BookText
                    && command.text == *title
                    && command.clip == Some(list_clip)
            })
            .collect::<Vec<_>>();
        main_assert_eq!(matches.len() => 1, "one clipped row label for {title}");
        main_assert_eq!(matches[0].x => list_x + item_h + 2);
        main_assert_eq!(matches[0].y => list_top + index as i32 * (item_h + 1) + 2);
    }
}

#[test]
fn scale_native_scensel_static_text_survives_backdrop_cache_hits() {
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "tutorial".to_string();
    scenario.title = "Tutorial".to_string();
    let mut app = new_menu_app_with_frontend_scenarios(640, 480, Some(vec![scenario]));
    install_classic_test_assets(&mut app);
    app.open_scenario_browser();
    app.menu_state.set_search_text("needle");
    app.menu_state.set_search_focused(true);
    install_native_test_fonts(&mut app, 2.0);

    // Cached pixels cannot carry CStdFont commands, so static text is re-emitted.
    let (_, _, cold) = render_ordered_test_frame(&mut app, 2.0, 1280, 960);
    let cached_backdrop = app.menu_backdrop_cache.pixels.clone();
    let (cached_chrome, cached_rendered, cached) =
        render_ordered_test_frame(&mut app, 2.0, 1280, 960);
    main_assert!(app.menu_backdrop_cache.key.is_some());
    main_assert_eq!(app.menu_backdrop_cache.pixels => cached_backdrop, "the second frame must reuse the unchanged raster backdrop");

    for (name, plan) in [("cold", &cold), ("cache hit", &cached)] {
        let commands = plan
            .batches
            .iter()
            .flat_map(|batch| batch.text.iter())
            .collect::<Vec<_>>();
        for (text, role) in [
            (
                "Start Game",
                clonk_graphics::clonk_font::ClonkFontRole::GuiTitle,
            ),
            (
                "Search:",
                clonk_graphics::clonk_font::ClonkFontRole::GuiText,
            ),
            ("needle", clonk_graphics::clonk_font::ClonkFontRole::GuiText),
        ] {
            main_assert_eq!(
                commands
                    .iter()
                    .filter(|command| command.text == text && command.role == role)
                    .count() =>
                1,
                "{name} must contain exactly one {text:?} native command"
            );
        }
    }

    let cached_commands = cached
        .batches
        .iter()
        .flat_map(|batch| batch.text.iter())
        .collect::<Vec<_>>();
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(640, 480, fonts);
    let title = cached_commands
        .iter()
        .find(|command| command.text == "Start Game")
        .test_value();
    main_assert_eq!(
        (title.x, title.y, title.align, title.clip) =>
        (
            layout.title_anchor.0,
            layout.title_anchor.1,
            clonk_graphics::clonk_font::TextAlign::Center,
            None
        )
    );

    let label = layout.search_label;
    let search = cached_commands
        .iter()
        .find(|command| command.text == "Search:")
        .test_value();
    let label_clip = Rect::new(label.x, label.y, (label.w + 1) as u32, (label.h + 1) as u32);
    main_assert_eq!(
        (search.x, search.y, search.align, search.clip) =>
        (
            label.x + label.w / 2,
            label.y + (label.h - fonts.text.line_height) / 2 - 1,
            clonk_graphics::clonk_font::TextAlign::Center,
            Some(label_clip)
        )
    );

    let edit = layout.search_edit;
    let (client_x, client_y, client_w, client_h) = (edit.x + 4, edit.y + 2, edit.w - 8, edit.h - 4);
    let query = cached_commands
        .iter()
        .find(|command| command.text == "needle")
        .test_value();
    let query_y = if client_h <= fonts.text.line_height {
        client_y - 1
    } else {
        client_y + (client_h - fonts.text.line_height) / 2
    };
    main_assert_eq!(
        (query.x, query.y, query.align, query.clip) =>
        (
            client_x,
            query_y,
            clonk_graphics::clonk_font::TextAlign::Left,
            Some(Rect::new(
                client_x - 2,
                client_y,
                (client_w + 4) as u32,
                (client_h + 1) as u32,
            ))
        )
    );

    let changed_in_search_label = (label.y * 2..(label.y + label.h) * 2).any(|y| {
        (label.x * 2..(label.x + label.w) * 2).any(|x| {
            let offset = (y as usize * 1280 + x as usize) * 4;
            cached_chrome[offset..offset + 4] != cached_rendered[offset..offset + 4]
        })
    });
    main_assert!(
        changed_in_search_label,
        "cache-hit Search: command must contribute visible physical pixels"
    );
}

#[test]
fn scenario_music_safe_random_does_not_advance_the_synchronized_lcg() {
    let dir = tempdir();
    let scenario = dir.path().join("Scenario.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("A.ogg"), silent_pcm_wav(20)).test_value();
    fs::write(scenario.join("B.ogg"), silent_pcm_wav(20)).test_value();
    let mut app = new_state_only_running_sandbox_app();
    let synchronized_before = app.engine.snapshot().rng;

    app.sound.runtime_music_enabled = true;
    app.play_scenario_audio(&scenario);

    main_assert_eq!(app.engine.snapshot().rng => synchronized_before, "the live scenario path must draw through libc SafeRandom, not Engine::LcgRng");
    app.test_audio_mut().stop_music();
}

#[test]
fn scenario_title_tooltip_tracks_native_readd_z_order_and_fresh_open_reset() {
    let mut app = new_real_classic_menu_app(640, 480);
    app.open_scenario_browser();
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(640, 480, fonts);
    let title = app.startup_tooltip_resource_no_amp("IDS_DLG_STARTGAME");
    let title_height = fonts.title.measure(&title, true).1;
    let overlap_y = layout.map_sheet.y.max(layout.title_anchor.1);
    main_assert!(overlap_y < layout.title_anchor.1 + title_height);
    let overlap = GuiPoint::new(layout.title_anchor.0 as f32, overlap_y as f32);

    main_assert!(!app.menu_state.scensel_title_topmost);
    main_assert_eq!(app.scenario_browser_tooltip_target_at(overlap) => None, "the initially constructed Tabular occludes the title");

    // Returning from HideTitle re-adds the label after the Tabular.
    app.menu_state.scensel_title_present = false;
    app.startup_tooltip.note_pointer_move(overlap);
    app.configure_current_folder_map();
    main_assert_eq!(app.startup_tooltip.pointer_position() => None);
    main_assert!(app.menu_state.scensel_title_present);
    main_assert!(app.menu_state.scensel_title_topmost);
    main_assert_eq!(app.scenario_browser_tooltip_target_at(overlap) => Some(StartupTooltip::text(title)));

    app.open_scenario_browser();
    main_assert!(app.menu_state.scensel_title_present);
    main_assert!(!app.menu_state.scensel_title_topmost);
    main_assert_eq!(app.scenario_browser_tooltip_target_at(overlap) => None);
}

#[test]
fn platform_cursor_tracks_client_area_and_focus_in_every_mode() {
    for (active, inside, visible) in [
        (false, false, true),
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        main_assert_eq!(classic_platform_cursor_visible(active, inside) => visible);
    }

    let mut app = new_menu_app(64, 48);
    app.test_cursor(PhysicalPosition::new(20.0, 18.0));
    main_assert!(!app.platform_cursor_visible());
    let retained = app.live_input.window_pointer;

    app.window_active = false;
    app.handle_focus_lost().test_value();
    main_assert!(app.platform_cursor_visible());
    main_assert_eq!(app.live_input.window_pointer => retained);
    main_assert!(app.live_input.pointer_inside_window);

    app.handle_focus_gained().test_value();
    main_assert_eq!(app.live_input.window_pointer => retained);
    for mode in [AppMode::Menu, AppMode::Loading, AppMode::Running] {
        app.mode = mode;
        main_assert!(!app.platform_cursor_visible(), "{mode:?}");
    }
    app.mode = AppMode::Menu;
    app.pointer_left().test_value();
    main_assert!(app.platform_cursor_visible());
    main_assert_eq!(app.live_input.window_pointer => None);
}

#[test]
fn irc_settings_use_cpp_defaults_and_network_nick_fallback() {
    let defaults = IrcSettings::from_config(b"");
    main_assert_eq!(defaults.server => "irc.euirc.net");
    main_assert_eq!(defaults.nick => "");
    main_assert_eq!(defaults.real_name => "");
    main_assert_eq!(defaults.channel => "#clonken,#legacyclonk");
    main_assert!(!defaults.hide_dangerous_warning);
    main_assert_eq!(defaults.login().password => "");

    let configured = IrcSettings::from_config(
                b"[Network]\nNick=FallbackClonker\n[IRC]\nServer2=irc.example.test\nNick=\nRealName=J\xfcrgen\nChannel=#test\n[Startup]\nHideMsgIRCDangerous=true\n",
            );
    main_assert_eq!(configured.server => "irc.example.test");
    main_assert_eq!(configured.nick => "FallbackClonker");
    main_assert_eq!(configured.real_name => "Jürgen");
    main_assert_eq!(configured.channel => "#test");
    main_assert!(configured.hide_dangerous_warning);

    let configured =
        IrcSettings::from_config(b"[Network]\nNick=FallbackClonker\n[IRC]\nNick=IrcClonker\n");
    main_assert_eq!(configured.nick => "IrcClonker");

    let invalid = IrcSettings::from_config(b"[Startup]\nHideMsgIRCDangerous=yes\n");
    main_assert!(!invalid.hide_dangerous_warning);

    let utf8_shaped = IrcSettings::from_config(
        b"[IRC]\nNick=\xc3\xa9\nRealName=Name \xc3\xa9\nChannel=#\xc3\xa9\n",
    );
    let presented = "\u{00c3}\u{00a9}";
    main_assert_eq!(utf8_shaped.nick => presented);
    main_assert_eq!(utf8_shaped.real_name => format!("Name {presented}"));
    main_assert_eq!(utf8_shaped.channel => format!("#{presented}"));
    main_assert_eq!(encode_startup_irc_text(&utf8_shaped.nick) => Some(vec![0xc3, 0xa9]), "valid UTF-8-shaped config bytes remain two native bytes");
}

#[test]
fn irc_persistence_preserves_legacy_native_config_bytes() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    fs::write(paths.config_file(), b"[General]\r\nName=\"M\x81ker\"\r\n[IRC]\r\nServer2=\"irc.native.test\"\r\nNick=\"Old\"\r\n[Vendor]\r\nOpaque=\"\xfe\"\r\n").test_value();
    persist_irc_login_settings(
        &paths,
        &clonk_frontend::startup_netdlg::NetDlgChatLogin {
            server: "must-not-replace-server.test".into(),
            nick: "NativeNick".into(),
            password: "never-persist-this".into(),
            real_name: "Grü1".into(),
            channel: "#native".into(),
        },
    )
    .test_value();
    persist_irc_warning_preference(&paths, true).test_value();

    let updated = fs::read(paths.config_file()).test_value();
    main_assert!(updated.starts_with(b"[General]\r\nName=\"M\x81ker\"\r\n"));
    main_assert!(updated
        .windows(b"[Vendor]\r\nOpaque=\"\xfe\"\r\n".len())
        .any(|window| window == b"[Vendor]\r\nOpaque=\"\xfe\"\r\n"));
    let native = |section, key| {
        clonk_app_netplay::configured_native_value(&updated, section, key)
            .unwrap_or_else(|| panic!("missing {section}.{key}"))
    };
    main_assert_eq!(native("IRC", "Server2").as_bytes() => b"irc.native.test");
    main_assert_eq!(native("IRC", "Nick").as_bytes() => b"NativeNick");
    main_assert_eq!(native("IRC", "RealName").as_bytes() => b"Gr\xfc1");
    main_assert_eq!(native("IRC", "Channel").as_bytes() => b"#native");
    main_assert!(clonk_app_netplay::configured_native_value(&updated, "IRC", "Password").is_none());
    main_assert_eq!(native("Startup", "HideMsgIRCDangerous").as_bytes() => b"1");
}

#[test]
fn startup_network_dialog_seeds_irc_login_from_config() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n[Network]\nNick=NetworkFallback\nMasterServerSignUp=0\n[IRC]\nServer2=irc.seeded.test\nNick=\nRealName=Seeded Name\nChannel=#seeded\n").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_classic_test_assets(&mut app);

    app.open_network_game_dialog();
    main_assert_eq!(
        n1_expect(&app.startup_network_dialog, "network dialog").chat_login() =>
        clonk_frontend::startup_netdlg::NetDlgChatLogin {
            server: "irc.seeded.test".into(),
            nick: "NetworkFallback".into(),
            password: String::new(),
            real_name: "Seeded Name".into(),
            channel: "#seeded".into(),
        }
    );
}

#[test]
fn network_tab_and_shift_tab_are_inverse_and_wrap() {
    use clonk_frontend::startup_netdlg::{
        NetDlgConfig, NetDlgControl, NetDlgController, NetDlgFontMetrics,
    };

    let mut app = new_classic_menu_app(640, 480);
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let mut dialog = NetDlgController::new(
        NetDlgConfig::default(),
        NetDlgFontMetrics::from_fonts(fonts),
    );
    dialog.resize(640, 480);
    app.startup_network_dialog = Some(dialog);
    app.replace_startup_view(StartupView::NetworkGame);

    n1_press_and_release_key(&mut app, VirtualKeyCode::Tab);
    main_assert_eq!(n1_expect(&app.startup_network_dialog, "network dialog").focused_control() => NetDlgControl::JoinAddress);

    app.test_modifiers(ModifiersState::SHIFT);
    n1_press_and_release_key(&mut app, VirtualKeyCode::Tab);
    main_assert_eq!(n1_expect(&app.startup_network_dialog, "network dialog").focused_control() => NetDlgControl::GameList);
    n1_press_and_release_key(&mut app, VirtualKeyCode::Tab);
    main_assert_eq!(n1_expect(&app.startup_network_dialog, "network dialog").focused_control() => NetDlgControl::ChatButton);

    app.test_modifiers(ModifiersState::empty());
    n1_press_and_release_key(&mut app, VirtualKeyCode::Tab);
    main_assert_eq!(n1_expect(&app.startup_network_dialog, "network dialog").focused_control() => NetDlgControl::GameList);

    app.test_modifiers(ModifiersState::CONTROL);
    n1_press_and_release_key(&mut app, VirtualKeyCode::Tab);
    main_assert_eq!(n1_expect(&app.startup_network_dialog, "network dialog").focused_control() => NetDlgControl::GameList);
}

#[test]
fn activation_overflow_reverts_row_and_shows_native_error() {
    let (_user_data, _guard, paths) = n1_test_paths(None);
    let player_model = |name: &str, activated: bool| clonk_frontend::startup_plrsel::PlrSelPlayer {
        name: name.to_string(),
        activated,
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
    let alpha = player_model("Alpha", true);
    let overflow = player_model("Overflow", true);
    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    app.startup.player_files.push(StartupPlayerFile {
        path: PathBuf::from("A"),
        file_name: "A".to_string(),
        player_file: PlayerFile {
            name: "Alpha".to_string(),
            ..PlayerFile::default()
        },
        render_model: alpha.clone(),
    });
    app.startup.player_models.push(alpha);
    for index in 1..19 {
        let model = player_model(&format!("Inactive {index}"), false);
        app.startup.player_files.push(StartupPlayerFile {
            path: PathBuf::from(format!("Inactive{index}.c4p")),
            file_name: format!("Inactive{index}.c4p"),
            player_file: PlayerFile::default(),
            render_model: model.clone(),
        });
        app.startup.player_models.push(model);
    }
    let overflow_index = app.startup.player_files.len();
    app.startup.player_files.push(StartupPlayerFile {
        path: PathBuf::from("Overflow.c4p"),
        file_name: "b".repeat(1023),
        player_file: PlayerFile {
            name: "Overflow".to_string(),
            ..PlayerFile::default()
        },
        render_model: overflow.clone(),
    });
    app.startup.player_models.push(overflow);
    app.players.selected_file = Some(PlayerFile {
        name: "Overflow".to_string(),
        ..PlayerFile::default()
    });
    app.open_player_selection_dialog();
    main_assert!(
        !app.startup.player_files[overflow_index]
            .render_model
            .activated
    );
    main_assert!(!app.startup.player_models[overflow_index].activated);
    main_assert_eq!(app.players.selected_file.as_ref().map(|player| player.name.as_str()) => Some("Alpha"));
    main_assert_eq!(app.dialogs.messages.len() => 1);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    let (actions, scroll_before) = {
        let dialog = app.startup.player_dialog.test_mut();
        dialog.set_selected_index(Some(overflow_index));
        let scroll_before = dialog.list_scroll_offset();
        main_assert!(scroll_before > 0, "fixture must exercise a scrolled list");
        let actions = dialog.handle_key_down(KeyCode::Space);
        main_assert_eq!(dialog.is_player_activated(overflow_index) => Some(true));
        (actions, scroll_before)
    };

    app.process_player_dialog_actions(actions).test_value();

    main_assert!(
        !app.startup.player_files[overflow_index]
            .render_model
            .activated
    );
    main_assert!(!app.startup.player_models[overflow_index].activated);
    let dialog = app.startup.player_dialog.test_ref();
    main_assert_eq!(dialog.is_player_activated(overflow_index) => Some(false));
    main_assert_eq!(dialog.selected_index() => Some(overflow_index));
    main_assert_eq!(dialog.list_scroll_offset() => scroll_before);
    main_assert_eq!(Config::load(paths.config_file()).expect("reload bounded participants").get_in(Some("General"), "Participants") => Some("A"));
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(app.dialogs.messages.len() => 1);
    let error = &app.dialogs.messages[0].state;
    main_assert_eq!(error.caption() => "Error");
    main_assert_eq!(error.message() => "Player \"Overflow\" has been deactivated: Too many activated players or path too long!");
    main_assert_eq!(error.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::OK);
    main_assert_eq!(error.icon() => clonk_frontend::message_dialog::MessageDialogIcon::ERROR);
}

#[test]
fn retained_gpu_gamma_mode_matches_native_device_switch_matrix() {
    for shader in [false, true] {
        for use_shader_gamma in [false, true] {
            for disable_gamma in [false, true] {
                let config = clonk_frontend::AdvancedRendererConfig {
                    shader,
                    use_shader_gamma,
                    disable_gamma,
                    ..clonk_frontend::AdvancedRendererConfig::DEFAULT
                };
                let expected = if disable_gamma {
                    GpuGammaMode::Disabled
                } else if shader && use_shader_gamma {
                    GpuGammaMode::Fragment
                } else {
                    GpuGammaMode::Monitor
                };
                main_assert_eq!(retained_gpu_gamma_mode(config) => expected);
            }
        }
    }
}

#[test]
fn graphical_debug_mode_reads_native_config_and_obeys_allow_debug() {
    let install = tempdir();
    let user_data = tempdir();
    let custom = tempdir();
    fs::create_dir_all(install.path().join("planet")).test_value();
    fs::write(install.path().join("planet/System.c4g"), b"stub").test_value();
    let config_file = custom.path().join("debug.config");
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("LC_CONFIG_FILE", None),
    ]);
    let paths = AppPaths::discover_with_config_file(Some(&config_file)).test_value();
    let mut engine = Engine::new();

    fs::write(&config_file, b"[General]\n").test_value();
    arm_configured_graphical_engine_debug_mode(&mut engine, Some(&paths));
    main_assert!(!engine.debug_mode(), "missing DebugMode defaults off");

    fs::write(&config_file, b"[General]\nDebugMode=true\n").test_value();
    arm_configured_graphical_engine_debug_mode(&mut engine, Some(&paths));
    main_assert!(engine.debug_mode(), "DebugMode=true arms a graphical game");

    engine.set_allow_debug(false);
    arm_configured_graphical_engine_debug_mode(&mut engine, Some(&paths));
    main_assert!(
        !engine.debug_mode(),
        "Parameters.AllowDebug=false is authoritative"
    );

    engine.set_allow_debug(true);
    fs::write(&config_file, b"[General]\nDebugMode= true\n").test_value();
    arm_configured_graphical_engine_debug_mode(&mut engine, Some(&paths));
    main_assert!(
        !engine.debug_mode(),
        "native Boolean grammar does not skip whitespace after '='"
    );
}

#[test]
fn replay_script_injection_obeys_native_config() {
    main_assert!(!configured_allow_scripting_in_replays(b"[General]\n"));
    main_assert!(!configured_allow_scripting_in_replays(
        b"[General]\nAllowScriptingInReplays= true\n"
    ));
    main_assert!(configured_allow_scripting_in_replays(
        b"[General]\nAllowScriptingInReplays=true\n"
    ));

    let (_fixture, _guard, paths) = n1_test_paths(None);
    fs::write(
        paths.config_file(),
        b"[General]\nAllowScriptingInReplays=false\n",
    )
    .test_value();

    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    app.synchronize_advanced_options_runtime();
    app.records.playback =
        Some(ControlRecordPlayback::from_bytes(&[0, clonk_engine::RCT_END]).test_value());
    app.engine.set_debug_mode(true);
    let initial_gravity = app.engine.physics().gravity;

    app.process_running_chat_text("/script SetGravity(77)");
    main_assert_eq!(app.engine.physics().gravity => initial_gravity, "replay scripting defaults to the configured denial");
    app.apply_ready_controls(
        1,
        vec![NetworkControl::EmMoveObject(
            clonk_engine::EmMoveObjectControlData {
                action: clonk_engine::EMMO_SCRIPT,
                objects: vec![999_999],
                script: LegacyCString::from_bytes(b"SetGravity(78)".to_vec())
                    .expect("editor script is NUL-free"),
                by_client: -1,
                ..Default::default()
            },
        )],
    )
    .test_value();
    main_assert_eq!(app.engine.physics().gravity => initial_gravity);

    fs::write(
        paths.config_file(),
        b"[General]\nAllowScriptingInReplays=true\n",
    )
    .test_value();
    app.synchronize_advanced_options_runtime();
    app.process_running_chat_text("/script SetGravity(88)");
    main_assert_eq!(app.engine.physics().gravity => 88, "the native config flag admits a non-host replay script");
    app.apply_ready_controls(
        2,
        vec![NetworkControl::EmMoveObject(
            clonk_engine::EmMoveObjectControlData {
                action: clonk_engine::EMMO_SCRIPT,
                objects: vec![999_999],
                script: LegacyCString::from_bytes(b"SetGravity(99)".to_vec())
                    .expect("editor script is NUL-free"),
                by_client: -1,
                ..Default::default()
            },
        )],
    )
    .test_value();
    main_assert_eq!(app.engine.physics().gravity => 99);
}

#[test]
fn startup_gamma_reload_uses_native_boolean_grammar_and_invalidates_caches() {
    let (_fixture, _guard, paths) = n1_test_paths(None);
    fs::write(
        paths.config_file(),
        b"[Graphics]\nDisableGamma=true\nGamma2=6579300\n",
    )
    .test_value();

    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    let (width, height) = {
        let surface = app.graphics.surface();
        (surface.width(), surface.height())
    };
    app.menu_backdrop_cache = StartupBackdropCache {
        key: Some(StartupBackdropKey {
            view: StartupView::MainMenu,
            width,
            height,
            fair_crew: false,
            record: false,
            network_host_selector: false,
        }),
        pixels: vec![1],
        retained: None,
    };
    app.startup.dialog_fade = Some(StartupDialogFade {
        outgoing: None,
        incoming: StartupDialog::MainMenu,
        step: 0,
        width,
        height,
        underlay: vec![0; width as usize * height as usize * 4],
        outgoing_frame: None,
        outgoing_native_frame: None,
        outgoing_native_text: Vec::new(),
        outgoing_native_fonts: None,
        underlay_gpu_recorder: None,
        outgoing_gpu_plan: None,
    });

    app.synchronize_advanced_options_runtime();
    main_assert!(app.graphics.advanced_renderer_config().disable_gamma);
    main_assert_eq!(app.loader_gamma => None);
    main_assert_eq!(app.startup_active_gamma() => clonk_graphics::GammaRamp::identity());
    main_assert!(app.menu_backdrop_cache.key.is_none());
    main_assert!(app.menu_backdrop_cache.pixels.is_empty());
    main_assert!(app.startup.dialog_fade.is_none());

    fs::write(
        paths.config_file(),
        b"[Graphics]\nDisableGamma= true\nGamma2=6579300\n",
    )
    .test_value();
    app.synchronize_advanced_options_runtime();
    main_assert!(!app.graphics.advanced_renderer_config().disable_gamma);
    main_assert_eq!(app.loader_gamma => Some(clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xffffff,])));
    main_assert_eq!(app.startup_active_gamma() => clonk_graphics::GammaRamp::from_control_points([0x000000, 0x646464, 0xffffff,]));
}

#[test]
fn options_program_font_combos_accept_native_alt_open_bindings() {
    use clonk_frontend::startup_options_dlg::OptionsProgramFocusTarget;

    let mut app = new_real_classic_menu_app(640, 480);
    app.open_options_menu();
    for expected in [
        OptionsProgramFocusTarget::LanguageCombo,
        OptionsProgramFocusTarget::FontFaceCombo,
    ] {
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )
        .test_value();
        main_assert_eq!(app.startup.options_dialog.as_ref().unwrap().focused_program_control() => Some(expected));
    }

    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    app.close_context_menu_silently();
    app.test_key(VirtualKeyCode::ArrowDown, ElementState::Released);
    app.test_modifiers(ModifiersState::empty());

    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(app.startup.options_dialog.as_ref().unwrap().focused_program_control() => Some(OptionsProgramFocusTarget::FontSizeCombo));
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
}

#[test]
fn malformed_active_gui_sheet_override_fails_typed_before_pixels() {
    let root = tempdir();
    let graphics_child = root.path().join("Scenario.c4s/Graphics.c4g");
    fs::create_dir_all(&graphics_child).test_value();
    fs::write(graphics_child.join("GUICaption.png"), b"not a png").test_value();
    let base_graphics = root.path().join("planet/Graphics.c4g");
    fs::create_dir_all(&base_graphics).test_value();

    let registrations = vec![LoaderGroupRegistration {
        priority: 1,
        registration_order: 0,
        group: Group::open(root.path().join("Scenario.c4s")).expect("open scenario registration"),
    }];
    let resolution = resolve_classic_global_gui_sheet_overrides(
        &registrations,
        &Group::open(&base_graphics).test_value(),
    );
    main_assert!(
        resolution.overrides.is_empty(),
        "a corrupt override must not be applied"
    );
    let failure = resolution.failures.get("GUICaption").test_value();
    main_assert!(
        failure.contains("GUICaption.png"),
        "the failure must name the winning source: {failure}"
    );

    let app = new_menu_app(320, 200);
    let error = app
        .assets
        .require_classic_global_gui_bootstrap_resources(&resolution.failures)
        .expect_err("a corrupt active override must fail before pixels");
    main_assert_eq!(
        error =>
        ClassicParityBoundary::GlobalGuiBootstrapResources {
            issues: vec![ClassicGuiBootstrapIssue::malformed(
                "GUICaption",
                "a readable selected bmp/jpeg/jpg/png RGBA surface",
                failure.clone(),
            )],
        }
    );
}

#[test]
fn global_gui_bootstrap_precedes_all_startup_roots_and_native_pixels() {
    let mut app = new_real_classic_menu_app(320, 200);
    let mut initial = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut initial);
    remove_global_gui_sheet(&mut app, "GUIBigArrows.png");
    let expected = vec![ClassicGuiBootstrapIssue::missing("GUIBigArrows")];

    for view in StartupView::ALL {
        n1_select_empty_startup_view(&mut app, view);
        app.status_text = format!("lower status for {view:?}");
        let mut frame = vec![0xa5; 320 * 200 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("global GUI must precede every startup root");
        assert_global_gui_boundary(&error, expected.clone());
        main_assert!(frame.iter().all(|byte| *byte == 0xa5));

        let mut native = vec![0x6d; 640 * 400 * 4];
        let error = app
            .render_native_main_menu_text(&mut native, 640, 400)
            .expect_err("native startup pass shares global preflight");
        assert_global_gui_boundary(&error, expected.clone());
        main_assert!(native.iter().all(|byte| *byte == 0x6d));
    }
}

#[test]
fn global_gui_bootstrap_precedes_every_retained_startup_child_logical_and_native() {
    let children = [
        RetainedStartupChild::Unported(ClassicStartupSubscreen::Options(
            clonk_frontend::startup_options_dlg::OptionsSheet::Graphics,
        )),
        RetainedStartupChild::OptionsSound,
        RetainedStartupChild::Unported(ClassicStartupSubscreen::Options(
            clonk_frontend::startup_options_dlg::OptionsSheet::Keyboard,
        )),
        RetainedStartupChild::Unported(ClassicStartupSubscreen::Options(
            clonk_frontend::startup_options_dlg::OptionsSheet::Gamepad,
        )),
        RetainedStartupChild::Unported(ClassicStartupSubscreen::Options(
            clonk_frontend::startup_options_dlg::OptionsSheet::Network,
        )),
        RetainedStartupChild::AboutLicenses,
    ];
    for child in children {
        let mut app = new_classic_menu_app(640, 480);
        enter_retained_startup_child(&mut app, child);
        remove_global_gui_sheet(&mut app, "GUISpinBoxArrow.png");
        let expected = vec![ClassicGuiBootstrapIssue::missing("GUISpinBoxArrow")];
        let mut frame = vec![0xc7; 640 * 480 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("global GUI must precede retained child boundary");
        assert_global_gui_boundary(&error, expected.clone());
        main_assert!(frame.iter().all(|byte| *byte == 0xc7));
        let mut native = vec![0x6d; 1280 * 960 * 4];
        let error = app
            .render_native_main_menu_text(&mut native, 1280, 960)
            .expect_err("native pass must precede retained child boundary");
        assert_global_gui_boundary(&error, expected);
        main_assert!(native.iter().all(|byte| *byte == 0x6d));
    }
}

#[test]
fn global_gui_bootstrap_precedes_constructed_recursive_overlay_stacks() {
    let assert_guard = |app: &mut GameApp, label: &str| {
        remove_global_gui_sheet(app, "GUISpinBoxArrow.png");
        let mut frame = vec![0xb6; 640 * 480 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("recursive overlay bypassed global GUI preflight");
        assert_global_gui_boundary(
            &error,
            vec![ClassicGuiBootstrapIssue::missing("GUISpinBoxArrow")],
        );
        main_assert!(frame.iter().all(|byte| *byte == 0xb6), "{label}");
    };

    let mut context = new_classic_menu_app(640, 480);
    let entries: Vec<ContextMenuEntry<AppContextMenuCommand>> =
        vec![ContextMenuEntry::new("Root")
            .with_lazy_submenu(|| vec![ContextMenuEntry::new("Nested")])];
    context
        .open_context_menu_at(entries, GuiPoint::new(100.0, 100.0))
        .test_value();
    assert_guard(&mut context, "recursive context menu");

    let mut definition = new_classic_menu_app(640, 480);
    definition
        .open_definition_selector(FrontendScenario::fallback())
        .test_value();
    assert_guard(&mut definition, "definition selector");

    let mut input = new_classic_menu_app(640, 480);
    input
        .open_game_option_input_dialog(GameOptionInputDialogRequest {
            kind: GameOptionInputKind::Password,
            message: "Password",
            caption: "Password",
            icon: clonk_frontend::game_option_buttons::GameOptionIcon::Locked,
            max_text: 31,
            initial_text: String::new(),
            chat_layout: false,
        })
        .test_value();
    assert_guard(&mut input, "input dialog");

    let mut messages = new_classic_menu_app(640, 480);
    for caption in ["First", "Nested"] {
        messages
            .push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    caption,
                    caption,
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .test_value();
    }
    assert_guard(&mut messages, "stacked message dialogs");
}

#[test]
fn network_join_applies_active_scenario_gui_overrides() {
    // Join InitGame applies synchronized GUI overrides and fails before pixels
    // (C4Game.cpp:2432-2450; C4GraphicsResource.cpp:278-292).
    let content = tempdir();
    let content_root = content.path();
    let (_user_data, _guard, paths) = n1_test_paths(Some(content_root));
    fs::create_dir_all(content_root.join("Material.c4g")).test_value();
    let pack = install_network_definition_pack(content_root, "GuiPack.c4d", "GUIP");
    let pack_graphics = pack.join("Graphics.c4g");
    fs::create_dir_all(&pack_graphics).test_value();
    image::RgbaImage::from_pixel(8, 4, image::Rgba([0x21, 0x42, 0x63, 0xff]))
        .save(pack_graphics.join("GUICaption.png"))
        .test_value();
    let scenario_path = content_root.join("JoinGui.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Join GUI\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nLocalOnly=1\n",
    )
    .test_value();

    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let staged = app
        .prepare_network_host_scenario(
            FrontendScenario {
                identifier: "JoinGui.c4s".to_string(),
                title: "Join GUI".to_string(),
                path: Some(scenario_path.clone()),
                ..FrontendScenario::fallback()
            },
            ScenarioDefinitionLoad::Seed {
                modules: vec!["GuiPack.c4d".to_string()],
                definition_root: None,
            },
        )
        .test_value();
    let prepared = prepare_staged_network_host(&app, &staged);
    let host = prepared.host_config();
    let snapshot = host.initial_join_snapshot.test_ref().clone();
    let host_files = host.resource_files.clone();
    let join_data = clonk_network::JoinDataEnvelope {
        client_id: 2,
        start_control_tick: snapshot.dynamic_tick,
        status: host.initial_status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    let complete_path = |core: &clonk_engine::NetworkResourceCore| {
        host_files
            .iter()
            .find(|resource| resource.core.id == core.id)
            .map(|resource| resource.path.clone())
    };
    let scenario_resources =
        resolve_client_scenario_resources(&join_data, complete_path).test_value();
    let game_resources = resolve_client_game_resources(&join_data, complete_path).test_value();
    let published_pack = game_resources
        .iter()
        .find(|resource| {
            resource.core.resource_type == clonk_network::HostResourceType::Definitions as u8
        })
        .test_value()
        .path
        .clone();
    let client_directory = tempdir();
    let combined_path = client_directory.path().join("Combined2.c4s");
    let preload_job = |app: &GameApp,
                       join_data: clonk_network::JoinDataEnvelope,
                       game_resources: Vec<ResolvedClientStartResource>| {
        GameApp::run_lobby_preload_job(LobbyPreloadJob {
            graphics: LobbyPreloadGraphicsContext {
                app_paths: app.app_paths.clone(),
                fallback: app.startup_game_graphics_resources(),
                liquid_animation_enabled: app.assets.liquid_animation_enabled(),
            },
            source: LobbyPreloadJobSource::Client {
                join_data,
                scenario_resources: Some(scenario_resources.clone()),
                game_resources,
                resource_directory: client_directory.path().to_path_buf(),
                maker: LegacyCString::from_bytes(b"Exact Host".to_vec()).test_value(),
                scenario_path: combined_path.clone(),
                staging_path: None,
            },
        })
        .test_value()
    };
    let artifact = preload_job(&app, join_data.clone(), game_resources.clone());

    app.network_mode = Some(NetworkMode::Client(ClientSettings::new(
        SocketAddr::from(([127, 0, 0, 1], 11_112)),
        "Client",
    )));
    app.pending_network_join_data = Some(join_data.clone());
    app.pending_client_start_status = Some(n1_fixture!(status:
        clonk_network::NETWORK_STATE_GO,
        host.initial_status.control_mode,
        0,
    ));
    for resource in &host_files {
        app.admission_resources.ensure_by_core(&resource.core);
        app.admission_resources
            .mark_complete(resource.core.id, resource.path.clone());
    }
    app.client_combined_scenario_path = Some(combined_path.clone());
    app.lobby_preload_artifact = Some(artifact);
    app.try_prepare_client_network_scenario().test_value();

    let loading = app.loading_state.test_ref();
    main_assert!(
        loading.refresh_requested,
        "the client GO must stage a GraphicsResource refresh"
    );
    let staged_overrides = loading.refreshed_gui_sheet_overrides.test_ref();
    let staged_caption = staged_overrides
        .iter()
        .find(|sheet| sheet.stem == "GUICaption")
        .test_value();
    main_assert!(
        staged_caption
            .source
            .contains(&published_pack.display().to_string()),
        "the winning source must be the synchronized resource: {}",
        staged_caption.source
    );
    main_assert_eq!(loading.refreshed_global_gui_failures.as_ref().map(HashMap::len) => Some(0));
    main_assert!(
        loading.refreshed_resources.is_some(),
        "the client refresh reinitializes the loader fonts"
    );
    let pristine_ptr = app
        .assets
        .startup_dialog_images
        .get("GUICaption.png")
        .test_value()
        .pixels()
        .as_ptr();
    app.apply_pending_loading_resource_refresh().test_value();
    let applied = app
        .assets
        .startup_dialog_images
        .get("GUICaption.png")
        .test_value();
    main_assert_eq!(applied.pixels()[..4] => [0x21, 0x42, 0x63, 0xff]);
    main_assert_ne!(applied.pixels().as_ptr() => pristine_ptr);
    main_assert!(app.active_global_gui_failures.is_empty());
    let applied_ptr = app
        .assets
        .startup_dialog_images
        .get("GUICaption.png")
        .test_value()
        .pixels()
        .as_ptr();

    // A malformed synchronized winner fails through the same pre-pixel gate.
    let corrupt_pack = install_network_definition_pack(content_root, "CorruptPack.c4d", "CORR");
    let corrupt_graphics = corrupt_pack.join("Graphics.c4g");
    fs::create_dir_all(&corrupt_graphics).test_value();
    fs::write(corrupt_graphics.join("GUIScroll.png"), b"not a png").test_value();
    let corrupt_core = n1_fixture!(resource {
        resource_type: clonk_network::HostResourceType::Definitions as u8,
        id: 990,
        loadable: true,
        filename: clonk_engine::LegacyCString::from_bytes(b"CorruptPack.c4d".to_vec()).test_value(),
    });
    let mut corrupt_join_data = join_data;
    corrupt_join_data
        .parameters
        .game_resources
        .push(corrupt_core.clone());
    let mut corrupt_game_resources = game_resources;
    corrupt_game_resources.push(ResolvedClientStartResource {
        core: corrupt_core.clone(),
        path: corrupt_pack.clone(),
    });
    let corrupt_artifact = preload_job(&app, corrupt_join_data.clone(), corrupt_game_resources);
    app.admission_resources.ensure_by_core(&corrupt_core);
    app.admission_resources
        .mark_complete(corrupt_core.id, corrupt_pack.clone());
    app.loading_state = None;
    app.pending_network_join_data = Some(corrupt_join_data);
    app.lobby_preload_artifact = Some(corrupt_artifact);
    app.try_prepare_client_network_scenario().test_value();
    let failures = app
        .loading_state
        .as_ref()
        .test_value()
        .refreshed_global_gui_failures
        .test_ref();
    main_assert!(failures.contains_key("GUIScroll"));
    let error = app
        .apply_pending_loading_resource_refresh()
        .expect_err("the malformed winner fails typed before pixels");
    assert_engine_parity_boundary(
                error,
                ClassicParityBoundary::GlobalGuiBootstrapResources {
                    issues: vec![ClassicGuiBootstrapIssue::malformed(
                        "GUIScroll",
                        "a readable selected bmp/jpeg/jpg/png RGBA surface",
                        format!(
                            "{root}:GUIScroll.png: failed to decode exact classic image entry `GUIScroll.png` from {root}",
                            root = corrupt_graphics.display()
                        ),
                    )],
                },
            );
    main_assert_eq!(
        app.assets
            .startup_dialog_images
            .get("GUICaption.png")
            .expect("gated caption sheet")
            .pixels()
            .as_ptr() =>
        applied_ptr,
        "the failed gate must not touch any applied sheet"
    );
}

#[test]
fn startup_bootstrap_precedes_all_seven_roots_status_models_and_native_pixels() {
    let mut app = new_real_classic_menu_app(320, 200);
    let mut initial = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut initial);
    Arc::get_mut(&mut app.assets)
        .test_value()
        .startup_dialog_images
        .remove("StartupPlrPropBG.png")
        .test_value();
    let expected = vec![ClassicStartupBootstrapIssue::missing(
        "StartupPlrPropBG.png",
    )];

    for view in StartupView::ALL {
        n1_select_empty_startup_view(&mut app, view);
        app.status_text = format!("lower-priority status for {view:?}");

        let mut frame = vec![0xa5; 320 * 200 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("bootstrap must precede every root");
        assert_startup_bootstrap_boundary(&error, expected.clone());
        main_assert!(
            frame.iter().all(|byte| *byte == 0xa5),
            "{view:?} must not touch the frame before its bootstrap boundary"
        );

        let mut native = vec![0x6d; 640 * 400 * 4];
        let error = app
            .render_native_main_menu_text(&mut native, 640, 400)
            .expect_err("native pass must use the same all-root preflight");
        assert_startup_bootstrap_boundary(&error, expected.clone());
        main_assert!(native.iter().all(|byte| *byte == 0x6d));
    }
}

#[test]
fn retained_main_requires_scaled_native_fonts_but_headless_cpu_renders_without_them() {
    let mut app = new_real_classic_menu_app(320, 200);
    Arc::make_mut(&mut app.assets).startup_native_font_source = None;
    app.configure_native_startup_fonts(3.0, false);
    main_assert!(app.native_startup_fonts.is_none());

    let mut frame = vec![0xb4; 320 * 200 * 4];
    app.test_render(&mut frame);

    let error = app
        .render_retained_gpu_frame(GpuPresentation {
            physical_extent: [960, 600],
            scale: 3.0,
            crop_top: 0,
            world_zoom: 1.0,
        })
        .err()
        .test_value();
    assert_startup_bootstrap_boundary(
        &error,
        vec![ClassicStartupBootstrapIssue::missing(
            "ScaleNativeStartupFonts",
        )],
    );

    app.configure_native_startup_fonts(1.0, false);
    app.test_render(&mut frame);

    let error = app
        .render_retained_gpu_frame(GpuPresentation::identity(320, 200))
        .err()
        .test_value();
    assert_startup_bootstrap_boundary(
        &error,
        vec![ClassicStartupBootstrapIssue::missing(
            "ScaleNativeStartupFonts",
        )],
    );
}

#[test]
fn scale_one_builds_native_fonts_and_enables_ordered_text_capture() {
    let mut app = new_real_classic_menu_app(320, 200);
    app.configure_native_startup_fonts(1.0, false);
    let fonts = app.native_startup_fonts.test_ref();
    main_assert_eq!(fonts.scale() => 1.0);
    main_assert!(app.can_present_ordered_native_text(1.0));

    app.mode = AppMode::Running;
    main_assert!(app.can_present_ordered_native_text(1.0));
}

#[test]
fn startup_status_boundary_precedes_native_main_text_pixels() {
    let mut app = new_real_classic_menu_app(320, 200);
    app.status_text = "native-pass diagnostic".to_string();
    let mut frame = vec![0x6b; 960 * 600 * 4];

    let error = app
        .render_native_main_menu_text(&mut frame, 960, 600)
        .expect_err("native text pass must reject arbitrary startup status");
    main_assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::StartupStatusOverlay {
            view: StartupView::MainMenu,
            status,
        }) if status == "native-pass diagnostic"
    ));
    main_assert_eq!(app.status_text => "native-pass diagnostic");
    main_assert!(
        frame.iter().all(|byte| *byte == 0x6b),
        "native pass must fail before touching the physical frame"
    );
}

#[test]
fn network_team_switch_rechecks_gate_then_queues_authenticated_control() {
    let mut app = new_state_only_running_sandbox_app();
    let primary = app.players.local_owner;
    let primary_team = app
        .engine
        .player(primary)
        .and_then(clonk_engine::Player::team);
    let owner = 17;
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Red", 0x00f4_0000),
        clonk_engine::TeamInfo::new(2, "Blue", 0x0000_00f4),
    ]);
    app.engine
        .register_player(PlayerConfig::new(owner, "Secondary"))
        .test_value();
    app.engine
        .test_player_mut(owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(7));
    app.engine.set_player_team(owner, Some(1)).test_value();
    app.snapshot = app.engine.snapshot();
    let mut teams = app.engine.team_configuration();
    teams.allow_team_switch = true;
    app.engine.set_team_configuration(teams);
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);

    app.apply_ingame_menu_action_for_player(owner, MenuAction::ActivateTeamSelection)
        .test_value();
    let outcome = {
        let menu = app.ingame_menu.get_mut(owner).test_value();
        menu.set_selection(1);
        menu.handle_command(ControlCommand::MenuEnter, CommandKind::Press)
            .test_value()
    };
    teams.allow_team_switch = false;
    app.engine.set_team_configuration(teams);
    app.execute_ingame_menu_outcome_for_player(owner, outcome)
        .test_value();
    main_assert!(commands.take_submitted_internal_player_scripts().is_empty());
    main_assert_eq!(app.engine.player(owner).and_then(clonk_engine::Player::team) => Some(1));
    main_assert!(app.ingame_menu.get(owner).is_none());

    teams.allow_team_switch = true;
    app.engine.set_team_configuration(teams);
    app.apply_ingame_menu_action_for_player(owner, MenuAction::ActivateTeamSelection)
        .test_value();
    let outcome = {
        let menu = app.ingame_menu.get_mut(owner).test_value();
        menu.set_selection(1);
        menu.handle_command(ControlCommand::MenuEnter, CommandKind::Press)
            .test_value()
    };
    let tick = app.local_control_submission_tick();
    app.execute_ingame_menu_outcome_for_player(owner, outcome)
        .test_value();
    let control = clonk_engine::SetPlayerTeamControlData {
        team: 2,
        player: owner,
        by_client: 7,
    };
    main_assert_eq!(commands.take_submitted_internal_player_scripts() => vec![(tick, clonk_engine::ControlPacket::SetPlayerTeam(control))]);
    main_assert_eq!(app.engine.player(owner).and_then(clonk_engine::Player::team) => Some(1), "submission waits for synchronized execution");

    app.apply_ready_controls(tick, vec![NetworkControl::SetPlayerTeam(control)])
        .test_value();
    main_assert_eq!(app.engine.player(owner).and_then(clonk_engine::Player::team) => Some(2));
    main_assert_eq!(
        app.engine
            .player(primary)
            .and_then(clonk_engine::Player::team) =>
        primary_team,
        "the primary local player is not substituted for the menu owner"
    );
}

#[test]
fn hostility_menu_lists_other_players_and_toggles_hostility() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.players.local_owner;
    let ada = 17;
    let bot = 18;
    let hidden = 19;
    let ada_info = 117;
    let bot_info = 118;
    let hidden_info = 119;

    app.engine
        .register_player(PlayerConfig::new(ada, "Ada").with_player_info_id(ada_info))
        .test_value();
    app.engine
        .register_player(PlayerConfig::new(bot, "Bot").with_player_info_id(bot_info))
        .test_value();
    app.engine
        .register_player(PlayerConfig::new(hidden, "Hidden").with_player_info_id(hidden_info))
        .test_value();
    app.engine
        .test_player_mut(owner)
        .set_at_client(clonk_engine::PlayerAtClient::new(3));
    app.engine
        .test_player_mut(ada)
        .set_hostile_towards(owner, true);
    app.engine
        .test_player_mut(owner)
        .set_hostile_towards(bot, true);
    let mut teams = app.engine.team_configuration();
    teams.allow_hostility_change = true;
    app.engine.set_team_configuration(teams);
    app.control_player_infos.apply(n1_fixture!(player_data {
        client_id: 0,
        players: vec![
            n1_fixture!(player {
                id: ada_info,
                player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
            }),
            n1_fixture!(player {
                id: bot_info,
                player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
            }),
            n1_fixture!(player {
                id: hidden_info,
                player_type: clonk_engine::PLAYER_INFO_TYPE_USER,
                flags: clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
            }),
        ],
    }));
    app.snapshot = app.engine.snapshot();
    let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(3);
    app.network = Some(manager);

    app.apply_ingame_menu_action(MenuAction::ActivateHostility)
        .test_value();
    let menu = app.ingame_menu.get(owner).test_value();
    main_assert_eq!(menu.page() => ingame_menu::MenuPage::Hostility);
    main_assert_eq!(menu.caption() => "Don't attack Ada");
    main_assert!(menu.is_permanent());
    main_assert_eq!(menu.close_action() => Some(&MenuAction::ActivateMain));
    main_assert_eq!(menu.items().iter().map(|item| item.caption.as_str()).collect::<Vec<_>>() => vec!["Don't attack Ada", "Attack Bot"]);
    main_assert_eq!(menu.items()[0].info_caption.as_deref() => Some("Ada is currently hostile and will not be attacked."));
    main_assert_eq!(menu.items()[1].info_caption.as_deref() => Some("Bot is currently friendly and will be attacked."));
    main_assert!(
        matches!(menu.items()[1].symbol, ingame_menu::MenuSymbol::Hostility {opponent, hostile: true} if opponent == bot)
    );

    let tick = app.local_control_submission_tick();
    let outcome = {
        let menu = app.ingame_menu.get_mut(owner).test_value();
        menu.set_selection(0);
        menu.handle_command(ControlCommand::MenuEnter, CommandKind::Press)
            .test_value()
    };
    main_assert!(
        matches!(outcome, MenuOutcome::Action {action: MenuAction::ToggleHostility(opponent), close_menu: false} if opponent == ada)
    );
    app.execute_ingame_menu_outcome_for_player(owner, outcome)
        .test_value();
    let control = clonk_engine::ToggleHostilityControlData {
        opponent: ada,
        player: owner,
        by_client: 3,
    };
    main_assert_eq!(commands.take_submitted_internal_player_scripts() => vec![(tick, clonk_engine::ControlPacket::ToggleHostility(control))]);
    main_assert!(!app
        .engine
        .player(owner)
        .expect("menu owner")
        .is_hostile_towards(ada));
    main_assert_eq!(app.ingame_menu.get(owner).expect("permanent page").items()[0].caption => "Don't attack Ada");

    app.apply_ready_controls(tick, vec![NetworkControl::ToggleHostility(control)])
        .test_value();
    main_assert!(app
        .engine
        .player(owner)
        .expect("menu owner")
        .is_hostile_towards(ada));
    main_assert_eq!(
        app.ingame_menu.get(owner).expect("permanent page").items()[0].caption =>
        "Don't attack Ada",
        "native waits for C4Menu::Execute's Tick35 refill"
    );
    app.refresh_hostility_menus();
    main_assert_eq!(app.ingame_menu.get(owner).expect("Tick35-refreshed page").items()[0].caption => "Attack Ada");

    app.apply_ingame_menu_action_for_player(owner, MenuAction::ToggleHostility(bot))
        .test_value();
    main_assert!(commands.take_submitted_internal_player_scripts().is_empty());

    teams.allow_hostility_change = false;
    app.engine.set_team_configuration(teams);
    app.apply_ingame_menu_action_for_player(owner, MenuAction::ToggleHostility(ada))
        .test_value();
    main_assert!(commands.take_submitted_internal_player_scripts().is_empty());
    main_assert!(app
        .engine
        .player(owner)
        .expect("menu owner")
        .is_hostile_towards(ada));

    teams.allow_hostility_change = true;
    app.engine.set_team_configuration(teams);
    app.network = None;
    app.apply_ingame_menu_action_for_player(owner, MenuAction::ToggleHostility(ada))
        .test_value();
    main_assert!(!app
        .engine
        .player(owner)
        .expect("menu owner")
        .is_hostile_towards(ada));
    main_assert_eq!(app.ingame_menu.get(owner).expect("permanent page").items()[0].caption => "Attack Ada", "local execution also waits for the periodic refill");
    app.refresh_hostility_menus();
    main_assert_eq!(app.ingame_menu.get(owner).expect("locally refreshed page").items()[0].caption => "Don't attack Ada");
}

#[test]
fn all_graphical_modes_produce_retained_scenes() {
    let mut menu = new_real_menu_app(320, 200);
    menu.startup.dialog_fade = None;
    menu.graphics.set_runtime_sprite_filtering(1.0, false);
    menu.configure_native_startup_fonts(1.0, false);
    let menu_presentation = retained_test_presentation(&menu);
    let menu_frame = menu
        .render_retained_gpu_frame(menu_presentation)
        .test_value();
    assert_retained_frame_has_commands("menu", &menu_frame);

    let mut loading = new_real_menu_app(320, 200);
    loading.graphics.set_runtime_sprite_filtering(1.0, false);
    loading.configure_native_startup_fonts(1.0, false);
    let fonts = loading.assets.clonk_fonts.clone().test_value();
    loading.loader_screen = Some(
        LoaderScreen::new(
            LoaderSelection::startup("LoaderRetained.png")
                .expect("valid retained loader selection"),
            ImageData::new(1, 1, vec![7, 8, 9, 255]),
            LoaderResources::new(fonts, ImageData::new(3, 1, vec![255; 12]))
                .expect("valid retained loader resources"),
            LoaderState::initial("Loading"),
        )
        .test_value(),
    );
    loading.loader_error = None;
    loading.loader_render_error = None;
    loading.mode = AppMode::Loading;
    let loading_presentation = retained_test_presentation(&loading);
    let loading_frame = loading
        .render_retained_gpu_frame(loading_presentation)
        .test_value();
    assert_retained_frame_has_commands("loading", &loading_frame);

    let mut running = new_classic_running_sandbox_app();
    running.graphics.set_runtime_sprite_filtering(1.0, false);
    running.configure_native_startup_fonts(1.0, false);
    let running_presentation = retained_test_presentation(&running);
    let running_frame = running
        .render_retained_gpu_frame(running_presentation)
        .test_value();
    assert_retained_frame_has_commands("running", &running_frame);

    let mut console = new_state_only_menu_app(320, 200);
    console.console_mode = true;
    let console_presentation = retained_test_presentation(&console);
    let console_frame = console
        .render_retained_gpu_frame(console_presentation)
        .test_value();
    assert_retained_frame_has_commands("console", &console_frame);
}

#[test]
fn scale_native_text_keeps_logical_physical_painter_order() {
    let mut app = new_real_menu_app(320, 200);
    app.startup.dialog_fade = None;
    app.graphics.set_runtime_sprite_filtering(2.0, false);
    app.configure_native_startup_fonts(2.0, false);
    let presentation = GpuPresentation {
        physical_extent: [640, 400],
        scale: 2.0,
        crop_top: 0,
        world_zoom: 1.0,
    };

    let frame = app.render_retained_gpu_frame(presentation).test_value();
    assert_retained_frame_has_commands("scale-native menu", &frame);
    let logical = frame
        .layers
        .iter()
        .position(|layer| layer.presentation == presentation)
        .test_value();
    let physical = frame
        .layers
        .iter()
        .position(|layer| layer.presentation == GpuPresentation::identity(640, 400))
        .test_value();
    main_assert!(
        logical < physical,
        "native text must follow the logical chrome batch that produced it"
    );
    main_assert!(frame
        .layers
        .iter()
        .all(|layer| { layer.presentation.physical_extent == presentation.physical_extent }));
}

#[test]
fn startup_gamma_uses_the_native_snapshot_and_scalar_grammar() {
    let config =
        b"[Graphics]\nGamma1=0x010203tail\nGamma2=6579300junk\nGamma3=-1\nDisableGamma=false\n";
    main_assert_eq!(load_classic_loader_gamma_from_native(config) => Some(clonk_graphics::GammaRamp::from_control_points([0x010203, 0x646464, u32::MAX,])));
    main_assert_eq!(load_classic_loader_gamma_from_native(b"[Graphics]\nGamma2=0x707070\nDisableGamma=true\n") => None);
}

#[test]
fn fractional_loader_scales_reach_native_text() {
    let (_user_data, _guard, paths) = n1_repository_paths();
    for (scale, physical_width, physical_height) in [(1.5, 480_u32, 300_u32), (0.5, 160, 100)] {
        let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
        app.configure_native_startup_fonts(scale, false);
        main_assert_eq!(app.mode => AppMode::Loading);
        main_assert!(app.can_defer_native_loader_text(scale));
        let mut presenter =
            clonk_scaling::FramePresenter::new(scale, physical_width, physical_height);
        let mut frame = vec![0_u8; physical_width as usize * physical_height as usize * 4];
        let refreshed = presenter
            .present(&mut frame, |logical| {
                app.render_for_presentation(logical, false, true, false)
            })
            .test_value();
        main_assert!(refreshed);
        app.render_native_loader_text(&mut frame, physical_width, physical_height)
            .test_value();
        main_assert!(frame.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }
}

#[test]
fn scale_fifty_host_and_client_waits_keep_ordered_loader_composition() {
    let (_user_data, _guard, paths) = n1_repository_paths();

    for client in [false, true] {
        let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
        app.configure_native_startup_fonts(0.5, false);
        app.loader_screen
            .test_mut()
            .update(LoaderUpdate::SetTitle("scale-fifty loader".into()));
        app.loader_screen
            .test_mut()
            .update(LoaderUpdate::ReplaceLog(vec!["process".into()]));
        app.loader_screen
            .test_mut()
            .update(LoaderUpdate::SetProcess(Some(50)));
        n1_install_loading_state(&mut app);
        if client {
            app.network_mode = Some(NetworkMode::Client(client_network_settings()));
            app.show_reached_network_start_wait().test_value();
            app.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "upper message",
                    "upper caption",
                    clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                ),
                MessageDialogContinuation::None,
            )
            .test_value();
        } else {
            app.network_mode = Some(NetworkMode::Host(host_network_settings()));
            app.begin_network_start_wait(n1_fixture!(status:
                clonk_network::NETWORK_STATE_GO,
                1,
                4,
            ));
            app.show_reached_network_start_wait().test_value();
        }

        let (chrome, rendered, plan) = render_ordered_test_frame(&mut app, 0.5, 320, 240);
        main_assert_ne!(rendered => chrome, "native text must reach the physical frame");
        let loader_batch = n1_loader_batch(&plan);
        let wait_batch = n1_text_batch(&plan, "Waiting for start...");
        main_assert_eq!(loader_batch => 0);
        main_assert!(wait_batch > loader_batch);
        main_assert!(plan.batches[wait_batch].logical_layer.is_some());
        if client {
            let upper_batch = n1_text_batch(&plan, "upper message");
            main_assert!(upper_batch > wait_batch);
            main_assert!(plan.batches[upper_batch].logical_layer.is_some());
        }
    }
}

#[test]
fn scale_three_host_start_wait_renders_after_native_loader_text() {
    let (_user_data, _guard, paths) = n1_repository_paths();
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.configure_native_startup_fonts(3.0, false);
    app.loader_screen
        .test_mut()
        .update(LoaderUpdate::SetTitle("Session|host loader".into()));
    app.loader_screen
        .test_mut()
        .update(LoaderUpdate::ReplaceLog(vec!["process".into()]));
    app.loader_screen
        .test_mut()
        .update(LoaderUpdate::SetProcess(Some(73)));
    n1_install_loading_state(&mut app);
    app.network_mode = Some(NetworkMode::Host(host_network_settings()));
    let status = n1_fixture!(status: clonk_network::NETWORK_STATE_GO, 1, 4);
    app.begin_network_start_wait(status);
    app.show_reached_network_start_wait().test_value();

    let (_chrome, _rendered, plan) = render_ordered_test_frame(&mut app, 3.0, 1920, 1440);
    let loader_batch = n1_loader_batch(&plan);
    let wait_batch = n1_text_batch(&plan, "Waiting for start...");
    main_assert_eq!(loader_batch => 0, "loader text owns the native base batch");
    main_assert!(
        wait_batch > loader_batch && plan.batches[wait_batch].logical_layer.is_some(),
        "wait dialog chrome/text must be composited after loader text"
    );
}

#[test]
fn fractional_client_wait_and_upper_dialog_keep_native_layer_order() {
    let (_user_data, _guard, paths) = n1_repository_paths();
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    app.configure_native_startup_fonts(1.5, false);
    app.loader_screen
        .test_mut()
        .update(LoaderUpdate::SetTitle("client loader".into()));
    n1_install_loading_state(&mut app);
    app.network_mode = Some(NetworkMode::Client(client_network_settings()));
    app.show_reached_network_start_wait().test_value();
    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "upper message",
            "upper caption",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    let (upper_layout, title_point) =
        {
            let resources = app.assets.message_dialog_resources().test_value();
            let layout = app.dialogs.messages.last().test_value().state.layout(
                640,
                480,
                &resources.fonts.text,
            );
            let caption = layout.caption.test_value();
            let point = GuiPoint::new((caption.x + 10) as f32, (caption.y + 10) as f32);
            (layout, point)
        };
    let tooltip_started = Instant::now()
        .checked_sub(clonk_frontend::context_menu::CLASSIC_TOOLTIP_DELAY + Duration::from_millis(1))
        .test_value();
    app.startup_tooltip = ClassicTooltipTracker::new_at(tooltip_started);
    app.startup_tooltip
        .note_pointer_move_at(title_point, tooltip_started);
    app.dialogs
        .messages
        .last_mut()
        .test_value()
        .state
        .handle_pointer_move(title_point, &upper_layout);

    let (_chrome, _rendered, plan) = render_ordered_test_frame(&mut app, 1.5, 960, 720);
    let loader_batch = n1_loader_batch(&plan);
    let client_wait_batch = n1_text_batch(&plan, "Waiting for start...");
    let upper_batch = n1_text_batch(&plan, "upper message");
    main_assert_eq!(loader_batch => 0);
    main_assert!(client_wait_batch > loader_batch);
    main_assert!(upper_batch > client_wait_batch);
    main_assert!(plan.batches[client_wait_batch].logical_layer.is_some());
    main_assert!(plan.batches[upper_batch].logical_layer.is_some());
    let upper_caption_batches = plan
        .batches
        .iter()
        .enumerate()
        .filter_map(|(index, batch)| {
            batch
                .text
                .iter()
                .any(|command| command.text == "upper caption")
                .then_some(index)
        })
        .collect::<Vec<_>>();
    main_assert_eq!(upper_caption_batches.len() => 2);
    let tooltip_batch = *upper_caption_batches.last().test_value();
    main_assert!(tooltip_batch > upper_batch);
    main_assert!(plan.batches[tooltip_batch].logical_layer.is_some());

    app.loader_screen = None;
    let mut failed_frame = vec![0_u8; 640 * 480 * 4];
    let error = app
        .render_ordered_native_base(&mut failed_frame)
        .expect_err("missing loader must still fail closed");
    main_assert!(error.to_string().contains("no selected classic loader"));
    main_assert!(app.pending_native_presentation.is_none());
    main_assert!(!app.graphics.surface().is_clonk_text_capture_active());
}

#[test]
fn scale_three_clipped_loader_uses_native_text_after_chrome_upscale() {
    let (_user_data, _guard, paths) = n1_repository_paths();
    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    app.configure_native_startup_fonts(3.0, false);
    // C++ rounds 598/3 up to 200 logical rows and clips two nominal GL rows.
    let mut presenter = clonk_scaling::FramePresenter::new(3.0, 960, 598);
    let mut physical = vec![0_u8; 960 * 598 * 4];
    presenter
        .present(&mut physical, |frame| {
            app.render_for_presentation(frame, false, true, false)
        })
        .test_value();
    app.render_native_loader_text(&mut physical, 960, 598)
        .test_value();
    main_assert!(physical.chunks_exact(4).any(|pixel| pixel[3] != 0));
}

#[test]
fn scale_three_clipped_main_menu_commits_native_captions_after_bilinear_base() {
    // CStdFont scales natively while StdGL filters image textures
    // (C4Fonts.cpp:158-173; StdFont.cpp:319-352,841-842; StdGL.cpp:527-532).
    let (_user_data, _guard, paths) = n1_repository_paths();
    let mut app = test_game_app(640, 480, AudioOptions::default(), Some(&paths)).test_value();
    wait_for_menu(&mut app);
    app.configure_native_startup_fonts(3.0, false);
    main_assert!(app.can_defer_native_main_menu_text(3.0));

    let mut presenter = clonk_scaling::FramePresenter::new(3.0, 1920, 1438);
    let mut output = vec![0_u8; 1920 * 1438 * 4];
    let refreshed = presenter
        .present(&mut output, |frame| {
            app.render_for_presentation(frame, true, false, false)
        })
        .test_value();
    main_assert!(refreshed);
    let filtered_base = output.clone();
    app.render_native_main_menu_text(&mut output, 1920, 1438)
        .test_value();
    main_assert_ne!(output => filtered_base, "physical caption pass must draw");

    let button = clonk_frontend::main_menu_layout(640, 480).buttons[0];
    let changed_in_caption = (button.y * 3..(button.y + button.h) * 3).any(|y| {
        (button.x * 3..(button.x + button.w) * 3).any(|x| {
            let index = (y as usize * 1920 + x as usize) * 4;
            output[index..index + 4] != filtered_base[index..index + 4]
        })
    });
    main_assert!(
        changed_in_caption,
        "Start Game changed in physical button bounds"
    );
    main_assert_eq!(&output[..4] => &filtered_base[..4], "background remains filtered");

    let with_native_text = output.clone();
    let refreshed = presenter
        .present(&mut output, |frame| {
            app.render_for_presentation(frame, true, false, false)
        })
        .test_value();
    main_assert!(refreshed);
    main_assert_eq!(output => filtered_base, "an unchanged menu recomposes the same deferred-text base");
    app.render_native_main_menu_text(&mut output, 1920, 1438)
        .test_value();
    main_assert_eq!(output => with_native_text, "the physical caption pass is deterministic over an identical base");
}

#[test]
fn scale_three_open_startup_dialog_keeps_native_text_in_z_order() {
    let mut app = new_real_classic_menu_app(640, 480);
    app.graphics.set_runtime_sprite_filtering(3.0, false);
    app.configure_native_startup_fonts(3.0, false);
    for label in ["LOWER", "H"] {
        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                label,
                label,
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .test_value();
    }

    let (chrome, rendered, plan) = render_ordered_test_frame(&mut app, 3.0, 1920, 1440);
    main_assert!(
        plan.batches
            .first()
            .is_some_and(|batch| !batch.text.is_empty()
                && batch
                    .text
                    .iter()
                    .all(|command| command.text != "LOWER" && command.text != "H")),
        "main-menu captions remain in the native base batch while dialogs are open"
    );
    let (lower_batch, _) = n1_text_command(&plan, "LOWER");
    let (upper_batch, upper) = n1_text_command(&plan, "H");
    main_assert!(
                lower_batch > 0 && upper_batch > lower_batch,
                "stacked dialogs must alternate chrome/text in ownership order: lower={lower_batch}, upper={upper_batch}"
            );
    main_assert!(
        plan.batches[upper_batch].logical_layer.is_some(),
        "the upper dialog chrome is composited after lower native text"
    );
    assert_one_pixel_native_edge(&chrome, &rendered, 1920, 1440, upper, 3.0);
}

/// The async deadline drops absent control; late input is stale, so the client
/// records the loss (`PackCompleteCtrl`, C4GameControlNetwork.cpp:741-784).
#[test]
fn a_client_learns_when_the_async_deadline_dropped_its_control() {
    let mut app = new_state_only_menu_app(320, 200);
    main_assert_eq!(app.discarded_control_ticks => 0);
    main_assert_eq!(app.last_reported_discarded_control_tick => None);

    app.note_discarded_control_tick(41);
    main_assert_eq!(app.discarded_control_ticks => 1);
    main_assert_eq!(app.last_reported_discarded_control_tick => Some(41));

    // A repeated tick is reported once but counted on every observation.
    app.note_discarded_control_tick(41);
    main_assert_eq!(app.discarded_control_ticks => 2);
    main_assert_eq!(app.last_reported_discarded_control_tick => Some(41));

    app.note_discarded_control_tick(42);
    main_assert_eq!(app.discarded_control_ticks => 3);
    main_assert_eq!(app.last_reported_discarded_control_tick => Some(42));

    // The diagnostic queues no synchronized control.
    main_assert!(app.network.is_none());
}
