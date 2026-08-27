// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! runtime_fixture {
    (network_connection: $connection_id:expr, $client_id:expr, $usage:expr, $protocol:expr, $packet_loss:expr, $ping_ms:expr, $lag_ms:expr $(,)?) => {
        clonk_network::RuntimeNetworkConnection {
            connection_id: $connection_id,
            client_id: $client_id,
            usage: $usage,
            protocol: $protocol,
            peer_address: None,
            packet_loss: $packet_loss,
            ping_ms: $ping_ms,
            lag_ms: $lag_ms,
        }
    };
    (gpu_profile_renderer_surface_capture: $renderer:expr, $surface:expr, $capture:expr $(,)?) => {
        RetainedGpuFrameProfile {
            frame_preparation: Duration::from_nanos(2),
            renderer: $renderer,
            surface: $surface,
            capture: $capture,
            context: RetainedGpuFrameContext::default(),
        }
    };
    (player_control_defaults $(,)?) => {
        clonk_engine::PlayerControlData {
            player: 2,
            command: i32::from(clonk_engine::COM_LEFT),
            data: 0,
            by_client: 1,
        }
    };
    (client: $client_id:expr, $activated:expr $(,)?) => {
        clonk_engine::ClientCoreControlData {
            client_id: $client_id,
            activated: $activated,
            ..Default::default()
        }
    };
    (player_selection: $name:expr, $comment:expr $(,)?) => {
        clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: $name,
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: 0xff,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: $comment,
        }
    };
}

macro_rules! runtime_assert_eq {
    ($actual:expr => $expected:expr, $($message:tt)+) => {
        assert_eq!($actual, $expected, $($message)+);
    };
    ($($actual:expr => $expected:expr);+ $(;)?) => {
        $(assert_eq!($actual, $expected);)+
    };
}

macro_rules! runtime_assert {
    ($condition:expr, $($message:tt)+) => {
        assert!($condition, $($message)+);
    };
    ($($condition:expr);+ $(;)?) => {
        $(assert!($condition);)+
    };
}

macro_rules! runtime_assert_ne {
    ($actual:expr => $unexpected:expr, $($message:tt)+) => {
        assert_ne!($actual, $unexpected, $($message)+);
    };
    ($($actual:expr => $unexpected:expr);+ $(;)?) => {
        $(assert_ne!($actual, $unexpected);)+
    };
}

fn tap_runtime_key(app: &mut GameApp, key: VirtualKeyCode) {
    app.test_key(key, ElementState::Pressed);
    app.test_key(key, ElementState::Released);
}

fn startup_player_focus(app: &GameApp) -> clonk_frontend::startup_plrsel::PlrSelControl {
    app.startup_player_dialog
        .as_ref()
        .expect("player dialog")
        .focused_control()
}

fn selected_options_control_set(
    app: &GameApp,
    device: clonk_frontend::startup_options_controls::ControlDevice,
) -> usize {
    app.startup_options_dialog
        .as_ref()
        .unwrap()
        .controls()
        .selected_set(device)
}

fn install_runtime_key_config(
    app: &mut GameApp,
    config: std::result::Result<RuntimeKeyConfig, String>,
) {
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache.set(config).test_value();
}

fn open_test_console_viewport(app: &mut GameApp, player: Option<i32>) -> u64 {
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::NewViewport(player)])
        .test_value();
    app.physical_viewports.last().test_value().physical_identity
}

fn open_local_test_console_viewport(app: &mut GameApp) -> u64 {
    open_test_console_viewport(app, Some(app.local_owner))
}

fn runtime_console_network_fixture(
    mode: ConsoleEditMode,
) -> (
    GameApp,
    network::NetworkEventSender,
    network::TestNetworkCommands,
    u64,
) {
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = mode;
    let (events, commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let identity = open_test_console_viewport(&mut app, None);
    (app, events, commands, identity)
}

fn runtime_scenario_app(
    user_data: &Path,
    player_name: &str,
    scenario_id: &str,
) -> (EnvGuard, AppPaths, GameApp, FrontendScenario) {
    let (paths_guard, paths) = exact_loader_test_paths(user_data, None);
    configure_test_startup_participant(&paths, user_data);
    let mut app = GameApp::new(
        320,
        200,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with(player_name.to_string(), false),
    )
    .test_value();
    wait_for_menu(&mut app);
    let scenario = resolve_next_mission_scenario(&app.scenario_catalog, scenario_id).test_value();
    (paths_guard, paths, app, scenario)
}

fn runtime_overlay_object(
    id: ObjectId,
    definition: &str,
    position: Vector2,
    energy: i32,
    magic_energy: i32,
    info_physical: clonk_engine::PhysicalInfo,
    breath: i32,
) -> ObjectSnapshot {
    let mut object = make_object(id.as_u64(), definition, position);
    object.energy = energy;
    object.magic_energy = magic_energy;
    object.info_physical = Some(info_physical);
    object.breath = breath;
    object
}

fn runtime_config_value<T>(body: Option<&str>, load: impl FnOnce(&AppPaths) -> T) -> T {
    let root = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(root.path().join("planet/System.c4g")).test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(root.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    if let Some(body) = body {
        fs::write(paths.config_file(), body).test_value();
    }
    load(&paths)
}

struct RuntimeInstallFixture {
    install: tempfile::TempDir,
    _user_data: tempfile::TempDir,
    _guard: EnvGuard,
    system: PathBuf,
    paths: AppPaths,
}

fn runtime_install_fixture(config: Option<&str>) -> RuntimeInstallFixture {
    let install = tempdir();
    let user_data = tempdir();
    let system = install.path().join("planet/System.c4g");
    fs::create_dir_all(&system).test_value();
    let (guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    if let Some(config) = config {
        fs::write(paths.config_file(), config).test_value();
    }
    RuntimeInstallFixture {
        install,
        _user_data: user_data,
        _guard: guard,
        system,
        paths,
    }
}

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
    .test_value();
    let classic = parse_classic_command_line(&cli.classic_arguments);

    assert_eq!(cli.test_load, Some(PathBuf::from("Fixture.c4s")));
    assert_eq!(cli.player_name, "/network");
    assert_eq!(cli.test_frames, 7);
    assert_eq!(classic.scenario, Some(PathBuf::from("/tmp/Direct.C4S")));
    assert_eq!(classic.network_active, None);
    assert!(Cli::try_parse_from(["clonk-app", "--future"]).is_err());
}

#[test]
fn debug_hud_requires_developer_interactive_launch() {
    runtime_assert!(
        debug_hud_enabled(Some("1"), true, DebugHudLaunch::Interactive, false,);
        !debug_hud_enabled(None, true, DebugHudLaunch::Interactive, false,);
        !debug_hud_enabled(Some("true"), true, DebugHudLaunch::Interactive, false,);
        !debug_hud_enabled(Some("1"), false, DebugHudLaunch::Interactive, false,);
    );
}

#[test]
fn debug_hud_is_suppressed_for_parity_and_compatibility_launches() {
    for launch in [DebugHudLaunch::ParityCapture, DebugHudLaunch::Compatibility] {
        assert!(!debug_hud_enabled(Some("1"), true, launch, false));
    }
    runtime_assert!(!debug_hud_enabled(
        Some("1"),
        true,
        DebugHudLaunch::Interactive,
        true,
    ));
}

#[test]
fn debug_hud_launch_classification_is_fail_closed() {
    let parity_launches: &[&[&str]] = &[
        &["clonk-app", "--test-load", "Fixture.c4s"],
        &["clonk-app", "--integration-test", "Fixture.c4s"],
        &["clonk-app", "--dump-frame", "frame.png"],
        &["clonk-app", "--dump-menu-frame", "frame.png"],
        &["clonk-app", "--headless"],
        &[
            "clonk-app",
            "--headed-surface-smoke",
            "headed-surface-report.json",
        ],
    ];
    for arguments in parity_launches {
        let cli = Cli::try_parse_from(*arguments).test_value();
        assert_eq!(debug_hud_launch(&cli), DebugHudLaunch::ParityCapture);
    }

    let compatibility = Cli::try_parse_from(["clonk-app", "Scenario.c4s"]).test_value();
    runtime_assert_eq!(debug_hud_launch(&compatibility) => DebugHudLaunch::Compatibility);

    let interactive = Cli::try_parse_from(["clonk-app"]).test_value();
    assert_eq!(debug_hud_launch(&interactive), DebugHudLaunch::Interactive);
}

/// C++ selects a dedicated server at build time
/// (`option(USE_CONSOLE ...)`, CMakeLists.txt:178), so it is fixed for the
/// life of the process and cannot be re-chosen mid-run. A single shipped
/// binary has no build flag, and the closest honest analogue is a `Cli`
/// switch: those are read once in `run()`, where classic arguments are
/// re-applied every round and are re-parsed at runtime by the console
/// `/open` command.
#[test]
fn headless_is_a_process_lifetime_switch_not_a_classic_argument() {
    runtime_assert!(
        !Cli::try_parse_from(["clonk-app"])
            .expect("a bare invocation parses")
            .headless
    );

    let cli = Cli::try_parse_from([
        "clonk-app",
        "--headless",
        "HarpoonRace.c4s",
        "/network",
        "/lobby",
    ])
    .test_value();
    assert!(cli.headless);
    let classic = parse_classic_command_line(&cli.classic_arguments);
    assert_eq!(classic.scenario, Some(PathBuf::from("HarpoonRace.c4s")));
    assert_eq!(classic.network_active, Some(true));

    // Deliberately not a classic switch: `/open <params>` re-parses a
    // classic command line into the running process, so a classic spelling
    // would let a client-visible command turn headlessness on or off.
    runtime_assert_eq!(parse_classic_command_line(&[OsString::from("/headless")]) => ClassicCommandLine::default());
}

#[test]
fn headed_surface_smoke_is_an_explicit_process_lifetime_diagnostic() {
    let cli = Cli::try_parse_from([
        "clonk-app",
        "--headed-surface-smoke",
        "headed-surface-report.json",
    ])
    .test_value();

    runtime_assert_eq!(cli.headed_surface_smoke => Some(PathBuf::from("headed-surface-report.json")));
    runtime_assert!(
        Cli::try_parse_from([
            "clonk-app",
            "--headless",
            "--headed-surface-smoke",
            "headed-surface-report.json",
        ])
        .is_err(),
        "the diagnostic must reach a real event loop, window and GPU surface",
    );
    runtime_assert!(
        Cli::try_parse_from([
            "clonk-app",
            "--headed-surface-smoke",
            "headed-surface-report.json",
            "HarpoonRace.c4s",
        ])
        .is_err(),
        "the diagnostic must not launch classic command-line work first",
    );
    runtime_assert!(
        !<Cli as clap::CommandFactory>::command()
            .render_long_help()
            .to_string()
            .contains("--headed-surface-smoke"),
        "the hardware diagnostic is deliberately absent from ordinary user help",
    );
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

    runtime_assert_eq!(
        classic.scenario => Some(PathBuf::from("Missions/Last"));
        classic.player_files => vec![ PathBuf::from("Players/Alice.C4P"), PathBuf::from("Players/Bob.c4p") ];
        classic.definition_files => vec![ PathBuf::from("Defs/ExtraOne.c4d"), PathBuf::from("Defs/ExtraTwo.C4D") ];
        classic.incoming_update => Some(PathBuf::from("Patch.c4u"));
        classic.record_stream => Some(PathBuf::from("Round.c4r"));
        classic_command_line_definition_modules( b"[General]\nDefinitions=Base.c4d;Second.c4d\n", &classic.definition_files, ) =>
            vec![ "Base.c4d", "Second.c4d", "Defs/ExtraOne.c4d", "Defs/ExtraTwo.C4D", ];
    );
    runtime_assert_eq!(
        classic_command_line_definition_modules(b"[General]\nDefinitions=;Base.c4d;;Second.c4d;\n", &[],) => vec!["", "Base.c4d", "", "Second.c4d"],
        "std::getline preserves leading/interior empty modules but not a trailing delimiter",
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
    runtime_assert_eq!(
        classic.runtime_join => Some(false);
        classic.tcp_port => Some(2222);
        classic.udp_port => Some(3333);
        classic.password.as_deref() => Some("secret");
        classic.comment.as_deref() => Some("launch comment");
        classic.record_dump.as_deref() => Some("dump.TXT");
        classic.record_stream => Some(PathBuf::from("record.example:11114"));
        classic.fair_crew => Some(false);
        classic.config_file => Some(PathBuf::from("portable.cfg"));
    );
    assert!(classic.verbose);
    assert_eq!(classic.language.as_deref(), Some("DE,US"));

    let mut app = new_state_only_menu_app(320, 200);
    app.apply_classic_command_line(&classic).test_value();
    assert!(app.scenario_game_options.values().master_server_signup);
    assert!(!app.scenario_game_options.values().league_server_signup);
    assert_eq!(app.scenario_game_options.values().password, "secret");
    assert_eq!(app.scenario_game_options.values().comment, "launch comment");
    assert!(!app.scenario_game_options.values().fair_crew);
    assert_eq!(app.runtime_network_join_allowed, Some(false));
}

#[test]
fn console_open_real_scenario_reaches_running() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    configure_test_startup_participant(&paths, user_data.path());
    persist_config_value(&paths, "General", "Participants", "MissingConfigured.c4p").test_value();
    let player_path = user_data.path().join("Exact.c4p");
    let scenario_path = paths.scenario_dir().join("Direct.c4s");
    let definition_path = scenario_path.join("Defs.c4d");
    fs::create_dir_all(&definition_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Direct command line\nMaxPlayer=1\n",
    )
    .test_value();
    fs::write(
        definition_path.join("DefCore.txt"),
        "[DefCore]\nid=TEST\nName=Test\nCategory=1\n",
    )
    .test_value();
    write_test_definition_graphics(&definition_path);

    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    let boot_result = app
        .boot_loading
        .take()
        .expect("real boot worker")
        .receiver
        .recv_timeout(Duration::from_secs(30))
        .test_value();
    let (boot_sender, boot_receiver) = mpsc::channel();
    app.boot_loading = Some(BootLoadingState::new(boot_receiver));
    app.console_mode = true;
    let command = format!(
        "/open \"{}\" \"{}\" \"{}\"",
        scenario_path.display(),
        player_path.display(),
        definition_path.display(),
    );
    app.process_console_command(&command).test_value();
    assert!(app.loading_state.is_none());
    assert!(app.auto_start_classic_command_line_scenario);
    boot_sender.send(boot_result).test_value();
    app.poll_boot_loading();
    assert!(!app.auto_start_classic_command_line_scenario);
    assert!(app.loading_state.is_some());

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if matches!(app.mode, AppMode::Running) {
            break;
        }
        runtime_assert_ne!(app.mode => AppMode::Menu, "startup menu must stay suppressed; status={:?}; loader_error={:?}", app.status_text, app.loader_error);
        runtime_assert!(
            Instant::now() < deadline,
            "direct scenario did not finish loading; status={:?}",
            app.status_text
        );
        app.test_update();
        thread::sleep(Duration::from_millis(2));
    }
    runtime_assert_eq!(app.active_scenario.as_ref().and_then(|scenario| scenario.path.as_deref()) => Some(scenario_path.as_path()));
    assert!(app.startup_dialog_fade.is_none());
    reset_cached_app_paths();
}

#[test]
fn command_and_hash_routes_bypass_plain_script_control() {
    let mut app = new_state_only_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);

    runtime_assert!(
        app.process_developer_console_input("/help", false).expect("slash input reaches ProcessCommand");
        app.process_developer_console_input("#/sound Bell", false).expect("hash input reaches ProcessInput");
    );
    let (controls, messages) = commands.take_submitted_decided_controls_and_messages();
    assert!(controls.is_empty());
    runtime_assert_eq!(
        messages => vec![MessageControlData { message_type: MESSAGE_TYPE_SOUND, player: app.local_owner, to_player: -1, message: legacy_cstring(b"Bell"), by_client: 0, }];
    );
}

#[test]
fn plain_script_checks_editing_and_emits_decide_console_scope() {
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    let mut config = Config::new();
    config.set_in(Some("Developer"), "ConsoleScriptStrictness", "Strict2");
    config.save(paths.config_file()).test_value();

    let mut app = new_state_only_running_sandbox_app();
    app.app_paths = Some(paths);
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 0, 2);

    runtime_assert!(!app
        .process_developer_console_input("SetGravity(41)", false)
        .expect("replay editing gate refuses plain script"));
    assert_eq!(app.status_text, "No editing while replaying.");
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty();
        app.process_developer_console_input("SetGravity(42)", true).expect("editable console accepts plain script");
    );
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
fn property_script_wraps_live_selection_as_emmo_script() {
    let mut app = new_state_only_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);

    app.submit_editor_selection_script("Mark()", &[41, 7, 41])
        .test_value();

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

// C4EditCursor.cpp:551-556 — ApplyToolBrush routes the drawing tools
// through EMControl exactly as the object editors do, so a draw gesture is
// a queued control rather than a direct landscape write.
#[test]
fn draw_tool_gesture_queues_a_decided_em_draw_tool_control() {
    let mut app = new_state_only_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);

    app.submit_or_execute_editor_draw_tool(clonk_engine::EmDrawToolControlData {
        action: clonk_engine::EMDT_BRUSH,
        mode: clonk_engine::LANDSCAPE_MODE_EXACT,
        x: 12,
        y: 34,
        grade: 5,
        ift: true,
        material: legacy_cstring(b"Earth"),
        texture: legacy_cstring(b"Rough"),
        ..Default::default()
    })
    .test_value();

    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDrawTool(control), false)] = decided.as_slice() else {
        panic!("expected one queued draw-tool control, got {decided:?}");
    };
    assert_eq!(control.action, clonk_engine::EMDT_BRUSH);
    assert_eq!((control.x, control.y), (12, 34));
    assert_eq!(control.grade, 5);
    assert!(control.ift);
    assert_eq!(control.material.as_bytes(), b"Earth");
    assert_eq!(control.texture.as_bytes(), b"Rough");
    // The submitting client stamps itself, like every other decided
    // editor control (`C4ControlPacket::SetByClient`).
    assert_eq!(control.by_client, 7);
}

#[test]
fn classic_command_line_config_and_language_override_are_process_local() {
    let repository = test_repository_root();
    let user_data = tempdir();
    let custom_config = user_data.path().join("portable/custom.cfg");
    fs::create_dir_all(custom_config.parent().test_value()).test_value();
    let original = b"[General]\nLanguage=US\nLanguageEx=US\nParticipants=Configured.c4p\n\n[Network]\nPortRefServer=23456\n";
    fs::write(&custom_config, original).test_value();
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
    let paths = AppPaths::discover_with_config_file(classic.config_file.as_deref()).test_value();

    runtime_assert_eq!(
        paths.config_file() => custom_config;
        paths.language_override() => Some("DE,US");
        scenario_title_language(Some(&paths)) => "US";
        classic_direct_reference_endpoint("127.0.0.1", Some(&paths)) .expect("custom reference port") => clonk_network::ReferenceEndpoint::Address(SocketAddr::from(([127, 0, 0, 1], 23_456,)));
        classic_loader_language_sequence(&paths).expect("command-line language sequence") => vec!["DE", "US"];
        fs::read(paths.config_file()).expect("read unchanged custom config") => original;
    );
}

#[test]
fn offline_seed_resolution_matches_cpp_time_pin_and_parameters() {
    let first_second = 1_700_000_000_u64;
    let next_second = first_second + 1;

    runtime_assert_ne!(
        resolve_offline_round_random_seed(None, first_second, None) => resolve_offline_round_random_seed(None, next_second, None),
        "different C++ time(nullptr) seconds produce different fresh rounds",
    );
    runtime_assert_eq!(resolve_offline_round_random_seed(None, first_second, Some("")) => first_second, "an empty LC_PIN_SEED is ignored");
    runtime_assert_eq!(resolve_offline_round_random_seed(None, first_second, Some("0")) => 0);
    runtime_assert_eq!(resolve_offline_round_random_seed(None, first_second, Some(" \t-7tail")) => u64::from((-7_i32) as u32), "atoi accepts whitespace, sign, and a decimal prefix");
    runtime_assert_eq!(resolve_offline_round_random_seed(None, first_second, Some("not-a-number")) => 0, "a nonempty malformed atoi input pins zero instead of falling back to time");
    runtime_assert_eq!(
        resolve_offline_round_random_seed(None, first_second, Some("73")) => resolve_offline_round_random_seed(None, next_second, Some("73")),
        "a pin reproduces the same round across different start times",
    );
    runtime_assert_eq!(resolve_offline_round_random_seed(Some(44), first_second, Some("73")) => 44, "compiled Parameters.txt wins and bypasses LC_PIN_SEED");
}

#[test]
fn pinned_offline_seed_reaches_dynamic_map_and_engine() {
    let _pin_guard = EnvGuard::set(&[
        ("LC_PIN_SEED", Some(Path::new("7"))),
        ("LC_RUST_ENGINE_RANDOM_SEED", None),
        ("LC_RUST_ENGINE_MAP_SEED", None),
    ]);
    let user_data = tempdir();
    let (_paths_guard, _paths, mut app, scenario) = runtime_scenario_app(
        user_data.path(),
        "Seed parity",
        "Tutorial.c4f/Tutorial07.c4s",
    );

    app.start_scenario(scenario).test_value();
    runtime_assert_eq!(app.loading_state.as_ref().and_then(|loading| loading.offline_random_seed) => Some(7), "the main thread freezes LC_PIN_SEED before spawning the loader");
    // This loads the shipped definition tree and dynamic landscape. Give
    // the loader thread room to run alongside the parallel full suite.
    wait_for_running_with_attempts(&mut app, 2_400);

    assert_eq!(app.engine.random_seed(), 7);
    runtime_assert_eq!(app.engine.landscape().expect("Tutorial07 dynamic landscape").map_seed() => 42_711, "the dynamic map consumes seed 7 before activation (seed 0 would yield 59,893)");
}

/// Drachenfels ships `SavePlayerInfos.txt` without `Head.SaveGame`, so
/// `C4GameParameters::Load` fills `RestorePlayerInfos` anyway
/// (C4GameParameters.cpp:378-385) and `InitPlayers` recreates its
/// `GameNumber=10` script player "for savegames or regular scenarios with
/// restore infos" (C4Game.cpp:2841-2843). 27 of its `Objects.txt` rows
/// carry `Owner=10`.
#[test]
fn dragon_rock_restores_its_shipped_script_player() {
    let user_data = tempdir();
    let (_paths_guard, _paths, mut app, scenario) = runtime_scenario_app(
        user_data.path(),
        "Restore parity",
        "Fantasy.c4f/Drachenfels.c4s",
    );

    app.start_scenario(scenario).test_value();
    wait_for_running_with_attempts(&mut app, 4_800);

    let script_player = app.engine.test_player(10);
    assert!(script_player.is_script_player());
    assert!(script_player.no_elimination_check());
    assert_eq!(script_player.team(), Some(2));
}

/// Arso-Morf proves the ordering as well as the gating: its `Initialize()`
/// runs `CreateObject(CLNK, ..., GetPlayerByName("I.S.I"))`, so the
/// scenario constructor must run at all (C4Game.cpp:2747 skips it only for
/// `Head.SaveGame`) *and* must run after `InitPlayers` has joined the
/// restore row (C4Game.cpp:479 before :484).
#[test]
fn arso_morf_runs_its_constructor_after_restoring_its_script_player() {
    let user_data = tempdir();
    let (_paths_guard, _paths, mut app, scenario) = runtime_scenario_app(
        user_data.path(),
        "Restore parity",
        "EkeReloaded.c4f/TheStippelAge.c4f/Arso-Morf.c4s",
    );

    app.start_scenario(scenario).test_value();
    wait_for_running_with_attempts(&mut app, 4_800);

    let scientist = app
        .engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.custom_name.as_deref() == Some("Mad Scientist"))
        .test_value();
    let owner = app.engine.test_player(scientist.owner);
    runtime_assert!(
        owner.is_script_player(),
        "the constructor ran after the restore row joined"
    );
}

#[test]
fn fresh_offline_skyparcour_retries_and_activates_the_accepted_seed() {
    let _pin_guard = EnvGuard::set(&[
        ("LC_PIN_SEED", Some(Path::new("1784903470"))),
        ("LC_RUST_ENGINE_RANDOM_SEED", None),
        ("LC_RUST_ENGINE_MAP_SEED", None),
    ]);
    let user_data = tempdir();
    let (_paths_guard, _paths, mut app, scenario) = runtime_scenario_app(
        user_data.path(),
        "SkyParcour seed retry",
        "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/HarpoonRace.c4s",
    );

    app.start_scenario(scenario).test_value();
    runtime_assert_eq!(app.loading_state.as_ref().and_then(|loading| loading.offline_random_seed) => Some(1_784_903_470), "the candidate seed is frozen before asynchronous validation");
    wait_for_running_with_attempts(&mut app, 4_800);

    runtime_assert_eq!(app.engine.random_seed() => 1_784_903_471, "activation, saves, and recordings must use the accepted seed");
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
    let mut crew = test_definition("CREW", "Crew", script);
    crew.set_crew_member(true);
    engine.register_test_definition(crew);
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
        .test_value();
    let crew_id = engine.test_player(0).crew()[0];
    let crew_index = engine.find_object_index(crew_id).test_value();
    match engine
        .call_object_function(crew_index, "Award", Vec::new())
        .test_value()
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
        app.test_update();
    }

    app.sec1_timer().test_value();
    assert_eq!(app.frames_per_second, 3);
    assert_eq!(app.frames_since_second, 0);

    app.test_update();
    app.sec1_timer().test_value();
    assert_eq!(app.frames_per_second, 1);
    assert_eq!(app.frames_since_second, 0);
}

#[test]
fn new_game_and_teardown_reset_transient_speed_state() {
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
    runtime_assert_eq!(parse_presentation_benchmark_window("5") => Some(Duration::from_secs(5)));
    for rejected in ["", "0", "-1", "1.5", "five"] {
        assert_eq!(parse_presentation_benchmark_window(rejected), None);
    }
}

#[test]
fn presentation_benchmark_player_team_parser_is_ordered_and_fail_closed() {
    runtime_assert_eq!(parse_presentation_benchmark_player_teams("1,2") => Ok(vec![1, 2]));
    for rejected in ["", "0", "-1", "1,", ",1", "1, 2", "one,2"] {
        runtime_assert!(
            parse_presentation_benchmark_player_teams(rejected).is_err(),
            "`{rejected}` must not silently change the benchmark workload"
        );
    }
}

#[test]
fn presentation_benchmark_team_controls_preserve_player_order_and_scope() {
    runtime_assert_eq!(
        presentation_benchmark_team_selection_controls(true, false, &[7, 3], Some("1,2")) =>
            Ok(vec![clonk_engine::InitScenarioPlayerControlData { team: 1, player: 7, by_client: 0, }, clonk_engine::InitScenarioPlayerControlData { team: 2, player: 3, by_client: 0, },]);
        presentation_benchmark_team_selection_controls(false, false, &[7, 3], Some("invalid")) => Ok(Vec::new());
        presentation_benchmark_team_selection_controls(true, true, &[7, 3], Some("invalid")) => Ok(Vec::new());
    );
    runtime_assert!(
        presentation_benchmark_team_selection_controls(true, false, &[7], Some("1,2")).is_err()
    );
}

fn join_presentation_benchmark_teamless_player(
    app: &mut GameApp,
    name: &str,
    startup_player_count: i32,
) -> i32 {
    app.engine
        .join_player(clonk_engine::JoinPlayerConfig {
            name: name.to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count,
        })
        .test_value()
        .number()
}

#[test]
fn presentation_benchmark_team_controls_resume_existing_pending_players() {
    let mut app = new_running_sandbox_app();
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Red", 0x00f4_0000),
        clonk_engine::TeamInfo::new(2, "Blue", 0x0000_c800),
    ]);
    app.engine.set_runtime_join_team_choice(true);
    let players = [
        join_presentation_benchmark_teamless_player(&mut app, "Profiler A", 2),
        join_presentation_benchmark_teamless_player(&mut app, "Profiler B", 2),
    ];
    let controls =
        presentation_benchmark_team_selection_controls(true, false, &players, Some("1,2"))
            .test_value();

    app.execute_presentation_benchmark_team_selection_controls(&controls)
        .test_value();

    runtime_assert_eq!(
        app.engine.player(players[0]).and_then(|player| player.team()) => Some(1);
        app.engine .player(players[1]) .and_then(|player| player.team()) => Some(2);
    );
    for player in players {
        runtime_assert!(!matches!(
            app.engine.player(player).map(|player| player.status()),
            Some(PlayerStatus::TeamSelection | PlayerStatus::TeamSelectionPending)
        ));
    }
    assert_eq!(app.engine.players().count(), 3);
}

#[test]
fn presentation_benchmark_team_controls_reject_an_unavailable_team() {
    let mut app = new_running_sandbox_app();
    app.engine
        .set_teams(vec![clonk_engine::TeamInfo::new(1, "Only", 0x00f4_0000)]);
    app.engine.set_runtime_join_team_choice(true);
    let player = join_presentation_benchmark_teamless_player(&mut app, "Profiler", 1);
    let controls = [clonk_engine::InitScenarioPlayerControlData {
        team: 9,
        player,
        by_client: 0,
    }];

    let error = app
        .execute_presentation_benchmark_team_selection_controls(&controls)
        .expect_err("an unavailable benchmark team must abort activation");

    assert!(error.contains("team 9"), "unexpected error: {error}");
    runtime_assert_eq!(app.engine.player(player).map(|player| player.status()) => Some(PlayerStatus::TeamSelection));
}

#[test]
fn graphics_pass_percentiles_use_nearest_rank() {
    let samples = (1..=20)
        .rev()
        .map(Duration::from_millis)
        .collect::<Vec<_>>();

    runtime_assert_eq!(
        graphics_pass_percentiles(&samples) => (Duration::from_millis(10), Duration::from_millis(19), Duration::from_millis(20),);
        graphics_pass_percentiles(&[]) => (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    );
}

#[test]
fn presentation_benchmark_context_reports_actual_network_players() {
    assert_eq!(
                presentation_benchmark_context_line(24, 24, 24, 24, 24, 24, 1_000, 1_000),
                "LC_APP_PRESENTATION_BENCHMARK_CONTEXT runtime_players=24 synchronized_player_infos=24 activated_nonhost_clients=24 runtime_crew_objects=24 runtime_players_with_live_crew=24 runtime_players_with_exactly_one_live_sf5b_crew=24 runtime_st5b_objects_at_measurement_start=1000 runtime_st5b_objects_at_measurement_end=1000"
            );
}

#[test]
fn presentation_benchmark_network_evidence_uses_unique_preferred_message_routes() {
    let connections = vec![
        runtime_fixture!(
            network_connection:
                1,
                0,
                "Data/Msg".to_string(),
                clonk_network::NetworkProtocol::Tcp,
                0,
                7,
                9,
        ),
        runtime_fixture!(
            network_connection:
                2,
                2,
                "Msg".to_string(),
                clonk_network::NetworkProtocol::Udp,
                3,
                -1,
                12,
        ),
        runtime_fixture!(
            network_connection:
                3,
                2,
                "Data".to_string(),
                clonk_network::NetworkProtocol::Tcp,
                99,
                100,
                101,
        ),
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
    assert_eq!(evidence.host_message_route_lag_ms, Some(9));
    assert_eq!(evidence.max_packet_loss, 3);
    assert_eq!(evidence.control_presend, 4);
    assert_eq!(evidence.avg_control_send_time_us, 26_813);
    assert_eq!(
                evidence.machine_line(),
                "LC_APP_PRESENTATION_BENCHMARK_NETWORK inspection_status=ok local_client_id=1 preferred_message_route_peer_count=2 preferred_message_route_peer_ids=[0,2] tcp_preferred_message_routes=1 udp_preferred_message_routes=1 unknown_preferred_message_routes=0 nonnegative_ping_peer_count=1 nonnegative_lag_peer_count=2 max_nonnegative_ping_ms=7 max_nonnegative_lag_ms=12 host_message_route_lag_ms=9 max_packet_loss=3 control_presend=4 avg_control_send_time_us=26813"
            );
}

#[test]
fn presentation_benchmark_counts_live_player_crew_objects() {
    // C4Player::MakeCrewMember only retains owned, active CrewMember
    // objects in the player's Crew (src/C4Player.cpp:1173-1203).
    let mut app = new_lightweight_running_sandbox_app();
    let crew = app.snapshot.players[0].crew[0];
    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == crew)
        .test_value()
        .alive = true;
    assert_eq!(runtime_crew_object_count(&app.snapshot), 1);
    assert_eq!(runtime_players_with_live_crew(&app.snapshot), 1);

    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == crew)
        .test_value()
        .owner += 1;
    assert_eq!(runtime_crew_object_count(&app.snapshot), 1);
    assert_eq!(runtime_players_with_live_crew(&app.snapshot), 0);

    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == crew)
        .test_value()
        .alive = false;
    assert_eq!(runtime_crew_object_count(&app.snapshot), 0);
}

#[test]
fn presentation_benchmark_counts_only_active_stippels() {
    let mut app = new_lightweight_running_sandbox_app();
    let object = app.snapshot.objects.first_mut().test_value();
    object.definition_id = "ST5B".to_string();

    assert_eq!(runtime_stippel_object_count(&app.snapshot), 1);

    app.snapshot.objects[0].status = ObjectStatus::Inactive;
    assert_eq!(runtime_stippel_object_count(&app.snapshot), 0);
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
        .test_value();
    first_crew_object.definition_id = "SF5B".to_string();
    first_crew_object.alive = true;

    let mut second_crew = app.snapshot.object(first_crew).test_value().clone();
    second_crew.id = ObjectId::new(first_crew.as_u64() + 1);
    app.snapshot.objects.push(second_crew);
    app.snapshot.players[0]
        .crew
        .push(ObjectId::new(first_crew.as_u64() + 1));

    let mut second_player = app.snapshot.players[0].clone();
    second_player.id += 1;
    second_player.crew.clear();
    app.snapshot.players.push(second_player);

    runtime_assert_eq!(
        runtime_crew_object_count(&app.snapshot) => 2;
        runtime_players_with_exactly_one_live_sf5b_crew(&app.snapshot) => 0;
    );

    let second_crew = app.snapshot.players[0].crew.pop().test_value();
    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == second_crew)
        .test_value()
        .owner = app.snapshot.players[1].id;
    app.snapshot.players[1].crew.push(second_crew);
    runtime_assert_eq!(runtime_players_with_exactly_one_live_sf5b_crew(&app.snapshot) => 2);

    app.snapshot
        .objects
        .iter_mut()
        .find(|object| object.id == second_crew)
        .test_value()
        .alive = false;
    runtime_assert_eq!(runtime_players_with_exactly_one_live_sf5b_crew(&app.snapshot) => 1);
}

#[test]
fn presentation_benchmark_keep_running_requires_explicit_one() {
    assert!(parse_presentation_benchmark_keep_running(Some("1")));
    for value in [None, Some(""), Some("0"), Some("true")] {
        assert!(!parse_presentation_benchmark_keep_running(value));
    }
}

#[test]
fn presentation_benchmark_readiness_waits_for_one_executed_simulation_frame() {
    // Native queues the initial joins at GO and executes that control batch
    // before advancing frame one (C4Network2Players.cpp:465-482;
    // C4Game.cpp:796-801).
    let mut readiness = PresentationBenchmarkRuntimeReadiness::default();

    assert!(!readiness.ready(AppMode::Running));
    readiness.observe(AppMode::Running, 0);
    assert!(!readiness.ready(AppMode::Running));
    readiness.observe(AppMode::Running, 1);
    assert!(readiness.ready(AppMode::Running));
    readiness.observe(AppMode::Running, 0);
    assert!(readiness.ready(AppMode::Running));

    assert!(!readiness.ready(AppMode::Loading));
    assert!(!readiness.ready(AppMode::Running));

    let base = Instant::now();
    let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));
    runtime_assert_eq!(
        benchmark.poll(readiness.ready(AppMode::Running), base, 0) => None;
        benchmark.poll(readiness.ready(AppMode::Running), base + Duration::from_secs(10), 0,) => None;
    );

    readiness.observe(AppMode::Running, 1);
    let ready_at = base + Duration::from_secs(10);
    runtime_assert_eq!(
        benchmark.poll(readiness.ready(AppMode::Running), ready_at, 1) => None;
        benchmark.poll(readiness.ready(AppMode::Running), ready_at + PRESENTATION_BENCHMARK_WARMUP, 1,) => None;
    );
    runtime_assert!(
        benchmark
            .poll(
                readiness.ready(AppMode::Running),
                ready_at + PRESENTATION_BENCHMARK_WARMUP + Duration::from_secs(3),
                116,
            )
            .is_some(),
        "the window starts only after one frame and the complete warmup",
    );
}

#[test]
fn retained_gpu_cpu_profile_reconciles_named_stages_with_the_outer_graphics_pass() {
    let raw = runtime_fixture!(
        gpu_profile_renderer_surface_capture:
            gpu_renderer::GpuRendererStats {
                        cpu_stages: gpu_renderer::GpuRendererCpuStages {
                            validation: Duration::from_nanos(3),
                            texture_synchronization: Duration::from_nanos(5),
                            stream_packing_upload: Duration::from_nanos(7),
                            command_encoding: Duration::from_nanos(11),
                        },
                        ..gpu_renderer::GpuRendererStats::default()
                    },
            clonk_surface::WindowSurfaceCpuStages {
                        drawable_acquisition: Duration::from_nanos(13),
                        command_encoder_finalization: Duration::from_nanos(17),
                        queue_submission: Duration::from_nanos(19),
                        presentation: Duration::from_nanos(23),
                    },
            clonk_graphics::GpuSceneCaptureStats::default(),
    );

    let residual = raw.reconcile(Duration::from_nanos(84));
    assert_eq!(residual.named_cpu, Duration::from_nanos(100));
    assert_eq!(residual.unclassified_cpu, Duration::ZERO);
    assert_eq!(residual.overrun_cpu, Duration::from_nanos(16));
    assert!(residual.has_exact_reconciliation());

    let residual = raw.reconcile(Duration::from_nanos(106));
    assert_eq!(residual.named_cpu, Duration::from_nanos(100));
    assert_eq!(residual.unclassified_cpu, Duration::from_nanos(6));
    assert_eq!(residual.overrun_cpu, Duration::ZERO);
    assert!(residual.has_exact_reconciliation());
}

#[test]
fn retained_gpu_artifact_frame_preserves_raw_structural_and_cpu_samples() {
    let raw = runtime_fixture!(
        gpu_profile_renderer_surface_capture:
            gpu_renderer::GpuRendererStats {
                        cpu_stages: gpu_renderer::GpuRendererCpuStages {
                            validation: Duration::from_nanos(3),
                            texture_synchronization: Duration::from_nanos(5),
                            stream_packing_upload: Duration::from_nanos(7),
                            command_encoding: Duration::from_nanos(11),
                        },
                        object_sprite_instances: 13,
                        object_sprite_upload_bytes: 17,
                        landscape_instances: 19,
                        landscape_instance_upload_bytes: 1_368,
                        ..gpu_renderer::GpuRendererStats::default()
                    },
            clonk_surface::WindowSurfaceCpuStages {
                        drawable_acquisition: Duration::from_nanos(19),
                        command_encoder_finalization: Duration::from_nanos(23),
                        queue_submission: Duration::from_nanos(29),
                        presentation: Duration::from_nanos(31),
                    },
            clonk_graphics::GpuSceneCaptureStats {
                        owner_mask_fallbacks: 1,
                        ..clonk_graphics::GpuSceneCaptureStats::default()
                    },
    );
    let reconciled = raw.reconcile(Duration::from_nanos(137));

    let frame = RetainedGpuProfileFrame::from_reconciled(4, reconciled).unwrap();

    assert_eq!(frame.sample_index, 4);
    assert_eq!(frame.end_to_end_ns, 137);
    assert_eq!(frame.cpu.command_encoding_ns, 34);
    assert_eq!(frame.cpu.named_total_ns, 130);
    assert_eq!(frame.cpu.unclassified_ns, 7);
    assert_eq!(frame.renderer.object_sprite_instances, 13);
    assert_eq!(frame.renderer.object_sprite_upload_bytes, 17);
    assert_eq!(frame.renderer.landscape_instances, 19);
    assert_eq!(frame.renderer.landscape_instance_upload_bytes, 1_368);
    assert_eq!(frame.frontend_capture.owner_mask_fallbacks, 1);
}

#[test]
fn retained_gpu_profile_rejects_a_surface_context_change_during_measurement() {
    let original = RetainedGpuFrameContext::default();
    let first = RetainedGpuFrameProfile {
        context: original,
        ..RetainedGpuFrameProfile::default()
    }
    .reconcile(Duration::ZERO);
    let resized = RetainedGpuFrameContext {
        surface_extent: [801, 600],
        ..original
    };
    let second = RetainedGpuFrameProfile {
        context: resized,
        ..RetainedGpuFrameProfile::default()
    }
    .reconcile(Duration::ZERO);

    runtime_assert!(
        retained_gpu_profile_context_is_stable(&[first], original);
        !retained_gpu_profile_context_is_stable(&[first, second], resized);
        !retained_gpu_profile_context_is_stable(&[first], resized);
    );
}

#[test]
fn retained_gpu_artifact_preserves_timestamp_rollover_as_raw_invalid_evidence() {
    let record = RetainedGpuTimestampPassRecord::from(gpu_renderer::GpuTimestampSample {
        pass: gpu_renderer::GpuTimestampPass::Scene,
        begin_tick: u64::MAX,
        end_tick: 4,
        duration_ns: None,
        validity: gpu_renderer::GpuTimestampSampleValidity::CounterRollover,
    });

    assert_eq!(record.pass, "scene");
    assert_eq!(record.begin_tick, u64::MAX);
    assert_eq!(record.end_tick, 4);
    assert_eq!(record.duration_ns, None);
    assert_eq!(record.validity, "counter_rollover");
}

#[test]
fn retained_gpu_artifact_fingerprints_the_device_adapter_fields() {
    let record = RetainedGpuAdapterRecord::from(wgpu::AdapterInfo {
        name: "Adapter".to_owned(),
        vendor: 0x1234,
        device: 0x5678,
        device_type: wgpu::DeviceType::DiscreteGpu,
        device_pci_bus_id: "0000:01:00.0".to_owned(),
        driver: "Driver".to_owned(),
        driver_info: "1.2.3".to_owned(),
        backend: wgpu::Backend::Vulkan,
        subgroup_min_size: 16,
        subgroup_max_size: 64,
        transient_saves_memory: Some(true),
        limit_bucket: None,
    });

    assert_eq!(record.name, "Adapter");
    assert_eq!(record.vendor_id, 0x1234);
    assert_eq!(record.device_id, 0x5678);
    assert_eq!(record.device_type, "discrete_gpu");
    assert_eq!(record.pci_bus_id.as_deref(), Some("0000:01:00.0"));
    assert_eq!(record.driver, "Driver");
    assert_eq!(record.driver_info, "1.2.3");
    assert_eq!(record.backend, "vulkan");
    assert_eq!(record.subgroup_min_size, 16);
    assert_eq!(record.subgroup_max_size, 64);
    assert_eq!(record.transient_saves_memory, Some(true));
}

#[test]
fn presentation_benchmark_warms_up_counts_successes_and_reports_one_window() {
    let base = Instant::now();
    let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));

    assert_eq!(benchmark.poll(false, base, 10), None);
    benchmark.record_successful_presentation(
        base,
        Duration::from_millis(100),
        true,
        PresentationPath::RetainedGpu,
    );
    benchmark.record_automatic_graphics_skip();
    runtime_assert_eq!(
        benchmark.poll(true, base, 10) => None;
        benchmark.poll(true, base + PRESENTATION_BENCHMARK_WARMUP - Duration::from_millis(1), 69,) => None;
        benchmark.poll(true, base + PRESENTATION_BENCHMARK_WARMUP, 70) => None;
    );
    benchmark.record_automatic_graphics_skip();
    benchmark.record_successful_presentation(
        base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_millis(10),
        Duration::from_millis(10),
        true,
        PresentationPath::RetainedGpu,
    );
    benchmark.record_successful_presentation(
        base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_millis(20),
        Duration::from_millis(20),
        false,
        PresentationPath::Cpu,
    );
    runtime_assert_eq!(benchmark.poll(true, base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_millis(2_999), 174,) => None);

    let report = benchmark
        .poll(
            true,
            base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_secs(3),
            175,
        )
        .test_value();
    runtime_assert_eq!(
        report.elapsed => Duration::from_secs(3);
        report.submissions => 2;
        report.retained_gpu_submissions => 1;
        report.cpu_submissions => 1;
        report.refreshed_frames => 1;
        report.simulation_frames => 105;
        report.automatic_graphics_skips => 1;
        report.graphics_average => Duration::from_millis(15);
        report.graphics_max => Duration::from_millis(20);
        report.graphics_p50 => Duration::from_millis(10);
        report.graphics_p95 => Duration::from_millis(20);
        report.graphics_p99 => Duration::from_millis(20);
        report.graphics_samples => vec![Duration::from_millis(10), Duration::from_millis(20)];
        report.machine_line() =>
            "LC_APP_PRESENTATION_BENCHMARK elapsed_seconds=3.000000 successful_present_submissions=2 retained_gpu_present_submissions=1 cpu_present_submissions=1 presentation_submission_fps=0.666667 refreshed_frames=1 simulation_frames=105 simulation_fps=35.000000 automatic_graphics_skips=1 average_graphics_pass_ms=15.000000 max_graphics_pass_ms=20.000000 graphics_pass_sample_count=2 graphics_pass_p50_ms=10.000000 graphics_pass_p95_ms=20.000000 graphics_pass_p99_ms=20.000000 graphics_pass_samples_ns=[10000000,20000000]";
        benchmark.poll(true, base + Duration::from_secs(10), 999) => None;
    );
}

#[test]
fn presentation_benchmark_captures_stippels_at_measurement_boundaries() {
    let base = Instant::now();
    let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));

    runtime_assert_eq!(
        benchmark.poll(true, base, 10) => None;
        benchmark.poll_with_runtime_stippel_census(true, base + PRESENTATION_BENCHMARK_WARMUP, 70, || 1_000,) => None;
    );
    let report = benchmark
        .poll_with_runtime_stippel_census(
            true,
            base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_secs(3),
            175,
            || 998,
        )
        .test_value();

    assert_eq!(report.runtime_stippels_at_start, 1_000);
    assert_eq!(report.runtime_stippels_at_end, 998);
}

#[test]
fn runtime_benchmark_window_does_not_require_a_visible_surface() {
    let base = Instant::now();
    let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));

    runtime_assert_eq!(
        benchmark.poll(true, base, 10) => None;
        benchmark.poll(true, base + PRESENTATION_BENCHMARK_WARMUP, 70) => None;
    );
    let deadline = base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_secs(3);
    benchmark.record_successful_presentation(
        deadline,
        Duration::from_millis(10),
        true,
        PresentationPath::RetainedGpu,
    );
    let report = benchmark.poll(true, deadline, 70).test_value();

    assert_eq!(report.simulation_frames, 0);
    assert_eq!(report.submissions, 0);
    assert!(report.graphics_samples.is_empty());
}

#[test]
fn presentation_benchmark_retains_raw_gpu_profiles_only_inside_its_half_open_window() {
    let base = Instant::now();
    let started = base + PRESENTATION_BENCHMARK_WARMUP;
    let deadline = started + Duration::from_secs(3);
    let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));
    let profile = RetainedGpuFrameProfile {
        frame_preparation: Duration::from_nanos(5),
        ..RetainedGpuFrameProfile::default()
    };

    assert_eq!(benchmark.poll(true, base, 10), None);
    assert_eq!(benchmark.poll(true, started, 70), None);
    benchmark.record_successful_retained_gpu_presentation(
        started,
        Duration::from_nanos(11),
        true,
        profile,
    );
    benchmark.record_successful_retained_gpu_presentation(
        deadline,
        Duration::from_nanos(13),
        true,
        profile,
    );
    let report = benchmark.poll(true, deadline, 71).test_value();

    assert_eq!(report.submissions, 1);
    assert_eq!(report.retained_gpu_profiles.len(), 1);
    let retained = report.retained_gpu_profiles[0];
    assert_eq!(retained.raw, profile);
    assert_eq!(retained.graphics_duration, Duration::from_nanos(11));
    assert!(retained.has_exact_reconciliation());
}

#[test]
fn presentation_benchmark_consumes_timestamp_results_only_while_measuring() {
    let base = Instant::now();
    let started = base + PRESENTATION_BENCHMARK_WARMUP;
    let deadline = started + Duration::from_secs(3);
    let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));
    let frame = |frame_id| gpu_renderer::GpuTimestampFrame {
        frame_id,
        renderer_generation: 1,
        timestamp_period_ns: 1.0,
        passes: Vec::new(),
    };

    benchmark.record_gpu_timestamp_frames(vec![frame(1)]);
    assert_eq!(benchmark.poll(true, base, 10), None);
    assert_eq!(benchmark.poll(true, started, 70), None);
    benchmark.record_gpu_timestamp_frames(vec![frame(2)]);
    let report = benchmark.poll(true, deadline, 71).test_value();

    runtime_assert_eq!(report.gpu_timestamp_frames.iter().map(|frame| frame.frame_id).collect::<Vec<_>>() => vec![2]);
}

#[test]
fn runtime_benchmark_does_not_reselect_a_window_after_runtime_stops() {
    let base = Instant::now();
    let mut benchmark = PresentationBenchmark::new(Duration::from_secs(3));

    runtime_assert_eq!(
        benchmark.poll(true, base, 70) => None;
        benchmark.poll(true, base + PRESENTATION_BENCHMARK_WARMUP, 70) => None;
        benchmark.poll(false, base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_secs(1), 70,) => None;
    );
    let report = benchmark
        .poll(
            false,
            base + PRESENTATION_BENCHMARK_WARMUP + Duration::from_secs(3),
            70,
        )
        .test_value();

    assert_eq!(report.elapsed, Duration::from_secs(3));
    assert_eq!(report.simulation_frames, 0);
}

#[test]
fn input_latency_benchmark_starts_with_the_measurement_and_keeps_its_interval() {
    let base = Instant::now();
    let mut benchmark = InputLatencyBenchmark::new(Duration::from_millis(500));

    assert!(!benchmark.pair_due(base));
    benchmark.start(base);
    assert!(benchmark.pair_due(base));
    assert!(!benchmark.pair_due(base + Duration::from_millis(499)));
    assert!(benchmark.pair_due(base + Duration::from_millis(500)));
}

#[test]
fn input_latency_benchmark_resets_when_a_new_measurement_window_starts() {
    let base = Instant::now();
    let mut benchmark = InputLatencyBenchmark::new(Duration::from_millis(500));
    let control = runtime_fixture!(player_control_defaults);
    benchmark.start(base);
    assert!(benchmark.pair_due(base));
    benchmark.record_submission(7, &control, base);

    let restarted = base + Duration::from_secs(10);
    benchmark.start(restarted);
    assert!(benchmark.pair_due(restarted));
    let report = benchmark.report(Duration::from_secs(1));
    assert_eq!(report.submitted_inputs, 0);
    assert_eq!(report.executed_inputs, 0);
    assert_eq!(report.pending_inputs, 0);
}

#[test]
fn input_latency_benchmark_interval_requires_positive_milliseconds() {
    runtime_assert_eq!(parse_input_latency_benchmark_interval("500") => Some(Duration::from_millis(500)));
    for value in ["", "0", "-1", "no"] {
        assert_eq!(parse_input_latency_benchmark_interval(value), None);
    }
}

#[test]
fn input_latency_benchmark_matches_the_exact_local_control_fifo() {
    let base = Instant::now();
    let mut benchmark = InputLatencyBenchmark::new(Duration::from_millis(500));
    let press = runtime_fixture!(player_control_defaults);
    let release = clonk_engine::PlayerControlData {
        command: i32::from(clonk_engine::COM_LEFT + clonk_engine::COM_RELEASE_OFFSET),
        ..press
    };

    benchmark.record_submission(7, &press, base);
    benchmark.record_submission(7, &release, base);
    runtime_assert!(
        !benchmark.record_execution(7, &clonk_engine::PlayerControlData { by_client: 3,..press }, base + Duration::from_millis(20),);
        benchmark.record_execution(7, &press, base + Duration::from_millis(100));
        benchmark.record_execution(7, &release, base + Duration::from_millis(101));
    );

    let report = benchmark.report(Duration::from_secs(5));
    runtime_assert_eq!(
        report.submitted_inputs => 2;
        report.executed_inputs => 2;
        report.pending_inputs => 0;
        report.latency_samples => vec![Duration::from_millis(100), Duration::from_millis(101)];
        report.p50 => Duration::from_millis(100);
        report.p95 => Duration::from_millis(101);
        report.p99 => Duration::from_millis(101);
        report.max => Duration::from_millis(101);
        report.players.len() => 1;
        report.players[0].player => 2;
        report.players[0].submitted_inputs => 2;
        report.players[0].executed_inputs => 2;
        report.players[0].pending_inputs => 0;
        report.players[0].latency_samples => vec![Duration::from_millis(100), Duration::from_millis(101)];
        report.players[0].machine_line(report.elapsed) =>
            "LC_APP_PRESENTATION_BENCHMARK_INPUT_PLAYER player=2 elapsed_seconds=5.000000 submitted_inputs=2 executed_inputs=2 pending_inputs=0 input_latency_sample_count=2 input_latency_p50_ms=100.000000 input_latency_p95_ms=101.000000 input_latency_p99_ms=101.000000 input_latency_max_ms=101.000000 input_latency_samples_ns=[100000000,101000000]";
        report.machine_line() =>
            "LC_APP_PRESENTATION_BENCHMARK_INPUT elapsed_seconds=5.000000 submitted_inputs=2 executed_inputs=2 pending_inputs=0 input_latency_sample_count=2 input_latency_p50_ms=100.000000 input_latency_p95_ms=101.000000 input_latency_p99_ms=101.000000 input_latency_max_ms=101.000000 input_latency_samples_ns=[100000000,101000000]";
    );
}

#[test]
fn input_latency_benchmark_does_not_let_one_drop_poison_later_matches() {
    let base = Instant::now();
    let mut benchmark = InputLatencyBenchmark::new(Duration::from_millis(500));
    let press = runtime_fixture!(player_control_defaults);
    let release = clonk_engine::PlayerControlData {
        command: i32::from(clonk_engine::COM_LEFT + clonk_engine::COM_RELEASE_OFFSET),
        ..press
    };

    benchmark.record_submission(7, &press, base);
    benchmark.record_submission(7, &release, base);
    assert!(benchmark.record_execution(7, &release, base + Duration::from_millis(20)));
    benchmark.record_submission(9, &press, base + Duration::from_millis(500));
    benchmark.record_submission(9, &release, base + Duration::from_millis(500));
    assert!(benchmark.record_execution(9, &press, base + Duration::from_millis(520)));
    assert!(benchmark.record_execution(9, &release, base + Duration::from_millis(521)));

    let report = benchmark.report(Duration::from_secs(1));
    assert_eq!(report.submitted_inputs, 4);
    assert_eq!(report.executed_inputs, 3);
    assert_eq!(report.pending_inputs, 1);
}

#[test]
fn input_latency_benchmark_submits_two_unmatched_releases_for_each_local_player() {
    // C4Player::InCom drops a release whose press bit is clear before it
    // dispatches DirectCom (src/C4Player.cpp:1541-1548). Two distinct
    // unmatched releases exercise lockstep without changing game state.
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    let second_owner = owner + 1;
    let first_crew = app.snapshot.players[0].crew[0];
    let mut second_crew = app.snapshot.object(first_crew).test_value().clone();
    second_crew.id = ObjectId::new(first_crew.as_u64() + 1);
    second_crew.owner = second_owner;
    let second_crew_id = second_crew.id;
    app.snapshot.objects.push(second_crew);
    let mut second_player = app.snapshot.players[0].clone();
    second_player.id = second_owner;
    second_player.crew = vec![second_crew_id];
    app.snapshot.players.push(second_player);
    app.local_controls
        .initialize(test_local_control_init(second_owner, 1, false, false));
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.input_latency_benchmark = Some(InputLatencyBenchmark::new(Duration::from_millis(500)));
    let started = Instant::now();
    let tick = app.local_control_submission_tick();

    app.submit_due_input_latency_benchmark_pair(started, started);
    runtime_assert_eq!(
        commands.take_submitted_local() =>
            vec![(owner, ControlEvent::Release(ControlButton::Left), tick), (owner, ControlEvent::Release(ControlButton::Right), tick), (second_owner, ControlEvent::Release(ControlButton::Left), tick,), (second_owner, ControlEvent::Release(ControlButton::Right), tick,),];
    );
    app.submit_due_input_latency_benchmark_pair(started, started + Duration::from_millis(499));
    assert!(commands.take_submitted_local().is_empty());
    let report = app
        .input_latency_benchmark
        .as_ref()
        .map(|benchmark| benchmark.report(Duration::from_secs(1)))
        .test_value();
    runtime_assert_eq!(
        report.submitted_inputs => 4;
        report.pending_inputs => 4;
        report.players.iter().map(|player| (player.player, player.submitted_inputs)).collect::<Vec<_>>() => vec![(owner, 2), (second_owner, 2)];
    );
}

#[test]
fn input_latency_benchmark_requires_a_live_local_crew() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    app.snapshot
        .players
        .iter_mut()
        .find(|player| player.id == owner)
        .test_value()
        .crew
        .clear();
    let (network, _events, mut commands) = NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(network);
    app.input_latency_benchmark = Some(InputLatencyBenchmark::new(Duration::from_millis(500)));
    let started = Instant::now();

    app.submit_due_input_latency_benchmark_pair(started, started);

    assert!(commands.take_submitted_local().is_empty());
    let report = app
        .input_latency_benchmark
        .as_ref()
        .map(|benchmark| benchmark.report(Duration::from_secs(1)))
        .test_value();
    assert_eq!(report.submitted_inputs, 0);
}

#[test]
fn high_dpi_cursor_defaults_off_and_reads_the_native_boolean() {
    // Deliberate divergence, so it must stay opt-in: with the key absent
    // the cursor keeps C4GraphicsResource's sheet choice exactly
    // (src/C4GraphicsResource.cpp:468-491).
    assert!(!configured_high_dpi_cursor(b""));
    assert!(!configured_high_dpi_cursor(
        b"[Graphics]\nHighDpiCursor=0\n"
    ));
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
    runtime_assert!(
        configured_high_dpi_cursor(remastered);
        configured_sky_dither(remastered);
        configured_mipmaps(remastered);
        configured_smooth_landscape(remastered);
        !configured_sky_dither(b"[Graphics]\nRemaster=1\nSkyDither=0\n");
        configured_mipmaps(b"[Graphics]\nRemaster=0\nMipmaps=1\n");
    );

    // ShaderLandscape joins the same set. It deliberately has no advanced
    // settings row, so `Graphics.Remaster` keeps reaching it after a config
    // repair writes every editor row back out.
    runtime_assert!(
        !configured_shader_landscape(b"");
        configured_shader_landscape(remastered);
        !configured_shader_landscape(b"[Graphics]\nRemaster=1\nShaderLandscape=0\n");
        configured_shader_landscape(b"[Graphics]\nRemaster=0\nShaderLandscape=1\n");
    );
}

#[test]
fn landscape_detail_defaults_to_the_cpp_exact_level_and_clamps_hand_edits() {
    // Detail 1 is byte-identical to the CPU composer, so an absent key must
    // leave the composition C++-exact. Everything else is clamped HERE
    // rather than only in the editor: a hand-edited config reaches this
    // reader directly, and the composer rejects 0 outright.
    runtime_assert_eq!(
        configured_landscape_detail(b"") => 1;
        configured_landscape_detail(b"[Graphics]\nRemaster=1\n") => 1;
        configured_landscape_detail(b"[Graphics]\nLandscapeDetail=3\n") => 3;
    );
    for (written, clamped) in [("0", 1), ("-2", 1), ("9", 4), ("400", 4)] {
        let config = format!("[Graphics]\nLandscapeDetail={written}\n");
        runtime_assert_eq!(configured_landscape_detail(config.as_bytes()) => clamped, "LandscapeDetail={written} must clamp to {clamped}");
    }
    // The reader is the C++ strtol mirror, so hex and trailing junk parse
    // the way StdCompilerINIRead::ReadNum does.
    runtime_assert_eq!(
        configured_landscape_detail(b"[Graphics]\nLandscapeDetail=0x2\n") => 2;
        configured_landscape_detail(b"[Graphics]\nLandscapeDetail=2junk\n") => 2;
    );
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
    runtime_assert_eq!(
        configured_max_refresh_delay_ms(b"") => 30;
        configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=0\n") => 30;
        configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=-5\n") => 30;
        configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=30\n") => 30;
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
        runtime_assert_eq!(configured_max_refresh_delay_ms(config) => 30, "{:?} must resolve to the native default", String::from_utf8_lossy(config));
    }
    // A trailing suffix is not invalid: StdCompiler reads the numeric
    // prefix and ignores the rest, so `16ms` is the positive value 16.
    runtime_assert_eq!(configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=16ms\n") => 16);

    // A valid positive value is kept verbatim.
    runtime_assert_eq!(
        configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=50\n") => 50;
        configured_max_refresh_delay_ms(b"[Graphics]\nMaxRefreshDelay=16\n") => 16;
    );

    // The advanced-config editor materializes the same default rather than
    // inventing a faster one.
    let row = crate::advanced_config::sections(&Config::new())
        .into_iter()
        .flat_map(|section| section.rows)
        .find(|row| row.name == "MaxRefreshDelay")
        .test_value();
    assert_eq!(row.value.serialized(), "30");

    // The retained value still feeds the divisor: 30 leaves the 28 ms game
    // timer as one graphics opportunity, a smaller ceiling splits it.
    runtime_assert_eq!(
        frame_schedule_for_mode(AppMode::Running, 28, 1, 30).refresh_interval => Duration::from_millis(28);
        frame_schedule_for_mode(AppMode::Running, 28, 1, 16).refresh_interval => Duration::from_millis(14);
    );
}

#[test]
fn max_refresh_delay_uses_cpp_divisor_without_speeding_simulation() {
    let default = frame_schedule_for_mode(AppMode::Running, 28, 1, 16);
    assert_eq!(default.simulation_interval, Duration::from_millis(28));
    assert_eq!(default.refresh_interval, Duration::from_millis(14));

    let explicit_native_default = frame_schedule_for_mode(AppMode::Running, 28, 1, 30);
    runtime_assert_eq!(
        explicit_native_default.simulation_interval => Duration::from_millis(28);
        explicit_native_default.refresh_interval => Duration::from_millis(28);
    );

    let slow = frame_schedule_for_mode(AppMode::Running, 1_000, 1, 16);
    assert_eq!(slow.simulation_interval, Duration::from_millis(1_000));
    assert_eq!(slow.refresh_interval, Duration::from_millis(15));
}

#[test]
fn smooth_presentation_substitutes_the_display_period_for_the_native_ceiling() {
    use crate::effective_max_refresh_delay_ms;

    // Nothing configured stays exactly on the C++ default, whatever the
    // panel reports: the oracle's 30 ms ceiling is the parity default.
    assert_eq!(effective_max_refresh_delay_ms(b"", None), 30);
    assert_eq!(effective_max_refresh_delay_ms(b"", Some(8)), 30);

    // The remaster master switch supplies the default, exactly like every
    // other presentation-only divergence.
    runtime_assert_eq!(
        effective_max_refresh_delay_ms(b"[Graphics]\nRemaster=1\n", Some(8)) => 8;
        effective_max_refresh_delay_ms(b"[Graphics]\nSmoothPresentation=1\n", Some(8)) => 8;
    );

    // A key the player wrote explicitly wins in both directions.
    runtime_assert_eq!(
        effective_max_refresh_delay_ms(b"[Graphics]\nRemaster=1\nSmoothPresentation=0\n", Some(8)) => 30;
        effective_max_refresh_delay_ms(b"[Graphics]\nSmoothPresentation=1\nMaxRefreshDelay=30\n", Some(8)) => 30;
    );

    // An unknown or slower-than-native panel must never make presentation
    // slower than the oracle default.
    runtime_assert_eq!(
        effective_max_refresh_delay_ms(b"[Graphics]\nSmoothPresentation=1\n", None) => 16;
        effective_max_refresh_delay_ms(b"[Graphics]\nSmoothPresentation=1\n", Some(100)) => 30;
    );
}

#[test]
fn smooth_presentation_subdivides_only_the_startup_timer() {
    use crate::RefreshCeilings;

    // Measured on an M4 Max at the reporter's own Scale=300 fullscreen
    // settings: subdividing the *game* timer to 7 ms moved presentation
    // from 35.66 to 36.30 FPS while the average graphics pass grew from
    // 10.49 ms to 18.17 ms and automatic skips went from 2 to 98. In game
    // the pass cost and swapchain back-pressure bind long before the timer
    // does, so the game timer keeps the oracle ceiling and only the
    // startup timer — which measured 62.88 FPS with a 0.83 ms GPU pass and
    // a 96 %-idle event loop — is subdivided.
    let ceilings = RefreshCeilings {
        running_ms: 30,
        startup_ms: 8,
    };

    let menu = frame_schedule_for_mode(AppMode::Menu, 28, 1, ceilings);
    assert_eq!(menu.simulation_interval, Duration::from_millis(16));
    assert_eq!(menu.refresh_interval, Duration::from_millis(8));

    let running = frame_schedule_for_mode(AppMode::Running, 28, 1, ceilings);
    assert_eq!(running.simulation_interval, Duration::from_millis(28));
    runtime_assert_eq!(running.refresh_interval => Duration::from_millis(28), "the game timer keeps the oracle ceiling even while the menu is subdivided");

    // A bare ceiling still means "both", so every existing caller and the
    // explicit `Graphics.MaxRefreshDelay` path are unchanged.
    runtime_assert_eq!(frame_schedule_for_mode(AppMode::Running, 28, 1, 16).refresh_interval => Duration::from_millis(14));
}

#[test]
fn startup_refresh_honours_the_same_refresh_ceiling_as_the_game_timer() {
    // The startup screens are where the pointer is used most, and their
    // cursor is drawn into the frame, so its update rate is the refresh
    // rate. The 16 ms startup timer was previously its own hard floor, so
    // a configured ceiling below it was ignored and the menu stayed at
    // 62.5 Hz on any display. Apply the same divisor the game timer uses.
    for mode in [AppMode::Menu, AppMode::Loading] {
        // At or above the startup interval the divisor is the identity, so
        // the native default keeps the menu byte-for-byte as it was.
        for ceiling in [16, 30, 100] {
            let schedule = frame_schedule_for_mode(mode, 28, 1, ceiling);
            assert_eq!(schedule.simulation_interval, Duration::from_millis(16));
            assert_eq!(schedule.refresh_interval, Duration::from_millis(16));
            assert_eq!(schedule.running_revision, None);
        }

        // A lower ceiling subdivides the startup timer without touching the
        // 16 ms logic tick that ages menu animations.
        let smooth = frame_schedule_for_mode(mode, 28, 1, 8);
        assert_eq!(smooth.simulation_interval, Duration::from_millis(16));
        assert_eq!(smooth.refresh_interval, Duration::from_millis(8));
        assert_eq!(smooth.running_revision, None);
    }
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
    let install = tempdir();
    let user_data = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let scenario_path = install.path().join("Scenarios/TwoPlayers.c4s");
    let definition_path = scenario_path.join("Defs.c4d");
    fs::create_dir_all(&definition_path).test_value();
    fs::write(scenario_path.join("Scenario.txt"), "[Head]\nTitle=Two players\nMaxPlayer=3\n\n[Definitions]\nDefinition1=Scenarios/TwoPlayers.c4s/Defs.c4d\n").test_value();
    fs::write(
        definition_path.join("DefCore.txt"),
        "[DefCore]\nid=TEST\nName=Test\nCategory=1\n",
    )
    .test_value();
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
                    ).test_value();
        fs::write(&path, group.pack().test_value()).test_value();
        path
    };
    write_player("Alice.c4p", "Alice", 0, false);
    write_player("Bob.c4p", "Bob", 1, true);
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=US\nParticipants=\"Alice.c4p;Bob.c4p\"\n",
    )
    .test_value();

    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    wait_for_menu(&mut app);
    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=US\nParticipants=\"Alice.c4p;Bob.c4p;Alice.c4p\"\n",
    )
    .test_value();
    let scenario = app
        .scenario_catalog
        .get("TwoPlayers.c4s")
        .cloned()
        .test_value();
    app.start_scenario(scenario).test_value();
    wait_for_running(&mut app);

    runtime_assert_eq!(
        app.snapshot.players.iter().map(|player| (player.id, player.player_info_id, player.name.as_str())).collect::<Vec<_>>() => vec![(0, 1, "Alice"), (1, 2, "Bob")];
    );
    assert_eq!(app.snapshot.frame, 0, "joins precede the first game tick");
    assert_eq!(app.snapshot.hud.local_players, vec![0, 1]);
    assert_eq!(app.control_player_infos.player_count(), 3);
    for (info_id, filename) in [
        (1, b"Alice.c4p".as_slice()),
        (2, b"Bob.c4p".as_slice()),
        (3, b"Alice.c4p".as_slice()),
    ] {
        let info = app.control_player_infos.get(info_id).test_value();
        runtime_assert_eq!(
            info.filename.as_bytes() => filename;
            info.flags & clonk_engine::PLAYER_INFO_FLAG_JOINED != 0 => info_id != 3;
        );
    }

    let bob_down = app
        .bindings
        .key_for_set(1, ControlBindingId::Down)
        .test_value();
    app.test_key(bob_down, ElementState::Pressed);
    let control = |app: &GameApp, owner| {
        app.engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner)
            .test_value()
            .control
    };
    runtime_assert_eq!(control(&app, 0).pressed_coms & (1 << clonk_engine::COM_DOWN) => 0);
    runtime_assert_ne!(control(&app, 1).pressed_coms & (1 << clonk_engine::COM_DOWN) => 0);
    app.test_key(bob_down, ElementState::Released);
    runtime_assert_eq!(control(&app, 1).pressed_coms & (1 << clonk_engine::COM_DOWN) => 0);

    let alice_left = app
        .bindings
        .key_for_set(0, ControlBindingId::Left)
        .test_value();
    app.test_key(alice_left, ElementState::Pressed);
    app.test_key(bob_down, ElementState::Pressed);
    assert_ne!(control(&app, 0).pressed_coms, 0);
    assert_ne!(control(&app, 1).pressed_coms, 0);
    app.handle_focus_lost().test_value();
    // No native backend clears player controls on focus loss
    // (C4FullScreen.cpp:139-145,310-315,432-447).
    assert_ne!(control(&app, 0).pressed_coms, 0);
    assert_ne!(control(&app, 1).pressed_coms, 0);

    app.return_to_menu();
    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=US\nParticipants=\"\"\n",
    )
    .test_value();
    let scenario = app
        .scenario_catalog
        .get("TwoPlayers.c4s")
        .cloned()
        .test_value();
    // This deliberately bypasses C4StartupScenSelDlg::DoOK/CanOpen and
    // exercises C4Game's independent late fullscreen guard. The actual
    // ScenarioBrowser route is covered by
    // local_scenario_start_with_no_participants_shows_cpp_error_before_loading.
    app.start_scenario(scenario).test_value();
    for _ in 0..480 {
        app.test_update();
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
        test_runtime_config_with("Fresh player".to_string(), false),
    )
    .test_value();
    install_classic_test_assets(&mut app);

    let mut definition = test_definition("JMPR", "Jumper", walker_script());
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
    app.engine.register_test_definition(definition);
    app.engine
        .set_player_starts(vec![clonk_engine::scenario::PlayerStart {
            ready_crew: vec![("JMPR".to_string(), 1)],
            ..Default::default()
        }]);
    app.join_local_player().test_value();
    app.mode = AppMode::Running;

    let cursor = app.engine.test_crew_cursor(app.local_owner);
    runtime_assert!(
        app.engine
            .player(app.local_owner)
            .expect("fresh player")
            .control_style(),
        "new players default to AutoStopControl like C++"
    );
    runtime_assert_eq!(app.bindings.key_for(ControlBindingId::Up) => Some(VirtualKeyCode::KeyS));

    app.test_key(VirtualKeyCode::ArrowUp, ElementState::Pressed);
    runtime_assert!(
        app.engine
            .object_snapshot(cursor)
            .expect("cursor after arrow press")
            .command_stack
            .command_names()
            .is_empty(),
        "the default Up arrow must not alias keyboard-set-1 Up"
    );

    app.test_key(VirtualKeyCode::KeyS, ElementState::Pressed);
    runtime_assert_eq!(
        app.engine.object_snapshot(cursor).expect("cursor after S press").command_stack.command_names() => vec!["Jump".to_string()],
        "S must traverse GameApp input and queue C4CMD_Jump",
    );
    runtime_assert_ne!(
        app.engine.snapshot().players.into_iter().find(|player| player.id == app.local_owner).expect("player after S press").control.pressed_coms & (1 << clonk_engine::COM_UP) =>
            0,
        "the Up press must be registered before release",
    );

    app.engine.test_tick();
    let jumping = app.engine.test_object_snapshot(cursor);
    assert_eq!(jumping.action.name, "Jump");
    runtime_assert!(
        jumping.velocity.y < 0,
        "ObjectComJump launches upward (C4ObjectCom.cpp:280-307)"
    );

    app.test_key(VirtualKeyCode::KeyS, ElementState::Released);
    runtime_assert_eq!(
        app.engine.snapshot().players.into_iter().find(|player| player.id == app.local_owner).expect("player after S release").control.pressed_coms & (1 << clonk_engine::COM_UP) =>
            0,
        "AutoStop key-up clears the registered Up press",
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
    let status = control_script_error_to_status(output_error).test_value();
    runtime_assert!(
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
    runtime_assert!(
        matches!(recoverable, ScenarioActivationError::Recoverable(ref message) if message.contains("Broken scenario"))
    );
}

#[cfg(unix)]
#[test]
fn writable_config_repairs_when_parent_forbids_staging() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir();
    let path = dir.path().join("clonk-rust.config");
    fs::write(&path, "[General]\nConfigResetSafety=7\n").test_value();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).test_value();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).test_value();

    let repair = validate_or_repair_startup_config(&path, false);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).test_value();

    assert!(repair.expect("repair writable config in place"));
    runtime_assert_eq!(Config::load(&path).expect("reload in-place repaired config").get_in(Some("General"), "ConfigResetSafety") => Some("42"));
}

#[test]
fn custom_corrupt_config_aborts_without_default_replacement() {
    let install = tempdir();
    let user_data = tempdir();
    let dir = tempdir();
    fs::create_dir_all(install.path().join("planet")).test_value();
    fs::write(install.path().join("planet/System.c4g"), b"stub").test_value();
    let path = dir.path().join("portable.config");
    let original = b"[General]\nConfigResetSafety=7\nName=Portable\n";
    fs::write(&path, original).test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("LC_CONFIG_FILE", None),
    ]);

    let error =
        discover_validated_startup_paths(Some(&path)).expect_err("custom corruption must abort");

    runtime_assert_eq!(
        error.to_string() => CUSTOM_CONFIG_CORRUPTED_ERROR;
        fs::read(&path).expect("read untouched custom config") => original;
    );
}

#[test]
fn standard_macos_entrypoint_recovers_planet_before_validated_path_discovery() {
    let directory = tempdir();
    let bundle = directory.path().join("Clonk Rust.app");
    let resources = bundle.join("Contents/Resources");
    let executable = bundle.join("Contents/MacOS/clonk-app");
    fs::create_dir_all(&resources).test_value();
    fs::create_dir_all(executable.parent().test_value()).test_value();
    fs::write(&executable, b"runtime").test_value();
    let nonce = "macos-missing-planet";
    let work = clonk_update::InstallLayout::macos_bundle(&bundle).work_dir();
    let backup = work.join(format!("clonk-update-backup-{nonce}/planet"));
    fs::create_dir_all(&backup).test_value();
    fs::write(backup.join("System.c4g"), b"stub").test_value();
    let mut step =
        clonk_update::JournalStep::new("planet", &"aa".repeat(32), "Contents/Resources/planet");
    step.state = clonk_update::StepState::BackupMoved;
    let mut journal = clonk_update::Journal::new(
        "0.7.0",
        nonce,
        fs::canonicalize(&bundle).test_value(),
        vec![step],
    );
    journal.previous_bundle_icon_present = Some(false);
    fs::create_dir_all(&work).test_value();
    journal.save(&work).test_value();
    let user_data = tempdir();
    let _guard = test_env_guard(resources.as_path(), user_data.path());

    assert!(!resources.join("planet/System.c4g").exists());
    let outcome =
        recover_interrupted_update_before_path_discovery_with(&clonk_update::FakePlatform::new())
            .test_value();
    let paths = discover_validated_startup_paths(None)
        .expect("validate recovered paths")
        .test_value();

    runtime_assert_eq!(
        outcome => clonk_update::ResumeOutcome::RolledBack { version: "0.7.0".to_string() };
        fs::read(paths.system_group_path()).expect("read restored system group") => b"stub";
    );
}

#[test]
fn launcher_recovery_marker_skips_a_second_exclusive_recovery() {
    let install = tempdir();
    fs::write(
        install.path().join(clonk_update::JOURNAL_FILE_NAME),
        b"{ malformed",
    )
    .test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_GAME_UPDATE_RECOVERY_COMPLETE", Some(Path::new("1"))),
    ]);

    runtime_assert_eq!(recover_interrupted_update_before_path_discovery().expect("the launcher already completed recovery") => clonk_update::ResumeOutcome::NothingToDo);
}

#[test]
fn environment_config_repairs_instead_of_custom_abort() {
    let install = tempdir();
    let user_data = tempdir();
    let custom = tempdir();
    fs::create_dir_all(install.path().join("planet")).test_value();
    fs::write(install.path().join("planet/System.c4g"), b"stub").test_value();
    let path = custom.path().join("environment.config");
    fs::write(&path, "[General]\nConfigResetSafety=7\n").test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("LC_CONFIG_FILE", Some(path.as_path())),
    ]);

    let paths = discover_validated_startup_paths(None)
        .expect("repair environment-selected config")
        .test_value();

    assert_eq!(paths.config_file(), path);
    let repaired = Config::load(paths.config_file()).test_value();
    runtime_assert_eq!(repaired.get_in(Some("General"), "ConfigResetSafety") => Some("42"));
}

#[test]
fn missing_integrity_fields_use_typed_defaults() {
    let dir = tempdir();
    let path = dir.path().join("clonk-rust.config");
    let original = b"[General]\nName=Keep\n\n[Graphics]\nResolutionY=0\n";
    fs::write(&path, original).test_value();

    runtime_assert!(!validate_or_repair_startup_config(&path, false)
        .expect("missing integrity fields are defaults"));
    assert_eq!(fs::read(&path).expect("read unchanged config"), original);
}

#[test]
fn default_repair_discards_cached_corrupt_user_path() {
    let install = tempdir();
    let home = tempdir();
    let poison = tempdir();
    fs::create_dir_all(install.path().join("planet")).test_value();
    fs::write(install.path().join("planet/System.c4g"), b"stub").test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("HOME", Some(home.path())),
        ("LC_USER_DATA_DIR", None),
        ("LC_CONFIG_FILE", None),
        ("XDG_DATA_HOME", None),
        ("LOCALAPPDATA", None),
        ("APPDATA", None),
    ]);
    let initial = cached_app_paths_with_config_file(None).test_value();
    let config_path = initial.config_file();
    fs::create_dir_all(config_path.parent().test_value()).test_value();
    fs::write(
        &config_path,
        format!(
            "[General]\nConfigResetSafety=7\nUserPath={}\n",
            poison.path().display()
        ),
    )
    .test_value();

    reset_cached_app_paths();
    let poisoned = cached_app_paths_with_config_file(None).test_value();
    assert_eq!(poisoned.user_data_dir(), poison.path());
    let repaired = discover_validated_startup_paths(None)
        .expect("repair poisoned config")
        .test_value();
    let expected_language = if input::is_german_system() {
        "DE"
    } else {
        "US"
    };

    assert_eq!(repaired.config_file(), config_path);
    assert_ne!(repaired.user_data_dir(), poison.path());
    runtime_assert_eq!(
        classic_loader_language_sequence(&repaired).expect("post-repair language default") => vec![expected_language.to_string()];
        cached_app_paths_with_config_file(None).expect("cache repaired paths").user_data_dir() => repaired.user_data_dir();
    );
}

#[test]
fn cli_config_flag_selects_the_explicit_file() {
    let install = tempdir();
    let user_data = tempdir();
    let custom = tempdir();
    fs::create_dir_all(install.path().join("planet")).test_value();
    fs::write(install.path().join("planet/System.c4g"), b"stub").test_value();
    let config_file = custom.path().join("command-line.config");
    let cli = Cli::try_parse_from([
        OsString::from("clonk-app"),
        OsString::from("--config"),
        config_file.as_os_str().to_os_string(),
    ])
    .test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("LC_CONFIG_FILE", None),
    ]);

    let paths = cached_app_paths_with_config_file(cli.config_file.as_deref()).test_value();

    assert_eq!(paths.config_file(), config_file);
}

#[test]
fn environment_config_file_routes_app_reads_and_writes() {
    let install = tempdir();
    let user_data = tempdir();
    let custom = tempdir();
    fs::create_dir_all(install.path().join("planet")).test_value();
    fs::write(install.path().join("planet/System.c4g"), b"stub").test_value();
    let environment_file = custom.path().join("environment.config");
    let command_line_file = custom.path().join("command-line.config");
    let command_line_sentinel = b"[Graphics]\nResolutionX=321\nResolutionY=234\n";
    fs::write(
        &environment_file,
        "[Graphics]\nResolutionX=777\nResolutionY=555\nScale=100\n",
    )
    .test_value();
    fs::write(&command_line_file, command_line_sentinel).test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("LC_CONFIG_FILE", Some(environment_file.as_path())),
    ]);

    let paths = cached_app_paths_with_config_file(Some(&command_line_file)).test_value();
    paths.ensure_user_dirs().test_value();
    let mut display = DisplayOptions::load(Some(&paths));
    assert_eq!(display.actual_size(), (777, 555));
    display.record_actual_size(888, 666);
    display.persist_if_dirty(&paths);

    let persisted = Config::load(&environment_file).test_value();
    runtime_assert_eq!(
        persisted.get_in(Some("Graphics"), "ResolutionX") => Some("888");
        persisted.get_in(Some("Graphics"), "ResolutionY") => Some("666");
        fs::read(&command_line_file).expect("read untouched command-line config") => command_line_sentinel;
    );
}

#[test]
fn collect_player_overlay_marks_focus_and_energy() {
    let focus = ObjectId::new(1);
    let teammate = ObjectId::new(2);

    let objects = vec![
        runtime_overlay_object(
            focus,
            "Clonk",
            Vector2::new(0, 0),
            80,
            25_000,
            clonk_engine::PhysicalInfo {
                energy: 100,
                breath: 100,
                magic: 50_000,
                ..clonk_engine::PhysicalInfo::default()
            },
            50,
        ),
        // This fixture has no matching live Engine object. Supply the
        // physical backing explicitly: native DrawEnergy always uses
        // GetPhysical()->Energy and never invents a 100-point range.
        runtime_overlay_object(
            teammate,
            "Balloon",
            Vector2::new(10, 0),
            40,
            0,
            clonk_engine::PhysicalInfo {
                energy: 100,
                ..clonk_engine::PhysicalInfo::default()
            },
            0,
        ),
    ];

    let mut snapshot = make_snapshot(
        objects,
        vec![HudPlayerSnapshot {
            owner: 1,
            crew: vec![focus, teammate],
            focus: Some(focus),
            eliminated: false,
            wealth: 120,
            score: 0,
        }],
    );
    snapshot.hud.local_players.push(1);

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
            input::legacy_gamepad_button_key(2, button).test_value(),
        );
    }
    let mut engine = Engine::new();
    let mut clonk_definition = test_definition("Clonk", "Clonk", "");
    clonk_definition.set_hide_hud_elements(0x3f);
    clonk_definition
        .set_hide_hud_bars(clonk_engine::HIDE_HUD_BAR_ENERGY | clonk_engine::HIDE_HUD_BAR_BREATH);
    engine.register_test_definition(clonk_definition);
    engine
        .register_script_definition("Balloon", "Balloon", "")
        .test_value();
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
    runtime_assert_eq!(
        player.show_control => 1 | 1 << 10;
        player.show_control_position => 3;
        player.last_com => 5;
        player.control_key_labels.len() => 10;
        player.control_key_labels[3] => gamepad_bindings.key_label_for_set(2, ControlBindingId::Throw);
    );
    runtime_assert_eq!(player.control_key_labels[9] => gamepad_bindings.key_label_for_set(2, ControlBindingId::PlayerMenu), "the viewport menu hint follows the player's live Gamepad3 set");

    snapshot.players[0].control_set = 2;
    let keyboard3 = collect_player_overlays(
        &mut engine,
        &snapshot,
        Some(focus),
        &bindings,
        &gamepad_bindings,
    );
    runtime_assert_eq!(keyboard3[0].control_key_labels[9] => format_key_label(VirtualKeyCode::F8), "the viewport menu hint follows the player's live Keyboard3 set");
    snapshot.players[0].control_set = 4;
    let unassigned_gamepad = collect_player_overlays(
        &mut engine,
        &snapshot,
        Some(focus),
        &bindings,
        &GamepadBindings::default(),
    );
    runtime_assert!(
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
    runtime_assert!(
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
    let focus_entry = focused.pop().test_value();
    assert!(focus_entry.label.contains("Clonk"));
    runtime_assert_eq!(
        (focus_entry.energy, focus_entry.energy_capacity) => (80, 100);
        focus_entry.magic_energy => 25_000;
        focus_entry.magic_capacity => 50_000;
        focus_entry.breath => 50;
        focus_entry.breath_capacity => 100;
        focus_entry.object_id => focus;
        focus_entry.hide_hud_elements => 0x3f;
        focus_entry.hide_hud_bars => clonk_engine::HIDE_HUD_BAR_ENERGY | clonk_engine::HIDE_HUD_BAR_BREATH;
    );
    assert!(focus_entry.portrait.is_none());

    let other_entry = player
        .crew
        .iter()
        .find(|crew| crew.label.contains("Balloon"))
        .test_value();
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
    runtime_assert_eq!(clonk_script::c4_string_bytes(&snapshot.players[0].name) => [0xe9], "presentation decoding does not rewrite synchronized player state");

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
    runtime_assert_eq!(overlay[0].crew.len() => 2, "non-roster ViewCursor is projected");
    runtime_assert_eq!(overlay[0].crew.iter().find(|crew| crew.object_id == teammate).map(|crew| (crew.hide_hud_elements, crew.hide_hud_bars)) => Some((0, 0)));
}

#[test]
fn participant_module_count_matches_cpp_smodulecount() {
    assert_eq!(c4_module_count(""), 0);
    assert_eq!(c4_module_count("   ;  ;; "), 0);
    assert_eq!(c4_module_count("Alice.c4p;; Bob.c4p"), 2);
    assert_eq!(c4_module_count(" Alice.c4p ; Bob.c4p ;"), 2);
    runtime_assert_eq!(c4_module_count("\t") => 1, "C++ SModuleCount ignores ASCII spaces only");
}

#[test]
fn configured_mission_access_reaches_fresh_engines_and_survives_replacement() {
    fn install_probe(engine: &mut Engine) -> usize {
        engine.register_test_definition(test_definition(
            "MACC",
            "Mission access probe",
            r#"#strict 2
                public func Has(password) { return GetMissionAccess(password); }
                public func Grant(password) { return GainMissionAccess(password); }
                "#,
        ));
        let object = engine.spawn_test_object(SpawnConfig::new("MACC"));
        engine.find_object_index(object).test_value()
    }

    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "MissionAccess", "Alpha; Beta").test_value();

    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let probe = install_probe(&mut app.engine);
    for password in ["alpha", "BETA"] {
        runtime_assert_eq!(
            app.engine.call_object_function(probe, "Has", vec![Value::String(password.to_string().into())],).expect("configured access query executes") => Value::Bool(true);
        );
    }
    runtime_assert_eq!(
        app.engine.call_object_function(probe, "Has", vec![Value::Nil]).expect("nil access query executes") => Value::Bool(false);
        app.engine .call_object_function( probe, "Grant", vec![Value::String("Runtime".to_string().into())], ) .expect("runtime access grant executes") => Value::Bool(true);
    );

    app.return_to_menu();
    let probe = install_probe(&mut app.engine);
    runtime_assert_eq!(
        app.engine.call_object_function(probe, "Has", vec![Value::String("runtime".to_string().into())],).expect("replacement engine sees process-local access") => Value::Bool(true);
    );
}

#[test]
fn team_options_submit_exact_sets_and_refresh_from_echoes() {
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
    runtime_assert_eq!(
        sets => [clonk_network::LegacyControlSet { value_type: 3, data: 4, by_client: 0, }, clonk_network::LegacyControlSet { value_type: 4, data: 1, by_client: 0, },];
    );
    let teams = app.network_team_assignment.as_ref().test_value().teams();
    runtime_assert_eq!(teams.team_distribution => clonk_engine::InitialNetworkTeamDistribution::Free);
    assert!(!teams.team_colors, "menu selections wait for host echoes");

    app.execute_control_set(sets[0]);
    app.execute_control_set(sets[1]);
    let options = app
        .classic_host_lobby
        .as_ref()
        .test_value()
        .controller
        .option_rows();
    runtime_assert!(
        options.iter().any(|row| { row.kind == LobbyOptionKind::TeamDistribution && row.value == "surprise random!" });
        options.iter().any(|row| { row.kind == LobbyOptionKind::TeamColors && row.value == "enabled" });
        options.iter().any(|row| row.kind == LobbyOptionKind::RandomTeamCount);
    );

    app.submit_classic_lobby_team_setting(LobbyOptionKind::TeamDistribution, 2);
    runtime_assert!(
        commands.take_submitted_control_sets().is_empty(),
        "None is not offered for predefined teams"
    );
}

#[test]
fn teams_sheet_groups_in_team_member_order_and_filters_inactive_or_invisible_players() {
    let mut clients = ControlClientRegistry::default();
    clients.replace_snapshot([
        runtime_fixture!(client: 0, true),
        runtime_fixture!(client: 7, false),
        runtime_fixture!(client: 8, true),
    ]);
    let player = |id, team, flags, player_type| clonk_engine::ControlPlayerInfoEntry {
        id,
        team,
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
                    player(10, 1, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                    player(
                        11,
                        2,
                        clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                        clonk_engine::PLAYER_INFO_TYPE_USER,
                    ),
                    player(99, 0, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                ],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                7,
                0,
                vec![player(20, 2, 0, clonk_engine::PLAYER_INFO_TYPE_USER)],
                -1,
            ),
            clonk_engine::PlayerInfoControlData::new(
                8,
                0,
                vec![
                    player(30, 2, 0, clonk_engine::PLAYER_INFO_TYPE_USER),
                    player(31, 1, 0, clonk_engine::PLAYER_INFO_TYPE_SCRIPT),
                ],
                -1,
            ),
        ],
    );
    let team = |id, name: &[u8], player_ids| clonk_engine::InitialNetworkTeam {
        id,
        name: LegacyCString::from_bytes(name.to_vec()).test_value(),
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
    runtime_assert_eq!(
        rows.iter().map(LobbyRosterRow::id).collect::<Vec<_>>() =>
            vec![LobbyRosterId::Header(LobbyRosterHeader::Team(2)), LobbyRosterId::Player(30), LobbyRosterId::Header(LobbyRosterHeader::Team(1)), LobbyRosterId::Player(31), LobbyRosterId::Player(10), LobbyRosterId::Header(LobbyRosterHeader::Team(3)),];
    );
    runtime_assert!(rows
        .iter()
        .all(|row| !matches!(row, LobbyRosterRow::Client(_))));

    let mut generated = metadata.clone();
    generated.auto_generate_teams = true;
    let generated =
        classic_lobby_roster_projection(&clients, &infos, Some(&generated), 0, LobbySheet::Teams).0;
    runtime_assert!(!generated.iter().any(|row| matches!(
        row,
        LobbyRosterRow::Header(LobbyHeaderRow {
            kind: LobbyRosterHeader::Team(3),
            ..
        })
    )));
}

#[test]
fn exhausted_script_player_names_pick_from_configured_list() {
    let (mut app, mut commands) = script_player_add_fixture(
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
fn empty_script_player_names_keep_computer_fallback() {
    let (mut app, mut commands) = script_player_add_fixture(b"", &[], 1);
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
fn player_shift_tab_wraps_and_continues_backwards() {
    use clonk_frontend::startup_plrsel::{PlrSelControl, PlrSelController};

    let mut app = new_classic_menu_app(640, 480);
    let mut dialog = PlrSelController::new(1);
    dialog.resize(640, 480);
    app.startup_player_dialog = Some(dialog);
    app.replace_startup_view(StartupView::PlayerSelection);
    app.test_modifiers(ModifiersState::SHIFT);

    tap_runtime_key(&mut app, VirtualKeyCode::Tab);
    runtime_assert_eq!(startup_player_focus(&app) => PlrSelControl::Crew);
    tap_runtime_key(&mut app, VirtualKeyCode::Tab);
    runtime_assert_eq!(startup_player_focus(&app) => PlrSelControl::Properties);

    app.test_modifiers(ModifiersState::empty());
    tap_runtime_key(&mut app, VirtualKeyCode::Tab);
    runtime_assert_eq!(startup_player_focus(&app) => PlrSelControl::Crew);
}

#[test]
fn player_shift_tab_covers_back_list_and_crew_edges() {
    use clonk_frontend::startup_plrsel::{PlrSelControl, PlrSelController};

    let mut app = new_classic_menu_app(640, 480);
    let mut dialog = PlrSelController::new(1);
    dialog.resize(640, 480);
    app.startup_player_dialog = Some(dialog);
    app.replace_startup_view(StartupView::PlayerSelection);

    runtime_assert_eq!(startup_player_focus(&app) => PlrSelControl::PlayerList);
    tap_runtime_key(&mut app, VirtualKeyCode::Tab);
    runtime_assert_eq!(startup_player_focus(&app) => PlrSelControl::Back);

    app.test_modifiers(ModifiersState::SHIFT);
    for (expected, _description) in [
        (PlrSelControl::PlayerList, "Back to PlayerList"),
        (PlrSelControl::Crew, "PlayerList to Crew"),
    ] {
        tap_runtime_key(&mut app, VirtualKeyCode::Tab);
        runtime_assert_eq!(startup_player_focus(&app) => expected);
    }

    app.test_modifiers(ModifiersState::empty());
    tap_runtime_key(&mut app, VirtualKeyCode::Tab);
    runtime_assert_eq!(startup_player_focus(&app) => PlrSelControl::PlayerList);
}

#[test]
fn player_typeahead_and_apps_route_through_selected_row() {
    let player = |name: &str| runtime_fixture!(player_selection: name.to_string(), String::new());
    let mut app = new_classic_menu_app(640, 480);
    app.startup_player_models = ["Thomas", "Ada", "tina", "Tori"]
        .map(player)
        .into_iter()
        .collect();
    app.open_player_selection_dialog();

    for (character, expected) in [('T', 2), ('T', 3), ('t', 0)] {
        app.test_text_input(character);
        runtime_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").selected_index() => Some(expected));
    }
    app.test_text_input('T');
    let (selected, anchor) = app
        .startup_player_dialog
        .test_ref()
        .keyboard_context_target()
        .test_value();
    assert_eq!(selected, 2);
    app.startup_player_dialog
        .test_mut()
        .set_pointer_position(Some(GuiPoint::new(639.0, 479.0)));

    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    let popup = app.context_menu.test_ref();
    let panel = &popup.layout().panels[0];
    runtime_assert_eq!(
        panel.rows.len() => 2;
        panel.selected => None;
        (panel.bounds.x, panel.bounds.y) => (anchor.x as i32, anchor.y as i32);
    );
    app.close_context_menu_silently();

    tap_runtime_key(&mut app, VirtualKeyCode::Tab);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
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
        .test_value()
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
    runtime_assert_eq!(controller.portrait_preview().map(|image| (image.width(), image.height())) => Some((150, 150)));
    runtime_assert!(controller
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

    runtime_assert_eq!(app.new_startup_player_properties_controller(0, 0).player().name => "Novice");
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

    runtime_assert_eq!(app.new_startup_player_properties_controller(0, 0).player().name => "Neuling");
}

#[test]
fn portrait_selector_uses_and_persists_last_folder_index() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let program_data = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(program_data.path().join("planet/System.c4g")).test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(program_data.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
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
    .test_value();

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
        .test_value();
    assert_eq!(selector.current_location_index(), 1);
    runtime_assert!(selector
        .items()
        .iter()
        .any(|item| item.filename() == Some("Program.BMP")));

    for _ in 0..6 {
        let actions = app
            .startup_player_properties_dialog
            .test_mut()
            .controller
            .handle_key_down(KeyCode::Tab);
        assert!(actions.is_empty());
    }
    // C4GuiDialogs.cpp:386-421 and C4FileSelDlg.cpp:162-169,564-572
    // put Location after six forward focus steps. C4GuiComboBox.cpp:66-86
    // and C4GuiMenu.cpp:240-299 then open, highlight, and choose row zero.
    // ContextMenu::Open and SelectionChanged raise DoorOpen followed by
    // Command (`C4GuiMenu.cpp:418,465`).
    for (key, sound) in [
        (
            KeyCode::Down,
            clonk_frontend::startup_portraitsel::PortraitSelSound::DoorOpen,
        ),
        (
            KeyCode::Down,
            clonk_frontend::startup_portraitsel::PortraitSelSound::Command,
        ),
    ] {
        let actions = app
            .startup_player_properties_dialog
            .test_mut()
            .controller
            .handle_key_down(key);
        assert_eq!(
            actions,
            vec![clonk_frontend::startup_plrproperties::PlayerPropertiesAction::GuiSound(sound)]
        );
    }
    let actions = app
        .startup_player_properties_dialog
        .test_mut()
        .controller
        .handle_key_down(KeyCode::Enter);
    app.process_startup_player_properties_actions(actions);
    let selector = app
        .startup_player_properties_dialog
        .as_ref()
        .and_then(|pending| pending.controller.portrait_selector())
        .test_value();
    assert_eq!(selector.current_location_index(), 0);
    runtime_assert!(selector
        .items()
        .iter()
        .any(|item| item.filename() == Some("Custom.PNG")));
    // C4FileSelDlg.cpp:189-194 changes the active path immediately, while
    // C4FileSelDlg.cpp:575-580 remembers its row only from OnClosed.
    runtime_assert_eq!(
        clonk_app_netplay::configured_native_value(&fs::read(paths.config_file()).expect("read portrait config"), "Startup", "LastPortraitFolderIdx",).expect("portrait location is not persisted before close").as_bytes() =>
            b"1";
    );

    let actions = app
        .startup_player_properties_dialog
        .test_mut()
        .controller
        .handle_key_down(KeyCode::Escape);
    runtime_assert_eq!(
        actions => vec![clonk_frontend::startup_plrproperties::PlayerPropertiesAction::PortraitSelectorClosed { location_index: 0 }],
        "C4FileSelDlg.cpp:209-228 and 575-580 remember the current row on Cancel",
    );
    app.process_startup_player_properties_actions(actions);
    runtime_assert_eq!(
        clonk_app_netplay::configured_native_value(&fs::read(paths.config_file()).expect("read portrait config after close"), "Startup", "LastPortraitFolderIdx",).expect("persisted portrait location after close").as_bytes() =>
            b"0";
    );
    persist_native_config_values(
        &paths,
        "Startup",
        &[(
            "LastPortraitFolderIdx",
            clonk_app_netplay::NativeConfigValue::RawAscii("1"),
        )],
    )
    .test_value();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::ChoosePicture,
    ]);
    runtime_assert_eq!(
        app.startup_player_properties_dialog.as_ref().and_then(|pending| pending.controller.portrait_selector()).expect("selector reopens at the persisted location").current_location_index() =>
            0,
        "C++ keeps the close-time config row in memory even when disk persistence fails \
             (`C4FileSelDlg.cpp:575-580`)",
    );
    reset_cached_app_paths();
}

#[test]
fn player_new_properties_enter_f2_and_insert_open_the_modal() {
    let mut app = new_real_classic_menu_app(640, 480);
    let model = runtime_fixture!(player_selection: "Entry Player".to_string(), "entry".to_string());
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
    .test_value();
    runtime_assert!(
        matches!(app.startup_player_properties_dialog.as_ref().map(|pending| pending.controller.mode()), Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::New));
    );
    let mut frame = vec![0; 640 * 480 * 4];
    app.test_render(&mut frame);
    app.startup_player_properties_dialog = None;

    for key in [VirtualKeyCode::Enter, VirtualKeyCode::F2] {
        app.test_key(key, ElementState::Pressed);
        runtime_assert!(
            matches!(app.startup_player_properties_dialog.as_ref().map(|pending| pending.controller.mode()), Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 0 }));
        );
        app.startup_player_properties_dialog = None;
        app.test_key(key, ElementState::Released);
    }

    app.test_key(VirtualKeyCode::Insert, ElementState::Pressed);
    runtime_assert!(
        matches!(app.startup_player_properties_dialog.as_ref().map(|pending| pending.controller.mode()), Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::New));
    );
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
        runtime_assert_eq!(app.startup_options_dialog.as_ref().expect("options state").focused_program_control() => Some(target));
    }

    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert_eq!(app.startup_options_dialog.as_ref().unwrap().focused_program_control() => None);
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Left,
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert_eq!(app.startup_options_dialog.as_ref().unwrap().focused_program_control() => Some(OptionsProgramFocusTarget::AdvancedButton));
}

#[test]
fn show_folder_maps_config_defaults_on_and_reads_an_explicit_false() {
    assert!(load_show_folder_maps(None));
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "Graphics", "ShowFolderMaps", "0").test_value();
    assert!(!load_show_folder_maps(Some(&paths)));
}

#[test]
fn packed_logical_folder_map_uses_case_insensitive_group_traversal() {
    let root = tempdir();
    let png_path = root.path().join("map.png");
    write_map_png(&png_path, 2, 2, [9, 8, 7, 255]);
    let png = fs::read(&png_path).test_value();
    let inner = packed_test_group(&[
        ("fOlDeRmAp.TxT", false, b"[FolderMap]\n"),
        ("FolderMap.png", false, png.as_slice()),
    ]);
    let outer = packed_test_file_group(&[("INNER.C4F", true, inner.as_slice())]);
    let outer_path = root.path().join("Outer.c4f");
    fs::write(&outer_path, outer).test_value();
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
    let menu =
        StartupMenu::new(build_menu_entries(&entries, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, entries);
    state.enter_folder("Outer.c4f");
    state.enter_folder(&inner_entry.identifier);
    runtime_assert!(state.configure_current_folder_map(
        true,
        640,
        480,
        &MissionAccessStore::default(),
        &["US".to_string()],
    ));
    runtime_assert_eq!(state.current_map().expect("packed map").source_path => logical_inner);
}

#[test]
fn editor_kind_and_edit_action_return_typed_boundaries() {
    let mut editor = FrontendScenario::fallback();
    editor.identifier = "Editor.c4s".to_string();
    editor.title = "Editor".to_string();
    editor.kind = ScenarioKind::Editor;
    let scenarios = vec![editor.clone()];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
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
        runtime_assert!(
            matches!(app.process_menu_actions(vec![action]), Err(ClassicParityBoundary::EditorScenario { ref identifier }) if identifier == &editor.identifier)
        );
    }
    runtime_assert!(
        matches!(app.process_menu_actions(vec![StartupMenuAction::EditEntry(summary)]), Err(ClassicParityBoundary::EditScenario { ref identifier }) if identifier == &editor.identifier);
    );
}

#[test]
fn empty_discovery_and_catalog_never_inject_player_facing_sandbox() {
    assert!(build_scenario_catalog(&[]).is_empty());

    let invalid_install = tempdir();
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
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    paths.ensure_user_dirs().test_value();
    let player = user_data.path().join("Players/Alice.c4p");
    fs::create_dir_all(&player).test_value();
    let mut raw = b"[General]\nName=\"M\x80ker\"\nParticipants=\"".to_vec();
    raw.extend_from_slice(player.as_os_str().as_encoded_bytes());
    raw.extend_from_slice(b"\"\n");
    fs::write(paths.config_file(), &raw).test_value();

    let mut app = GameApp::new(
        320,
        200,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Player".to_string(), false),
    )
    .test_value();
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
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nFontName=Endeavour\nFontSize=28\nVendorResetKey=remove\n[Graphics]\nScale=250\n").test_value();
    let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths)).test_value();
    wait_for_menu(&mut app);
    app.open_options_menu();
    app.startup_options_dialog
        .as_mut()
        .test_value()
        .program_mut()
        .preloading = true;
    let before_cancel = fs::read(paths.config_file()).test_value();

    app.process_options_dialog_actions(vec![OptionsDlgAction::ResetConfiguration])
        .test_value();
    let modal = app.message_dialogs.last().test_value();
    runtime_assert_eq!(
        modal.state.caption() => "Reset configuration";
        modal.state.message() => "Are you sure you want to reset all configuration values?|For changes to take effect the program has to be restarted.";
        modal.state.buttons() => MessageDialogButtons::YES_NO;
        modal.state.icon() => MessageDialogIcon::NOTIFY;
        modal.state.focused_button() => Some(MessageDialogButton::Yes);
    );
    app.finish_message_dialog(MessageDialogResult::No)
        .test_value();
    assert_eq!(fs::read(paths.config_file()).unwrap(), before_cancel);
    assert!(!app.configuration_reset_requested);
    assert!(!app.take_exit_request());

    app.process_options_dialog_actions(vec![OptionsDlgAction::ResetConfiguration])
        .test_value();
    app.finish_message_dialog(MessageDialogResult::Yes)
        .test_value();
    let reset = Config::load(paths.config_file()).test_value();
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
    app.test_modifiers(ModifiersState::CONTROL);
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
        app.test_key(VirtualKeyCode::Tab, ElementState::Released);
        runtime_assert_eq!(app.startup_options_dialog.as_ref().expect("options dialog").active_sheet() => expected);
    }
}

#[test]
fn options_control_set_digit_hotkeys_require_alt_and_respect_visible_sets() {
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
    let dialog = app.startup_options_dialog.test_mut();
    *dialog.controls_mut() = controls;
    dialog.restore_sheet(OptionsSheet::Keyboard);

    tap_runtime_key(&mut app, VirtualKeyCode::Digit2);
    runtime_assert_eq!(selected_options_control_set(&app, ControlDevice::Keyboard) => 0);

    app.test_modifiers(ModifiersState::ALT);
    tap_runtime_key(&mut app, VirtualKeyCode::Digit2);
    runtime_assert_eq!(selected_options_control_set(&app, ControlDevice::Keyboard) => 1);
    app.test_key(VirtualKeyCode::Numpad1, ElementState::Pressed);
    runtime_assert_eq!(selected_options_control_set(&app, ControlDevice::Keyboard) => 1);

    app.test_modifiers(ModifiersState::ALT | ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Digit4, ElementState::Pressed);
    runtime_assert_eq!(selected_options_control_set(&app, ControlDevice::Keyboard) => 3);
    app.test_modifiers(ModifiersState::ALT | ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::Digit1, ElementState::Pressed);
    runtime_assert_eq!(selected_options_control_set(&app, ControlDevice::Keyboard) => 3);

    app.startup_options_dialog
        .as_mut()
        .test_value()
        .restore_sheet(OptionsSheet::Gamepad);
    app.process_options_dialog_actions(vec![OptionsDlgAction::SheetChanged(OptionsSheet::Gamepad)])
        .test_value();
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Digit3, ElementState::Pressed);
    runtime_assert_eq!(
        selected_options_control_set(&app, ControlDevice::Gamepad) => 2;
        app.gamepads.options_open_slot() => Some(GamepadSlot::new(2));
    );

    for key in [VirtualKeyCode::Digit4, VirtualKeyCode::Digit0] {
        app.test_key(key, ElementState::Pressed);
    }
    runtime_assert_eq!(
        selected_options_control_set(&app, ControlDevice::Gamepad) => 2;
        app.gamepads.options_open_slot() => Some(GamepadSlot::new(2));
    );
}

#[test]
fn options_control_set_hotkeys_do_not_leak_through_modals() {
    use clonk_frontend::message_dialog::MessageDialogResult;
    use clonk_frontend::startup_options_controls::ControlDevice;
    use clonk_frontend::startup_options_dlg::{OptionsDlgAction, OptionsSheet};

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    let dialog = app.startup_options_dialog.test_mut();
    dialog.restore_sheet(OptionsSheet::Keyboard);
    assert!(dialog.controls_mut().select_set(ControlDevice::Keyboard, 3));
    app.test_modifiers(ModifiersState::ALT);

    app.process_options_dialog_actions(vec![OptionsDlgAction::ResetConfiguration])
        .test_value();
    app.test_key(VirtualKeyCode::Digit2, ElementState::Pressed);
    runtime_assert_eq!(selected_options_control_set(&app, ControlDevice::Keyboard) => 3);
    app.finish_message_dialog(MessageDialogResult::No)
        .test_value();

    app.process_options_dialog_actions(vec![OptionsDlgAction::OpenGraphicsScaleText])
        .test_value();
    app.test_key(VirtualKeyCode::Digit2, ElementState::Pressed);
    runtime_assert_eq!(selected_options_control_set(&app, ControlDevice::Keyboard) => 3);
}

#[test]
fn options_close_reports_disk_write_failure() {
    use clonk_frontend::message_dialog::MessageDialogIcon;

    let mut app = new_classic_menu_app(640, 480);
    app.open_options_menu();
    app.close_options_menu_with_persist_result(Some(Err(io::Error::other(
        "simulated config write failure",
    ))))
    .test_value();

    assert_eq!(app.startup_view, StartupView::MainMenu);
    let error = app.message_dialogs.last().test_value();
    runtime_assert_eq!(
        error.state.caption() => "Configuration error";
        error.state.message() => "Could not save configuration: simulated config write failure";
        error.state.icon() => MessageDialogIcon::ERROR;
    );
    runtime_assert!(matches!(
        error.continuation,
        MessageDialogContinuation::None
    ));
}

#[test]
fn options_language_loads_real_de_and_selection_reloads_and_persists() {
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        "[General]\nLanguage=DE - Deutsch\nLanguageEx=DE\n",
    )
    .test_value();

    let mut app = test_game_app(1280, 720, AudioOptions::default(), Some(&paths)).test_value();
    wait_for_menu(&mut app);
    app.open_options_menu();

    let program = app.startup_options_dialog.test_ref().program();
    runtime_assert_eq!(
        program.language_text => "DE - Deutsch";
        program.language_info => "Original-Sprachpaket von RedWolf Design.";
        program.no_language_info => "Sprachpaket nicht verf\u{00fc}gbar.";
    );
    let mut codes = program
        .language_infos
        .iter()
        .map(|info| info.code.as_str())
        .collect::<Vec<_>>();
    codes.sort_unstable();
    runtime_assert_eq!(
        codes => vec!["DE", "US"];
        app.needed_material_need => "%s|braucht noch";
        app.object_no_dig => "%s kann|nicht graben.";
        app.default_rank_names.as_deref().and_then(|names| names.get(1)).map(String::as_str) => Some("Fähnrich");
        app.loaded_default_rank_names => app.default_rank_names;
        app_default_rank_promotion_name(&app) => "Fähnrich";
    );

    app.process_options_dialog_actions(vec![
        clonk_frontend::startup_options_dlg::OptionsDlgAction::OpenLanguageCombo,
    ])
    .test_value();
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
    .test_value();

    let program = app.startup_options_dialog.test_ref().program();
    runtime_assert_eq!(
        program.language => "US";
        program.language_text => "US - English";
        program.language_ex => "US,DE";
        app.needed_material_need => "%s|needs";
        app.object_no_dig => "%s cannot dig.";
        app.default_rank_names.as_deref().and_then(|names| names.get(1)).map(String::as_str) => Some("Fähnrich");
        app.loaded_default_rank_names .as_deref() .and_then(|names| names.get(1)) .map(String::as_str) => Some("Ensign");
        app_default_rank_promotion_name(&app) => "Fähnrich";
    );

    app.return_to_menu();
    assert_eq!(app.default_rank_names, app.loaded_default_rank_names);
    assert_eq!(app_default_rank_promotion_name(&app), "Ensign");

    let config = Config::load(paths.config_file()).test_value();
    assert_eq!(config.get_in(Some("General"), "Language"), Some("US"));
    assert_eq!(config.get_in(Some("General"), "LanguageEx"), Some("US,DE"));
    assert_eq!(config.get_in(Some("General"), "LanguageCharset"), Some(""));
}

#[test]
fn options_non_tab_gui_bindings_require_the_exact_bare_modifier_mask() {
    use clonk_frontend::startup_options_dlg::{OptionsSheet, SoundCheckboxId};

    let modifier_masks = [
        ModifiersState::ALT,
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
    ];

    let mut checkbox = new_running_sandbox_app();
    checkbox.return_to_menu();
    enter_unported_startup_subscreen(
        &mut checkbox,
        ClassicStartupSubscreen::Options(OptionsSheet::Sound),
    );
    for modifiers in modifier_masks {
        checkbox.test_modifiers(modifiers);
        for key in [
            VirtualKeyCode::ArrowUp,
            VirtualKeyCode::ArrowDown,
            VirtualKeyCode::ArrowLeft,
            VirtualKeyCode::Backspace,
            VirtualKeyCode::Escape,
            VirtualKeyCode::ArrowRight,
        ] {
            checkbox
                .handle_key(key, ElementState::Pressed)
                .unwrap_or_else(|error| panic!("modified {key:?} down: {error}"));
            checkbox
                .handle_key(key, ElementState::Released)
                .unwrap_or_else(|error| panic!("modified {key:?} up: {error}"));
        }
        assert_eq!(checkbox.startup_view, StartupView::Options);
        runtime_assert_eq!(checkbox.startup_options_dialog.as_ref().expect("Options model").active_sheet() => OptionsSheet::Sound, "modified Up/Down must not switch sheets");
    }

    checkbox.test_modifiers(ModifiersState::empty());
    tap_runtime_key(&mut checkbox, VirtualKeyCode::Tab);
    tap_runtime_key(&mut checkbox, VirtualKeyCode::Tab);
    runtime_assert_eq!(checkbox.startup_options_dialog.as_ref().expect("Options model").focused_sound_checkbox() => Some(SoundCheckboxId::FrontendSoundEffects));
    let before_checkbox = checkbox.startup_options_dialog.test_ref().sound().clone();
    for modifiers in modifier_masks {
        checkbox.test_modifiers(modifiers);
        for key in [
            VirtualKeyCode::Space,
            VirtualKeyCode::ArrowLeft,
            VirtualKeyCode::Backspace,
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
        runtime_assert_eq!(checkbox.startup_options_dialog.as_ref().expect("Options model").sound() => &before_checkbox, "modified Space must not toggle the focused checkbox");
    }

    checkbox.test_modifiers(ModifiersState::SUPER);
    checkbox.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    runtime_assert_ne!(checkbox.startup_options_dialog.as_ref().expect("Options model").sound() => &before_checkbox);

    let mut back = new_running_sandbox_app();
    back.return_to_menu();
    enter_unported_startup_subscreen(
        &mut back,
        ClassicStartupSubscreen::Options(OptionsSheet::Sound),
    );
    back.test_modifiers(ModifiersState::SHIFT);
    tap_runtime_key(&mut back, VirtualKeyCode::Tab);
    for modifiers in modifier_masks {
        back.test_modifiers(modifiers);
        for key in [
            VirtualKeyCode::Enter,
            VirtualKeyCode::NumpadEnter,
            VirtualKeyCode::Space,
        ] {
            back.handle_key(key, ElementState::Pressed)
                .unwrap_or_else(|error| panic!("modified Back {key:?} down: {error}"));
            back.handle_key(key, ElementState::Released)
                .unwrap_or_else(|error| panic!("modified Back {key:?} up: {error}"));
        }
        runtime_assert_eq!(back.startup_view => StartupView::Options, "modified Enter/Space must not activate Back");
    }
}

#[test]
fn scenario_preset_replaces_seed_while_fixed_selection_wins_publication() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let content = tempdir();
    let content_root = content.path();
    fs::create_dir_all(content_root.join("Material.c4g")).test_value();
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
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(scenario_path.join("Scenario.txt"), "[Head]\nTitle=Definition Choice\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=./Preset.c4d\n").test_value();

    let (_guard, paths) = exact_loader_test_paths(user_data.path(), Some(content_root));
    persist_config_value(&paths, "General", "DefinitionPath", "Custom/").test_value();
    let app = new_menu_app_with_paths(640, 480, &paths);
    let custom_prefix = startup_definition_paths(&paths)
        .expect("read configured DefinitionPath")
        .active_custom_root
        .test_value();
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
        .test_value();
    let mismatch = build_network_host_preparation(
        &app,
        &seed.frontend,
        &seed.definition_load,
        &seed.effective_definition_modules,
        &[],
        Some((&seed.definition_executable_path, &seed.definition_path)),
        Some((&seed.lobby.local_name, &seed.lobby.nick)),
    )
    .test_value()
    .prepare()
    .expect_err("host preparation rejects a changed staged definition vector");
    runtime_assert!(

            matches!(mismatch, prepared_host_bootstrap::PrepareHostBootstrapError::StagedDefinitionResourcesChanged { staged, prepared, } if staged.is_empty() && prepared == vec![custom_root.join("Preset.c4d"), content_root.join("Preset.c4d"), outer.clone(),]);
    );
    let seed_prepared = prepare_staged_network_host(&app, &seed);
    runtime_assert_eq!(
        published_definition_wire_names(&seed_prepared) => vec![b"Custom/./Preset.c4d".to_vec(), b"./Preset.c4d".to_vec(), b"Outer.c4f".to_vec(),],
        "a nonempty scenario preset replaces the seed before rooted/local expansion",
    );
    fs::write(scenario_path.join("Scenario.txt"), "[Head]\nTitle=Definition Choice\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Preset.c4d\n").test_value();
    let changed_spelling = build_network_host_preparation(
        &app,
        &seed.frontend,
        &seed.definition_load,
        &seed.effective_definition_modules,
        &seed.definition_resources,
        Some((&seed.definition_executable_path, &seed.definition_path)),
        Some((&seed.lobby.local_name, &seed.lobby.nick)),
    )
    .test_value()
    .prepare()
    .expect_err("host preparation rejects changed staged publication spellings");
    runtime_assert!(
        matches!(changed_spelling, prepared_host_bootstrap::PrepareHostBootstrapError::StagedDefinitionPublicationChanged { staged, prepared, } if staged != prepared);
    );
    fs::write(scenario_path.join("Scenario.txt"), "[Head]\nTitle=Definition Choice\nMaxPlayer=1\nNoInitialize=1\n\n[Definitions]\nDefinition1=Seed.c4d\n").test_value();
    let changed_selection = build_network_host_preparation(
        &app,
        &seed.frontend,
        &seed.definition_load,
        &seed.effective_definition_modules,
        &seed.definition_resources,
        Some((&seed.definition_executable_path, &seed.definition_path)),
        Some((&seed.lobby.local_name, &seed.lobby.nick)),
    )
    .test_value()
    .prepare()
    .expect_err("host preparation rejects changed staged selection semantics");
    runtime_assert!(

            matches!(changed_selection, prepared_host_bootstrap::PrepareHostBootstrapError::StagedDefinitionSelectionChanged { staged, prepared, } if staged == vec!["Preset.c4d".to_owned()] && prepared == vec!["Seed.c4d".to_owned()]);
    );

    let fixed = app
        .prepare_network_host_scenario(
            frontend.clone(),
            ScenarioDefinitionLoad::Fixed {
                modules: vec!["FixedB.c4d".to_string(), "./FixedA.c4d".to_string()],
                definition_root: Some(custom_prefix.clone()),
            },
        )
        .test_value();
    let fixed_prepared = prepare_staged_network_host(&app, &fixed);
    runtime_assert_eq!(
        published_definition_wire_names(&fixed_prepared) =>
            vec![b"Custom/FixedB.c4d".to_vec(), b"Custom/./FixedA.c4d".to_vec(), b"FixedB.c4d".to_vec(), b"./FixedA.c4d".to_vec(), b"Outer.c4f".to_vec(),],
        "fixed selection stays authoritative and folder locals append exactly once",
    );
    runtime_assert_eq!(
        fixed_prepared.definition_modules() => ["Custom/FixedB.c4d", "Custom/./FixedA.c4d", "FixedB.c4d", "./FixedA.c4d", "Outer.c4f",],
        "the pre-SetModules vector remains available after publication",
    );
    runtime_assert_eq!(
        activated_definition_load(Some(fixed_prepared.definition_modules().to_vec()), ScenarioDefinitionLoad::Fixed { modules: vec!["final/retyped/paths.c4d".to_owned()], definition_root: None, },) =>
            ScenarioDefinitionLoad::Fixed { modules: vec!["Custom/FixedB.c4d".to_owned(), "Custom/./FixedA.c4d".to_owned(), "FixedB.c4d".to_owned(), "./FixedA.c4d".to_owned(), "Outer.c4f".to_owned(),], definition_root: None, },
        "activation retains Game.DefinitionFilenames instead of retyped resource paths",
    );
    let dynamic = fixed_prepared
        .host_config()
        .resource_files
        .iter()
        .find(|resource| {
            resource.core.resource_type == clonk_network::HostResourceType::Dynamic as u8
        })
        .test_value();
    let scenario = Group::open(&dynamic.path)
        .test_value()
        .read_file("Scenario.txt")
        .test_value();
    let expected = b"Definitions=\"FixedB.c4d\",\"./FixedA.c4d\",\"FixedB.c4d\",\"./FixedA.c4d\",\"Outer.c4f\"";
    runtime_assert!(scenario
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
        .test_value();
    let fixed_empty_prepared = prepare_staged_network_host(&app, &fixed_empty);
    runtime_assert_eq!(published_definition_wire_names(&fixed_empty_prepared) => vec![b"Outer.c4f".to_vec()], "fixed-empty suppresses the nonempty preset while folder locals still append");
}

#[test]
fn player_delete_confirmation_removes_refreshes_and_reports_failure() {
    let _lock = env_lock().lock();
    let install_root = test_repository_root();
    let user_data = tempdir();
    let player_root = user_data.path().join("Players");
    let ada = player_root.join("Ada.c4p");
    fs::create_dir_all(&ada).test_value();
    fs::write(
        ada.join("Player.txt"),
        "[Player]\nName=Ada\nTotalPlayingTime=36001\n\n[Preferences]\nColorDw=255\n",
    )
    .test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install_root), user_data.path());
    let mut config = Config::new();
    config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
    config.set_in(Some("General"), "Participants", ada.to_string_lossy());
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    config.save(paths.config_file()).test_value();

    let mut app = GameApp::new(
        1280,
        720,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Player".to_string(), false),
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_player_selection_dialog();
    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::DeletePlayer(0),
    ])
    .test_value();

    let confirm = &app.message_dialogs[0].state;
    runtime_assert_eq!(
        confirm.caption() => "Delete";
        confirm.message() => "Do you really want to delete player Ada? - this player has a total playing time of 10:00:01!";
        confirm.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::YES_NO;
        confirm.icon() => clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM;
        confirm.focused_button() => Some(clonk_frontend::message_dialog::MessageDialogButton::Yes);
    );

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();
    assert!(ada.exists());
    assert_eq!(app.startup_player_files.len(), 1);

    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::DeletePlayer(0),
    ])
    .test_value();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();
    assert!(!ada.exists());
    assert!(app.message_dialogs.is_empty());
    assert!(app.startup_player_files.is_empty());
    assert!(app.startup_player_models.is_empty());
    runtime_assert_eq!(
        app.startup_player_dialog.as_ref().expect("player controller").selected_index() => None;
        Config::load(paths.config_file()).expect("reload player config").get_in(Some("General"), "Participants") => Some("");
    );

    let broken = player_root.join("Broken.c4p");
    fs::create_dir_all(&broken).test_value();
    fs::write(
        broken.join("Player.txt"),
        "[Player]\nName=Broken\n\n[Preferences]\nColorDw=255\n",
    )
    .test_value();
    app.refresh_startup_player_list();
    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::DeletePlayer(0),
    ])
    .test_value();
    fs::remove_dir_all(&broken).test_value();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();
    assert_eq!(app.message_dialogs.len(), 1);
    let failure = &app.message_dialogs[0].state;
    runtime_assert_eq!(
        failure.caption() => "Clear";
        failure.message() => "Delete failure.";
        failure.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::OK;
        failure.icon() => clonk_frontend::message_dialog::MessageDialogIcon::ERROR;
    );
    assert!(app.startup_player_files.is_empty());
    reset_cached_app_paths();
}

#[test]
fn unconfigured_stick_and_hat_emit_no_gameplay_controls() {
    let mut app = new_running_sandbox_app();
    app.gamepad_bindings = GamepadBindings::from_config(&Config::new());
    app.local_controls = LocalControlRegistry::default();
    app.local_controls
        .initialize(test_local_control_init(app.local_owner, 4, false, false));
    let slot = GamepadSlot::new(0);

    app.test_gamepad_events([gamepad_direction_event(
        slot,
        ControlButton::Left,
        ElementState::Pressed,
    )]);
    runtime_assert_eq!(app.engine.player(app.local_owner).expect("control-set four player").control.pressed_coms => 0, "semantic direction alone must not restore the hardwired gameplay path");

    app.test_gamepad_events([
        gamepad_axis_event(
            slot,
            LegacyGamepadAxis::new(0, false),
            ElementState::Pressed,
        ),
        gamepad_direction_event(slot, ControlButton::Left, ElementState::Pressed),
        gamepad_axis_event(
            slot,
            LegacyGamepadAxis::new(6, false),
            ElementState::Pressed,
        ),
        gamepad_direction_event(slot, ControlButton::Left, ElementState::Pressed),
    ]);

    let pressed = app.engine.test_player(app.local_owner).control.pressed_coms;
    assert_eq!(pressed, 0);
}

#[test]
fn axis_up_fires_dig_and_hat_zero_fires_configured_left() {
    let mut config = Config::new();
    config.set_in(
        Some("Gamepad0"),
        "Button6",
        input::legacy_gamepad_axis_key(0, 1, false)
            .test_value()
            .to_string(),
    );
    config.set_in(
        Some("Gamepad0"),
        "Button7",
        input::legacy_gamepad_axis_key(0, 6, false)
            .test_value()
            .to_string(),
    );
    let mut app = new_running_sandbox_app();
    app.gamepad_bindings = GamepadBindings::from_config(&config);
    app.local_controls = LocalControlRegistry::default();
    app.local_controls
        .initialize(test_local_control_init(app.local_owner, 4, false, false));
    let slot = GamepadSlot::new(0);

    app.test_gamepad_events([
        gamepad_axis_event(
            slot,
            LegacyGamepadAxis::new(1, false),
            ElementState::Pressed,
        ),
        gamepad_direction_event(slot, ControlButton::Up, ElementState::Pressed),
        gamepad_axis_event(
            slot,
            LegacyGamepadAxis::new(6, false),
            ElementState::Pressed,
        ),
        gamepad_direction_event(slot, ControlButton::Left, ElementState::Pressed),
    ]);

    let pressed = app.engine.test_player(app.local_owner).control.pressed_coms;
    assert_ne!(pressed & (1 << clonk_engine::COM_DIG), 0);
    assert_ne!(pressed & (1 << clonk_engine::COM_LEFT), 0);
}

#[test]
fn runtime_status_report_failure_remains_stopped_and_unreached() {
    let mut app = new_state_only_running_sandbox_app();
    let (events, commands) = install_running_network_stub(&mut app, 7, 0, 1);
    drop(commands);
    let pause = clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_PAUSE, 1, 0);
    events
        .send(NetworkEvent::StatusRequested(pause))
        .test_value();

    app.test_network_events();

    assert!(!app.network_control_running);
    runtime_assert_eq!(app.runtime_network_status_barrier => Some(RuntimeNetworkStatusBarrier { status: pause, local_reached: false, actual_control_tick: None, }));
}

#[test]
fn chart_toggle_key_is_default_unbound_configurable_and_escape_owned() {
    assert!(RuntimeKeyConfig::default().chart_toggle.is_empty());
    let parsed =
        parse_runtime_key_config(b"[Keys]\nChartToggle=F8\n[Keys]\nChartToggle=F7\n").test_value();
    runtime_assert_eq!(parsed.chart_toggle => vec![RuntimeKeyChord::keyboard(VirtualKeyCode::F8, ModifiersState::empty(),)], "StdCompilerINIRead keeps the first action value");

    let mut app = new_running_sandbox_app();
    install_runtime_key_config(&mut app, Ok(parsed));
    app.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(app.network_chart_dialog.is_some());
    app.test_key(VirtualKeyCode::F8, ElementState::Released);

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    assert!(app.network_chart_dialog.is_none());
    runtime_assert!(
        app.message_dialogs.is_empty(),
        "chart Escape must not also open the abort dialog"
    );
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);

    tap_runtime_key(&mut app, VirtualKeyCode::F8);

    runtime_assert!(
        !app.handle_network_chart_key(VirtualKeyCode::ArrowUp, ElementState::Pressed),
        "the non-exclusive chart must not invent GUI-scope arrow navigation"
    );
    runtime_assert_eq!(app.network_chart_dialog.as_ref().expect("chart remains open").active_tab_index() => 0);

    app.start_running_chat(RunningChatMode::All);
    assert!(app.running_chat_active());
    tap_runtime_key(&mut app, VirtualKeyCode::Escape);
    assert!(app.running_chat_controller().is_none());
    runtime_assert!(
        app.network_chart_dialog.is_some(),
        "closing foreground chat must retain the background chart"
    );
    tap_runtime_key(&mut app, VirtualKeyCode::Escape);
    assert!(app.network_chart_dialog.is_none());
    assert!(app.message_dialogs.is_empty());

    tap_runtime_key(&mut app, VirtualKeyCode::F8);
    app.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(app.network_chart_dialog.is_none());

    let mut priority = new_running_sandbox_app();
    install_runtime_key_config(
        &mut priority,
        Ok(parse_runtime_key_config(b"[Keys]\nChartToggle=F2\n").test_value()),
    );
    priority.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    assert!(priority.running_chat_active());
    assert!(priority.network_chart_dialog.is_none());

    let mut remapped_priority = new_running_sandbox_app();
    install_runtime_key_config(
        &mut remapped_priority,
        Ok(parse_runtime_key_config(b"[Keys]\nChatOpen=F8\nChartToggle=F8\n").test_value()),
    );
    remapped_priority.test_key(VirtualKeyCode::F8, ElementState::Pressed);
    assert!(remapped_priority.running_chat_active());
    assert!(remapped_priority.network_chart_dialog.is_none());
}

#[test]
fn running_script_uses_symbolic_console_strictness_and_frozen_sync() {
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    let mut config = Config::new();
    config.set_in(Some("Developer"), "ConsoleScriptStrictness", "Strict1");
    config.save(paths.config_file()).test_value();

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
fn runtime_key_config_compiles_lists_modifiers_raw_joy_and_disable_codes() {
    let parsed = parse_runtime_key_config(
                b"[Keys]\nNetObsNextPlayer=F5\nChatOpen=Ctrl+Shift+F2,Return\nScoreboardToggle=None\nGameAbort=Joy2A\nKbd1Key1=\\x0042010a\nUnknownAction=F9\n[Keys]\nNetObsNextPlayer=F6\n",
            ).test_value();
    runtime_assert_eq!(
        parsed.net_observer_next_player => vec![RuntimeKeyChord::keyboard(VirtualKeyCode::F5, ModifiersState::empty(),)];
        parsed.override_for("ChatOpen") =>
            Some([RuntimeKeyChord::keyboard(VirtualKeyCode::F2, ModifiersState::CONTROL | ModifiersState::SHIFT,), RuntimeKeyChord::keyboard(VirtualKeyCode::Enter, ModifiersState::empty(),),].as_slice());
        parsed.override_for("ScoreboardToggle").unwrap()[0].physical => RuntimePhysicalKey::Disabled;
    );
    runtime_assert_eq!(parsed.override_for("GameAbort").unwrap()[0].physical => RuntimePhysicalKey::Gamepad { slot: 1, button: 1 }, "the first sscanf Joy branch owns every canonical JoyN suffix");
    runtime_assert_eq!(parsed.override_for("Kbd1Key1").unwrap()[0].physical => RuntimePhysicalKey::Gamepad { slot: 1, button: 10 });
    assert!(parsed.override_for("UnknownAction").is_none());

    let unknown = parse_runtime_key_config(b"[Keys]\nNetObsNextPlayer=F01\n").test_value();
    runtime_assert_eq!(unknown.net_observer_next_player[0].physical => RuntimePhysicalKey::Disabled);

    let partial = parse_runtime_key_config(
        b"[Keys] ; comment\nChatOpen=CapsLock,F2 ; trailing,Bogus+Q\nGameAbort=Keypad Enter\n",
    )
    .test_value();
    runtime_assert_eq!(
        partial.override_for("ChatOpen") =>
            Some([RuntimeKeyChord::keyboard(VirtualKeyCode::CapsLock, ModifiersState::empty(),), RuntimeKeyChord::keyboard(VirtualKeyCode::F2, ModifiersState::empty(),),].as_slice());
    );
    runtime_assert!(
        partial.override_for("GameAbort").is_none(),
        "the corrupt lexicographically earlier registration aborts later compilation"
    );
    let keypad = parse_runtime_key_config(b"[Keys]\nGameAbort=Keypad Enter\n").test_value();
    runtime_assert_eq!(keypad.override_for("GameAbort").unwrap()[0].physical => RuntimePhysicalKey::Keyboard(VirtualKeyCode::NumpadEnter));
    let lowercase_keypad = parse_runtime_key_config(b"[Keys]\nGameAbort=keypad 1\n").test_value();
    runtime_assert_eq!(lowercase_keypad.override_for("GameAbort").unwrap()[0].physical => RuntimePhysicalKey::Keyboard(VirtualKeyCode::Numpad1));

    let caps_raw = input::encode_virtual_key_code(VirtualKeyCode::CapsLock).test_value();
    let raw_caps = format!("[Keys]\nToggleChat=\\x{caps_raw:x}\n");
    let raw_caps = parse_runtime_key_config(raw_caps.as_bytes()).test_value();
    runtime_assert_eq!(raw_caps.override_for("ToggleChat").unwrap()[0].physical => RuntimePhysicalKey::Keyboard(VirtualKeyCode::CapsLock));

    let noncanonical = parse_runtime_key_config(b"[Keys]\nKbd01Key01=F2\n").test_value();
    assert!(noncanonical.override_for("Kbd01Key01").is_none());
}

#[test]
fn ownerless_arrow_scroll_carries_momentum_without_player_mutation() {
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
    app.film_view_player = Some(owner);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);

    let initial = app.graphics.active_viewport_projections()[0];
    assert_eq!(initial.owner, owner);
    assert!(initial.is_no_owner_viewport);
    assert!(app.primary_physical_viewport_is_no_owner());
    let players_before = app.engine.snapshot().players;

    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    let production_left = app.graphics.active_viewport_projections()[0];
    assert_eq!(production_left.target_x, initial.target_x - 5);
    assert_eq!(production_left.target_y, initial.target_y);
    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Released);
    runtime_assert_eq!(app.graphics.active_viewport_projections()[0].target_x => production_left.target_x);

    app.free_view_scroll_momentum = FreeViewScrollMomentum::default();
    let start = app.graphics.active_viewport_projections()[0];
    let now = Instant::now();
    runtime_assert!(app.handle_viewport_player_cycle_key_at(
        VirtualKeyCode::ArrowLeft,
        ElementState::Pressed,
        now,
    ));
    let first_left = app.graphics.active_viewport_projections()[0];
    assert_eq!(first_left.target_x, start.target_x - 5);
    assert_eq!(first_left.target_y, start.target_y);

    runtime_assert!(!app.handle_viewport_player_cycle_key_at(
        VirtualKeyCode::ArrowLeft,
        ElementState::Released,
        now + Duration::from_millis(25),
    ));
    assert_eq!(app.graphics.active_viewport_projections()[0], first_left);

    runtime_assert!(app.handle_viewport_player_cycle_key_at(
        VirtualKeyCode::ArrowLeft,
        ElementState::Pressed,
        now + Duration::from_millis(50),
    ));
    let second_left = app.graphics.active_viewport_projections()[0];
    assert_eq!(second_left.target_x, start.target_x - 15);
    assert_eq!(second_left.target_y, start.target_y);

    runtime_assert!(app.handle_viewport_player_cycle_key_at(
        VirtualKeyCode::ArrowUp,
        ElementState::Pressed,
        now + Duration::from_millis(75),
    ));
    let cross_axis = app.graphics.active_viewport_projections()[0];
    assert_eq!(cross_axis.target_x, start.target_x - 25);
    assert_eq!(cross_axis.target_y, start.target_y - 5);

    runtime_assert!(app.handle_viewport_player_cycle_key_at(
        VirtualKeyCode::ArrowRight,
        ElementState::Pressed,
        now + Duration::from_millis(175),
    ));
    let reset_right = app.graphics.active_viewport_projections()[0];
    assert_eq!(reset_right.target_x, start.target_x - 20);
    assert_eq!(reset_right.target_y, start.target_y - 5);

    runtime_assert!(app.handle_viewport_player_cycle_key_at(
        VirtualKeyCode::ArrowDown,
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
    owned.test_render(&mut owned_frame);
    assert!(!owned.primary_physical_viewport_is_no_owner());
    let owned_camera = owned.graphics.active_viewport_projections()[0];
    owned
        .engine
        .test_player_mut(owned.local_owner)
        .control
        .control_style = true;
    for (binding, key, command) in [
        (
            ControlBindingId::Left,
            VirtualKeyCode::ArrowLeft,
            clonk_engine::COM_LEFT,
        ),
        (
            ControlBindingId::Right,
            VirtualKeyCode::ArrowRight,
            clonk_engine::COM_RIGHT,
        ),
        (
            ControlBindingId::Up,
            VirtualKeyCode::ArrowUp,
            clonk_engine::COM_UP,
        ),
        (
            ControlBindingId::Down,
            VirtualKeyCode::ArrowDown,
            clonk_engine::COM_DOWN,
        ),
    ] {
        owned.bindings.rebind(binding, key);
        owned.test_key(key, ElementState::Pressed);
        runtime_assert_ne!(owned.engine.player(owned.local_owner).expect("local player").control.pressed_coms & (1 << command) => 0);
        owned.test_key(key, ElementState::Released);
        runtime_assert_eq!(owned.engine.player(owned.local_owner).expect("local player").control.pressed_coms & (1 << command) => 0);
    }
    let owned_after = owned.graphics.active_viewport_projections()[0];
    runtime_assert_eq!((owned_after.target_x, owned_after.target_y) => (owned_camera.target_x, owned_camera.target_y));
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
        [clonk_engine::PlayerInfoControlData::new(
            0,
            0,
            vec![clonk_engine::ControlPlayerInfoEntry {
                id: 1,
                ..Default::default()
            }],
            -1,
        )],
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
        .test_value();

    app.engine
        .call_scenario_script_function("RejectLimitAndSpawn", Vec::new())
        .test_value();

    let globals = app.engine.snapshot().script_globals.named;
    runtime_assert_eq!(globals.get("set_result") => Some(&Value::Int(0)), "FnSetMaxPlayer has the C4ValueInt false result");
    assert_eq!(app.engine.max_players(), Some(1));

    app.handle_script_player_info_updates().test_value();

    assert_eq!(app.network_max_players, 1);
    runtime_assert!(
        app.control_player_infos
            .client_info_ids(0)
            .into_iter()
            .filter_map(|id| app.control_player_infos.get(id))
            .all(|info| info.name.as_bytes() != b"Rejected Bot"),
        "the unchanged full cap rejects the PlayerInfo",
    );
    runtime_assert!(
        app.engine
            .snapshot()
            .players
            .iter()
            .all(|player| player.name != "Rejected Bot"),
        "a rejected PlayerInfo cannot reach JoinPlayer"
    );
}

#[test]
fn retargeted_primary_survives_its_original_local_player() {
    let mut app = new_lightweight_running_sandbox_app();
    let original = app.local_owner;
    let target = original + 1;
    app.engine
        .register_player(PlayerConfig::new(target, "Film target"))
        .test_value();
    let target_control = app
        .local_controls
        .initialize(test_local_control_init(target, 1, false, false));
    app.engine
        .set_player_runtime_control(target, target_control.runtime_control())
        .test_value();
    app.engine.set_local_players([original, target]);
    app.engine
        .replace_player_viewports(
            original,
            vec![clonk_engine::PlayerViewport::new(Vector2::new(300, 180)).with_zoom(1.75)],
        )
        .test_value();
    app.snapshot = app.engine.snapshot();
    let _ = app.create_physical_viewport(target, false, true, true);
    app.engine.clear_scenario_script();
    app.engine.install_scenario_script_with_convention("PhysicalViewport.c",
    &format!(
        "#strict 3\nfunc Probe() {{ SetViewOffset({original}, 17, 19); SetFilmView({target}); SetViewOffset({original}, 91, 92); }}"
    ),
    true,).test_value();
    app.engine.set_replay_control(true);
    app.engine
        .call_scenario_script_function("Probe", Vec::new())
        .test_value();
    let _ = app.apply_pending_viewport_presentation_requests();
    runtime_assert_eq!(
        app.physical_viewports.iter().map(|viewport| viewport.displayed_player).collect::<Vec<_>>() => vec![target, target];
        app.physical_viewports[0].preserved_zoom => 1.75;
        app.physical_viewports[0].preserved_offset => Vector2::new(17, 19);
    );

    app.ui_sound_log.clear();
    app.remove_runtime_player_with_viewport_feedback(original)
        .test_value();
    assert_eq!(app.physical_viewports.len(), 2);
    runtime_assert!(app
        .physical_viewports
        .iter()
        .all(|viewport| viewport.displayed_player == target));
    assert!(app.ui_sound_log.is_empty(), "CloseViewport(A) matches none");

    app.snapshot = app.engine.snapshot();
    let rendered =
        collect_viewport_inputs_from_physical_state(&app.snapshot, &app.physical_viewports)
            .test_value();
    assert_eq!(rendered.len(), 2);
    assert!(rendered.iter().all(|viewport| viewport.owner == target));
    assert_eq!(rendered[0].zoom, 1.75);
    assert_eq!(rendered[0].offset, Vector2::new(17, 19));

    app.remove_runtime_player_with_viewport_feedback(target)
        .test_value();
    runtime_assert_eq!(app.ui_sound_log.iter().filter(|sound| sound.as_str() == "CloseViewport").count() => 1, "closing both matching physical viewports requests one sound");
    assert_eq!(app.physical_viewports.len(), 1);
    assert!(app.physical_viewports[0].is_no_owner_viewport);
}

// Two console/fullscreen asymmetries that a port loses by sharing one
// creation helper between them.
//
// `C4Console::ViewportNew` is just `Game.CreateViewport(NO_OWNER)`
// (C4Console.cpp:1203-1206), and `fSilent` defaults to false
// (C4Game.h:222) — so the console's *ownerless* viewport announces itself,
// where `C4FullScreen::ViewportCheck` explicitly passes
// `iPlrNum == NO_OWNER` to silence exactly that case
// (C4FullScreen.cpp:517). The per-player console rows default the same way
// (C4Console.cpp:223, :1828).
//
// And `C4GraphicsSystem::RecalculateViewports` opens with
// `if (!Application.isFullScreen) return;` (C4GraphicsSystem.cpp:335-336),
// before `SortViewportsByPlayerControl()` at :339 — so a console viewport
// never reorders the list it joins, however the players are controlled.
#[test]
fn console_viewport_creation_announces_itself_and_keeps_list_order() {
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    // Layout order 3 then 1 (C4Console-side control sets 1 and 2), so a
    // fullscreen sort would swap them and a console one must not.
    let late_layout = app.local_owner + 1;
    let early_layout = app.local_owner + 2;
    for (player, name, control_set) in [
        (late_layout, "Late layout", 1),
        (early_layout, "Early layout", 2),
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

    app.ui_sound_log.clear();
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::NewViewport(None)])
        .test_value();
    runtime_assert_eq!(app.ui_sound_log => ["CloseViewport"], "the console's ownerless viewport is not silent");

    let before = app
        .physical_viewports
        .iter()
        .map(|viewport| viewport.displayed_player)
        .collect::<Vec<_>>();
    // One menu activation per dispatch, as the console produces them.
    for player in [late_layout, early_layout] {
        app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::NewViewport(Some(
            player,
        ))])
        .test_value();
    }
    let mut expected = before;
    expected.extend([late_layout, early_layout]);
    runtime_assert_eq!(app.physical_viewports.iter().map(|viewport| viewport.displayed_player).collect::<Vec<_>>() => expected, "console mode never runs SortViewportsByPlayerControl");
}

// C4Viewport.cpp:1126-1155 — a windowed viewport draws the one viewport
// its window owns, at that window's own extent, and hands the pixels back
// for the window to blit (`BlitOutput`, :1121-1124).
#[test]
fn console_viewport_render_uses_the_windows_own_extent_and_identity() {
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    let second = app.local_owner + 1;
    app.engine
        .register_player(PlayerConfig::new(second, "Second window"))
        .test_value();
    app.snapshot = app.engine.snapshot();
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::NewViewport(None)])
        .test_value();
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::NewViewport(Some(second))])
        .test_value();

    let identities = app
        .physical_viewports
        .iter()
        .map(|viewport| viewport.physical_identity)
        .collect::<Vec<_>>();
    assert!(identities.len() >= 2);

    // Every live viewport draws, each into its own window extent — the
    // windows are deliberately different sizes so a shared surface or a
    // fullscreen layout split would show up here.
    for (index, identity) in identities.iter().enumerate() {
        let width = 320 + index as u32 * 16;
        let height = 200 + index as u32 * 8;
        let surface = app
            .render_console_viewport(*identity, width, height)
            .unwrap_or_else(|| panic!("viewport {identity} is live"));
        assert_eq!((surface.width(), surface.height()), (width, height));
    }

    // A closed viewport's window goes blank rather than adopting the
    // remaining viewport's view.
    let closed = identities[0];
    app.physical_viewports
        .retain(|viewport| viewport.physical_identity != closed);
    assert!(app.render_console_viewport(closed, 320, 200).is_none());
    assert!(app.render_console_viewport(u64::MAX, 320, 200).is_none());
}

#[test]
fn console_viewport_render_applies_the_live_pxs_graphics_flag() {
    // A windowed viewport still reaches the same C4PXSSystem::Draw flag
    // (src/C4GraphicsSystem.cpp:167-169; src/C4PXS.cpp:259-260,279-281).
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    let identity = open_test_console_viewport(&mut app, None);
    app.display_flags.pxs_gfx = false;
    assert!(app.graphics.pxs_graphics_enabled());

    assert!(app.render_console_viewport(identity, 320, 200).is_some());

    runtime_assert!(
        !app.graphics.pxs_graphics_enabled(),
        "the detached viewport must honor the same live flag as the main PXS draw"
    );
}

#[test]
fn console_shell_render_applies_the_live_pxs_graphics_flag() {
    // C4GraphicsSystem::Execute renders every viewport before its
    // fullscreen-only chrome, so console and fullscreen draws reach the same
    // global PXSGfx check
    // (src/C4GraphicsSystem.cpp:167-177; src/C4PXS.cpp:259-260,279-281).
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.display_flags.pxs_gfx = false;
    assert!(app.graphics.pxs_graphics_enabled());
    let frame_len = app.graphics.surface().pixels().len();
    let mut frame = vec![0; frame_len];

    assert!(app.test_render(&mut frame));

    runtime_assert!(
        !app.graphics.pxs_graphics_enabled(),
        "the console shell must synchronize the flag before its early return"
    );
}

// The editor's first end-to-end gesture: a window-local click reaches the
// selection. C4Viewport converts the pointer through that viewport's own
// ViewX/ViewY (C4Viewport.cpp:181), C4EditCursor::Move picks the target
// with Game.FindObject (C4EditCursor.cpp:150), and LeftButtonDown edits
// the selection (:201-229).
// C4Viewport.cpp:1107 — Console.EditCursor.Draw runs inside the viewport
// pass, so a selection mark is part of the frame the window blits. This is
// the mark reaching pixels, not just the geometry being right.
#[test]
fn a_selected_object_draws_its_mark_into_the_viewport_frame() {
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Edit;
    // An owned viewport follows the player, so the crew object it follows
    // is inside the view — an ownerless one is centred on the map and the
    // mark would legitimately fall outside it.
    let identity = open_local_test_console_viewport(&mut app);

    let unmarked = app.render_console_viewport(identity, 320, 200).test_value();
    let projection = app.console_viewport_projections[&identity];

    // Select whatever sits under the view's own centre.
    let object = app
        .snapshot
        .objects
        .iter()
        .find(|object| {
            let x = object.position.x - projection.target_x;
            let y = object.position.y - projection.target_y;
            (0..320).contains(&x) && (0..200).contains(&y)
        })
        .map(|object| object.id)
        .test_value();
    app.developer_selection.replace(
        clonk_engine::developer_selection::SelectionWriter::EditCursor,
        object,
    );

    let marked = app.render_console_viewport(identity, 320, 200).test_value();
    runtime_assert_ne!(marked.pixels() => unmarked.pixels(), "the select mark is drawn into the frame");

    // And it goes away again, so the difference is the mark and not some
    // unrelated per-frame drift.
    app.developer_selection
        .clear(clonk_engine::developer_selection::SelectionWriter::EditCursor);
    let cleared = app.render_console_viewport(identity, 320, 200).test_value();
    runtime_assert_eq!(cleared.pixels() => unmarked.pixels(), "clearing the selection restores the unmarked frame");
}

// C4Game.cpp:2413-2424,2738 + :2306-2320 — the monitor arms only in a
// windowed dev session, registers before it starts, and its callback is
// bound straight to ReloadFile's dispatcher.
#[test]
fn developer_file_monitor_arms_registers_then_dispatches_definition_reloads() {
    let dir = tempfile::tempdir().test_value();
    let group = dir.path().join("Rock.c4d");
    std::fs::create_dir_all(&group).test_value();
    std::fs::write(
        group.join("DefCore.txt"),
        "[DefCore]\nid=ROCK\nVersion=4,9,8\nName=Rock\n",
    )
    .test_value();

    let mut app = new_lightweight_running_sandbox_app();
    let mut definition =
        clonk_engine::Definition::from_script("ROCK".to_string(), "Rock".to_string(), "")
            .test_value();
    definition.set_source_path(Some(group.clone()));
    app.engine.register_test_definition(definition);

    // A fullscreen session never watches, however the key is set.
    app.console_mode = false;
    app.arm_developer_file_monitor(true);
    assert!(app.file_monitor.is_none());

    // Nor does a console session with the key off.
    app.console_mode = true;
    app.arm_developer_file_monitor(false);
    assert!(app.file_monitor.is_none());

    app.arm_developer_file_monitor(true);
    let monitor = app.file_monitor.test_ref();
    assert_eq!(monitor.watched(), std::slice::from_ref(&group));
    runtime_assert!(
        monitor.started(),
        "registration closes before the first poll"
    );

    // Arming twice does not replace a running monitor.
    app.arm_developer_file_monitor(true);
    runtime_assert_eq!(app.file_monitor.as_ref().expect("still armed").watched() => std::slice::from_ref(&group));

    // A quiet tree dispatches nothing.
    app.poll_developer_file_monitor();
    assert!(app.engine.definition("ROCK").is_some());

    // Breaking the group and touching it routes to ReloadDef, whose
    // failure arm removes the definition.
    std::fs::write(group.join("DefCore.txt"), "not a defcore").test_value();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let _ = std::fs::File::open(group.join("DefCore.txt")).map(|file| file.set_modified(later));
    app.poll_developer_file_monitor();
    runtime_assert!(
        app.engine.definition("ROCK").is_none(),
        "a watched change reached ReloadDef and its failure arm"
    );
}

#[test]
fn console_viewport_pointer_gestures_select_move_and_frame() {
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Edit;
    let identity = open_test_console_viewport(&mut app, None);
    // Drawing is what publishes this window's own projection.
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    let projection = app.console_viewport_projections[&identity];

    let subject = app.snapshot.objects.first().test_value();
    let (id, position) = (subject.id, subject.position);
    let local = (
        position.x - projection.target_x,
        position.y - projection.target_y,
    );

    runtime_assert_eq!(
        app.console_viewport_press(identity, local, 1.0, false, false).expect("the click changed the selection").objects => vec![id],
        "a plain click selects the object under the cursor",
    );
    assert!(app.edit_cursor_hold, "a press always holds");

    // A held drag over a selected object moves it: C4EditCursor::Move's
    // Edit arm sends MoveSelection(xoff, yoff), and MoveSelection is
    // EMMoveObject(EMMO_Move, ...) — a control, not a direct mutation, so
    // a network game stays in lockstep.
    // Offline the control is applied straight away, so the object itself
    // is the observable: it moves by the pointer delta.
    let before = app.engine.snapshot().object(id).test_value().position;
    app.console_viewport_motion(identity, (local.0 + 7, local.1 - 3), 1.0, false, false);
    let after = app.engine.snapshot().object(id).test_value().position;
    assert_ne!(before, after, "a held drag moves the selection");

    // A motion that does not move the pointer emits nothing: the
    // zero-offset re-issue is `Execute`'s per-tick path, not this one.
    let settled = app.engine.snapshot().object(id).test_value().position;
    app.console_viewport_motion(identity, (local.0 + 7, local.1 - 3), 1.0, false, false);
    runtime_assert_eq!(app.engine.snapshot().object(id).map(|object| object.position) => Some(settled), "a zero-delta motion emits no move");

    // The mark frames the object's *live* shape, which
    // `ObjectSnapshot::current_shape` carries only when it is not
    // reconstructible. Resolving it through the same world view the hit
    // test uses is what makes the two agree about what was clicked.
    let shape = clonk_engine::EditCursorHitTest::new(&app.snapshot)
        .shape_rect(id)
        .test_value();
    runtime_assert!(
        shape.width > 0 && shape.height > 0,
        "a clickable object has a shape to frame"
    );

    // `DrawSelectMark` frames `cobj->x + cobj->Shape.x` relative to the
    // view origin. `object_live_shape_rect` already returns that left-hand
    // side in world coordinates, so the mark is the shape minus ViewX/ViewY
    // — adding the object's position again would double-count it, which is
    // exactly the bug this pins.
    runtime_assert_eq!((shape.x, shape.y) => (position.x, position.y), "the live shape rect is in world coordinates, not object-relative");

    // Drawn into a viewport whose origin is the object's own position, the
    // mark lands at the frame origin rather than one object-width away.
    let marks = clonk_engine::developer_overlay::select_mark_pixels(
        shape.x - position.x,
        shape.y - position.y,
        shape.width,
        shape.height,
    );
    assert!(!marks.is_empty(), "a shape at least a pixel wide marks");
    // The corner Ls point outward, so they reach one pixel beyond the
    // shape on each side — but no further. This is what catches a mark
    // computed an object-width away from where it belongs.
    runtime_assert!(
        marks
            .iter()
            .all(|(x, y)| (-1..=shape.width).contains(x) && (-1..=shape.height).contains(y)),
        "the mark frames the shape it belongs to: {marks:?}"
    );

    // Clicking the same object again changes nothing, which is what keeps
    // a selection draggable rather than collapsing it.
    runtime_assert!(app
        .console_viewport_press(identity, local, 1.0, false, false)
        .is_none());

    // A plain click on empty space clears and arms the rubber band, and
    // the anchor is in world coordinates.
    let empty = (local.0 + 100_000, local.1 + 100_000);
    runtime_assert!(app
        .console_viewport_press(identity, empty, 1.0, false, false)
        .expect("clearing the selection is a change")
        .objects
        .is_empty());
    // `DragFrame = true; X2 = X; Y2 = Y` — both corners start at the press.
    let world_empty = (projection.target_x + empty.0, projection.target_y + empty.1);
    assert_eq!(app.edit_cursor_drag_frame, Some((world_empty, world_empty)));

    // `C4EditCursor::Execute` re-issues a zero-offset EMMO_Move every
    // tick while Hold is set (C4EditCursor.cpp:65-69), so a stationary
    // held selection still produces control traffic — but once per engine
    // tick, not once per event-loop wake.
    assert!(app.edit_cursor_hold);
    app.console_edit_cursor_tick();
    let ticked = app.edit_cursor_tick_frame;
    assert!(ticked.is_some(), "a held selection ticks");
    app.console_edit_cursor_tick();
    runtime_assert_eq!(app.edit_cursor_tick_frame => ticked, "a second wake in the same tick emits nothing further");

    // A rubber band drawn over the object frames it on release.
    // C4EditCursor::LeftButtonUp runs FrameSelection() then clears Hold and
    // DragFrame regardless (C4EditCursor.cpp:287-341).
    let corner = (local.0 + 40, local.1 + 40);
    app.console_viewport_motion(identity, corner, 1.0, false, false);
    let (anchor, live) = app.edit_cursor_drag_frame.test_value();
    runtime_assert_eq!(live => (projection.target_x + corner.0, projection.target_y + corner.1), "the band's live corner follows the pointer");
    assert_ne!(anchor, live, "the anchor stays at the press");

    // Drag the band back so it spans the object, then release.
    app.console_viewport_motion(identity, (local.0 - 40, local.1 - 40), 1.0, false, false);
    let framed = app.console_viewport_release().test_value();
    runtime_assert!(
        framed.objects.contains(&id),
        "an object inside the band is framed: {:?}",
        framed.objects
    );
    assert!(!app.edit_cursor_hold, "the release always clears Hold");
    runtime_assert!(
        app.edit_cursor_drag_frame.is_none(),
        "the release always clears DragFrame"
    );

    // Play mode is ordinary mouse control, not the editor sink.
    app.developer_console_edit_mode = ConsoleEditMode::Play;
    runtime_assert!(app
        .console_viewport_press(identity, local, 1.0, false, false)
        .is_none());
}

// C4EditCursor.cpp:244-274,332-340,350-359,376-380,582-628,640-651 — the
// viewport context menu and the three object commands it is the only way
// to reach. Their executors and wire codecs were landed and pinned long
// before anything could emit one.
#[test]
fn console_viewport_context_menu_emits_the_object_commands() {
    use clonk_frontend::developer_context_menu::{ViewportContextEntry, ViewportContextItem};

    let (mut app, _events, mut commands, identity) =
        runtime_console_network_fixture(ConsoleEditMode::Edit);
    // Drawing is what publishes this window's own projection.
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    let projection = app.console_viewport_projections[&identity];

    let subject = app.snapshot.objects.first().test_value();
    let (id, position) = (subject.id, subject.position);
    let local = (
        position.x - projection.target_x,
        position.y - projection.target_y,
    );
    let empty = (2, 2);

    // `RightButtonDown` off the selection and over an object selects it,
    // exactly as a plain left click would.
    runtime_assert_eq!(app.console_viewport_right_press(identity, local, 1.0, false).expect("the right press changed the selection").objects => vec![id]);
    // A second right press *on* that selection leaves it alone — this is
    // `fCursorIsOnSelection`, and it is what lets a multi-object selection
    // survive the click that opens the menu.
    runtime_assert!(app
        .console_viewport_right_press(identity, local, 1.0, false)
        .is_none());

    // `RightButtonUp` opens the menu. A selected object with no contents
    // greys Grab contents and nothing else.
    app.open_console_viewport_context_menu(identity, local);
    let (open, menu) = app.console_viewport_context_menu.test_ref();
    assert_eq!(*open, identity);
    let live = |menu: &clonk_frontend::developer_context_menu::ViewportContextMenu| {
        menu.entries()
            .iter()
            .filter_map(|entry| match entry {
                ViewportContextEntry::Item {
                    item,
                    enabled: true,
                    ..
                } => Some(*item),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    runtime_assert_eq!(live(menu) => vec![ViewportContextItem::Delete, ViewportContextItem::Duplicate, ViewportContextItem::Properties,], "an empty container greys Grab contents alone");
    // The popup is drawn onto the viewport's own frame, so rendering it
    // must stay clean at the extent the window presents.
    assert!(app.render_console_viewport(identity, 320, 200).is_some());

    // Choosing Duplicate emits `EMMoveObject(EMMO_Duplicate, 0, 0,
    // nullptr, &Selection)` and closes the menu.
    let rows = app
        .console_viewport_context_menu
        .test_ref()
        .1
        .layout(320, 200);
    let center = |index: usize| {
        let rect = rows[index].rect;
        (rect.x + rect.w / 2, rect.y + rect.h / 2)
    };
    assert!(app.console_viewport_context_menu_click(identity, center(1), (320, 200)));
    runtime_assert!(
        app.console_viewport_context_menu.is_none(),
        "a chosen item closes the menu"
    );
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmMoveObject(duplicate), false)] = decided.as_slice()
    else {
        panic!("expected one duplicate control, got {decided:?}");
    };
    assert_eq!(duplicate.action, clonk_engine::EMMO_DUPLICATE);
    assert_eq!(duplicate.objects, vec![id.as_u64() as i32]);
    assert_eq!(duplicate.by_client, 7);

    // Delete is the same shape under `EMMO_Remove`.
    app.open_console_viewport_context_menu(identity, local);
    assert!(app.console_viewport_context_menu_click(identity, center(0), (320, 200)));
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmMoveObject(remove), false)] = decided.as_slice() else {
        panic!("expected one remove control, got {decided:?}");
    };
    assert_eq!(remove.action, clonk_engine::EMMO_REMOVE);
    assert_eq!(remove.objects, vec![id.as_u64() as i32]);

    // A disabled row swallows its click and the menu *stays up*, as a
    // grayed Win32 item and an insensitive GTK one both do — only a click
    // outside cancels. Grab contents is grey while the selection holds
    // nothing.
    app.open_console_viewport_context_menu(identity, local);
    assert!(app.console_viewport_context_menu_click(identity, center(2), (320, 200)));
    runtime_assert!(
        app.console_viewport_context_menu.is_some(),
        "a greyed row does not dismiss the menu"
    );
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a greyed item runs nothing"
    );
    runtime_assert!(app.console_viewport_context_menu_click(
        identity,
        (rows[0].rect.x - 4, rows[0].rect.y - 4),
        (320, 200)
    ));
    runtime_assert!(
        app.console_viewport_context_menu.is_none(),
        "a click outside cancels it"
    );

    // Escape closes the popup without choosing anything, and only the
    // viewport that owns it can be closed out from under it.
    app.open_console_viewport_context_menu(identity, local);
    assert!(!app.dismiss_console_viewport_context_menu_for(identity ^ 0xff));
    assert!(app.dismiss_console_viewport_context_menu_for(identity));
    assert!(!app.dismiss_console_viewport_context_menu());
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a dismissed menu runs nothing"
    );

    // A right-click on empty space drops the selection, which greys every
    // object command and leaves Properties as the only live row.
    app.console_viewport_right_press(identity, local, 1.0, false);
    runtime_assert!(app
        .console_viewport_right_press(identity, empty, 1.0, false)
        .is_some_and(|snapshot| snapshot.objects.is_empty()));
    app.open_console_viewport_context_menu(identity, empty);
    runtime_assert_eq!(live(&app.console_viewport_context_menu.as_ref().expect("the menu opened over nothing").1) => vec![ViewportContextItem::Properties]);
}

// C4Console.cpp:1328-1351 and C4ComponentHost.cpp:231-236,330-334 — the
// Script/Title/Info editors. Their commit rules were ported and pinned
// long before anything could open one.
#[test]
fn developer_component_editors_commit_accept_and_cancel_like_the_native_host() {
    use clonk_engine::developer_components::{ComponentSaveAction, EditableComponent};

    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;

    // The scenario the components are read from and written back to.
    let directory = tempfile::tempdir().test_value();
    let scenario = directory.path().join("Round.c4s");
    std::fs::create_dir(&scenario).test_value();
    std::fs::write(scenario.join("Title.txt"), "Round\n").test_value();
    app.active_scenario.test_mut().path = Some(scenario.clone());

    // Title opens on the component's own bytes.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditTitle])
        .test_value();
    let edit = app.developer_component_editor.test_ref();
    assert_eq!(edit.component, EditableComponent::Title);
    assert_eq!(edit.host.filename(), "Title.txt");
    assert_eq!(edit.text.lines(), ["Round", ""]);

    // Cancel mutates nothing — not the bytes and not the modified flag —
    // so the component contributes nothing to a save.
    app.cancel_developer_component_editor();
    assert!(app.developer_component_editor.is_none());
    assert!(app.developer_component_hosts.is_empty());

    // A component that does not exist yet opens empty rather than
    // refusing: that is how a scenario grows one.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditInfo])
        .test_value();
    let edit = app.developer_component_editor.test_mut();
    assert_eq!(edit.text.lines(), [""]);
    for character in "hello".chars() {
        edit.text.insert(character);
    }
    app.commit_developer_component_editor();
    assert!(app.developer_component_editor.is_none());
    let [host] = app.developer_component_hosts.as_slice() else {
        panic!("expected one committed host");
    };
    runtime_assert_eq!(host.save_action() => ComponentSaveAction::Write { filename: "Info.txt".to_owned(), data: b"hello".to_vec(), });

    // Emptying a component deletes it rather than writing zero bytes.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditTitle])
        .test_value();
    let edit = app.developer_component_editor.test_mut();
    for _ in 0..16 {
        edit.text
            .key(crate::developer_component_editor::ComponentEditorKey::Delete);
    }
    app.commit_developer_component_editor();
    runtime_assert_eq!(app.developer_component_hosts.last().expect("the emptied host").save_action() => ComponentSaveAction::Delete { filename: "Title.txt".to_owned(), });

    // Reopening a component edited earlier this round shows **its** bytes,
    // not the stale ones still on disk — C++ never has to think about
    // this because its hosts are live for the whole round. And the second
    // commit replaces the first rather than queueing a second write of
    // the same filename.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditInfo])
        .test_value();
    let edit = app.developer_component_editor.test_mut();
    assert_eq!(edit.text.lines(), ["hello"], "the committed bytes reopen");
    edit.text
        .key(crate::developer_component_editor::ComponentEditorKey::End);
    edit.text.insert('!');
    app.commit_developer_component_editor();
    runtime_assert_eq!(app.developer_component_hosts.iter().filter(|host| host.filename() == "Info.txt").count() => 1, "one host per component, however many times it was edited");
    runtime_assert_eq!(
        app.developer_component_hosts.iter().find(|host| host.filename() == "Info.txt").expect("the info host").save_action() =>
            ComponentSaveAction::Write { filename: "Info.txt".to_owned(), data: b"hello!".to_vec(), };
    );

    // A second editor cannot open over the first: `ShowDialog` is modal,
    // and letting one would discard whatever was being typed.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditTitle])
        .test_value();
    let open = app.developer_component_editor.test_ref().component;
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditInfo])
        .test_value();
    runtime_assert_eq!(app.developer_component_editor.as_ref().expect("the first editor survives").component => open, "the open editor is not replaced");
    app.cancel_developer_component_editor();

    // A network game refuses all three outright.
    let (_events, _commands) = install_running_network_stub(&mut app, 7, 0, 2);
    app.developer_console.clear_log();
    for action in [
        DeveloperConsoleAction::EditScript,
        DeveloperConsoleAction::EditTitle,
        DeveloperConsoleAction::EditInfo,
    ] {
        app.dispatch_developer_console_actions(vec![action])
            .test_value();
        runtime_assert!(
            app.developer_component_editor.is_none(),
            "a network game opens no component editor"
        );
    }
    assert!(!app.developer_console.log().text().is_empty());

    // Closing the round drops both the open editor and every committed
    // host: they belong to the scenario that was open, and carrying them
    // over would write one scenario's edit into the *next* one's save.
    app.network = None;
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditInfo])
        .test_value();
    assert!(app.developer_component_editor.is_some());
    app.open_developer_object_list();
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::CloseGame])
        .test_value();
    assert!(app.developer_component_editor.is_none());
    assert!(app.developer_component_hosts.is_empty());
    assert!(!app.developer_object_list_open);
}

/// Press and release one key, the way the console shell delivers it.
fn press_console_key(app: &mut GameApp, key: VirtualKeyCode, modifiers: ModifiersState) {
    app.handle_modifiers_changed(modifiers).test_value();
    app.handle_key(key, ElementState::Pressed).test_value();
    app.handle_key(key, ElementState::Released).test_value();
    app.handle_modifiers_changed(ModifiersState::empty())
        .test_value();
}

/// `C4Game.cpp:3433-3439` registers the C4ToolsDlg actions at
/// `KEYSCOPE_Console`, so they act only in console mode. `ChangeGrade` clamps
/// into `[C4TLS_GradeMin, C4TLS_GradeMax]` and has no availability gate of its
/// own (`C4ToolsDlg.cpp:739-745`); `ToggleTool` is `(Tool + 1) % 4`, which never
/// lands on Picker (`C4ToolsDlg.h:148`).
#[test]
fn console_tool_keys_drive_the_retained_tools_dialog_state() {
    use clonk_engine::developer_tools::{Tool, GRADE_MAX, GRADE_MIN};

    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Edit;

    let grade = app.developer_tools.grade();
    press_console_key(&mut app, VirtualKeyCode::NumpadAdd, ModifiersState::empty());
    runtime_assert_eq!(
        app.developer_tools.grade() => (grade + 5).min(GRADE_MAX),
        "the keyboard step is five grades, clamped at the maximum",
    );
    press_console_key(
        &mut app,
        VirtualKeyCode::NumpadSubtract,
        ModifiersState::empty(),
    );
    runtime_assert_eq!(app.developer_tools.grade() => grade);

    // The clamp is C++'s BoundBy, not a wrap.
    for _ in 0..40 {
        press_console_key(
            &mut app,
            VirtualKeyCode::NumpadSubtract,
            ModifiersState::empty(),
        );
    }
    runtime_assert_eq!(app.developer_tools.grade() => GRADE_MIN);

    let ift = app.developer_tools.ift();
    press_console_key(&mut app, VirtualKeyCode::KeyI, ModifiersState::CONTROL);
    runtime_assert_eq!(app.developer_tools.ift() => !ift);

    // Four tools, cycling, never Picker.
    let mut seen = Vec::new();
    for _ in 0..4 {
        press_console_key(&mut app, VirtualKeyCode::KeyW, ModifiersState::CONTROL);
        seen.push(app.developer_tools.tool());
    }
    runtime_assert!(
        !seen.contains(&Tool::Picker),
        "(Tool + 1) % 4 never reaches Picker, got {seen:?}"
    );
    runtime_assert_eq!(seen.last().copied() => Some(app.developer_tools.tool()));

    // Fullscreen must not see any of them: these are KEYSCOPE_Console.
    let mut fullscreen = new_lightweight_running_sandbox_app();
    let untouched = fullscreen.developer_tools.grade();
    press_console_key(
        &mut fullscreen,
        VirtualKeyCode::NumpadAdd,
        ModifiersState::empty(),
    );
    runtime_assert_eq!(
        fullscreen.developer_tools.grade() => untouched,
        "KEYSCOPE_Console actions are inert outside the console",
    );
}

/// `PopMaterial`/`PopTextures` return false when the dialog has no window
/// (`C4ToolsDlg.cpp:748-749,762-763`), so they only pop a combo while the
/// tools page is actually up.
#[test]
fn console_pop_keys_need_the_tools_page_the_way_cpp_needs_its_dialog() {
    use crate::developer_toolbox_view::ToolsCombo;

    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    // Draw mode is what opens the Tools page; `open_developer_prop_tools`
    // lands on Property in any other mode.
    app.developer_console_edit_mode = ConsoleEditMode::Draw;

    // No toolbox page: C++ would return false without touching anything.
    press_console_key(&mut app, VirtualKeyCode::KeyM, ModifiersState::CONTROL);
    runtime_assert!(
        app.developer_tools_open_combo.is_none(),
        "no tools dialog means no combo to pop"
    );

    // `C4ToolsDlg::Open`'s tail sets Active even with no dialog of its own.
    app.open_developer_prop_tools();
    press_console_key(&mut app, VirtualKeyCode::KeyM, ModifiersState::CONTROL);
    runtime_assert_eq!(app.developer_tools_open_combo => Some(ToolsCombo::Materials));

    press_console_key(&mut app, VirtualKeyCode::KeyT, ModifiersState::CONTROL);
    runtime_assert_eq!(app.developer_tools_open_combo => Some(ToolsCombo::Textures));
}

// C4Console.cpp:1353-1356 and C4ObjectListDlg.cpp:599-646,726-787 — the
// Objects component opens the object list, whose rows are the ported
// object tree and whose clicks write the edit cursor's selection.
#[test]
fn developer_object_list_opens_and_binds_the_selection_both_ways() {
    use clonk_engine::developer_selection::SelectionWriter;

    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Edit;
    assert!(!app.developer_object_list_open);

    // `EditObjects` is one line, and unlike Script/Title/Info it carries
    // no network refusal — the list only reads.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditObjects])
        .test_value();
    assert!(app.developer_object_list_open);
    // Opening again is idempotent: C++ builds the window only when it has
    // none.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::EditObjects])
        .test_value();
    assert!(app.developer_object_list_open);

    let extent = (
        crate::developer_object_list_view::OBJECT_LIST_WIDTH,
        crate::developer_object_list_view::OBJECT_LIST_HEIGHT,
    );
    let surface = app.render_developer_object_list(extent.0, extent.1);
    assert_eq!(surface.width(), extent.0);
    assert!(surface.pixels().chunks_exact(4).any(|pixel| pixel[3] != 0));

    // A click on the first row selects that object, stamped as the tree's
    // own write so the edit cursor can tell it from its own.
    let subject = app.snapshot.objects.first().test_value().id;
    app.developer_object_list_click((8, 8), extent);
    assert_eq!(app.developer_selection.objects(), &[subject]);

    // A click on empty space below the last row clears it — an empty
    // `gtk_tree_selection_get_selected_rows` still runs the handler.
    app.developer_object_list_click((8, extent.1 as i32 - 8), extent);
    assert!(app.developer_selection.is_empty());

    // The list mirrors a selection the *viewport* made, which is the other
    // half of the binding.
    app.developer_selection
        .replace(SelectionWriter::EditCursor, subject);
    let mirrored = app.render_developer_object_list(extent.0, extent.1);
    runtime_assert_ne!(mirrored.pixels() => surface.pixels(), "the selected row is drawn differently from an unselected one");

    // Closing destroys it rather than hiding it, so the next Objects click
    // builds a new window.
    app.close_developer_object_list();
    assert!(!app.developer_object_list_open);
}

// C4Viewport.cpp:225-240 and C4Game.cpp:1641-1676 — dropping a definition
// file on a console viewport is the editor's only way to create an object
// without typing script. `CID_EMDropDef`'s executor and wire codec landed
// long before anything could emit one.
/// `C4ViewportWindow::GetPositionData` (`C4Viewport.cpp:217-222`) puts every
/// detached viewport's geometry in the **console's** subkey, keyed
/// `DialogWindow::GetPositionData` names a console dialog's entry after the
/// dialog's own `GetID()` — `ConsoleGUI_Scoreboard` for `C4ScoreboardDlg`
/// (`C4GuiDialogs.cpp:285-297`; `C4Scoreboard.h:107`) — in the same `Console`
/// subkey the console and its viewports use, and sets `storeSize`, so the size
/// is written with the position.
#[test]
fn console_dialog_geometry_round_trips_under_its_own_dialog_id() {
    use crate::console_window_position::{console_dialog_position_key, ConsoleWindowPlacement};

    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);

    runtime_assert_eq!(
        console_dialog_position_key("Scoreboard") => Some("ConsoleGUI_Scoreboard".to_owned())
    );
    runtime_assert_eq!(
        console_dialog_position_key("") => None,
        "a dialog with no GetID gets no entry at all (C4GuiDialogs.cpp:287)",
    );

    runtime_assert!(
        load_console_dialog_window_position(Some(&paths), "Scoreboard").is_none(),
        "nothing is remembered before the dialog stores anything"
    );

    store_console_dialog_window_position(&paths, "Scoreboard", 40, 22, 260, 130).test_value();
    runtime_assert_eq!(
        load_console_dialog_window_position(Some(&paths), "Scoreboard") => Some(ConsoleWindowPlacement::PositionAndSize { x: 40, y: 22, width: 260, height: 130, }),
        "storeSize is set for a dialog, so the size survives with the position",
    );

    // A different dialog id is a different slot, and neither lands on the
    // console's own `Main` key or on a viewport's.
    runtime_assert!(load_console_dialog_window_position(Some(&paths), "Other").is_none());
    runtime_assert!(
        load_console_window_position(Some(&paths)).is_none(),
        "a dialog must not land on the console's `Main` key"
    );
    runtime_assert!(
        load_viewport_window_position(Some(&paths), clonk_engine::OWNER_NONE).is_none(),
        "nor on a viewport's slot"
    );
}

/// `Viewport{Player + 1}` with `storeSize` set — so unlike the console's own
/// `Main` entry the size comes back too, and two viewports following different
/// players never share a slot.
#[test]
fn viewport_window_geometry_round_trips_through_the_console_subkey() {
    use crate::console_window_position::ConsoleWindowPlacement;

    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);

    runtime_assert!(
        load_viewport_window_position(Some(&paths), clonk_engine::OWNER_NONE).is_none(),
        "nothing is remembered before a viewport stores anything"
    );

    // The ownerless viewport is `Viewport0`.
    store_viewport_window_position(&paths, clonk_engine::OWNER_NONE, 12, 34, 400, 250).test_value();
    runtime_assert_eq!(
        load_viewport_window_position(Some(&paths), clonk_engine::OWNER_NONE) => Some(ConsoleWindowPlacement::PositionAndSize { x: 12, y: 34, width: 400, height: 250, }),
        "storeSize is set, so the size survives with the position",
    );

    // Player 0 is `Viewport1` — a different slot, which is the point of keying
    // on the player.
    assert!(load_viewport_window_position(Some(&paths), 0).is_none());
    store_viewport_window_position(&paths, 0, 5, 6, 320, 200).test_value();
    runtime_assert_eq!(load_viewport_window_position(Some(&paths), 0) => Some(ConsoleWindowPlacement::PositionAndSize { x: 5, y: 6, width: 320, height: 200, }));
    runtime_assert_eq!(
        load_viewport_window_position(Some(&paths), clonk_engine::OWNER_NONE).and_then(ConsoleWindowPlacement::position) => Some((12, 34)),
        "and it did not disturb the ownerless viewport's slot",
    );

    // The section is shared with the console, whose own entry is separate.
    runtime_assert!(
        load_console_window_position(Some(&paths)).is_none(),
        "a viewport must not land on the console's `Main` key"
    );
}

/// `ScrollBarsByViewPosition` refuses outright while the player lock is on
/// (`C4Viewport.cpp:272`), and `C4Viewport::Default` starts that lock **set**
/// (`C4Viewport.cpp:1272`) — so a viewport shows no bars until it is unlocked.
///
/// The layout itself is pinned in `developer_viewport`; what this pins is that
/// the viewport actually draws it. Without this, removing the draw call leaves
/// every engine-side bar test passing.
#[test]
fn console_viewport_draws_scroll_bars_only_while_unlocked() {
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Edit;
    let identity = open_test_console_viewport(&mut app, None);

    // The thumb grey `draw_console_scroll_bars` paints. Counting it is enough:
    // the scene behind is the sandbox landscape, and the assertion that matters
    // is presence against absence, not a particular pixel.
    let thumb = clonk_graphics::Color::opaque(168, 168, 168);
    let thumb_pixels = |surface: &clonk_graphics::Surface| {
        let mut seen = 0_usize;
        for y in 0..surface.height() {
            for x in 0..surface.width() {
                if surface.get_pixel(x, y) == Some(thumb) {
                    seen += 1;
                }
            }
        }
        seen
    };

    runtime_assert!(
        app.console_viewport_player_lock(identity),
        "a fresh viewport starts locked, which is what makes the next assertion meaningful"
    );
    let locked = app.render_console_viewport(identity, 320, 200).test_value();
    runtime_assert_eq!(thumb_pixels(&locked) => 0, "a locked viewport draws no bars at all");

    app.toggle_console_viewport_player_lock(identity);
    runtime_assert!(
        !app.console_viewport_player_lock(identity),
        "unlocking always succeeds"
    );
    let unlocked = app.render_console_viewport(identity, 320, 200).test_value();
    runtime_assert!(
        thumb_pixels(&unlocked) > 0,
        "an unlocked viewport draws both thumbs"
    );
}

#[test]
fn console_viewport_file_drop_emits_a_definition_drop_control() {
    let (mut app, _events, mut commands, identity) =
        runtime_console_network_fixture(ConsoleEditMode::Edit);
    // Drawing is what publishes this window's own projection.
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    let projection = app.console_viewport_projections[&identity];

    // `DefFileGetID` reads the id out of the dropped group's own
    // `DefCore.txt`, so the file only has to declare a definition the
    // engine already holds — which is exactly the case C++ takes without
    // loading anything. The id has to be a real four-byte C4ID: that is
    // what `DefCore` parses and what the control's `id` field carries.
    let definition = "ROCK".to_owned();
    app.engine
        .register_test_definition(test_definition(&definition, "Rock", "#strict\n"));
    let directory = tempfile::tempdir().test_value();
    let source = directory.path().join("Dropped.c4d");
    std::fs::create_dir(&source).test_value();
    std::fs::write(
        source.join("DefCore.txt"),
        format!("[DefCore]\nid={definition}\nVersion=4,9,8\nWidth=1\nHeight=1\n"),
    )
    .test_value();

    let local = (40, 24);
    app.drop_file_on_console_viewport(identity, &source, local);
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDropDef(drop), false)] = decided.as_slice() else {
        panic!(
            "expected one definition drop control, got {decided:?}; console said {:?}",
            app.developer_console.log().text()
        );
    };
    assert_eq!(drop.id, definition.as_bytes());
    // The drop point is the viewport's own, added to the view origin.
    runtime_assert_eq!(
        (drop.x, drop.y) => (projection.target_x + local.0, projection.target_y + local.1);
        drop.by_client => 7;
    );

    // Anything that is not a `.c4d` is ignored in silence — C++'s failure
    // text lives inside that branch.
    app.drop_file_on_console_viewport(identity, std::path::Path::new("/tmp/Round.c4s"), local);
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a scenario file is not a definition drop"
    );

    // A `.c4d` the engine does not hold emits nothing and says so.
    app.developer_console.clear_log();
    app.drop_file_on_console_viewport(identity, std::path::Path::new("/tmp/Missing.c4d"), local);
    assert!(commands.take_submitted_decided_controls().is_empty());
    runtime_assert!(
        app.developer_console.log().text().contains("Missing.c4d"),
        "the failure names the file: {:?}",
        app.developer_console.log().text()
    );

    // A console that may not edit refuses the whole drop, with the same
    // message the draw tools use.
    app.developer_console_editing_enabled = false;
    app.developer_console.clear_log();
    app.drop_file_on_console_viewport(identity, &source, local);
    assert!(commands.take_submitted_decided_controls().is_empty());
    assert!(!app.developer_console.log().text().is_empty());
}

/// The other half of `C4Game::DropFile`: when `C4Id2Def` misses, C++ loads the
/// group and resolves the id a *second* time
/// (`Defs.Load(szFilename, C4D_Load_RX, …) && (cdef = C4Id2Def(c_id))`,
/// `C4Game.cpp:1647-1651`), so a definition dropped from outside the loaded
/// set reaches `DropDef` instead of reporting `IDS_CNS_DROPNODEF`.
///
/// The sibling test above covers the id the engine already holds, which is the
/// arm that never touches the loader. This one is what fails if the drop stops
/// loading: the engine-side loader has its own tests, but only this pins that
/// the drop actually reaches it.
#[test]
fn console_viewport_file_drop_loads_a_definition_outside_the_loaded_set() {
    let (mut app, _events, mut commands, identity) =
        runtime_console_network_fixture(ConsoleEditMode::Edit);
    assert!(app.render_console_viewport(identity, 320, 200).is_some());

    // Deliberately never registered, so the first `C4Id2Def` misses and the
    // load is the only way the id can resolve.
    let definition = "DRPD".to_owned();
    runtime_assert!(
        app.engine.definition(&definition).is_none(),
        "the id must start outside the loaded set for this to test anything"
    );
    let directory = tempfile::tempdir().test_value();
    let source = directory.path().join("Dropped.c4d");
    std::fs::create_dir(&source).test_value();
    std::fs::write(
        source.join("DefCore.txt"),
        format!("[DefCore]\nid={definition}\nVersion=4,9,8\nName=Dropped\nWidth=8\nHeight=8\n"),
    )
    .test_value();

    let local = (40, 24);
    app.drop_file_on_console_viewport(identity, &source, local);

    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDropDef(drop), false)] = decided.as_slice() else {
        panic!(
            "expected one definition drop control, got {decided:?}; console said {:?}",
            app.developer_console.log().text()
        );
    };
    assert_eq!(drop.id, definition.as_bytes());
    runtime_assert!(
        app.engine.definition(&definition).is_some(),
        "the dropped group was loaded, which is what lets the second lookup resolve"
    );
}

// C4EditCursor.cpp:361-374,503-518 and C4ToolsDlg.cpp:865-879 — the
// context menu's Properties/Tools row is the only thing that opens the
// toolbox, the page follows the cursor mode, and the landscape mode
// buttons are the one page control that travels as a control.
#[test]
fn developer_toolbox_opens_by_mode_and_its_mode_buttons_emit_controls() {
    use crate::developer_toolbox::ToolboxEffect;
    use crate::developer_toolbox_view::{TOOLBOX_HEIGHT, TOOLBOX_WIDTH};
    use crate::developer_tools_page::ToolsControl;
    use crate::developer_windows::ToolboxPage;

    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Draw;

    // Nothing opens the toolbox on its own: `SetMode` reopens it only when
    // one of the two pages was already active (`:508-516`).
    assert!(app.developer_toolbox_effects.is_empty());
    assert_eq!(app.developer_toolbox.current_page(), None);

    // Draw mode's Properties row opens the Tools page, and `C4ToolsDlg::
    // Open`'s tail sets Active even with no dialog of its own.
    app.open_developer_prop_tools();
    assert!(app.developer_tools.active());
    runtime_assert_eq!(app.developer_toolbox.current_page() => Some(ToolboxPage::Tools));
    runtime_assert_eq!(app.developer_toolbox.pages() => &[ToolboxPage::Tools, ToolboxPage::Property], "both pages are appended, whichever is switched to");
    let effects = std::mem::take(&mut app.developer_toolbox_effects);
    runtime_assert!(
        matches!(effects.first(), Some(ToolboxEffect::Create(_))),
        "the first page creates the window: {effects:?}"
    );
    runtime_assert!(
        effects.iter().any(|effect| matches!(
            effect,
            ToolboxEffect::Show {
                page: ToolboxPage::Tools,
                ..
            }
        )),
        "and it is shown on the Tools page: {effects:?}"
    );

    // Leaving Draw clears the tools and, because a page was up, reopens
    // the toolbox on Property (`:503-516`).
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::SetEditMode(
        ConsoleEditMode::Edit,
    )])
    .test_value();
    assert!(!app.developer_tools.active(), "Clear drops Active");
    runtime_assert_eq!(app.developer_toolbox.current_page() => Some(ToolboxPage::Property));

    // The page's own controls: everything but the landscape mode is local
    // dialog state, and only the mode leaves as a control.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::SetEditMode(
        ConsoleEditMode::Draw,
    )])
    .test_value();
    app.developer_toolbox_effects.clear();
    let extent = (TOOLBOX_WIDTH, TOOLBOX_HEIGHT);
    let center = |app: &GameApp, control: ToolsControl| {
        let rect = app
            .developer_tools_page_model()
            .layout(extent.0, extent.1)
            .into_iter()
            .find(|slot| slot.control == control)
            .test_value()
            .rect;
        (rect.x + rect.w / 2, rect.y + rect.h / 2)
    };

    // Offline the control applies straight away, so this is also the
    // fixture that puts the landscape somewhere the rest of the page is
    // live at all — everything but the mode buttons is dead below Static.
    let exact = center(&app, ToolsControl::ModeExact);
    app.developer_toolbox_click(exact, extent);
    runtime_assert_eq!(app.developer_tools_page_model().mode => clonk_engine::developer_tools::LandscapeMode::Exact);

    let (_events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    app.developer_toolbox_click(center(&app, ToolsControl::Line), extent);
    runtime_assert_eq!(app.developer_tools.tool() => clonk_engine::developer_tools::Tool::Line);
    app.developer_toolbox_click(center(&app, ToolsControl::NoIft), extent);
    assert!(!app.developer_tools.ift());
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "the tool and IFT are dialog state, not synchronized state"
    );

    // The three mode buttons are the exception: `SetLandscapeMode(iMode,
    // false)` enqueues `EMDT_SetMode` and changes nothing locally, so
    // every peer switches at the same tick. Exact is the button that is
    // always live — Dynamic is enabled only when the landscape already is
    // dynamic, and Static needs a map (`C4ToolsDlg.cpp:796-812`).
    let before = app.engine.landscape().test_value().mode();
    app.developer_toolbox_click(center(&app, ToolsControl::ModeExact), extent);
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDrawTool(set_mode), false)] = decided.as_slice() else {
        panic!("expected one set-mode control, got {decided:?}");
    };
    assert_eq!(set_mode.action, clonk_engine::EMDT_SET_MODE);
    assert_eq!(set_mode.mode, clonk_engine::landscape::LANDSCAPE_MODE_EXACT);
    runtime_assert_eq!(app.engine.landscape().expect("the sandbox landscape").mode() => before, "the local request changes nothing until the control executes");

    // Exact -> Static is the one destructive transition, and it is
    // **refused**: `SetLandscapeMode` returns false unless
    // `Console.Message` confirms, and past its two `#ifdef` bodies that
    // function is a bare `return false` (`C4Console.cpp:841-853`). So
    // nothing is enqueued.
    let model = app.developer_tools_page_model();
    runtime_assert_eq!(model.mode => clonk_engine::developer_tools::LandscapeMode::Exact);
    // Driven at the seam rather than through the button, because the
    // button carries a second gate — Static needs `Game.Landscape.Map`,
    // which this fixture's landscape may not have.
    app.submit_editor_landscape_mode(clonk_engine::developer_tools::LandscapeMode::Static);
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a refused Exact -> Static enqueues nothing"
    );
    // Every other transition still goes out, so the refusal is the
    // confirmation's and not a blanket block.
    app.submit_editor_landscape_mode(clonk_engine::developer_tools::LandscapeMode::Dynamic);
    runtime_assert_eq!(commands.take_submitted_decided_controls().len() => 1, "only the destructive transition is refused");

    // Closing the toolbox clears `ToolsDlg.Active` — C++ connects the
    // shared devmode window's "hide" to `OnWindowHide`, whose body is
    // exactly that (`C4ToolsDlg.cpp:393,1098-1101`). Without it the next
    // mode change would resurrect a toolbox the user closed.
    assert!(app.developer_tools.active());
    app.close_developer_toolbox(Some((120, 80)));
    assert!(!app.developer_tools.active());
    assert!(!app.developer_toolbox.visible());
    assert_eq!(app.developer_toolbox.remembered_position(), Some((120, 80)));
    app.developer_toolbox_effects.clear();
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::SetEditMode(
        ConsoleEditMode::Play,
    )])
    .test_value();
    runtime_assert!(
        app.developer_toolbox_effects.is_empty(),
        "a closed toolbox is not reopened by a mode change"
    );
    // Re-opening restores the remembered position rather than re-centring.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::SetEditMode(
        ConsoleEditMode::Draw,
    )])
    .test_value();
    app.open_developer_prop_tools();
    runtime_assert!(app.developer_toolbox_effects.iter().any(|effect| matches!(
        effect,
        ToolboxEffect::Show {
            position: Some((120, 80)),
            ..
        }
    )));
    app.developer_toolbox_effects.clear();

    // The property page is a read-only text box: clicking it emits
    // nothing at all.
    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::SetEditMode(
        ConsoleEditMode::Edit,
    )])
    .test_value();
    runtime_assert_eq!(app.developer_toolbox.current_page() => Some(ToolboxPage::Property));
    app.developer_toolbox_click(center(&app, ToolsControl::ModeExact), extent);
    assert!(commands.take_submitted_decided_controls().is_empty());

    // Both pages draw at the window's extent without panicking.
    for page in [ToolboxPage::Tools, ToolboxPage::Property] {
        let surface = app.render_developer_toolbox_page(page, TOOLBOX_WIDTH, TOOLBOX_HEIGHT);
        assert_eq!(surface.width(), TOOLBOX_WIDTH);
        assert!(surface.pixels().chunks_exact(4).any(|pixel| pixel[3] != 0));
    }
}

// C4EditCursor.cpp:640-651 — Grab contents replaces the selection with the
// container's own Contents and *then* exits them, so the command acts on
// what was inside rather than on the container.
#[test]
fn console_viewport_grab_contents_exits_the_container_it_selected() {
    use clonk_frontend::developer_context_menu::ViewportContextItem;

    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Edit;
    let identity = open_test_console_viewport(&mut app, None);
    assert!(app.render_console_viewport(identity, 320, 200).is_some());

    // Give one object a Contents list to grab.
    let subject = app.snapshot.objects.first().test_value();
    let (container, definition) = (subject.id, subject.definition_id.clone());
    let held = app
        .engine
        .spawn_test_object(clonk_engine::SpawnConfig::new(definition).with_container(container));
    app.snapshot = app.engine.snapshot();
    runtime_assert_eq!(app.snapshot.object(container).expect("the container is live").contents => vec![held]);

    let (_events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    app.developer_selection.replace(
        clonk_engine::developer_selection::SelectionWriter::EditCursor,
        container,
    );
    app.open_console_viewport_context_menu(identity, (4, 4));
    let menu = &app.console_viewport_context_menu.test_ref().1;
    let contents_row = menu
        .entries()
        .iter()
        .position(|entry| entry.item() == Some(ViewportContextItem::GrabContents))
        .test_value();
    let rect = menu.layout(320, 200)[contents_row].rect;
    runtime_assert!(app.console_viewport_context_menu_click(
        identity,
        (rect.x + rect.w / 2, rect.y + rect.h / 2),
        (320, 200)
    ));

    // The selection is now the contents, not the container, and `Hold` is
    // set before the control leaves — that is what lets the freed objects
    // be dragged straight out.
    assert_eq!(app.developer_selection.objects(), &[held]);
    assert!(app.edit_cursor_hold);
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmMoveObject(exit), false)] = decided.as_slice() else {
        panic!("expected one exit control, got {decided:?}");
    };
    assert_eq!(exit.action, clonk_engine::EMMO_EXIT);
    runtime_assert_eq!(exit.objects => vec![held.as_u64() as i32], "EMMO_Exit carries the contents, never the container");

    // The release completing that click belongs to the menu: C++'s popup
    // is modal, so `LeftButtonUp` never runs, and letting it through here
    // would clear the `Hold` the command set one line before its control
    // went out.
    runtime_assert!(
        app.take_console_viewport_pointer_grab(identity),
        "the popup grabbed the pointer for the whole click"
    );
    assert!(app.edit_cursor_hold, "and Hold survives it");
    runtime_assert!(
        !app.take_console_viewport_pointer_grab(identity),
        "exactly one release is swallowed"
    );
}

// C4EditCursor.cpp:224-236,551-572 — Draw mode routes the same viewport
// gestures into the landscape tools, and every stroke leaves as an
// EMDrawTool control rather than a direct raster write.
#[test]
fn console_viewport_draw_gestures_emit_landscape_tool_controls() {
    use clonk_engine::developer_tools::Tool;

    let (mut app, _events, mut commands, identity) =
        runtime_console_network_fixture(ConsoleEditMode::Draw);
    // Drawing is what publishes this window's own projection.
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    let projection = app.console_viewport_projections[&identity];
    let mode = app.engine.landscape().test_value().mode();
    let world = |local: (i32, i32)| (projection.target_x + local.0, projection.target_y + local.1);

    // `LeftButtonDown`'s Brush arm applies on the click itself (`:224`).
    let pressed = (40, 10);
    app.console_viewport_press(identity, pressed, 1.0, false, false);
    runtime_assert!(
        app.developer_tools.holding(),
        "a draw press arms Hold like every other gesture"
    );
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDrawTool(brush), false)] = decided.as_slice() else {
        panic!("expected one brush control, got {decided:?}");
    };
    assert_eq!(brush.action, clonk_engine::EMDT_BRUSH);
    assert_eq!((brush.x, brush.y), world(pressed));
    // The mode travels with the control, because the executor refuses a
    // packet whose mode no longer matches (`C4Control.cpp:1015-1016`).
    assert_eq!(brush.mode, mode);
    // `C4ToolsDlg::Default` — grade 5, IFT on, Earth over Rough.
    assert_eq!(brush.grade, clonk_engine::developer_tools::GRADE_DEFAULT);
    assert!(brush.ift);
    assert_eq!(brush.material.as_bytes(), b"Earth");
    assert_eq!(brush.texture.as_bytes(), b"Rough");
    assert_eq!(brush.by_client, 7);

    // The brush also draws on every drag step (`C4EditCursor::Move`:159).
    let dragged = (70, 10);
    app.console_viewport_motion(identity, dragged, 1.0, false, false);
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDrawTool(dragging), false)] = decided.as_slice() else {
        panic!("expected one dragged brush control, got {decided:?}");
    };
    assert_eq!((dragging.x, dragging.y), world(dragged));

    app.console_viewport_release();
    assert!(!app.developer_tools.holding(), "the release clears Hold");
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "the brush emits nothing on release"
    );

    // Line records its anchor on the press and emits once on release,
    // with the *live* cursor leading and the anchor second — C++'s own
    // argument order, `C4ControlEMDrawTool(EMDT_Line, Mode, X, Y, X2, Y2)`
    // (`:558`).
    app.developer_tools.set_tool(Tool::Line, false);
    let anchor = (12, 34);
    app.console_viewport_press(identity, anchor, 1.0, false, false);
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a line draws nothing until it is released"
    );
    let released = (56, 78);
    app.console_viewport_motion(identity, released, 1.0, false, false);
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a line draws nothing while it is dragged"
    );
    app.console_viewport_release();
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDrawTool(line), false)] = decided.as_slice() else {
        panic!("expected one line control, got {decided:?}");
    };
    assert_eq!(line.action, clonk_engine::EMDT_LINE);
    assert_eq!((line.x, line.y), world(released));
    assert_eq!((line.x2, line.y2), world(anchor));
}

// C4EditCursor.cpp:60-67,227-231,574-580 — Fill is the one tool that emits
// nothing on the click: it arms Hold and repeats from Execute, refusing
// outright while the game is halted.
#[test]
fn console_draw_fill_refuses_while_halted_and_otherwise_repeats_at_the_cursor() {
    use clonk_engine::developer_tools::Tool;

    let (mut app, _events, mut commands, identity) =
        runtime_console_network_fixture(ConsoleEditMode::Draw);
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    let projection = app.console_viewport_projections[&identity];
    let world = |local: (i32, i32)| (projection.target_x + local.0, projection.target_y + local.1);
    app.developer_tools.set_tool(Tool::Fill, false);

    // Halted: the click is refused with IDS_CNS_FILLNOHALT and Hold is
    // never armed, so the frame repeat cannot start either (`:227-231`).
    app.network_control_running = false;
    app.console_viewport_press(identity, (40, 10), 1.0, false, false);
    assert!(!app.developer_tools.holding(), "a halted fill never holds");
    runtime_assert!(app
        .developer_console
        .log()
        .text()
        .contains("The fill tool cannot be used in halt mode."));
    app.console_edit_cursor_tick();
    assert!(commands.take_submitted_decided_controls().is_empty());

    // Running: the click still emits nothing, but arms the repeat.
    app.network_control_running = true;
    let pressed = (40, 10);
    app.console_viewport_press(identity, pressed, 1.0, false, false);
    assert!(app.developer_tools.holding());
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "fill emits nothing on the click itself"
    );

    app.console_edit_cursor_tick();
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDrawTool(fill), false)] = decided.as_slice() else {
        panic!("expected one fill control, got {decided:?}");
    };
    assert_eq!(fill.action, clonk_engine::EMDT_FILL);
    assert_eq!((fill.x, fill.y), world(pressed));
    // `ApplyToolFill` passes 0 for X2 and forces IFT false (`:579`).
    assert_eq!(fill.x2, 0);
    assert!(!fill.ift);

    // Once per engine tick, not once per event-loop wake.
    app.console_edit_cursor_tick();
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a second wake inside the same frame emits nothing"
    );

    // A held fill follows the cursor, because `ApplyToolFill` reads the
    // same live X/Y every other tool does (`:579`).
    let moved = (90, 60);
    app.console_viewport_motion(identity, moved, 1.0, false, false);
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "fill emits nothing on the drag itself"
    );
    // The repeat is keyed on the engine frame; clear the latch to stand in
    // for the next one rather than running a whole simulation tick.
    app.edit_cursor_tick_frame = None;
    app.console_edit_cursor_tick();
    let decided = commands.take_submitted_decided_controls();
    let [(_, clonk_engine::ControlPacket::EmDrawTool(fill), false)] = decided.as_slice() else {
        panic!("expected one moved fill control, got {decided:?}");
    };
    assert_eq!((fill.x, fill.y), world(moved));

    app.console_viewport_release();
    app.edit_cursor_tick_frame = None;
    app.console_edit_cursor_tick();
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "releasing stops the repeat"
    );
}

// C4EditCursor.cpp:287-309,552,673-682 — `EditingOK` is not a predicate:
// refusing a stroke clears `Hold`, so the drag stops dead instead of
// retrying on every motion. And `LeftButtonUp` clears `Hold` whichever
// mode's finish ran, because C++ has one shared flag.
#[test]
fn a_refused_draw_stroke_and_a_mode_change_both_clear_the_held_gesture() {
    let (mut app, _events, mut commands, identity) =
        runtime_console_network_fixture(ConsoleEditMode::Draw);
    assert!(app.render_console_viewport(identity, 320, 200).is_some());

    // A console that cannot edit refuses the brush, says so, and drops the
    // hold — so the following drag steps are silent too (`:677`).
    app.developer_console_editing_enabled = false;
    app.console_viewport_press(identity, (40, 10), 1.0, false, false);
    assert!(commands.take_submitted_decided_controls().is_empty());
    assert!(!app.developer_tools.holding(), "EditingOK clears Hold");
    runtime_assert!(app
        .developer_console
        .log()
        .text()
        .contains("No editing while replaying."));
    app.console_viewport_motion(identity, (70, 10), 1.0, false, false);
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "a refused stroke does not resume on the next motion"
    );

    // A mode change between press and release must not strand the hold:
    // `LeftButtonUp` clears it unconditionally (`:300-304`).
    app.developer_console_editing_enabled = true;
    app.console_viewport_press(identity, (40, 10), 1.0, false, false);
    assert!(app.developer_tools.holding());
    let _ = commands.take_submitted_decided_controls();
    app.developer_console_edit_mode = ConsoleEditMode::Edit;
    app.console_viewport_release();
    runtime_assert!(
        !app.developer_tools.holding(),
        "the release clears Hold even though the Draw finish did not run"
    );
}

// C4EditCursor.cpp:236,698-731,773-792 — the picker reads the landscape
// into the tools instead of drawing, and Alt selects it temporarily.
#[test]
fn console_draw_alt_picks_the_landscape_into_the_tools_without_drawing() {
    use clonk_engine::developer_tools::Tool;

    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    app.developer_console_edit_mode = ConsoleEditMode::Draw;
    // `ApplyToolPicker` samples nothing outside Static and Exact.
    app.apply_ready_controls(
        1,
        vec![NetworkControl::EmDrawTool(
            clonk_engine::EmDrawToolControlData {
                action: clonk_engine::EMDT_SET_MODE,
                mode: clonk_engine::LANDSCAPE_MODE_EXACT,
                ..Default::default()
            },
        )],
    )
    .test_value();
    let (_events, mut commands) = install_running_network_stub(&mut app, 7, 0, 2);
    let identity = open_test_console_viewport(&mut app, None);
    assert!(app.render_console_viewport(identity, 320, 200).is_some());

    // Alt overrides the tool only while it is held, and only in Draw mode
    // (`C4EditCursor::AltDown`, `:773-780`).
    app.update_console_editor_modifiers(ModifiersState::ALT);
    assert_eq!(app.developer_tools.tool(), Tool::Picker);

    // Sampling empty landscape selects the sky pseudo-material and leaves
    // the texture alone (`:717`, `:727`) — set it away from its default
    // first, so "unchanged" cannot be confused with "reset".
    app.developer_tools.set_texture("Smooth");
    app.console_viewport_press(identity, (40, 10), 1.0, false, false);
    assert_eq!(app.developer_tools.material(), "Sky");
    assert_eq!(app.developer_tools.texture(), "Smooth");
    runtime_assert!(
        !app.developer_tools.holding(),
        "ApplyToolPicker ends with Hold = false (`:731`)"
    );
    runtime_assert!(
        commands.take_submitted_decided_controls().is_empty(),
        "the picker never draws"
    );

    // Releasing Alt restores the tool the dialog had before.
    app.update_console_editor_modifiers(ModifiersState::empty());
    assert_eq!(app.developer_tools.tool(), Tool::Brush);

    // Outside Draw mode Alt is inert.
    app.developer_console_edit_mode = ConsoleEditMode::Edit;
    app.update_console_editor_modifiers(ModifiersState::ALT);
    assert_eq!(app.developer_tools.tool(), Tool::Brush);
}

// C4Viewport.cpp:125-146,250-284,1162 — a console viewport window scrolls
// only once its player lock is off, because the lock is what makes
// UpdateViewPosition re-centre on the player every frame.
#[test]
fn console_viewport_scrolls_only_once_its_player_lock_is_off() {
    let mut app = new_lightweight_running_sandbox_app();
    app.console_mode = true;
    let identity = open_local_test_console_viewport(&mut app);
    assert!(app.render_console_viewport(identity, 320, 200).is_some());

    // `C4Viewport::Default` starts locked, so a scroll request is refused
    // outright — `ScrollBarsByViewPosition` returns false while locked
    // (`:272`).
    assert!(app.console_viewport_player_lock(identity));
    assert!(!app.scroll_console_viewport(identity, 3, 0));

    // Unlocking always succeeds and shows the bars again (`:253-259`).
    assert!(!app.toggle_console_viewport_player_lock(identity));
    assert!(!app.console_viewport_player_lock(identity));

    let before = app.console_viewport_projections[&identity];
    assert!(app.scroll_console_viewport(identity, 3, 0));
    // The scroll lands in the camera, and the next draw is what publishes
    // it as this window's projection (`cvp->Execute()` after the scroll).
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    let after = app.console_viewport_projections[&identity];
    runtime_assert_ne!((before.target_x, before.target_y) => (after.target_x, after.target_y), "an unlocked viewport keeps where the scroll put it");

    // `UpdateViewPosition`'s clamp is gated on `fIsNoOwnerViewport`
    // (`:1234-1254`): an *owned* viewport keeps whatever view position it
    // was given and grows its borders instead. A locked viewport would
    // instead re-derive the position from its player every frame, so
    // scrolling far past the landscape edge and finding it still there is
    // what pins the lock actually being off.
    let (view_x, ..) = app.graphics.detached_viewport_view(identity).test_value();
    runtime_assert_eq!(app.graphics.scroll_detached_viewport(identity, -(view_x + 400), 0) => Some((-400, after.target_y)));
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    runtime_assert_eq!(app.console_viewport_projections[&identity].target_x => -400, "an owned viewport keeps a view position outside the landscape");
    // And the next step moves relative to it rather than snapping back:
    // the line buttons apply their step unclamped (`:127-128`).
    assert!(app.scroll_console_viewport(identity, 4, 0));
    assert!(app.render_console_viewport(identity, 320, 200).is_some());
    assert_eq!(app.console_viewport_projections[&identity].target_x, -396);

    // Locking again needs a valid player, and hides the bars (`:263-265`).
    assert!(app.toggle_console_viewport_player_lock(identity));
    assert!(!app.scroll_console_viewport(identity, 3, 0));
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
        .test_value();

    app.test_network_events();

    runtime_assert_eq!(app.admission_resources.status(core.id) => Some(&AdmissionResourceState::Complete { path: mutable_path, removed: false, local: false, }));
}

#[test]
fn player_command_submission_queues_the_open_tick_without_local_execution() {
    let mut app = new_state_only_running_sandbox_app();
    let (manager, _event_tx, mut commands) =
        NetworkManager::test_stub_with_commands_for_client_id(7);
    app.network = Some(manager);
    let tick = app.local_control_submission_tick();
    let crew = app.engine.test_crew_cursor(app.local_owner);
    let before = app
        .engine
        .test_object_snapshot(crew)
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

    app.submit_or_execute_player_command(command).test_value();

    runtime_assert_eq!(commands.take_submitted_player_commands() => vec![(tick, PlayerCommandControlData { by_client: 7,..command })]);
    runtime_assert_eq!(app.engine.object_snapshot(crew).expect("cursor survives").command_stack.command_names() => before, "the command executes only when the synchronized tick returns");
}

#[test]
fn player_select_submission_queues_the_open_tick_without_local_execution() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    let first = app.engine.test_crew_cursor(owner);
    let definition = app.engine.test_object_snapshot(first).definition_id;
    let second = app.engine.spawn_test_object(
        SpawnConfig::new(definition)
            .with_owner(owner)
            .with_crew_member(true),
    );
    app.engine.select_crew(owner, [first, second]).test_value();
    app.engine.set_crew_cursor(owner, Some(first)).test_value();
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
        .test_value();

    runtime_assert_eq!(commands.take_submitted_player_selects() => vec![(tick, PlayerSelectControlData { by_client: 7,..selection })]);
    runtime_assert_eq!(app.engine.selected_crew(owner) => before, "selection executes only when the synchronized tick returns");
}

#[test]
fn runtime_flash_storage_uses_classic_bytes_and_snapshots_placement() {
    let mut app = new_state_only_running_sandbox_app();

    let cp1252 = app
        .prepare_runtime_flash_message("\u{fc}", RuntimeHelpCharset::Windows1252)
        .expect("encode CP1252 flash")
        .test_value();
    assert_eq!(cp1252.text, "\u{fc}");
    assert_eq!(cp1252.remaining_draws, 2, "CP1252 ü is one stored byte");

    let utf8 = app
        .prepare_runtime_flash_message("\u{fc}", RuntimeHelpCharset::Utf8)
        .expect("encode UTF-8 flash")
        .test_value();
    assert_eq!(utf8.remaining_draws, 4, "UTF-8 ü is two stored bytes");

    let unicode = app
        .prepare_runtime_flash_message("\u{100}", RuntimeHelpCharset::Utf8)
        .expect("FontRegular accepts non-CP1252 UTF-8")
        .test_value();
    assert_eq!(unicode.text, "\u{100}");
    assert_eq!(unicode.remaining_draws, 4);
    runtime_assert!(
        app.prepare_runtime_flash_message("\u{100}", RuntimeHelpCharset::Windows1252,)
            .is_err(),
        "the classic CP1252 encoder still rejects an unrepresentable scalar"
    );

    let ascii = "A".repeat(513);
    let truncated = app
        .prepare_runtime_flash_message(&ascii, RuntimeHelpCharset::Windows1252)
        .expect("truncate classic title buffer")
        .test_value();
    assert_eq!(truncated.text.len(), 512);
    assert_eq!(truncated.remaining_draws, 1024);

    let nul = app
        .prepare_runtime_flash_message("A\0ignored", RuntimeHelpCharset::Windows1252)
        .expect("SCopy stops at NUL")
        .test_value();
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
            .test_value();
        assert_eq!(message.y, expected_y, "mode {mode:?}");
    }

    let player = app
        .snapshot
        .players
        .iter_mut()
        .find(|player| player.id == app.local_owner)
        .test_value();
    player
        .viewports
        .push(player.viewports.first().test_value().clone());
    app.display_flags.upper_board = UpperBoardMode::Full;
    app.set_runtime_flash_message("AB", RuntimeHelpCharset::Windows1252)
        .test_value();
    assert_eq!(app.runtime_flash_message.as_ref().expect("flash").y, 124);
    app.display_flags.upper_board = UpperBoardMode::Hide;
    app.snapshot
        .players
        .iter_mut()
        .find(|player| player.id == app.local_owner)
        .test_value()
        .viewports
        .truncate(1);
    runtime_assert_eq!(app.runtime_flash_message.as_ref().expect("frozen flash").y => 124, "later board/viewport changes do not reposition an active flash");

    app.set_runtime_flash_message("", RuntimeHelpCharset::Windows1252)
        .test_value();
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
        .test_player_mut(modified_first.local_owner)
        .control
        .control_style = true;
    modified_first.test_modifiers(ModifiersState::ALT);
    modified_first.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    runtime_assert!(modified_first
        .pressed_engine_keys
        .contains(&VirtualKeyCode::F3));
    modified_first.test_modifiers(ModifiersState::empty());
    modified_first.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    runtime_assert_eq!(modified_first.engine.player(modified_first.local_owner).expect("local player").control.pressed_coms & left_mask => 0, "AutoStop must discard a held F3 repeat");
    assert!(modified_first.runtime_flash_message.is_none());

    let mut game_over = new_game_over_keyboard_app();
    game_over
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
    game_over
        .engine
        .test_player_mut(game_over.local_owner)
        .control
        .control_style = true;
    game_over.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    let global_flash = game_over.runtime_flash_message.clone();
    game_over.dismiss_game_over_dialog();
    game_over.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    runtime_assert_eq!(
        game_over.engine.player(game_over.local_owner).expect("local player").control.pressed_coms & left_mask => 0;
        game_over.runtime_flash_message => global_flash;
    );

    let mut changed_on_release = new_running_sandbox_app();
    changed_on_release
        .bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F3);
    changed_on_release
        .engine
        .test_player_mut(changed_on_release.local_owner)
        .control
        .control_style = true;
    changed_on_release.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    changed_on_release.test_modifiers(ModifiersState::CONTROL);
    changed_on_release.test_key(VirtualKeyCode::F3, ElementState::Released);
    runtime_assert!(!changed_on_release
        .pressed_engine_keys
        .contains(&VirtualKeyCode::F3));
    changed_on_release
        .engine
        .test_player_mut(changed_on_release.local_owner)
        .control
        .pressed_coms = 0;
    changed_on_release.test_modifiers(ModifiersState::empty());
    changed_on_release.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    runtime_assert_ne!(changed_on_release.engine.player(changed_on_release.local_owner).expect("local player").control.pressed_coms & left_mask => 0);

    let mut focus = new_running_sandbox_app();
    let sound_before = focus.test_audio_ref().options.sound_enabled;
    focus.test_modifiers(ModifiersState::CONTROL);
    focus.handle_focus_lost().test_value();
    assert!(focus.keyboard_modifiers.is_empty());
    focus.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    runtime_assert_eq!(focus.test_audio_ref().options.sound_enabled => sound_before);
    assert!(focus.runtime_flash_message.is_some());
}

#[test]
fn speed_keys_flash_clamp_and_honor_keyconfig_priority() {
    let mut app = new_running_sandbox_app();
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::NumpadAdd, ElementState::Pressed);
    assert!(app.full_speed);
    runtime_assert_eq!(
        app.frame_skip => 2;
        runtime_flash_text(&app) => Some("Speed: 2x");
    );
    app.test_key(VirtualKeyCode::NumpadAdd, ElementState::Released);
    app.test_key(VirtualKeyCode::NumpadAdd, ElementState::Pressed);
    assert_eq!(app.frame_skip, 3);

    for expected in [2, 1] {
        app.test_key(VirtualKeyCode::NumpadSubtract, ElementState::Pressed);
        assert_eq!(app.frame_skip, expected);
    }
    assert!(!app.full_speed);
    runtime_assert_eq!(runtime_flash_text(&app) => Some("Speed: 1x"));

    app.frame_skip = 50;
    app.full_speed = false;
    app.test_key(VirtualKeyCode::NumpadAdd, ElementState::Pressed);
    assert_eq!(app.frame_skip, 50);
    assert!(app.full_speed);
    runtime_assert_eq!(runtime_flash_text(&app) => Some("Speed: 50x"));

    let mut rebound = new_running_sandbox_app();
    install_runtime_key_config(
        &mut rebound,
        Ok(
            parse_runtime_key_config(b"[Keys]\nGameSpeedUp=G\nGameSlowDown=H\n")
                .expect("parse rebound speed keys"),
        ),
    );
    rebound.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert_eq!(rebound.frame_skip, 2);
    rebound.test_key(VirtualKeyCode::KeyH, ElementState::Pressed);
    assert_eq!(rebound.frame_skip, 1);
    assert!(!rebound.full_speed);

    let mut global_collision = new_running_sandbox_app();
    global_collision.app_paths = None;
    install_runtime_key_config(
        &mut global_collision,
        Ok(
            parse_runtime_key_config(b"[Keys]\nSoundToggle=G\nGameSpeedUp=G\n")
                .expect("parse earlier-global collision"),
        ),
    );
    let sound_enabled = global_collision.test_audio_ref().options.sound_enabled;
    global_collision.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    runtime_assert_eq!(
        global_collision.test_audio_ref().options.sound_enabled => !sound_enabled;
        global_collision.frame_skip => 1;
    );
    assert!(!global_collision.full_speed);
    assert!(global_collision.runtime_flash_message.is_none());

    let mut collision = new_running_sandbox_app();
    install_runtime_key_config(
        &mut collision,
        Ok(
            parse_runtime_key_config(b"[Keys]\nKbd1Key1=G\nGameSpeedUp=G\n")
                .expect("parse player/global collision"),
        ),
    );
    collision.test_key(VirtualKeyCode::KeyG, ElementState::Pressed);
    assert_eq!(collision.frame_skip, 1);
    assert!(!collision.full_speed);
    assert!(collision.runtime_flash_message.is_none());
    collision.test_key(VirtualKeyCode::KeyG, ElementState::Released);
}

#[test]
fn runtime_f3_priority_matrix_covers_every_recursive_running_layer() {
    #[derive(Clone, Copy, Debug)]
    enum Layer {
        Message,
        Context,
        Scoreboard,
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
                .test_value(),
            Layer::Context => {
                app.open_context_menu_at(
                    vec![ContextMenuEntry::<AppContextMenuCommand>::new("Root")
                        .with_submenu(vec![ContextMenuEntry::new("Child")])],
                    GuiPoint::new(24.0, 24.0),
                )
                .test_value();
            }
            Layer::Object => {
                assert!(app.open_object_menu().expect("open object state"));
            }
            Layer::Observer => {
                app.engine.remove_player(app.local_owner).test_value();
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
                    app.engine.restore_state(&state).test_value();
                    app.snapshot = app.engine.snapshot();
                }
                app.handle_game_over().test_value();
            }
            Layer::Scoreboard => {}
        }
        app
    };

    for layer in [
        Layer::Message,
        Layer::Context,
        Layer::Scoreboard,
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
            let before = default_app.runtime_flash_message.test_ref().remaining_draws;
            let mut frame = vec![0_u8; 320 * 200 * 4];
            default_app
                .render(&mut frame)
                .unwrap_or_else(|error| panic!("render F3 on {layer:?}: {error:#}"));
            runtime_assert_eq!(default_app.runtime_flash_message.as_ref().expect("music text lasts more than one draw").remaining_draws => before - 1);
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
        runtime_assert_eq!(rebound.runtime_flash_message.is_none() => player_scope, "{layer:?}");

        let mut sound = make_layer(layer);
        let before = sound.test_audio_ref().options.sound_enabled;
        sound.test_modifiers(ModifiersState::CONTROL);
        sound
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .unwrap_or_else(|error| panic!("Ctrl+F3 on {layer:?}: {error}"));
        runtime_assert_eq!(sound.test_audio_ref().options.sound_enabled => !before, "{layer:?}");
        assert!(sound.runtime_flash_message.is_none(), "{layer:?}");
    }
}

#[test]
fn generated_team_name_template_preserves_the_runtime_table_charset() {
    let cp1252 = RuntimeLanguageTable {
        charset: RuntimeHelpCharset::Windows1252,
        entries: HashMap::from([("IDS_MSG_TEAM".to_string(), "Équipe %d".to_string())]),
    };
    runtime_assert_eq!(generated_team_name_template(&cp1252).as_bytes() => b"\xc9quipe %d");

    let utf8 = RuntimeLanguageTable {
        charset: RuntimeHelpCharset::Utf8,
        entries: cp1252.entries,
    };
    runtime_assert_eq!(generated_team_name_template(&utf8).as_bytes() => "Équipe %d".as_bytes());
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

    runtime_assert_eq!(
        app.runtime_resource_text("IDS_TEST_PROCESS_RESOURCE", "fallback") => "process cached É";
        app.runtime_resource_bytes("IDS_TEST_PROCESS_RESOURCE") => b"process cached \xc9";
    );
}

#[test]
fn process_language_table_survives_disk_edits_until_an_explicit_options_reload() {
    let fixture = runtime_install_fixture(Some("[General]\nLanguage=US\nLanguageEx=US\n"));
    let language = fixture.system.join("LanguageUS.txt");
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
    fs::write(&language, table("Loaded", "")).test_value();
    let paths = &fixture.paths;

    let mut app = new_real_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    app.reload_application_language_resources().test_value();
    runtime_assert_eq!(app.runtime_language_charset => RuntimeHelpCharset::Windows1252);

    fs::write(&language, table("Mutated", "UTF-8")).test_value();
    let disk = load_runtime_language_table(Some(paths)).test_value();
    runtime_assert_eq!(
        disk.entries.get("IDS_MSG_SELECT").map(String::as_str) => Some("Mutated select %s");
        disk.charset => RuntimeHelpCharset::Utf8;
        load_options_program_state(Some(paths), Some(&app.startup_tooltip_resources),) .no_language_info => "Loaded no language info";
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
    runtime_assert!(
        loads_before_browser >= 2,
        "the probe observes the initial process load and explicit disk inspection"
    );
    for _ in 0..3 {
        app.sync_startup_network_game_rows();
        let row = &app.startup_network_dialog.test_ref().games()[0];
        assert_eq!(row.title, "Loaded HarpoonRace on Host");
        let mut frame = vec![0_u8; 640 * 480 * 4];
        app.test_render(&mut frame);
    }
    runtime_assert_eq!(runtime_language_table_load_count(paths.system_group_path()) => loads_before_browser, "network-browser row projection and rendering must not reopen System.c4g");

    runtime_assert_eq!(
        app.runtime_resource_text("IDS_MSG_SELECT", "fallback") => "Loaded select %s";
        app.runtime_resource_bytes("IDS_MSG_NOSPLITSCREENINLEAGUE") => b"Loaded players %s and %s";
        app.new_startup_player_properties_controller(0, 0).comment() => "Loaded new player";
    );

    app.control_clients
        .replace_snapshot([message_client(7, b"Remote")]);
    app.append_remote_lobby_ready_log(clonk_network::ReadyCheckPacket::new(
        7,
        clonk_network::ReadyCheckData::Ready,
    ));
    runtime_assert_eq!(latest_message_board_logical_entry(&app).as_deref() => Some("Loaded ready Remote."));

    install_test_classic_host_team_lobby(&mut app);
    app.process_classic_lobby_chat_request(LobbyChatRequest::Submit("/help".to_string()))
        .test_value();
    runtime_assert!(app
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
    .test_value();
    let menu = app.context_menu.test_mut();
    let first = menu.layout().panels[0].rows[0].rect;
    menu.handle_pointer_move(GuiPoint::new((first.x + 1) as f32, (first.y + 1) as f32));
    runtime_assert!(
        menu.hovered_tooltip_at(Instant::now() + Duration::from_secs(1))
            .is_some_and(|tooltip| tooltip.starts_with("Loaded select ")),
        "team selector tooltip must not observe the on-disk edit",
    );
    app.close_context_menu_silently();

    app.network_is_league = true;
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: 5,
        check_league_rules: true,
        confirm_unassociated_savegame_players: false,
    }])
    .test_value();
    let league = app.message_dialogs.last().test_value();
    runtime_assert_eq!(
        league.state.message() => "Loaded players Chooser and Companion";
        league.state.caption() => "Loaded league error";
    );
    app.message_dialogs.clear();
    app.network_is_league = false;

    app.process_classic_lobby_actions(vec![ClassicLobbyAction::StartRequested {
        countdown_seconds: 5,
        check_league_rules: false,
        confirm_unassociated_savegame_players: true,
    }])
    .test_value();
    let confirmation = app.message_dialogs.last().test_value();
    assert_eq!(confirmation.state.message(), "Loaded unassociated players");
    assert_eq!(confirmation.state.caption(), "Loaded player assignment");

    app.reload_application_language_resources().test_value();
    runtime_assert_eq!(
        app.runtime_language_charset => RuntimeHelpCharset::Utf8;
        app.runtime_resource_text("IDS_MSG_SELECT", "fallback") => "Mutated select %s";
        app.new_startup_player_properties_controller(0, 0).comment() => "Mutated new player";
        load_options_program_state(Some(paths), Some(&app.startup_tooltip_resources),).no_language_info => "Mutated no language info";
    );
}

#[test]
fn runtime_join_flash_keeps_the_process_language_charset_until_reload() {
    let fixture = runtime_install_fixture(Some("[General]\nLanguage=US\nLanguageEx=US\n"));
    let language = fixture.system.join("LanguageUS.txt");
    let mut initial = b"IDS_LANG_CHARSET=\nIDS_NET_RUNTIMEJOINFREE=".to_vec();
    initial.extend(std::iter::repeat_n(0xe9, 300));
    initial.extend_from_slice(b"\nIDS_NET_RUNTIMEJOINBARRED=Cached barred\nIDS_MSG_TEAM=Team %d\n");
    fs::write(&language, initial).test_value();
    let paths = &fixture.paths;

    let mut app = new_classic_running_sandbox_app();
    app.app_paths = Some(paths.clone());
    app.reload_application_language_resources().test_value();
    fs::write(
        &language,
        "IDS_LANG_CHARSET=UTF-8\n\
             IDS_NET_RUNTIMEJOINFREE=Reloaded free\n\
             IDS_NET_RUNTIMEJOINBARRED=Reloaded barred\n\
             IDS_MSG_TEAM=Team %d\n",
    )
    .test_value();

    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
    runtime_assert!(matches!(
        app.runtime_network_role(),
        RuntimeNetworkRole::Host
    ));
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);
    let acknowledgement = thread::spawn(move || {
        let (allowed, completion) = commands.receive_join_allowed();
        assert!(allowed);
        completion.send(Ok(())).test_value();
    });
    app.apply_runtime_client_list_option(LobbyOptionKind::RuntimeJoin, 1)
        .test_value();
    acknowledgement.test_join();

    let flash = app.runtime_flash_message.test_ref();
    runtime_assert_eq!(flash.text.chars().count() => 300, "the retained CP1252 table keeps all 300 one-byte characters");
    assert!(flash.text.chars().all(|character| character == '\u{e9}'));
    runtime_assert_eq!(app.runtime_language_charset => RuntimeHelpCharset::Windows1252);

    app.reload_application_language_resources().test_value();
    runtime_assert_eq!(
        app.runtime_language_charset => RuntimeHelpCharset::Utf8;
        app.classic_lobby_option_labels().runtime_join_free => "Reloaded free";
    );
}

#[test]
fn runtime_f1_language_lookup_is_case_insensitive_and_skips_empty_candidates() {
    let _lock = env_lock().lock();
    let fixture = runtime_install_fixture(Some("[General]\nLanguageEx=ZZ,DE\n"));
    fs::write(fixture.system.join("LANGUAGEzz.TXT"), []).test_value();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../planet/System.c4g/LanguageDE.txt"),
        fixture.system.join("lAnGuAgEdE.TxT"),
    )
    .test_value();

    let table = load_runtime_language_table(Some(&fixture.paths)).test_value();
    let (need, none) = needed_material_resource_strings(&table);
    runtime_assert_eq!(
        need => "%s|braucht noch";
        none => "%s braucht kein|weiteres Baumaterial.";
        object_no_dig_resource_string(&table) => "%s kann|nicht graben.";
        default_rank_resource_names(&table)[1] => "Fähnrich";
    );
    let columns = build_runtime_help_columns(&table.entries).test_value();
    assert!(columns.left.starts_with("[Spielfunktionen]\n"));
    assert!(columns.left.contains("F1</c> - Hilfe"));
}

#[test]
fn runtime_definition_overload_uses_the_active_language_resource() {
    // `planet/System.c4g/LanguageDE.txt:1210` carries the localized
    // `IDS_PRC_DEFOVERLOAD` template, including its native alignment prefix.
    let fixture = runtime_install_fixture(Some("[General]\nLanguageEx=DE\n"));
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../planet/System.c4g/LanguageDE.txt"),
        fixture.system.join("LanguageDE.txt"),
    )
    .test_value();

    let table = load_runtime_language_table(Some(&fixture.paths)).test_value();
    runtime_assert_eq!(definition_overload_resource_string(&table) => "   %s (%s) überladen.");

    let mut app = new_real_menu_app(320, 240);
    app.app_paths = Some(fixture.paths.clone());
    app.reload_application_language_resources().test_value();
    let template = clonk_engine::scenario::verbose_loading::definition_overload_template();
    runtime_assert_eq!(
        template.as_ref() => "   %s (%s) überladen.";
        clonk_engine::scenario::verbose_loading::definition_overload_lines(1, template.as_ref(), "Stein", "ROCK", "Objects.c4d/Rock.c4d", "Mods.c4d/Rock.c4d",) =>
            vec!["   Stein (ROCK) überladen.".to_owned()];
    );
    clonk_engine::scenario::verbose_loading::set_definition_overload_template(
        clonk_engine::scenario::verbose_loading::DEFAULT_DEFINITION_OVERLOAD_TEMPLATE,
    );
}

/// `C4GraphicsSystem::DrawHelp` asks `GetKeyboardInputName` for each
/// registered key's *current* code, so a `KeyConfig` override changes the
/// displayed chord as well as the dispatch
/// (C4GraphicsSystem.cpp:692-724). The two columns keep their native draw
/// order and read the same process language table as the rest of the UI.
#[test]
fn runtime_f1_help_displays_live_remapped_key_names() {
    let mut app = new_running_sandbox_app();
    let default_columns = app.runtime_help_columns().test_value().clone();
    assert!(default_columns.left.contains("F1</c> - "));
    assert!(default_columns.left.contains("Tab</c> - "));

    install_runtime_key_config(
        &mut app,
        Ok(parse_runtime_key_config(b"[Keys]\nToggleShowHelp=Shift+H\nScoreboardToggle=Escape,Return\n                  MusicToggle=Joy1A\nDbgModeToggle=Ctrl+Alt+D\n",).expect("parse remapped help chords")),
    );
    // The columns are rebuilt lazily, so drop the memoized text.
    app.runtime_help_text_cache = OnceLock::new();
    let columns = app.runtime_help_columns().test_value().clone();

    // Each remapped action shows its live ordered binding name.
    assert!(columns.left.contains("Shift+H</c> - "), "{}", columns.left);
    assert!(!columns.left.contains("F1</c> - "), "{}", columns.left);
    // Only the first chord of an ordered list is shown for a single slot.
    assert!(columns.left.contains("Escape</c> - "), "{}", columns.left);
    assert!(!columns.left.contains("Tab</c> - "), "{}", columns.left);
    // A gamepad override has no keyboard name, exactly like an
    // unresolvable code.
    runtime_assert!(
        columns.left.contains("<c ffff00></c> - "),
        "{}",
        columns.left
    );
    // Modifier order follows C4KeyCodeEx::ToString.
    runtime_assert!(
        columns.right.contains("Ctrl+Alt+D</c> - "),
        "{}",
        columns.right
    );

    // Draw order and the localized right-hand column are untouched.
    assert!(columns.left.starts_with('['));
    runtime_assert_eq!(columns.right.lines().count() => default_columns.right.lines().count());
}

#[test]
fn runtime_language_table_loads_from_language_pack() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    fs::create_dir(install.path().join("planet/System.c4g/LanguageFI.txt")).test_value();
    let pack_system = install
        .path()
        .join("planet/Language.c4g/Finnish.c4g/System.c4g");
    fs::create_dir_all(&pack_system).test_value();
    fs::write(
        pack_system.join("LanguageFI.txt"),
        "IDS_LANG_CHARSET=UTF-8\nProbe=paketti\n",
    )
    .test_value();
    let decoy_system = install.path().join("Language.c4g/Decoy.c4g/System.c4g");
    fs::create_dir_all(&decoy_system).test_value();
    fs::write(
        decoy_system.join("LanguageUS.txt"),
        "IDS_LANG_CHARSET=UTF-8\nProbe=wrong namespace\n",
    )
    .test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US, FI\n").test_value();

    runtime_assert_eq!(
        classic_loader_language_sequence(&paths).expect("component language sequence") => vec!["US".to_string(), " F".to_string()];
        classic_runtime_language_sequence(&paths).expect("LoadLanguage sequence") => vec!["US".to_string(), "FI".to_string()];
    );

    let table = load_runtime_language_table(Some(&paths)).test_value();
    runtime_assert_eq!(
        table.charset => RuntimeHelpCharset::Utf8;
        table.entries.get("Probe").map(String::as_str) => Some("paketti");
    );
}

#[test]
fn global_system_scripts_use_pack_only_string_table() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let user_data = tempdir();
    let system = install.path().join("planet/System.c4g");
    fs::create_dir_all(&system).test_value();
    fs::write(
        system.join("Probe.c"),
        "global func PackProbe() { return \"$PackProbe$\"; }\n",
    )
    .test_value();
    let pack_system = install
        .path()
        .join("planet/Language.c4g/Finnish.c4g/System.c4g");
    fs::create_dir_all(&pack_system).test_value();
    fs::write(
        pack_system.join("StringTblUS.txt"),
        "PackProbe=Pack-localized global\n",
    )
    .test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();

    let group = Group::open(&system).test_value();
    let scripts = load_classic_global_system_scripts(&paths, &group).test_value();
    assert_eq!(scripts.len(), 1);
    assert!(scripts[0].1.contains("Pack-localized global"));
    assert!(!scripts[0].1.contains("$PackProbe$"));
}

#[test]
fn global_system_scripts_do_not_hide_invalid_explicit_language_config() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let user_data = tempdir();
    let system = install.path().join("planet/System.c4g");
    fs::create_dir_all(&system).test_value();
    fs::write(
        system.join("Probe.c"),
        "global func Probe() { return true; }\n",
    )
    .test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        format!("[General]\nLanguageEx={}\n", "X".repeat(1025)),
    )
    .test_value();

    let group = Group::open(&system).test_value();
    let error = load_classic_global_system_scripts(&paths, &group)
        .expect_err("explicit invalid config must not use the platform fallback");
    assert!(error.to_string().contains("LanguageEx"));
    assert!(error.to_string().contains("1024-byte"));
}

#[test]
fn runtime_f1_language_table_is_frozen_at_application_construction() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    persist_config_value(&paths, "General", "LanguageEx", "DE").test_value();

    let columns = app.runtime_help_columns().test_value();
    assert!(columns.left.starts_with("[Game Functions]\n"));
    assert!(!columns.left.contains("Spielfunktionen"));
    assert_eq!(app.needed_material_need, "%s|needs");
    assert_eq!(app.needed_material_none, "%s needs|no more material.");
    assert_eq!(app.object_no_dig, "%s cannot dig.");
}

#[test]
fn runtime_key_config_loads_known_remaps_from_directory_and_packed_groups() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let user_data = tempdir();
    let system = install.path().join("planet/System.c4g");
    let extra = install.path().join("planet/Extra.c4g");
    fs::create_dir_all(&system).test_value();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../planet/System.c4g/LanguageUS.txt"),
        system.join("LanguageUS.txt"),
    )
    .test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();

    fs::create_dir_all(&extra).test_value();
    fs::write(
        extra.join("kEyCoNfIg.TxT"),
        "[Keys]\nNetObsNextPlayer=Right,F5\n",
    )
    .test_value();
    let loaded = load_runtime_global_key_config(Some(&paths)).test_value();
    runtime_assert_eq!(
        loaded.net_observer_next_player =>
            vec![RuntimeKeyChord::keyboard(VirtualKeyCode::ArrowRight, ModifiersState::empty()), RuntimeKeyChord::keyboard(VirtualKeyCode::F5, ModifiersState::empty()),];
    );
    fs::write(
        extra.join("kEyCoNfIg.TxT"),
        "[Keys]\nToggleShowHelp=Shift+F2\nUnknownAction=F9\n",
    )
    .test_value();
    let loaded = load_runtime_global_key_config(Some(&paths)).test_value();
    runtime_assert_eq!(loaded.override_for("ToggleShowHelp") => Some([RuntimeKeyChord::keyboard(VirtualKeyCode::F2, ModifiersState::SHIFT,)].as_slice()));
    assert!(loaded.override_for("UnknownAction").is_none());
    guard_runtime_global_key_config(Some(&paths)).test_value();

    fs::remove_dir_all(&extra).test_value();
    fs::write(
        &extra,
        packed_test_file_group(&[(
            "KEYCONFIG.TXT",
            false,
            b"[Keys]\nNetObsNextPlayer=Right,F5\n",
        )]),
    )
    .test_value();
    let loaded = load_runtime_global_key_config(Some(&paths)).test_value();
    runtime_assert_eq!(
        loaded.net_observer_next_player =>
            vec![RuntimeKeyChord::keyboard(VirtualKeyCode::ArrowRight, ModifiersState::empty()), RuntimeKeyChord::keyboard(VirtualKeyCode::F5, ModifiersState::empty()),];
    );
    fs::write(
        &extra,
        packed_test_file_group(&[(
            "KEYCONFIG.TXT",
            false,
            b"[Keys]\nToggleShowHelp=F2\nUnknownAction=F9\n",
        )]),
    )
    .test_value();
    let loaded = load_runtime_global_key_config(Some(&paths)).test_value();
    runtime_assert_eq!(loaded.override_for("ToggleShowHelp") => Some([RuntimeKeyChord::keyboard(VirtualKeyCode::F2, ModifiersState::empty(),)].as_slice()));
    guard_runtime_global_key_config(Some(&paths)).test_value();

    fs::write(&extra, b"not a C4Group archive").test_value();
    let ignored = load_runtime_global_key_config(Some(&paths)).test_value();
    assert_eq!(ignored, RuntimeKeyConfig::default());
    guard_runtime_global_key_config(Some(&paths)).test_value();
}

#[test]
fn runtime_f1_key_config_ownership_is_snapshotted_once_per_game() {
    let _lock = env_lock().lock();
    let mut app = new_classic_running_sandbox_app();
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    app.app_paths = Some(paths);
    app.configure_running_state("First game".to_string(), DEFAULT_GROUND_HEIGHT);

    let extra = install.path().join("planet/Extra.c4g");
    fs::create_dir_all(&extra).test_value();
    fs::write(extra.join("KeyConfig.txt"), "[Keys]\nToggleShowHelp=F2\n").test_value();
    app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(app.runtime_help_visible);

    app.configure_running_state("Second game".to_string(), DEFAULT_GROUND_HEIGHT);
    app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(!app.runtime_help_visible);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    assert!(app.runtime_help_visible);
}

#[test]
fn runtime_f1_supports_every_upper_board_mode_and_mode_aware_geometry() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install.path()), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[Graphics]\nUpperBoard=Small\n").test_value();
    runtime_assert_eq!(load_display_flags(Some(&paths)).upper_board => UpperBoardMode::Small, "the production guard must see the persisted mode");

    for (mode, expected_top) in [
        (UpperBoardMode::Hide, 0),
        (UpperBoardMode::Full, 50),
        (UpperBoardMode::Small, 25),
        (UpperBoardMode::Mini, 0),
    ] {
        let mut app = new_classic_running_sandbox_app();
        app.display_flags.upper_board = mode;
        app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
        assert!(app.runtime_help_visible, "mode {mode:?}");
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.test_render(&mut frame);
        runtime_assert_eq!(app.graphics.preferred_dialog_rect(None).y => expected_top, "mode {mode:?}");
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
    visible.test_render(&mut frame);
    assert_ne!(frame, sentinel);
    assert!(visible.runtime_help_visible);
    assert_eq!(visible.graphics.preferred_dialog_rect(None).y, 25);

    let mut recover = new_classic_running_sandbox_app();
    recover.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(recover.runtime_help_visible);
    recover.display_flags.upper_board = UpperBoardMode::Small;
    let mut frame = vec![0_u8; 320 * 200 * 4];
    recover.test_render(&mut frame);
    assert!(recover.runtime_help_visible);
    assert_eq!(recover.graphics.preferred_dialog_rect(None).y, 25);
    recover.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(!recover.runtime_help_visible);
}

#[test]
fn upper_board_display_toggle_reinitializes_geometry_synchronously() {
    let mut app = new_classic_running_sandbox_app();
    let owner = app.local_owner;
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let initial_strip_width = app.graphics.upper_board_text_strip_width();
    assert_eq!(app.graphics.preferred_dialog_rect(None).y, 50);
    assert_eq!(app.graphics.viewport_rect(owner).expect("viewport").y, 50);

    app.snapshot.game_time = 100 * 60 * 60;

    app.apply_ingame_menu_action(MenuAction::Display(DisplayToggle::UpperBoard))
        .test_value();

    assert_eq!(app.display_flags.upper_board, UpperBoardMode::Small);
    runtime_assert!(
        app.graphics.upper_board_text_strip_width() > initial_strip_width,
        "the synchronous reinitialization latches the current 100-hour game time"
    );
    runtime_assert_eq!(app.graphics.preferred_dialog_rect(None).y => 25, "Display:UpperBoard reinitializes viewport/dialog geometry before the next render");
    runtime_assert_eq!(
        app.graphics.viewport_rect(owner).expect("viewport").y => 25;
        app.graphics.preferred_dialog_rect(Some(owner)).y => 25;
        app.active_ingame_mouse_viewport().expect("active mouse viewport").rect.y => 25;
    );
}

#[test]
fn runtime_f1_help_toggles_beneath_nonmatching_running_layers() {
    let mut game_over = new_game_over_keyboard_app();
    game_over.test_key(VirtualKeyCode::F1, ElementState::Pressed);
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
        .test_value();
    message.test_key(VirtualKeyCode::F1, ElementState::Pressed);
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
        .test_value();
    assert!(context.context_menu.is_some());
    context.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(context.runtime_help_visible);
    assert!(context.context_menu.is_some());

    let mut object = new_classic_running_sandbox_app();
    assert!(object.open_object_menu().expect("open object menu"));
    object.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(object.runtime_help_visible);
    assert!(object.object_menu.is_some());

    let mut ingame = new_classic_running_sandbox_app();
    ingame.open_ingame_menu().test_value();
    ingame.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(ingame.runtime_help_visible);
    assert!(ingame.ingame_menu.is_some());
}

#[test]
fn custom_player_f1_binding_outranks_help_when_control_scope_is_active() {
    let mut app = new_running_sandbox_app();
    app.bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    app.engine
        .test_player_mut(app.local_owner)
        .control
        .control_style = true;
    app.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(!app.runtime_help_visible);
    runtime_assert_ne!(app.engine.player(app.local_owner).expect("local player").control.pressed_coms & (1 << clonk_engine::COM_LEFT) => 0);
    app.test_key(VirtualKeyCode::F1, ElementState::Released);
    assert!(!app.runtime_help_visible);

    let mut menu = new_running_sandbox_app();
    menu.bindings
        .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
    menu.open_ingame_menu().test_value();
    menu.test_key(VirtualKeyCode::F1, ElementState::Pressed);
    assert!(!menu.runtime_help_visible);
    assert!(menu.ingame_menu.is_some());
}

#[test]
fn secondary_auto_stop_key_config_f1_f3_binding_uses_matching_owner() {
    let mut app = new_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = add_secondary_local_player_for_mouse_option_test(&mut app);
    app.engine.test_player_mut(primary).control.control_style = false;
    app.engine.test_player_mut(secondary).control.control_style = true;
    app.snapshot = app.engine.snapshot();
    assert_eq!(app.local_controls.owner_for_set(1), Some(secondary));
    let left_mask = 1 << clonk_engine::COM_LEFT;

    for (key, source) in [
        (VirtualKeyCode::F1, b"[Keys]\nKbd2Key7=F1\n".as_slice()),
        (VirtualKeyCode::F3, b"[Keys]\nKbd2Key7=F3\n".as_slice()),
    ] {
        install_runtime_key_config(
            &mut app,
            Ok(parse_runtime_key_config(source).expect("parse secondary player remap")),
        );

        app.test_key(key, ElementState::Pressed);
        runtime_assert_eq!(app.engine.player(primary).expect("primary local player").control.pressed_coms & left_mask => 0);
        runtime_assert_ne!(app.engine.player(secondary).expect("secondary local player").control.pressed_coms & left_mask => 0);
        assert!(app.pressed_engine_keys.contains(&key));
        assert!(!app.runtime_help_visible);
        assert!(app.runtime_flash_message.is_none());

        app.test_key(key, ElementState::Released);
        runtime_assert_eq!(
            app.engine.player(secondary).expect("secondary local player").control.pressed_coms & left_mask => 0,
            "{key:?} release must use the matching secondary owner's auto-stop style",
        );
        runtime_assert_eq!(app.engine.player(primary).expect("primary local player").control.pressed_coms & left_mask => 0);
        assert!(!app.pressed_engine_keys.contains(&key));
        assert!(!app.runtime_help_visible);
        assert!(app.runtime_flash_message.is_none());
    }
}

#[test]
fn modified_f1_does_not_match_an_unmodified_player_binding() {
    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::SUPER | ModifiersState::SHIFT,
    ] {
        let mut app = new_running_sandbox_app();
        app.bindings
            .rebind(ControlBindingId::Left, VirtualKeyCode::F1);
        app.engine
            .test_player_mut(app.local_owner)
            .control
            .control_style = true;
        app.test_modifiers(modifiers);
        let pressed_coms = app.engine.test_player(app.local_owner).control.pressed_coms;
        let pressed_engine_keys = app.pressed_engine_keys.clone();
        assert!(app.show_startup_hint);

        for state in [ElementState::Pressed, ElementState::Released] {
            app.test_key(VirtualKeyCode::F1, state);
            assert!(!app.runtime_help_visible, "modifiers {modifiers:?}");
            runtime_assert_eq!(app.engine.player(app.local_owner).expect("local player").control.pressed_coms => pressed_coms, "modifiers {modifiers:?}, state {state:?}");
            let mut expected_raw_keys = pressed_engine_keys.clone();
            match state {
                ElementState::Pressed => {
                    expected_raw_keys.insert(VirtualKeyCode::F1);
                }
                ElementState::Released => {
                    expected_raw_keys.remove(&VirtualKeyCode::F1);
                }
            }
            runtime_assert_eq!(app.pressed_engine_keys => expected_raw_keys, "raw physical state precedes modified priority dispatch: modifiers {modifiers:?}, state {state:?}");
            runtime_assert!(
                app.show_startup_hint,
                "modifiers {modifiers:?}, state {state:?}"
            );
        }
    }
}

#[test]
fn modified_f1_refuses_an_unrepresented_key_config_on_both_edges() {
    let mut app = new_running_sandbox_app();
    install_runtime_key_config(
        &mut app,
        Err("Extra.c4g/KeyConfig.txt override".to_string()),
    );
    app.test_modifiers(ModifiersState::ALT);

    for state in [ElementState::Pressed, ElementState::Released] {
        let error = app
            .handle_key(VirtualKeyCode::F1, state)
            .expect_err("custom global-key ownership must precede modifier fallthrough");
        runtime_assert!(
            matches!(error, EngineError::ClassicMenuParityBoundary {..});
            !app.runtime_help_visible;
        );
    }
}

#[test]
fn unresolved_runtime_help_language_fails_typed_before_pixels() {
    let mut input_app = new_running_sandbox_app();
    install_runtime_key_config(
        &mut input_app,
        Err("Extra.c4g/KeyConfig.txt override".to_string()),
    );
    let error = input_app
        .handle_key(VirtualKeyCode::F1, ElementState::Pressed)
        .expect_err("unrepresented key config must fail before toggling");
    runtime_assert!(
        matches!(error, EngineError::ClassicMenuParityBoundary {..});
        !input_app.runtime_help_visible;
    );

    let mut app = new_classic_running_sandbox_app();
    app.runtime_help_visible = true;
    app.runtime_help_text_cache = OnceLock::new();
    app.runtime_help_text_cache
        .set(Err("LanguageZZ.txt cannot be resolved".to_string()))
        .test_value();
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
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL,
        ModifiersState::ALT | ModifiersState::SHIFT,
        ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
        ModifiersState::SUPER | ModifiersState::SHIFT,
    ] {
        let mut app = new_running_sandbox_app();
        app.status_text.clear();
        app.snapshot.hud.messages.clear();
        app.test_modifiers(modifiers);
        let before = runtime_global_ui_snapshot(&app);
        let mut before_pixels = vec![0_u8; 320 * 200 * 4];
        app.test_render(&mut before_pixels);

        tap_runtime_key(&mut app, VirtualKeyCode::F1);

        let after = runtime_global_ui_snapshot(&app);
        runtime_assert_eq!(
            after.status_text => before.status_text;
            after.message_dialogs => before.message_dialogs;
            after.game_over_open => before.game_over_open;
            after.ingame_page => before.ingame_page;
            after.object_menu_open => before.object_menu_open;
            after.context_menu_open => before.context_menu_open;
            after.runtime_help_visible => before.runtime_help_visible;
            after.pressed_engine_keys => before.pressed_engine_keys;
            after.message_dialog_consumed_keys => before.message_dialog_consumed_keys;
        );
        let mut after_pixels = vec![0_u8; 320 * 200 * 4];
        app.test_render(&mut after_pixels);
        assert_eq!(after_pixels, before_pixels);
    }
}

#[test]
fn f4_player_tooltip_names_follow_retained_visibility_and_effective_name() {
    let mut app = new_classic_running_sandbox_app();
    let (_events, _commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host"), message_client(7, b"Remote")]);
    app.control_player_infos.replace_snapshot(
        3,
        [clonk_engine::PlayerInfoControlData::new(
            7,
            0,
            vec![
                clonk_engine::ControlPlayerInfoEntry {
                    id: 1,
                    name: legacy_cstring(b"Raw"),
                    league_account: legacy_cstring(b"Visible account"),
                    ..Default::default()
                },
                clonk_engine::ControlPlayerInfoEntry {
                    id: 2,
                    name: legacy_cstring(b"Removed"),
                    flags: clonk_engine::PLAYER_INFO_FLAG_REMOVED,
                    ..Default::default()
                },
                clonk_engine::ControlPlayerInfoEntry {
                    id: 3,
                    name: legacy_cstring(b"Invisible"),
                    flags: clonk_engine::PLAYER_INFO_FLAG_INVISIBLE,
                    ..Default::default()
                },
            ],
            -1,
        )],
    );

    let (_, rows, _) = app.runtime_client_list_snapshot();
    runtime_assert_eq!(rows.iter().find(|row| row.client_id == 7).map(|row| row.player_names.clone()) => Some(vec!["Visible account".to_string()]));
}

#[test]
fn f4_control_mode_waits_for_status_commit() {
    let mut app = new_classic_running_sandbox_app();
    let (_events, mut commands) = install_running_network_stub(&mut app, 0, 40, 4);
    app.runtime_network_control_mode = Some(0);
    app.runtime_network_committed_control_mode = Some(0);
    app.control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    let labels = app.classic_lobby_option_labels();
    app.test_key(VirtualKeyCode::F4, ElementState::Pressed);

    app.apply_runtime_client_list_option(LobbyOptionKind::ControlMode, 1)
        .test_value();
    runtime_assert_eq!(
        app.runtime_network_control_mode => Some(1);
        app.runtime_network_committed_control_mode => Some(0);
        app.runtime_client_list .as_ref() .expect("F4 dialog remains open") .option_rows() .iter() .find(|row| row.kind == LobbyOptionKind::ControlMode) .map(|row| row.value.as_str()) =>
            Some(labels.control_mode_decentral.as_str());
    );

    let expected = clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_GO, 1, 40);
    runtime_assert!(commands
        .take_runtime_status_commands()
        .iter()
        .any(|command| command == &network::TestRuntimeStatusCommand::Change(expected)));
    app.handle_status_committed(expected).test_value();
    app.refresh_runtime_client_list();
    runtime_assert_eq!(
        app.runtime_network_committed_control_mode => Some(1);
        app.runtime_client_list .as_ref() .expect("F4 dialog remains open") .option_rows() .iter() .find(|row| row.kind == LobbyOptionKind::ControlMode) .map(|row| row.value.as_str()) =>
            Some(labels.control_mode_central.as_str());
    );

    app.host_reference_paused = true;
    app.apply_runtime_client_list_option(LobbyOptionKind::ControlMode, 0)
        .test_value();
    let paused = clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_PAUSE, 0, 40);
    runtime_assert!(commands
        .take_runtime_status_commands()
        .iter()
        .any(|command| command == &network::TestRuntimeStatusCommand::Change(paused)));
    app.handle_status_committed(paused).test_value();
    app.refresh_runtime_client_list();
    runtime_assert_eq!(
        app.runtime_network_committed_control_mode => Some(1);
        app.runtime_client_list .as_ref() .expect("F4 dialog remains open") .option_rows() .iter() .find(|row| row.kind == LobbyOptionKind::ControlMode) .map(|row| row.value.as_str()) =>
            Some(labels.control_mode_central.as_str());
    );

    let resumed = clonk_network::NetworkStatus {
        state: clonk_network::NETWORK_STATE_GO,
        ..paused
    };
    app.handle_status_committed(resumed).test_value();
    app.refresh_runtime_client_list();
    runtime_assert_eq!(
        app.runtime_network_committed_control_mode => Some(0);
        app.runtime_client_list .as_ref() .expect("F4 dialog remains open") .option_rows() .iter() .find(|row| row.kind == LobbyOptionKind::ControlMode) .map(|row| row.value.as_str()) =>
            Some(labels.control_mode_decentral.as_str());
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
        .test_value();

    let initial_frame = app.engine.frame();
    app.engine
        .call_scenario_script_function("Halt", Vec::new())
        .test_value();
    app.test_update();
    assert_eq!(app.engine.frame(), initial_frame);
    assert_ne!(app.offline_halt_count, 0);

    app.engine
        .call_scenario_script_function("Toggle", Vec::new())
        .test_value();
    app.test_update();
    assert_eq!(app.engine.frame(), initial_frame + 1);
    assert_eq!(app.offline_halt_count, 0);

    app.engine
        .call_scenario_script_function("Toggle", Vec::new())
        .test_value();
    app.test_update();
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
        .test_value();
    game_over
        .engine
        .call_scenario_script_function("Halt", Vec::new())
        .test_value();
    game_over
        .engine
        .call_scenario_script_function("Toggle", Vec::new())
        .test_value();
    game_over.test_update();
    runtime_assert_eq!(game_over.offline_halt_count => 1, "evaluation keeps the halt acquired by C4GameOverDlg::OnShown");
    game_over.test_modifiers(ModifiersState::ALT);
    game_over.test_key(VirtualKeyCode::KeyC, ElementState::Pressed);
    assert!(game_over.game_over_dialog.is_none());
    game_over.test_update();
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
        let go = clonk_network::NetworkStatus::new(clonk_network::NETWORK_STATE_GO, 1, 0);
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
                script: legacy_cstring(script),
                by_client: 0,
            })],
        );
        app.handle_status_committed(go).test_value();

        if local_client_id == 0 {
            let pause = app.runtime_network_status_barrier.test_value().status;
            assert_eq!(pause.state, clonk_network::NETWORK_STATE_PAUSE);
            assert!(app.league_votes.paused_for_vote);
        } else {
            runtime_assert!(
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
                by_client: i32::try_from(local_client_id).test_value(),
            })
            .collect::<Vec<_>>();
        runtime_assert_eq!(commands.take_submitted_votes() => expected_votes, "client {local_client_id} observes the native status ordering for {script:?}");
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
        runtime_assert_eq!(parse_native_config_bool(raw) => expected, "{raw:?} must follow the native Boolean grammar");
    }

    // Invalid input keeps each key's adapted default, in both directions.
    let flags =
        |body: &str| runtime_config_value(Some(body), |paths| load_display_flags(Some(paths)));

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
        runtime_assert!(
            kept.player_names,
            "{invalid:?} must leave the default-true ShowCrewNames alone"
        );
        runtime_assert!(
            !kept.clock,
            "{invalid:?} must leave the default-false ShowClock alone"
        );
    }
}

#[test]
fn ingame_display_toggles_wait_for_shutdown_and_reopen_the_same_selection() {
    // C4MainMenu.cpp:855-882 mutates Config in memory and reopens the page;
    // C4Config.cpp:381,455-466 declares the five persisted keys, and
    // C4Application.cpp:351-367 is the save site. The Display page owns only
    // these five persisted keys here; the remaining toggles belong to other
    // save-site audits and must not be written as a side effect.
    let mut app = new_state_only_lightweight_running_sandbox_app();
    let user_data = tempdir();
    let repository = test_repository_root();
    let (_guard, paths) = guarded_test_app_paths(Some(repository), user_data.path());
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        "[General]\nFPS=false\nUnrelatedGeneral=keep\n\n[Graphics]\nShowCrewNames=true\nShowCrewCNames=true\nShowClock=false\nUpperBoard=Full\nUnrelatedGraphics=keep\n",
    )
    .test_value();
    app.app_paths = Some(paths.clone());
    let initial_config = fs::read(paths.config_file()).test_value();

    app.apply_ingame_menu_action(MenuAction::ActivateDisplay)
        .test_value();
    app.ingame_menu
        .get_mut(app.local_owner)
        .test_mut()
        .set_selection(1);
    for toggle in [
        DisplayToggle::PlayerNames,
        DisplayToggle::ClonkNames,
        DisplayToggle::Clock,
        DisplayToggle::Fps,
        DisplayToggle::UpperBoard,
    ] {
        app.apply_ingame_menu_action(MenuAction::Display(toggle))
            .test_value();
    }
    for toggle in [
        DisplayToggle::Portraits,
        DisplayToggle::ShowCommands,
        DisplayToggle::ShowCommandKeys,
        DisplayToggle::WhiteChat,
    ] {
        app.apply_ingame_menu_action(MenuAction::Display(toggle))
            .test_value();
    }

    runtime_assert_eq!(app.ingame_menu.get(app.local_owner).test_value().selection() => 1);
    assert!(!app.display_flags.player_names);
    assert!(!app.display_flags.clonk_names);
    assert!(app.display_flags.clock);
    assert!(app.display_flags.fps);
    assert_eq!(app.display_flags.upper_board, UpperBoardMode::Small);
    assert_eq!(app.deferred_config.len(), 5);

    runtime_assert_eq!(fs::read(paths.config_file()).test_value() => initial_config, "Display toggles mutate the process-local config only until shutdown");
    app.flush_deferred_config();
    let after_flush = Config::load(paths.config_file()).test_value();
    runtime_assert_eq!(
        after_flush.get_in(Some("Graphics"), "ShowCrewNames") => Some("false");
        after_flush.get_in(Some("Graphics"), "ShowCrewCNames") => Some("false");
        after_flush.get_in(Some("Graphics"), "ShowClock") => Some("true");
        after_flush.get_in(Some("Graphics"), "UpperBoard") => Some("Small");
        after_flush.get_in(Some("General"), "FPS") => Some("true");
        after_flush.get_in(Some("General"), "UnrelatedGeneral") => Some("keep");
        after_flush.get_in(Some("Graphics"), "UnrelatedGraphics") => Some("keep");
    );

    let reopened = load_display_flags(Some(&paths));
    assert!(!reopened.player_names);
    assert!(!reopened.clonk_names);
    assert!(reopened.clock);
    assert!(reopened.fps);
    assert_eq!(reopened.upper_board, UpperBoardMode::Small);
}

/// `Graphics.ShowStats` has no oracle counterpart — it opts in to the
/// port's diagnostics overlay, which reports the presentation half of the
/// frame budget `General.FPS` (C4Game::FPS) structurally cannot see. It is
/// off unless the player asked for it, so a config LegacyClonk wrote still
/// produces LegacyClonk's screen, and it follows the same native Boolean
/// grammar as every key beside it.
#[test]
fn show_stats_is_opt_in_and_follows_the_native_boolean_grammar() {
    let flags =
        |body: &str| runtime_config_value(Some(body), |paths| load_display_flags(Some(paths)));

    runtime_assert!(
        !flags("[Graphics]\nShowClock=1\n").show_stats,
        "an untouched key leaves the overlay off"
    );
    assert!(flags("[Graphics]\nShowStats=1\n").show_stats);
    assert!(!flags("[Graphics]\nShowStats=0\n").show_stats);
    for invalid in ["TRUE", "yes", "on", "wobble"] {
        runtime_assert!(
            !flags(&format!("[Graphics]\nShowStats={invalid}\n")).show_stats,
            "{invalid:?} must leave the default-false ShowStats alone"
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
        let config = format!("[Sound]\n{body}\n");
        runtime_config_value(Some(&config), |paths| AudioOptions::load(Some(paths)))
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

/// `C4FullScreen::Init` titles its carrier window with `STD_PRODUCT`,
/// which is `C4ENGINECAPTION` = "LegacyClonk" (C4FullScreen.cpp:474-480;
/// C4Version.h:19,24). The developer console keeps its own caption, and
/// neither may be confused with `PRODUCT_NAME`, which names the port's
/// user-data directories.
#[test]
fn startup_window_builder_uses_legacyclonk_product_title() {
    assert_eq!(native_window_title(false), "LegacyClonk");
    assert_eq!(native_window_title(true), "LegacyClonk Console");
    assert_eq!(clonk_platform::ENGINE_CAPTION, "LegacyClonk");

    // The user-data product name is a deliberate divergence and must not
    // have been dragged along with the caption.
    assert_eq!(clonk_platform::PRODUCT_NAME, "Clonk Rust");
    assert_ne!(native_window_title(false), clonk_platform::PRODUCT_NAME);

    // The console caption is distinct from the game one and derived from
    // the same engine name.
    assert_ne!(native_window_title(true), native_window_title(false));
    assert!(native_window_title(true).starts_with(native_window_title(false)));
}

/// `C4GraphicsSystem::StartDrawing` refuses to draw while the application
/// is inactive unless `Graphics.RenderInactive` carries the *active
/// shell's* bit — Fullscreen `1 << 0`, Console `1 << 1`, adapted default
/// Console alone (C4Config.h:128-129; C4Config.cpp:481;
/// C4GraphicsSystem.cpp:96-106).
#[test]
fn the_profile_takes_the_cpp_render_inactive_default_but_never_a_written_one() {
    use crate::settings::CompatProfile;

    // C4Config.cpp:481 defaults RenderInactive to Console alone; the port ships
    // both bits so an Alt-Tabbed game keeps drawing (clonk-org/clonk-rs#57).
    // Only the *default* diverges, so the profile reverts the default and
    // leaves a written value exactly as the player wrote it -- reverting that
    // too would overrule an explicit choice, which no other overlay in the
    // profile does.
    let mask = |body, profile| {
        runtime_config_value(body, |paths| {
            load_render_inactive_mask(Some(paths), profile)
        })
    };

    assert_eq!(
        mask(None, CompatProfile::Normal),
        RENDER_INACTIVE_FULLSCREEN | RENDER_INACTIVE_CONSOLE
    );
    assert_eq!(
        mask(None, CompatProfile::LegacyClonk),
        RENDER_INACTIVE_CONSOLE
    );

    // An unparsable value still falls back to the profile's default.
    assert_eq!(
        mask(
            Some("[Graphics]\nRenderInactive=always\n"),
            CompatProfile::LegacyClonk
        ),
        RENDER_INACTIVE_CONSOLE
    );

    // A written value wins under either profile, including the port's own.
    for profile in [CompatProfile::Normal, CompatProfile::LegacyClonk] {
        assert_eq!(mask(Some("[Graphics]\nRenderInactive=1\n"), profile), 1);
        assert_eq!(mask(Some("[Graphics]\nRenderInactive=3\n"), profile), 3);
        assert_eq!(mask(Some("[Graphics]\nRenderInactive=0\n"), profile), 0);
    }
}

#[test]
fn render_inactive_bitmask_gates_unfocused_fullscreen_and_console_redraw() {
    let mask = |body| {
        runtime_config_value(body, |paths| {
            load_render_inactive_mask(Some(paths), crate::settings::CompatProfile::Normal)
        })
    };

    // The shipped default survives an unparsable value or an absent key. It
    // carries both bits rather than C++'s Console alone; see
    // `the_shipped_default_keeps_an_unfocused_game_window_drawing`.
    let shipped = RENDER_INACTIVE_FULLSCREEN | RENDER_INACTIVE_CONSOLE;
    assert_eq!(mask(None), shipped);
    assert_eq!(mask(Some("[Graphics]\nName=Tester\n")), shipped);
    assert_eq!(mask(Some("[Graphics]\nRenderInactive=always\n")), shipped);
    // Explicit masks, including hex, are honoured verbatim.
    assert_eq!(mask(Some("[Graphics]\nRenderInactive=0\n")), 0);
    assert_eq!(mask(Some("[Graphics]\nRenderInactive=1\n")), 1);
    assert_eq!(mask(Some("[Graphics]\nRenderInactive=3\n")), 3);
    assert_eq!(mask(Some("[Graphics]\nRenderInactive=0x3\n")), 3);

    // An active window always draws, whatever the mask says.
    for shell_is_console in [false, true] {
        for configured in [0, 1, 2, 3] {
            runtime_assert!(
                render_inactive_allows_drawing(configured, true, shell_is_console, true, false),
                "an active window always draws (mask={configured}, console={shell_is_console})"
            );
        }
    }

    // Inactive: each shell consults only its own bit.
    let game = |mask| render_inactive_allows_drawing(mask, false, false, true, false);
    let console = |mask| render_inactive_allows_drawing(mask, false, true, true, false);
    assert!(!game(0) && !console(0), "an empty mask suppresses both");
    assert!(game(RENDER_INACTIVE_FULLSCREEN));
    assert!(!console(RENDER_INACTIVE_FULLSCREEN));
    assert!(console(RENDER_INACTIVE_CONSOLE));
    assert!(!game(RENDER_INACTIVE_CONSOLE));
    let both = RENDER_INACTIVE_FULLSCREEN | RENDER_INACTIVE_CONSOLE;
    assert!(game(both) && console(both));

    // The oracle default therefore keeps an unfocused game window from
    // drawing while the developer console still repaints.
    assert!(!game(RENDER_INACTIVE_CONSOLE));
    assert!(console(RENDER_INACTIVE_CONSOLE));
}

/// The *shipped* default carries the Fullscreen bit as well, so an
/// Alt-Tabbed game keeps drawing. This is a deliberate divergence from the
/// oracle, approved 2026-08-05, fixing clonk-org/clonk-rs#57.
///
/// `C4GraphicsSystem::StartDrawing` refuses to draw an inactive window unless
/// the active shell's bit is set (C4GraphicsSystem.cpp:96-106), and `C4Config`
/// defaults that mask to `Console` alone (C4Config.cpp:481). But only the
/// graphics half of `C4Application::Execute` is gated on activity —
/// `Game.Execute()` is not (C4Application.cpp:451-478): the round keeps
/// executing, and in a network game it must, because lockstep means every
/// other peer is waiting on this client's control. The oracle default
/// therefore stops the picture at the moment of deactivation while the world
/// runs on, and refocus snaps it forward — a freeze followed by the
/// fast-forward filed as clonk-org/clonk-rs#56, over a session that never
/// actually stalled. Measured on macOS before this change: an inactive shell
/// presented **1 frame in 197 s while the simulation executed 7062**.
///
/// The divergence is the *default* only; a `RenderInactive` the player writes
/// is honoured verbatim in both directions, and `RenderInactive=2` restores
/// C++ exactly. It is safe because it is presentation-only and in the safe
/// direction: it adds frames, never removes a simulation step. Nothing on the
/// path reads or writes `C4Fixed`, `C4Random`, movement or control ordering,
/// and the gate is per-client local state no peer can observe, so two clients
/// configured differently stay in lockstep and cross-play against a stock
/// LegacyClonk client is unaffected. Neither `parity verify` nor
/// `engine-snapshots verify` can see it; neither presents.
#[test]
fn the_shipped_default_keeps_an_unfocused_game_window_drawing() {
    let mask = |body| {
        runtime_config_value(body, |paths| {
            load_render_inactive_mask(Some(paths), crate::settings::CompatProfile::Normal)
        })
    };

    let shipped = mask(None);
    runtime_assert_eq!(shipped => RENDER_INACTIVE_FULLSCREEN | RENDER_INACTIVE_CONSOLE, "both shells draw while inactive unless the player says otherwise");
    runtime_assert!(
        render_inactive_allows_drawing(shipped, false, false, true, false),
        "an Alt-Tabbed game window keeps its picture current"
    );
    runtime_assert!(
        render_inactive_allows_drawing(shipped, false, true, true, false),
        "the developer console keeps repainting exactly as C++ does"
    );

    // The oracle default remains reachable, and still withholds the game.
    let oracle = mask(Some("[Graphics]\nRenderInactive=2\n"));
    assert_eq!(oracle, RENDER_INACTIVE_CONSOLE);
    runtime_assert!(!render_inactive_allows_drawing(
        oracle, false, false, true, false
    ));

    // The advanced-config editor materializes what the engine actually
    // does, so opening the dialog and saving cannot write the oracle
    // default back in as an explicit key.
    let row = crate::advanced_config::sections(&Config::new())
        .into_iter()
        .flat_map(|section| section.rows)
        .find(|row| row.name == "RenderInactive")
        .test_value();
    assert_eq!(row.value.serialized(), "3");
}

/// The gate withholds frames from a window the display server is already
/// showing; it must never withhold the *first* one.
///
/// Win32 and SDL show a window whether or not the application has drawn
/// into it, so C++ cannot observe the difference. Wayland can: a surface is
/// mapped by its first committed buffer, and an unmapped surface is never
/// given keyboard focus — so suppressing frame one there deadlocks the
/// window into permanent invisibility (no frame → no map → no focus → no
/// frame). The same holds for any backend that maps on first content.
#[test]
fn the_inactive_gate_never_withholds_the_first_frame() {
    for shell_is_console in [false, true] {
        for configured in [0, 1, 2, 3] {
            runtime_assert!(
                render_inactive_allows_drawing(configured, false, shell_is_console, false, false),
                "frame one maps the window (mask={configured}, console={shell_is_console})"
            );
        }
    }

    // Once a frame is on screen the configured mask governs again.
    runtime_assert!(!render_inactive_allows_drawing(
        RENDER_INACTIVE_CONSOLE,
        false,
        false,
        true,
        false
    ));
}

/// A window the display server has hidden entirely gets no frames, whatever
/// the mask says.
///
/// This is the guard that makes drawing-while-inactive affordable: C++ never
/// needed one because Win32 deactivation *minimizes* the fullscreen window
/// (C4FullScreen.cpp:139-145), so its inactive gate already covered the
/// hidden case. Once the port keeps an unfocused window current, "inactive"
/// and "invisible" come apart — a second monitor still shows the game, a
/// minimized one shows nothing — and only the second deserves the refusal.
/// `WindowEvent::Occluded` is how the backends that can say so report it:
/// macOS from the window's occlusion state, X11 from a
/// `VisibilityFullyObscured` notify. Wayland and Windows never send it, and
/// there a hidden window keeps drawing exactly as if it were visible, which
/// costs a repaint nobody sees rather than risking a stall.
#[test]
fn a_hidden_window_draws_no_frames_however_the_mask_is_set() {
    for active in [false, true] {
        for console_shell in [false, true] {
            for configured in [0, 1, 2, 3] {
                assert!(
                    !render_inactive_allows_drawing(configured, active, console_shell, true, true),
                    "an occluded window has no picture to keep fresh \
                         (mask={configured}, active={active}, console={console_shell})"
                );
            }
        }
    }

    // Frame one still maps the window: the same deadlock
    // `the_inactive_gate_never_withholds_the_first_frame` describes applies
    // to any refusal here.
    assert!(render_inactive_allows_drawing(0, false, false, false, true));

    // Revealing it draws again on the very next opportunity.
    runtime_assert!(render_inactive_allows_drawing(
        RENDER_INACTIVE_FULLSCREEN,
        false,
        false,
        true,
        false
    ));
}

/// `C4Application::DoInit` leaves `Application.Active` set: only an OS
/// deactivation message clears it (C4Application.cpp; `CStdApp` activation
/// handling). Seeding it from the windowing system's focus report instead
/// starts every Wayland session inactive, because a surface cannot hold
/// focus before it has committed its first buffer.
#[test]
fn startup_activity_matches_native_rather_than_the_windowing_systems_focus_report() {
    runtime_assert!(
        initial_window_active();
        render_inactive_allows_drawing(RENDER_INACTIVE_CONSOLE, initial_window_active(), false, true, false);
    );
}

/// `C4ConfigLogging` (C4Config.cpp:699-718) carries a stdout level plus one
/// nested section per component, each with its own `LogLevel`. The port
/// projects them onto tracing filter directives so a shared config tunes
/// verbosity; `LC_LOG` keeps priority over it.
#[test]
fn logging_section_sets_stdout_level_and_per_component_overrides() {
    let directive =
        |body| runtime_config_value(body, |paths| load_logging_config_directive(Some(paths)));

    // Nothing configured leaves the caller's default in force.
    runtime_assert_eq!(
        directive(None) => None;
        directive(Some("[General]\nName=Tester\n")) => None;
        directive(Some("[Logging]\nLogLevelStdout=nonsense\n")) => None;
    );

    // LogLevelStdout raises the global level.
    runtime_assert_eq!(directive(Some("[Logging]\nLogLevelStdout=debug\n")) => Some("debug".to_string()));
    // spdlog spellings the port accepts.
    runtime_assert_eq!(
        directive(Some("[Logging]\nLogLevelStdout=warning\n")) => Some("warn".to_string());
        directive(Some("[Logging]\nLogLevelStdout=off\n")) => Some("off".to_string());
    );

    // A per-component override changes only that component.
    runtime_assert_eq!(directive(Some("[Network]\nLogLevel=trace\n")) => Some("clonk_network=trace".to_string()));
    // With both, the global level leads and the component follows.
    runtime_assert_eq!(directive(Some("[Logging]\nLogLevelStdout=info\n[Network]\nLogLevel=trace\n")) => Some("info,clonk_network=trace".to_string()));
    // An unknown component name is ignored rather than pinning a target
    // that does not exist.
    assert_eq!(directive(Some("[Nonsense]\nLogLevel=trace\n")), None);

    // Every component C++ compiles maps to a target.
    assert_eq!(clonk_logging::LOGGING_COMPONENTS.len(), 11);
    for (component, target) in clonk_logging::LOGGING_COMPONENTS {
        assert!(!target.is_empty(), "{component} has no target");
    }
}

/// `LoadLanguage`'s hardcoded US fallback, which runs when *nothing* in the
/// configured sequence resolves (`src/C4Language.cpp:262-263`, oracle
/// `7d43b47`):
///
/// ```cpp
/// for (int i = 0; SCopySegment(strLanguages, i, strLanguageCode, ',', 2, true); i++)
///     if (InitStringTable(strLanguageCode)) return true;
/// // No matching string table found: hardcoded fallback to US
/// if (InitStringTable("US")) return true;
/// ```
///
/// Every existing language test here has a code in the sequence that resolves,
/// so the fallback has never been the thing under test — "US" simply happened
/// to be requested. Here nothing in the sequence exists, which is the only way
/// to tell the fallback from an ordinary hit.
///
/// It also has to be a *full* `InitStringTable("US")`, walking System.c4g and
/// then the registered packs — not a System-only read — so the pack case is
/// covered too.
#[test]
fn runtime_language_falls_back_to_us_when_no_requested_code_resolves() {
    let _lock = env_lock().lock();
    let fixture = runtime_install_fixture(Some("[General]\nLanguageEx=ZZ,QQ\n"));
    fs::write(
        fixture.system.join("LanguageUS.txt"),
        "IDS_LANG_CHARSET=UTF-8\nProbe=fallback from System\n",
    )
    .test_value();

    let table = load_runtime_language_table(Some(&fixture.paths)).test_value();
    runtime_assert_eq!(
        table.entries.get("Probe").map(String::as_str) => Some("fallback from System");
    );

    // The same fallback, with LanguageUS.txt only inside a registered pack.
    let pack = runtime_install_fixture(Some("[General]\nLanguageEx=ZZ,QQ\n"));
    let pack_system = pack
        .install
        .path()
        .join("planet/Language.c4g/Finnish.c4g/System.c4g");
    fs::create_dir_all(&pack_system).test_value();
    fs::write(
        pack_system.join("LanguageUS.txt"),
        "IDS_LANG_CHARSET=UTF-8\nProbe=fallback from pack\n",
    )
    .test_value();

    let table = load_runtime_language_table(Some(&pack.paths)).test_value();
    runtime_assert_eq!(
        table.entries.get("Probe").map(String::as_str) => Some("fallback from pack");
    );
}

/// The other arm of the same tail: with no string table anywhere, C++ logs
/// `Error loading language string table.` and returns false
/// (`src/C4Language.cpp:264-266`).
///
/// The distinction worth pinning is that a missing table is an *error* rather
/// than an empty table — an empty one would silently render every UI string as
/// its raw identifier.
#[test]
fn runtime_language_reports_an_error_when_even_us_is_missing() {
    let _lock = env_lock().lock();
    let fixture = runtime_install_fixture(Some("[General]\nLanguageEx=ZZ,QQ\n"));
    // A decoy that is not a string table, so the group exists but resolves
    // nothing.
    fs::write(fixture.system.join("Probe.c"), "// not a string table\n").test_value();

    let error = load_runtime_language_table(Some(&fixture.paths))
        .expect_err("no string table is an error, not an empty table");
    assert!(
        error.to_string().contains("LanguageUS.txt is unavailable"),
        "the failure must name the missing fallback: {error}",
    );
}

/// `C4Group::LoadEntryString` fails outright on a zero-length entry
/// (`src/C4Group.cpp:2260-2261`: "other parts crash when they get a zero length
/// buffer, so fail here"), so an empty `LanguageXX.txt` is a miss rather than an
/// empty table — and the search continues past it, all the way to the US
/// fallback if need be.
///
/// `runtime_f1_language_lookup_is_case_insensitive_and_skips_empty_candidates`
/// covers an empty file being skipped in favour of a *later sequence entry*.
/// This covers it being skipped in favour of the fallback, which is the path
/// where an "accept the empty table" bug would strand the UI on raw identifiers.
#[test]
fn runtime_language_treats_an_empty_table_as_a_miss_not_an_empty_table() {
    let _lock = env_lock().lock();
    let fixture = runtime_install_fixture(Some("[General]\nLanguageEx=ZZ\n"));
    fs::write(fixture.system.join("LanguageZZ.txt"), []).test_value();
    fs::write(
        fixture.system.join("LanguageUS.txt"),
        "IDS_LANG_CHARSET=UTF-8\nProbe=US after the empty ZZ\n",
    )
    .test_value();

    let table = load_runtime_language_table(Some(&fixture.paths)).test_value();
    runtime_assert_eq!(
        table.entries.get("Probe").map(String::as_str) => Some("US after the empty ZZ");
    );
}

/// A player control outranks a pause chord rebound onto the same key
/// (clonk-org/clonk-rs#577).
///
/// `FullscreenPauseToggle` is registered with no explicit priority
/// (`C4Game.cpp:3429`), so it takes `C4CustomKey`'s default `PRIO_Base`
/// (`C4KeyboardInput.h:362`). `PRIO_PlrControl` is 7 against its 1
/// (`C4KeyboardInput.h:344-354`) — the highest real priority in the ladder —
/// so a player control always wins the collision.
///
/// This only becomes reachable once the key is *rebound*: the default Pause
/// key is not a player control, so the default configuration never collides
/// and the wrong order stays invisible. That is exactly why it needs pinning
/// rather than trusting the dispatch order.
#[test]
fn a_rebound_pause_chord_loses_to_the_player_control_on_the_same_key() {
    // Rebind pause onto a key the local player already drives.
    let mut app = new_running_sandbox_app();
    let player_key = VirtualKeyCode::KeyW;
    runtime_assert!(
        app.local_player_key_binding_in_scope(player_key),
        "the fixture must actually bind {player_key:?} as a player control"
    );
    install_runtime_key_config(
        &mut app,
        Ok(parse_runtime_key_config(b"[Keys]\nFullscreenPauseToggle=W\n").test_value()),
    );

    let before = app.runtime_halt_active();
    app.test_key(player_key, ElementState::Pressed);
    app.test_key(player_key, ElementState::Released);

    runtime_assert_eq!(
        app.runtime_halt_active() => before,
        "PRIO_PlrControl (7) outranks the pause key's PRIO_Base (1), so the \
         rebound chord must reach the player and not toggle pause"
    );

    // The same rebinding on a key the player does *not* drive still pauses,
    // so the guard is a collision rule rather than a blanket refusal.
    let mut free = new_running_sandbox_app();
    let free_key = VirtualKeyCode::F8;
    runtime_assert!(
        !free.local_player_key_binding_in_scope(free_key),
        "{free_key:?} must be free for this half to say anything"
    );
    install_runtime_key_config(
        &mut free,
        Ok(parse_runtime_key_config(b"[Keys]\nFullscreenPauseToggle=F8\n").test_value()),
    );
    let before = free.runtime_halt_active();
    free.test_key(free_key, ElementState::Pressed);
    free.test_key(free_key, ElementState::Released);
    runtime_assert!(
        free.runtime_halt_active() != before,
        "a rebound pause chord with no player control on it still toggles pause"
    );
}

/// The console Play/Halt buttons stay in step with the live halt state
/// (clonk-org/clonk-rs#577).
///
/// `C4Console::UpdateHaltCtrls(fHalt)` sets Play active to `!fHalt` and Halt
/// active to `fHalt` (`C4Console.cpp:709-730`), and every caller passes
/// `!!Game.HaltCount` (`:1711,1720`). The two buttons are therefore strictly
/// complementary and always reflect the *engine's* halt count rather than
/// whichever button was last clicked — which is the property that breaks if a
/// port latches button state locally.
///
/// The offline round trip was uncovered; only the network barrier case was.
#[test]
fn console_play_and_halt_track_the_live_offline_halt_count() {
    let mut app = new_running_sandbox_app();
    app.console_mode = true;
    runtime_assert!(
        !app.developer_console_view_model().halted,
        "a running offline round starts unhalted"
    );

    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::Halt])
        .test_value();
    runtime_assert!(
        app.developer_console_view_model().halted,
        "Halt raises the offline halt count and the toolbar follows"
    );
    runtime_assert!(
        app.runtime_halt_active(),
        "the button reflects real engine state, not a latched toolbar flag"
    );

    app.dispatch_developer_console_actions(vec![DeveloperConsoleAction::Play])
        .test_value();
    runtime_assert!(
        !app.developer_console_view_model().halted,
        "Play clears it again"
    );
    runtime_assert!(!app.runtime_halt_active());

    // Pausing by any other route moves the toolbar too, because the view model
    // is derived from the halt count rather than from the last button press.
    app.toggle_runtime_pause();
    runtime_assert!(
        app.developer_console_view_model().halted,
        "a pause that did not come from the toolbar still updates it"
    );
    app.toggle_runtime_pause();
    runtime_assert!(!app.developer_console_view_model().halted);
}


// `FullscreenPauseToggle` is registered with a gamepad-capable `C4CustomKey`
// like every other global action (src/C4Game.cpp:3429), and
// `LoadCustomConfig` replaces its code with whatever `[Keys]` holds
// (src/C4Game.cpp:3481-3482). A configured button therefore has to reach
// `C4Game::TogglePause`, and — the callback has no `Up` handler — only on the
// press.
#[test]
fn runtime_pause_gamepad_button_override_toggles_once_per_press() {
    let config =
        parse_runtime_key_config(b"[Keys]\nFullscreenPauseToggle=\\x0042000a\n").test_value();
    let mut app = new_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache.set(Ok(config)).test_value();
    runtime_assert!(!app.runtime_halt_active());

    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0),
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert!(app.runtime_halt_active(), "the rebound button pauses");

    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0),
        ElementState::Released,
    )
    .test_value();
    runtime_assert!(
        app.runtime_halt_active(),
        "the callback has no Up handler, so the release must not toggle back"
    );

    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0),
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert!(!app.runtime_halt_active(), "a second press unpauses");
}

// The network globals are registered with gamepad-capable `C4CustomKey`s like
// every other action (src/C4Game.cpp:3379,3442-3448), and `LoadCustomConfig`
// replaces any of their codes with whatever `[Keys]` holds
// (src/C4Game.cpp:3481-3482). Five of the six ship default-unbound, which is
// exactly why a configured code is the only way they are ever reachable.
#[test]
fn network_global_gamepad_overrides_reach_their_callbacks() {
    let bound = |name: &str| {
        let source = format!("[Keys]\n{name}=\\x0042000a\n");
        let mut app = new_running_sandbox_app();
        app.runtime_key_config_cache = OnceLock::new();
        app.runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(source.as_bytes()).test_value()))
            .test_value();
        app
    };
    let press = |app: &mut GameApp| {
        app.handle_gamepad_button(
            GamepadSlot::new(0),
            LegacyGamepadButton::new(0),
            ElementState::Pressed,
        )
        .test_value();
    };

    // `C4GraphicsSystem::ToggleShowNetStatus` has no DebugMode guard and
    // flashes nothing (src/C4GraphicsSystem.cpp:811-815).
    let mut stats = bound("NetStatsToggle");
    runtime_assert!(!stats.graphics.debug_draw_flags().show_net_status);
    press(&mut stats);
    runtime_assert!(
        stats.graphics.debug_draw_flags().show_net_status,
        "the rebound button reaches ToggleShowNetStatus"
    );

    // `C4Network2::ToggleClientListDlg` (src/C4Game.cpp:3379) — the one of
    // the six with a shipped default, and the only one whose dialog needs a
    // live network role to open at all.
    let mut clients = bound("NetClientListDlgToggle");
    let (_events, _commands) = install_running_network_stub(&mut clients, 0, 40, 4);
    clients
        .control_clients
        .replace_snapshot([message_client(0, b"Host")]);
    runtime_assert!(clients.runtime_client_list.is_none());
    press(&mut clients);
    runtime_assert!(
        clients.runtime_client_list.is_some(),
        "the rebound button opens the client list"
    );
    press(&mut clients);
    runtime_assert!(
        clients.runtime_client_list.is_none(),
        "and the same button closes it, as a toggle"
    );

    // `C4GameControl::KeyAdjustControlRate` only emits its relative CID_Set
    // from the control host (src/C4GameControl.cpp:548-552).
    for (name, delta) in [("CtrlRateUp", 1), ("CtrlRateDown", -1)] {
        let mut rate = new_classic_running_sandbox_app();
        let (_events, mut commands) = install_running_network_stub(&mut rate, 0, 40, 4);
        rate.runtime_key_config_cache = OnceLock::new();
        rate.runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                format!("[Keys]\n{name}=\\x0042000a\n").as_bytes(),
            )
            .test_value()))
            .test_value();
        press(&mut rate);
        let decided = commands.take_submitted_decided_controls();
        runtime_assert_eq!(
            decided
                .iter()
                .map(|(_, control, _)| {
                    clonk_network::LegacyControlSet::from_control_packet(control).test_value()
                })
                .collect::<Vec<_>>() =>
            [n2_fixture!(control_set: 0, delta, 0)],
            "{name} submits its registered relative adjustment",
        );
    }

    // `C4Network2::ToggleAllowJoin` consumes the code whether or not this
    // process is the host, and changes nothing when it is not
    // (src/C4Network2.cpp:799-804).
    let mut guest = bound("NetAllowJoinToggle");
    let before = guest.runtime_network_join_allowed;
    press(&mut guest);
    runtime_assert_eq!(
        guest.runtime_network_join_allowed => before,
        "a non-host toggles no admission gate",
    );
}

// `Joy%d` spellings resolve to a direction rather than a button
// (src/C4KeyboardInput.cpp's `sscanf` branch), so a direction override has to
// dispatch on the same terms as a button one.
#[test]
fn runtime_pause_gamepad_direction_override_toggles_the_hold() {
    let config = parse_runtime_key_config(b"[Keys]\nFullscreenPauseToggle=Joy1A\n").test_value();
    let mut app = new_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache.set(Ok(config)).test_value();

    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Left,
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert!(app.runtime_halt_active());

    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert!(
        app.runtime_halt_active(),
        "only the configured direction is the pause binding"
    );
}

// The registered default is `K_PAUSE` — a keyboard key. Without a custom
// entry no gamepad input is a pause binding, and the keyboard default keeps
// working.
#[test]
fn runtime_pause_has_no_default_gamepad_binding() {
    let mut app = new_running_sandbox_app();

    for button in 0..4 {
        app.handle_gamepad_button(
            GamepadSlot::new(0),
            LegacyGamepadButton::new(button),
            ElementState::Pressed,
        )
        .test_value();
    }
    for direction in [
        ControlButton::Left,
        ControlButton::Right,
        ControlButton::Up,
        ControlButton::Down,
    ] {
        app.handle_gamepad_direction(GamepadSlot::new(0), direction, ElementState::Pressed)
            .test_value();
    }
    runtime_assert!(!app.runtime_halt_active());

    app.handle_key(VirtualKeyCode::Pause, ElementState::Pressed)
        .test_value();
    runtime_assert!(app.runtime_halt_active(), "the K_PAUSE default still pauses");
}

// `C4Game::TogglePause` refuses while the evaluation dialog owns the halt, and
// the gamepad route reaches the same guard the keyboard one does.
#[test]
fn runtime_pause_gamepad_override_is_refused_under_the_evaluation_dialog() {
    let config =
        parse_runtime_key_config(b"[Keys]\nFullscreenPauseToggle=\\x0042000a\n").test_value();
    let mut app = new_game_over_keyboard_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache.set(Ok(config)).test_value();
    runtime_assert!(
        app.runtime_halt_active(),
        "the evaluation dialog owns a halt of its own"
    );

    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0),
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert!(
        app.runtime_halt_active(),
        "an accepted toggle would have released the dialog's hold"
    );
}


// Player controls register with `PRIO_PlrControl` (src/C4Game.cpp:3455-3461),
// above the `PRIO_Base` pause callback, so a set that claims the same gamepad
// code owns it and the game must not pause.
#[test]
fn runtime_pause_gamepad_override_yields_to_a_colliding_player_control() {
    let config = parse_runtime_key_config(
        b"[Keys]\nFullscreenPauseToggle=\\x0042000a\nKbd1Key1=\\x0042000a\n",
    )
    .test_value();
    let mut app = new_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache.set(Ok(config)).test_value();
    runtime_assert!(
        !app.runtime_control_candidates_for_gamepad_button(0, 0, ElementState::Pressed)
            .is_empty(),
        "the player set claims the same code"
    );

    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0),
        ElementState::Pressed,
    )
    .test_value();
    runtime_assert!(!app.runtime_halt_active());
}


// `C4Game::InitKeyboard` registers every global action through one
// `C4CustomKey` path and `LoadCustomConfig` rebinds any of them
// (src/C4Game.cpp:3365-3482), so a configured gamepad code has to reach each
// callback — not only the six the dispatcher used to know
// (clonk-org/clonk-rs#1219).
#[test]
fn runtime_gamepad_overrides_reach_every_fullscreen_global_action() {
    // One physical button per action, so a single config can drive them all.
    // `\x0042ssbb` is slot `ss`, button `bb`; button 10 is physical button 0.
    let entries = [
        ("MusicToggle", 0x0a),
        ("SoundToggle", 0x0b),
        ("Screenshot", 0x0c),
        ("ScreenshotEx", 0x0d),
        ("ToggleChat", 0x0e),
        ("ToggleShowHelp", 0x0f),
        ("MsgBoardScrollUp", 0x10),
        ("MsgBoardScrollDown", 0x11),
        ("StatsToggle", 0x12),
    ];
    let mut config = String::from("[Keys]\n");
    for (name, code) in entries {
        config.push_str(&format!("{name}=\\x004200{code:02x}\n"));
    }
    let press = |app: &mut GameApp, code: u8| {
        app.handle_gamepad_button(
            GamepadSlot::new(0),
            LegacyGamepadButton::new(code - 0x0a),
            ElementState::Pressed,
        )
        .test_value();
    };

    // The F1 help needs the classic UpperBoard resource, as it does for the
    // keyboard route.
    let mut app = new_classic_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(config.as_bytes()).test_value()))
        .test_value();

    // Screenshot and ScreenshotEx queue the two distinct kinds, in order.
    press(&mut app, 0x0c);
    press(&mut app, 0x0d);
    runtime_assert_eq!(
        app.pending_screenshots
            .iter()
            .map(|request| request.kind)
            .collect::<Vec<_>>() =>
        vec![ScreenshotKind::PresentedFrame, ScreenshotKind::FullLandscape]
    );

    // ToggleShowHelp is a toggle, and the release must not toggle it back:
    // the callback has no Up handler.
    runtime_assert!(!app.runtime_help_visible);
    press(&mut app, 0x0f);
    runtime_assert!(app.runtime_help_visible);
    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0x0f - 0x0a),
        ElementState::Released,
    )
    .test_value();
    runtime_assert!(app.runtime_help_visible, "the release has no callback");
    press(&mut app, 0x0f);
    runtime_assert!(!app.runtime_help_visible);

    // The port-only stats overlay is off by default and toggles like the rest.
    runtime_assert!(!app.display_flags.show_stats);
    press(&mut app, 0x12);
    runtime_assert!(app.display_flags.show_stats);

    // The message board scrolls in both directions without erroring.
    press(&mut app, 0x10);
    press(&mut app, 0x11);

    // Sound and music toggles run their config-writing callbacks.
    press(&mut app, 0x0a);
    press(&mut app, 0x0b);

    // ToggleChat opens the external IRC dialog.
    runtime_assert!(!app.external_irc_dialog_visible);
    press(&mut app, 0x0e);
    runtime_assert!(app.external_irc_dialog_visible);
}

// Every one of these actions is default-unbound on a gamepad: C++ registers
// them with keyboard codes, and only `Config.Controls.GamepadGuiControl`
// adds gamepad codes — to the fullscreen *menu* keys, not to these
// (src/C4Game.cpp:3395-3401).
#[test]
fn runtime_gamepad_fullscreen_globals_have_no_default_binding() {
    let mut app = new_running_sandbox_app();
    let before = (
        app.pending_screenshots.len(),
        app.runtime_help_visible,
        app.display_flags.show_stats,
        app.external_irc_dialog_visible,
    );

    for button in 0..8 {
        app.handle_gamepad_button(
            GamepadSlot::new(0),
            LegacyGamepadButton::new(button),
            ElementState::Pressed,
        )
        .test_value();
    }
    for direction in [
        ControlButton::Left,
        ControlButton::Right,
        ControlButton::Up,
        ControlButton::Down,
    ] {
        app.handle_gamepad_direction(GamepadSlot::new(0), direction, ElementState::Pressed)
            .test_value();
    }

    runtime_assert_eq!(
        (
            app.pending_screenshots.len(),
            app.runtime_help_visible,
            app.display_flags.show_stats,
            app.external_irc_dialog_visible,
        ) => before
    );
}

// `StatsToggle` registers after every C++ action and yields its chord to all
// of them, so a code it shares with `ChartToggle` belongs to the chart.
#[test]
fn runtime_gamepad_stats_toggle_yields_its_code_to_the_actions_it_shadows() {
    let mut app = new_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nStatsToggle=\\x0042000a\nChartToggle=\\x0042000a\n",
        )
        .test_value()))
        .test_value();

    app.handle_gamepad_button(
        GamepadSlot::new(0),
        LegacyGamepadButton::new(0),
        ElementState::Pressed,
    )
    .test_value();

    runtime_assert!(
        !app.display_flags.show_stats,
        "the earlier registration owns the shared code"
    );
}


// `NetObsNextPlayer` is KEYSCOPE_FreeView (src/C4Game.cpp:3443), so it needs
// an ownerless primary viewport exactly as the scroll keys do — even though it
// registers after `ChartToggle` rather than beside them, which is why a code
// bound to both a scroll and this one goes to the scroll.
#[test]
fn observer_next_player_gamepad_override_keeps_its_free_view_scope() {
    let bound = || {
        let mut app = new_running_sandbox_app();
        app.runtime_key_config_cache = OnceLock::new();
        app.runtime_key_config_cache
            .set(Ok(parse_runtime_key_config(
                b"[Keys]\nNetObsNextPlayer=\\x0042000a\n",
            )
            .test_value()))
            .test_value();
        app
    };

    let owned = bound();
    runtime_assert!(
        !owned.primary_physical_viewport_is_no_owner(),
        "the sandbox player owns the primary viewport"
    );
    runtime_assert_eq!(
        owned.runtime_custom_gamepad_button_action(0, 0) => None,
        "an owned viewport is outside KEYSCOPE_FreeView",
    );

    let mut observing = bound();
    observing.clear_physical_viewport_states();
    let observer = observing.ownerless_physical_viewport_state();
    observing.physical_viewports.push(observer);
    observing.physical_viewports_authoritative = true;
    runtime_assert!(observing.primary_physical_viewport_is_no_owner());
    runtime_assert_eq!(
        observing.runtime_custom_gamepad_button_action(0, 0) =>
        Some(RuntimeCustomGamepadAction::ObserverNextPlayer)
    );
}

// Five of the six network globals register with `KEY_Default`
// (src/C4Game.cpp:3443-3447), which binds nothing at all. An unconfigured
// gamepad button must therefore reach none of them.
#[test]
fn unbound_network_globals_claim_no_gamepad_code() {
    let mut app = new_running_sandbox_app();
    for button in 0..4 {
        runtime_assert_eq!(
            app.runtime_custom_gamepad_button_action(0, button) => None,
            "button {button} is bound to nothing",
        );
    }
    let before = runtime_global_ui_snapshot(&app);
    for button in 0..4 {
        app.handle_gamepad_button(
            GamepadSlot::new(0),
            LegacyGamepadButton::new(button),
            ElementState::Pressed,
        )
        .test_value();
    }
    runtime_assert_eq!(runtime_global_ui_snapshot(&app) => before);
    runtime_assert!(!app.graphics.debug_draw_flags().show_net_status);
}

// `FilmNextPlayer` is KEYSCOPE_FilmView and the free-view scrolls are
// KEYSCOPE_FreeView (src/C4Game.cpp:3415, 3423-3426), so a bound gamepad code
// is inert while an owned viewport holds the screen — the same scope gate the
// keyboard route applies.
#[test]
fn runtime_gamepad_view_actions_stay_inside_their_keyboard_scope() {
    let mut app = new_running_sandbox_app();
    app.runtime_key_config_cache = OnceLock::new();
    app.runtime_key_config_cache
        .set(Ok(parse_runtime_key_config(
            b"[Keys]\nFreeViewScrollLeft=\\x0042000a\nFilmNextPlayer=\\x0042000b\n",
        )
        .test_value()))
        .test_value();
    runtime_assert!(
        !app.engine.film_replay(),
        "a sandbox round is not a film replay"
    );
    runtime_assert!(
        !app.primary_physical_viewport_is_no_owner(),
        "the sandbox player owns the primary viewport"
    );
    let before = runtime_global_ui_snapshot(&app);

    for button in [0, 1] {
        app.handle_gamepad_button(
            GamepadSlot::new(0),
            LegacyGamepadButton::new(button),
            ElementState::Pressed,
        )
        .test_value();
    }

    runtime_assert_eq!(runtime_global_ui_snapshot(&app) => before);
}
