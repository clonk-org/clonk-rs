// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! audio_fixture {
    (music_pair: $dir:ident, $global:ident, $audio:ident $(,)?) => {
        let $dir = tempdir();
        let $global = $dir.path().join("Music.c4g");
        fs::create_dir_all(&$global).test_value();
        fs::write($global.join("A.ogg"), b"A").test_value();
        fs::write($global.join("B.ogg"), b"B").test_value();
        let mut $audio = AudioContext::try_new(AudioOptions::default()).test_value();
        $audio.music_resolver =
            MusicResolver::with_global_group(Group::open(&$global).test_value()).test_value();
    };
    (sound_resolver: $global:expr, $scenario:expr, $scenario_root:expr, $base_sample_loads:expr $(,)?) => {
        SoundResolver {
            global: $global,
            scenario: $scenario,
            scenario_root: $scenario_root,
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: $base_sample_loads,
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        }
    };
    (audio_max_channels: $max_channels:expr $(,)?) => {
        AudioOptions {
            max_channels: $max_channels,
            ..AudioOptions::default()
        }
    };
    (set_volume: $name:expr, $volume:expr $(,)?) => {
        AudioCommand::SetSoundVolume {
            name: $name,
            target: None,
            volume: $volume,
        }
    };
    (hud_player: $crew:expr, $focus:expr $(,)?) => {
        HudPlayerSnapshot {
            owner: 1,
            crew: $crew,
            focus: $focus,
            eliminated: false,
            wealth: 0,
            score: 0,
        }
    };
    (audio_sound_enabled_menu_sound_enabled_max_channels: $sound_enabled:expr, $menu_sound_enabled:expr, $max_channels:expr $(,)?) => {
        AudioOptions {
            sound_enabled: $sound_enabled,
            menu_sound_enabled: $menu_sound_enabled,
            max_channels: $max_channels,
            ..AudioOptions::default()
        }
    };
    (player_state_id_view_cursor_viewports: $id:expr, $view_cursor:expr, $viewports:expr $(,)?) => {
        PlayerState {
            id: $id,
            view_cursor: $view_cursor,
            viewports: $viewports,
            ..Default::default()
        }
    };
    (checkbox: $id:expr, $checked:expr $(,)?) => {
        SoundSheetAction::CheckboxChanged {
            id: $id,
            checked: $checked,
        }
    };
}

#[test]
fn console_quit_is_global_and_headless_loop_exits_cleanly() {
    let mut app = new_state_only_menu_app(320, 200);
    for mode in [AppMode::Menu, AppMode::Loading, AppMode::Running] {
        app.mode = mode;
        app.process_console_command("/quit").test_value();
        main_assert!(app.take_exit_request(), "quit must exit from {mode:?}");
    }

    let app = new_menu_app(320, 200);
    let (sender, receiver) = mpsc::channel();
    sender
        .send(ConsoleInputEvent::Command("/quit".to_string()))
        .test_value();
    run_console_event_loop(app, receiver).test_value();

    let app = new_menu_app(320, 200);
    let (sender, receiver) = mpsc::channel();
    sender
        .send(ConsoleInputEvent::Error(io::Error::other(
            "fixture stdin failure",
        )))
        .test_value();
    let error = run_console_event_loop(app, receiver)
        .expect_err("stdin read errors must fail the console process");
    main_assert!(error.to_string().contains("fixture stdin failure"));

    let mut boot = new_state_only_menu_app(320, 200);
    let (sender, receiver) = mpsc::channel();
    sender.send(BootLoadingEvent::Finished(None)).test_value();
    boot.boot_loading = Some(BootLoadingState::new(receiver));
    boot.mode = AppMode::Loading;
    boot.console_mode = true;
    boot.loader_screen = None;
    boot.poll_boot_loading();
    main_assert_eq!(boot.mode => AppMode::Menu);
    main_assert!(boot.boot_loading.is_none());
}

/// A dedicated server has no loader screen to fail on:
/// `C4Application::PreInit` calls `InitLoaderScreen` only for a
/// startup-dialog run (C4Application.cpp:239), and a `USE_CONSOLE` build
/// has no `C4FontLoader` at all (C4Game.h:132-135) yet still reaches
/// `C4AS_Startup` (C4Application.cpp:422-429). Headless must therefore
/// leave `Loading` on the same terms `/console` does — and must do so
/// without claiming console authority, which is a separate flag.
#[test]
fn headless_boot_leaves_loading_without_a_loader_screen_or_console_authority() {
    let finished_boot_app = || {
        let mut app = new_state_only_menu_app(320, 200);
        let (sender, receiver) = mpsc::channel();
        sender.send(BootLoadingEvent::Finished(None)).test_value();
        app.boot_loading = Some(BootLoadingState::new(receiver));
        app.mode = AppMode::Loading;
        app.loader_screen = None;
        app
    };

    // The gate is real: an ordinary windowed boot stays in Loading so the
    // next redraw can report the typed loader boundary.
    let mut windowed = finished_boot_app();
    windowed.poll_boot_loading();
    main_assert_eq!(windowed.mode => AppMode::Loading);

    let mut server = finished_boot_app();
    server.headless = true;
    server.poll_boot_loading();
    main_assert_eq!(server.mode => AppMode::Menu);
    main_assert!(server.boot_loading.is_none());
    main_assert!(!server.console_mode, "headless must not grant developer-console authority");
}

/// `CStdApp::Execute` abandons a deadline more than two seconds overdue -
/// `LastExecute = tv` - and then fires `pWindow->Sec1Timer()` at most once,
/// on a plain `seconds != LastExecute.tv_sec` comparison that cannot queue a
/// backlog (StdAppUnix.cpp:261-291). A long stall must therefore advance the
/// clock by one second, not by the whole gap.
#[test]
fn sec1_timer_coalesces_stalls_over_two_seconds_like_cpp() {
    let mut app = new_menu_app(320, 200);
    let mut seconds = Duration::ZERO;
    app.engine.test_tick();

    // Sub-second input accumulates without a pulse, and the phase is kept.
    for _ in 0..3 {
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(300))
            .test_value();
    }
    main_assert_eq!(app.game_time_seconds() => 0);
    main_assert_eq!(seconds => Duration::from_millis(900));

    // The exact one-second boundary pulses once and keeps the remainder.
    advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(250))
        .test_value();
    main_assert_eq!(app.game_time_seconds() => 1);
    main_assert_eq!(seconds => Duration::from_millis(150));

    // A five-second stall is one pulse, not five, and the sub-second phase
    // survives so the timer cannot drift.
    app.engine.test_tick();
    let before = app.game_time_seconds();
    advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(5_400))
        .test_value();
    main_assert_eq!(app.game_time_seconds() - before => 1, "a stall beyond the C++ reset threshold dispatches exactly one Sec1 callback");
    main_assert_eq!(seconds => Duration::from_millis(550), "the accumulator reanchors to the sub-second phase instead of holding the backlog");

    // The next ordinary second still pulses exactly once from there.
    app.engine.test_tick();
    let before = app.game_time_seconds();
    advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(450))
        .test_value();
    main_assert_eq!(app.game_time_seconds() - before => 1);
    main_assert_eq!(seconds => Duration::ZERO);
}

#[test]
fn sec1_timer_backlog_invokes_callback_once_and_preserves_phase() {
    // CStdApp::Execute compares seconds with LastExecute.tv_sec and fires
    // Sec1Timer at most once (LegacyClonk src/StdAppUnix.cpp:288-291), while
    // Win32 never queues WM_TIMER twice (LegacyClonk src/StdAppWin32.cpp:132).
    let mut app = new_menu_app(320, 200);
    let mut seconds = Duration::ZERO;
    app.engine.test_tick();

    advance_game_clock_from_elapsed(
        &mut app,
        &mut seconds,
        Duration::from_secs(60) + Duration::from_millis(250),
    )
    .test_value();

    main_assert_eq!(app.sec1_timer_call_count => 1);
    main_assert_eq!(seconds => Duration::from_millis(250));
}

#[test]
fn event_loop_second_accumulator_pulses_the_engine_clock() {
    // StdApp's one-second callback is independent from frame scheduling
    // (StdAppUnix.cpp:286-291); C4Game::Sec1Timer consumes TimeGo
    // (C4Game.cpp:1755-1759). Partial elapsed durations accumulate, but
    // headless Engine::tick calls alone never advance Game.Time.
    let mut app = new_menu_app(320, 200);
    let mut seconds = Duration::ZERO;
    app.engine.test_tick();
    advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(400))
        .test_value();
    main_assert_eq!(app.game_time_seconds() => 0);
    advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(600))
        .test_value();
    main_assert_eq!(app.game_time_seconds() => 1);

    // A gap longer than one second collapses to a single pulse. C++ fires
    // `pWindow->Sec1Timer()` at most once per Execute, on a plain
    // `seconds != LastExecute.tv_sec` comparison that cannot queue a
    // backlog (StdAppUnix.cpp:288-291), and Win32 never queues WM_TIMER
    // more than once (StdAppWin32.cpp:132). Replaying one pass per elapsed
    // second instead froze the app after any suspend or long load.
    advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_secs(2)).test_value();
    main_assert_eq!(app.game_time_seconds() => 1);

    // The sub-second phase survives the coalescing, so the timer cannot
    // drift: 60.25s of backlog leaves exactly the 0.25s remainder pending.
    seconds = Duration::ZERO;
    advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(60_250))
        .test_value();
    main_assert_eq!(seconds => Duration::from_millis(250));
}

#[test]
fn event_loop_uses_cpp_startup_and_ingame_tick_delays() {
    // C4Application starts and returns to startup at 16 ms, while
    // C4Game::Init switches the running simulation to 28 ms. Rust's
    // intentional missing-key refresh cap divides that into 14 ms
    // presentation opportunities (C4Application.cpp:44,234,510-531;
    // C4Game.cpp:63,443).
    let startup = FrameSchedule {
        simulation_interval: STARTUP_FRAME_INTERVAL,
        refresh_interval: STARTUP_FRAME_INTERVAL,
        running_revision: None,
    };
    main_assert_eq!(frame_schedule_for_mode(AppMode::Menu, 28, 1, 16) => startup);
    main_assert_eq!(frame_schedule_for_mode(AppMode::Loading, 28, 1, 16) => startup);
    main_assert_eq!(
        frame_schedule_for_mode(AppMode::Running, 28, 1, 16) =>
        FrameSchedule {
            simulation_interval: INGAME_FRAME_INTERVAL,
            refresh_interval: Duration::from_millis(14),
            running_revision: Some(1),
        }
    );

    let mut schedule = startup;
    let mut accumulator = Duration::from_millis(15);
    accumulate_frame_time_for_mode(
        AppMode::Running,
        28,
        1,
        16,
        &mut schedule,
        &mut accumulator,
        Duration::from_millis(10),
    );
    main_assert_eq!(schedule.simulation_interval => INGAME_FRAME_INTERVAL);
    main_assert_eq!(accumulator => Duration::ZERO, "elapsed time measured under the startup cadence must not leak into the game timer");

    accumulate_frame_time_for_mode(
        AppMode::Running,
        28,
        1,
        16,
        &mut schedule,
        &mut accumulator,
        Duration::from_millis(27),
    );
    main_assert_eq!(accumulator => Duration::from_millis(27));

    main_assert!(synchronize_frame_schedule(AppMode::Running, 28, 2, 16, &mut schedule, &mut accumulator,));
    main_assert_eq!(accumulator => Duration::ZERO);

    main_assert!(synchronize_frame_schedule(AppMode::Running, 1_000, 3, 16, &mut schedule, &mut accumulator,));
    main_assert_eq!(schedule.simulation_interval => Duration::from_millis(1_000));
    main_assert_eq!(schedule.refresh_interval => Duration::from_millis(15));
    accumulate_frame_time_for_mode(
        AppMode::Running,
        1_000,
        3,
        16,
        &mut schedule,
        &mut accumulator,
        Duration::from_millis(1_000),
    );
    main_assert_eq!(accumulator => Duration::from_millis(1_000));

    accumulate_frame_time_for_mode(
        AppMode::Menu,
        1_000,
        3,
        16,
        &mut schedule,
        &mut accumulator,
        Duration::from_millis(1),
    );
    main_assert_eq!(schedule => startup);
    main_assert_eq!(accumulator => Duration::ZERO);
}

#[test]
fn frame_timer_reanchors_only_after_two_seconds_of_timer_debt_like_cpp() {
    // CStdApp::Execute reanchors LastExecute only when its strict
    // more-than-two-second test is met (src/StdAppUnix.cpp:261-284). The
    // Rust accumulator is the elapsed time since that scheduled tick, so
    // it must participate in the same boundary. The callback still runs
    // once, leaving one normal game interval to consume.
    let mut schedule = frame_schedule_for_mode(AppMode::Running, 28, 1, 16);
    let mut accumulator = Duration::from_millis(12);

    accumulate_frame_time_for_mode(
        AppMode::Running,
        28,
        1,
        16,
        &mut schedule,
        &mut accumulator,
        Duration::from_millis(20),
    );
    main_assert_eq!(accumulator => Duration::from_millis(32), "ordinary frame gaps retain their timer debt");

    accumulator = Duration::from_millis(12);
    let reanchor_threshold = Duration::from_secs(2);
    accumulate_frame_time_for_mode(
        AppMode::Running,
        28,
        1,
        16,
        &mut schedule,
        &mut accumulator,
        reanchor_threshold - Duration::from_millis(12),
    );
    main_assert_eq!(accumulator => MAX_ACCUMULATED_TIME, "exactly two seconds of timer debt keeps the bounded catch-up debt");

    accumulator = Duration::from_millis(12);
    accumulate_frame_time_for_mode(
        AppMode::Running,
        28,
        1,
        16,
        &mut schedule,
        &mut accumulator,
        reanchor_threshold - Duration::from_millis(11),
    );
    main_assert_eq!(accumulator => schedule.simulation_interval, "timer debt beyond two seconds resumes with one immediate normal-speed tick");

    let mut app = new_running_sandbox_app();
    let frame_before = app.engine.frame();
    let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator).test_value();
    main_assert_eq!(outcome.executed_frames => 1);
    main_assert_eq!(app.engine.frame() => frame_before + 1);
    main_assert_eq!(accumulator => Duration::ZERO);
}

#[test]
fn focus_loss_clears_controls_repeat_tracking_and_pointer_state() {
    let mut app = GameApp::new(
        320,
        200,
        AudioOptions::default(),
        None,
        test_runtime_config_with("Focus tester".to_string(), false),
    )
    .test_value();
    install_classic_test_assets(&mut app);

    let mut definition = test_definition("WLKR", "Walker", walker_script());
    definition.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("Walk"),
        )]),
    );
    definition.set_movement_profile(MovementProfile::default());
    definition.set_crew_member(true);
    app.engine.register_test_definition(definition);
    app.engine
        .set_player_starts(vec![clonk_engine::scenario::PlayerStart {
            ready_crew: vec![("WLKR".to_string(), 1)],
            ..Default::default()
        }]);
    app.join_local_player().test_value();
    app.mode = AppMode::Running;
    app.ingame_pointer = Some(ViewportPointer {
        owner: app.local_owner,
        world: FloatVector2::new(10.0, 20.0),
        screen: GuiPoint::new(30.0, 40.0),
    });

    AppVirtualKeyboard::new(&mut app).press(VirtualKeyCode::KeyX);
    main_assert!(!app.pressed_engine_keys.is_empty());
    main_assert_ne!(app.engine.snapshot().players.into_iter().find(|player| player.id == app.local_owner).expect("local player").control.pressed_coms => 0);

    app.handle_focus_lost().test_value();

    main_assert!(app.pressed_engine_keys.is_empty());
    main_assert_ne!(
        app.engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == app.local_owner)
            .expect("local player")
            .control
            .pressed_coms =>
        0,
        "no native backend clears player controls on focus loss \
             (C4FullScreen.cpp:139-145,310-315,432-447)"
    );
    main_assert_eq!(app.ingame_pointer => None, "focus loss retains the old pointer_left lifecycle cleanup");
}

#[test]
fn invalid_sound_override_keeps_previous_decoded_sample() {
    let dir = tempdir();
    let global = dir.path().join("Sound.c4g");
    let scenario = dir.path().join("Override.c4s");
    fs::create_dir_all(&global).test_value();
    fs::create_dir_all(&scenario).test_value();
    fs::write(global.join("Voice.wav"), silent_pcm_wav(1_000)).test_value();
    fs::write(scenario.join("VOICE.WAV"), b"not an audio stream").test_value();

    let mut audio = empty_test_audio_context();
    audio.resolver.global = collect_sound_libraries_for_path(&global);
    audio.resolver.scenario = collect_sound_libraries_for_path(&scenario);
    audio.refresh_sound_catalog();

    main_assert_eq!(audio.available_sound_samples() => ["voice.wav"]);
    let resolved = audio
        .ensure_sound_with_key("Voice")
        .expect("validated catalog lookup")
        .test_value();
    main_assert_eq!(resolved.handle.duration_ms() => Some(1_000));
    main_assert!(resolved.sample_key.contains("sound.c4g"), "the undecodable scenario entry must not replace the global handle");
    main_assert_eq!(resolved.sample_order => 0);
    main_assert!(audio.missing_sounds.iter().any(|key| key.starts_with("asset::") && key.contains("voice.wav")), "the rejected decode is retained in diagnostics");
}

#[test]
fn unreadable_sound_override_keeps_previous_decoded_sample() {
    let dir = tempdir();
    let global = dir.path().join("Sound.c4g");
    let scenario = dir.path().join("Unreadable.c4s");
    fs::create_dir_all(&global).test_value();
    fs::create_dir_all(&scenario).test_value();
    fs::write(global.join("Alert.wav"), silent_pcm_wav(750)).test_value();
    let override_path = scenario.join("ALERT.WAV");
    fs::write(&override_path, silent_pcm_wav(1_500)).test_value();

    let global_libraries = collect_sound_libraries_for_path(&global);
    let scenario_libraries = collect_sound_libraries_for_path(&scenario);
    fs::remove_file(&override_path).test_value();

    let mut audio = empty_test_audio_context();
    audio.resolver.global = global_libraries;
    audio.resolver.scenario = scenario_libraries;
    audio.refresh_sound_catalog();

    let resolved = audio
        .ensure_sound_with_key("Alert")
        .expect("validated catalog lookup")
        .test_value();
    main_assert_eq!(resolved.handle.duration_ms() => Some(750));
    main_assert!(resolved.sample_key.contains("sound.c4g"));
    main_assert!(audio.missing_sounds.iter().any(|key| key.starts_with("asset::") && key.contains("alert.wav")), "the rejected read is retained in diagnostics");
}

#[test]
fn unresolvable_sound_name_logs_at_debug_but_a_broken_asset_still_warns() {
    // C4SoundSystem::NewInstance returns nullptr for a name no sample
    // matches and logs nothing on the way out (src/C4SoundSystem.cpp:301-337).
    // Its only log sites are the absent audio system (:85) and a sample
    // that fails to decode (:132, spdlog::level::err). Shipped content
    // carries stale ActMap names — ClonkMars' `Sound=metaldoor` against the
    // `Door_Metal.wav` it actually ships — so warning on an unresolved name
    // reports a third-party authoring slip as an engine fault. The decode
    // failure keeps its warning, because C++ reports that one too.
    let dir = tempdir();
    let global = dir.path().join("Sound.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Door_Metal.wav"), silent_pcm_wav(500)).test_value();
    fs::write(global.join("Broken.wav"), b"not an audio stream").test_value();

    let capture = clonk_logging::ConsoleLogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();

    let mut audio = empty_test_audio_context();
    tracing::subscriber::with_default(subscriber, || {
        audio.resolver.global = collect_sound_libraries_for_path(&global);
        audio.refresh_sound_catalog();
        for _ in 0..2 {
            main_assert!(
                audio
                    .ensure_sound_with_key("metaldoor")
                    .expect("validated catalog lookup")
                    .is_none(),
                "the stale ActMap name resolves to nothing, exactly as in C++"
            );
        }
        main_assert!(audio.ensure_sound_with_key("Door_Metal").expect("validated catalog lookup").is_some(), "the sample the door script really plays still resolves");
    });

    let logged = capture.take();
    let name_lines = logged
        .lines()
        .filter(|line| line.contains("missing sound asset"))
        .collect::<Vec<_>>();
    main_assert_eq!(name_lines.len() => 1, "the unresolved name is reported once, deduplicated: {logged}");
    main_assert!(name_lines[0].contains("DEBUG"), "an unresolvable name is a content defect, not an engine fault: {}", name_lines[0]);
    main_assert!(
        logged
            .lines()
            .filter(|line| line.contains("sound candidate"))
            .all(|line| line.contains("WARN")),
        "a sample that fails to decode keeps C++'s error report: {logged}"
    );
    main_assert!(audio.missing_sounds.contains("request::metaldoor"), "the unresolved name stays in diagnostics whatever level it logged at");
}

#[test]
fn undecodable_speech_sample_does_not_suppress_text_fallback() {
    let dir = tempdir();
    let scenario = dir.path().join("Speech.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("BrokenSpeech.wav"), b"corrupt speech").test_value();

    let mut audio = empty_test_audio_context();
    audio.resolver.scenario = collect_sound_libraries_for_path(&scenario);
    audio.refresh_sound_catalog();
    let advertised = audio.available_sound_samples();
    main_assert!(advertised.is_empty(), "undecodable speech is not advertised");

    for (script, expected, expected_kind) in [
        (
            r#"Message("Message text$BrokenSpeech")"#,
            "Message text",
            MessageKind::Global,
        ),
        (
            r#"PlayerMessage(0,"Player text$BrokenSpeech")"#,
            "Player text",
            MessageKind::GlobalPlayer,
        ),
        (
            r#"PlrMessage("Plr text$BrokenSpeech",0)"#,
            "Plr text",
            MessageKind::GlobalPlayer,
        ),
    ] {
        let mut engine = Engine::with_seed(1);
        engine
            .register_player(
                PlayerConfig::new(0, "Player")
                    .with_viewports([clonk_engine::PlayerViewport::new(Vector2::ZERO)]),
            )
            .test_value();
        engine.configure_sound_samples(advertised.iter());
        let control = clonk_engine::ScriptControlData {
            script: LegacyCString::from_bytes(script.as_bytes().to_vec()).test_value(),
            by_client: 0,
            ..clonk_engine::ScriptControlData::default()
        };
        engine
            .execute_script_control(&control, ScriptControlPolicy::live(false))
            .expect("message script executes")
            .test_value();

        main_assert!(engine.pending_audio.is_empty());
        let messages = engine.snapshot().hud.messages;
        main_assert_eq!(messages.len() => 1);
        main_assert_eq!(messages[0].kind => expected_kind);
        main_assert_eq!(messages[0].lines => [expected]);
    }
}

#[test]
fn matches_sound_pattern_uses_cpp_prepared_question_wildcards() {
    main_assert!(matches_sound_pattern("sound?.wav", "sound1.wav"));
    main_assert!(!matches_sound_pattern("sound?.wav", "sound12.wav"));
    main_assert!(matches_sound_pattern("mix???.ogg", "mix001.ogg"));
    main_assert!(!matches_sound_pattern("mix???.ogg", "mix01.ogg"));
}

#[test]
fn sound_search_terms_converts_star_to_cpp_one_character_wildcard() {
    let terms = SoundSearchTerms::new("Sound*");
    main_assert_eq!(terms.wildcard_pattern.as_deref() => Some("sound?.wav"));
    main_assert!(terms.search_names.is_empty());

    let explicit_extension = SoundSearchTerms::new("Sound.*");
    main_assert_eq!(explicit_extension.wildcard_pattern.as_deref() => Some("sound.?"));
    main_assert!(explicit_extension.search_names.is_empty());
}

#[test]
fn sound_search_terms_preserves_cpp_literal_whitespace_and_dotfile_extensions() {
    main_assert_eq!(SoundSearchTerms::new(" Fire ").search_names => [" fire .wav"]);
    main_assert_eq!(SoundSearchTerms::new(".wav").search_names => [".wav"]);
    main_assert_eq!(SoundSearchTerms::new("Fire.").search_names => ["fire..wav"]);
    let nested = format!("dir.name{}Fire", std::path::MAIN_SEPARATOR);
    let nested_wav = format!("dir.name{}fire.wav", std::path::MAIN_SEPARATOR);
    main_assert_eq!(SoundSearchTerms::new(&nested).search_names => [nested_wav]);
}

#[test]
fn extensionless_sound_names_resolve_only_wav_across_libraries() {
    main_assert_eq!(SoundSearchTerms::new("Boom").search_names => ["boom.wav"]);
    main_assert_eq!(SoundSearchTerms::new("Boom.ogg").search_names => ["boom.ogg"]);
    main_assert_eq!(SoundSearchTerms::new("Boom.mp3").search_names => ["boom.mp3"]);
    let dir = tempdir();
    let scenario = dir.path().join("Codec.c4s");
    let global = dir.path().join("Sound.c4g");
    fs::create_dir_all(&scenario).test_value();
    fs::create_dir_all(&global).test_value();
    fs::write(scenario.join("OnlyOgg.ogg"), b"only ogg").test_value();
    fs::write(scenario.join("OnlyMp3.mp3"), b"only mp3").test_value();
    fs::write(scenario.join("Prefer.ogg"), b"scenario ogg").test_value();
    fs::write(scenario.join("Prefer.mp3"), b"scenario mp3").test_value();
    fs::write(global.join("Prefer.wav"), b"global wav").test_value();

    let resolver = audio_fixture!(
        sound_resolver:
            collect_sound_libraries_for_path(&global),
            collect_sound_libraries_for_path(&scenario),
            Some(scenario),
            Vec::new(),
    );

    main_assert!(resolver.resolve_entry("OnlyOgg").is_none());
    main_assert!(resolver.resolve_entry("OnlyMp3").is_none());
    main_assert_eq!(resolver.resolve_entry("OnlyOgg.ogg").expect("explicit ogg resolves").load_audio().expect("read ogg bytes") => b"only ogg");
    main_assert_eq!(resolver.resolve_entry("OnlyMp3.mp3").expect("explicit mp3 resolves").load_audio().expect("read mp3 bytes") => b"only mp3");
    main_assert_eq!(
        resolver
            .resolve_entry("Prefer")
            .expect("extensionless request finds lower-priority wav")
            .load_audio()
            .expect("read wav bytes") =>
        b"global wav"
    );
    main_assert_eq!(resolver.resolve_entry("Prefer.ogg").expect("explicit ogg keeps scenario precedence").load_audio().expect("read ogg bytes") => b"scenario ogg");
    main_assert_eq!(resolver.resolve_entry("Prefer.mp3").expect("explicit mp3 keeps scenario precedence").load_audio().expect("read mp3 bytes") => b"scenario mp3");
}

#[test]
fn sound_resolver_star_matches_exactly_one_extra_character() {
    let dir = tempdir();
    let scenario = dir.path().join("Wildcard.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Foo.wav"), b"no extra character").test_value();
    fs::write(scenario.join("Foo12.wav"), b"two extra characters").test_value();

    let make_resolver = || audio_fixture!(sound_resolver: Vec::new(), Vec::new(), None, Vec::new());
    let mut resolver = make_resolver();
    main_assert!(resolver.configure_scenario(Some(&scenario)));
    main_assert!(resolver.resolve_entry("Foo*").is_none());

    fs::write(scenario.join("Foo1.wav"), b"one extra character").test_value();
    let mut resolver = make_resolver();
    main_assert!(resolver.configure_scenario(Some(&scenario)));
    main_assert_eq!(resolver.resolve_entry("Foo*").expect("one-character wildcard resolves").load_audio().expect("resolved sample loads") => b"one extra character");
}

fn wildcard_sound_resolver_fixture() -> (tempfile::TempDir, SoundResolver) {
    let dir = tempdir();
    let scenario = dir.path().join("Scenario.c4s");
    let global = dir.path().join("Sound.c4g");
    let definition = dir.path().join("Blast.c4d");
    for path in [&scenario, &global, &definition] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(scenario.join("Blast1.wav"), b"scenario blast").test_value();
    fs::write(global.join("Blast2.wav"), b"global blast").test_value();
    fs::write(definition.join("Blast3.wav"), b"definition blast").test_value();

    let mut resolver = audio_fixture!(
        sound_resolver:
            collect_sound_libraries_for_path(&global),
            collect_sound_libraries_for_path(&scenario),
            Some(scenario),
            Vec::new(),
    );
    let definition_group = Group::open(&definition).test_value();
    resolver.register_definition_group("BLST", &definition_group);
    (dir, resolver)
}

#[test]
fn wildcard_sound_resolution_varies_without_advancing_synced_rng() {
    let (_dir, resolver) = wildcard_sound_resolver_fixture();
    let mut with_sound = clonk_engine::LcgRng::seed_from_u64(0xc4);
    let mut without_sound = with_sound.clone();
    let resolved = (0..100)
        .map(|_| resolver.resolve_entry("Blast*").test_value().cache_key())
        .collect::<HashSet<_>>();

    main_assert!(resolved.len() > 1, "one hundred SafeRandom selections must not collapse to one sample");
    main_assert_eq!(with_sound => without_sound, "sound resolution must not touch the synchronized LCG state");
    let with_sound_draws = (0..16)
        .map(|_| with_sound.random(10_000))
        .collect::<Vec<_>>();
    let without_sound_draws = (0..16)
        .map(|_| without_sound.random(10_000))
        .collect::<Vec<_>>();
    main_assert_eq!(with_sound_draws => without_sound_draws);
}

#[test]
fn wildcard_sound_resolution_spans_scenario_global_and_definitions() {
    let (_dir, resolver) = wildcard_sound_resolver_fixture();
    let resolved = (0..3)
        .map(|selected| {
            resolver
                .resolve_entry_with_random("Blast*", |range| {
                    main_assert_eq!(range => 3, "every resolved library contributes a match");
                    selected
                })
                .expect("selected wildcard sound")
                .load_audio()
                .test_value()
        })
        .collect::<HashSet<_>>();
    let expected = [
        b"scenario blast".to_vec(),
        b"global blast".to_vec(),
        b"definition blast".to_vec(),
    ]
    .into_iter()
    .collect::<HashSet<_>>();
    main_assert_eq!(resolved => expected);

    let exact = resolver
        .resolve_entry_with_random("Blast2", |_| {
            panic!("exact sound resolution must not consume SafeRandom")
        })
        .test_value();
    main_assert_eq!(exact.load_audio().expect("exact sound bytes") => b"global blast");
}

#[test]
fn definition_sound_overrides_global_while_scenario_remains_first() {
    let dir = tempdir();
    let global = dir.path().join("Sound.c4g");
    let first_definition = dir.path().join("Objects.c4d");
    let second_definition = dir.path().join("MoreObjects.c4d");
    let scenario = dir.path().join("Override.c4s");
    for path in [&global, &first_definition, &second_definition, &scenario] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(global.join("Clang.wav"), b"global clang").test_value();
    fs::write(
        first_definition.join("CLANG.WAV"),
        b"first definition clang",
    )
    .test_value();
    fs::write(
        second_definition.join("clang.wav"),
        b"second definition clang",
    )
    .test_value();
    fs::write(scenario.join("ClAnG.WaV"), b"scenario clang").test_value();

    let mut resolver = audio_fixture!(
        sound_resolver:
            collect_sound_libraries_for_path(&global),
            Vec::new(),
            None,
            direct_sound_sample_loads(&Group::open(&global).test_value()),
    );
    resolver.rebuild_sample_ranks();
    main_assert_eq!(resolver.resolve_entry("Clang").expect("global sample resolves").load_audio().expect("read global sample") => b"global clang");

    let first_group = Group::open(&first_definition).test_value();
    resolver.register_definition_group("CLNK", &first_group);
    main_assert_eq!(
        resolver
            .resolve_entry("Clang")
            .expect("definition sample overrides global")
            .load_audio()
            .expect("read definition sample") =>
        b"first definition clang"
    );

    let second_group = Group::open(&second_definition).test_value();
    resolver.register_definition_group("MORE", &second_group);
    main_assert_eq!(
        resolver
            .resolve_entry("Clang")
            .expect("later definition sample overrides earlier definition")
            .load_audio()
            .expect("read later definition sample") =>
        b"second definition clang"
    );

    main_assert!(resolver.configure_scenario(Some(&scenario)));
    main_assert_eq!(resolver.resolve_entry("Clang").expect("scenario sample overrides definition").load_audio().expect("read scenario sample") => b"scenario clang");
    let wildcard = resolver
        .resolve_entry_with_random("Clang.???", |range| {
            main_assert_eq!(range => 1, "shadowed filenames are one C++ sample");
            0
        })
        .test_value();
    main_assert_eq!(wildcard.load_audio().expect("read wildcard sample") => b"scenario clang");
}

#[test]
fn external_pure_c4d_sound_folder_without_defcore_is_playable() {
    struct FixtureResolver {
        definitions: Group,
    }

    impl LegacyDefinitionResolver for FixtureResolver {
        fn resolve_definition_groups(
            &self,
            _scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            if identifier.eq_ignore_ascii_case("Objects.c4d") {
                Ok(vec![self.definitions.clone()])
            } else {
                Err(ScenarioError::LegacyDefinitionNotFound {
                    path: identifier.to_string(),
                })
            }
        }
    }

    let dir = tempdir();
    let definitions = dir.path().join("Objects.c4d");
    let valid = definitions.join("Valid.c4d");
    let pure_sounds = definitions.join("Potions.c4d");
    let scenario_path = dir.path().join("SoundTest.c4s");
    for path in [&valid, &pure_sounds, &scenario_path] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(
        valid.join("DefCore.txt"),
        "[DefCore]\nid=TEST\nName=Test\nCategory=1\n",
    )
    .test_value();
    write_test_definition_graphics(&valid);
    fs::write(pure_sounds.join("Drink.wav"), silent_pcm_wav(20)).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Sound Test\n\n[Definitions]\nDefinition1=Objects.c4d\n",
    )
    .test_value();

    let fixture_resolver = FixtureResolver {
        definitions: Group::open(&definitions).test_value(),
    };
    let scenario = Scenario::load_from_path_with(&scenario_path, &fixture_resolver).test_value();
    main_assert!(scenario.sound_effect_groups().iter().any(|group| group.root() == pure_sounds), "the live C4DefList event stream retains the DefCore-less child");

    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.resolver = SoundResolver::empty();
    audio.refresh_sound_catalog();
    let advertised = configure_scenario_sound_samples(Some(&mut audio), &scenario, &scenario_path);
    main_assert!(advertised.iter().any(|name| name == "drink.wav"));
    main_assert!(
        audio
            .ensure_sound_with_key("Drink")
            .expect("decode pure-container sample")
            .is_some(),
        "the advertised pure-container sample must also be playable"
    );
}

#[test]
fn nested_non_definition_audio_is_not_admitted() {
    let dir = tempdir();
    let root = dir.path().join("Objects.c4d");
    let pure = root.join("Pure.c4d");
    let child = pure.join("Child.c4d");
    let ordinary = root.join("Ordinary");
    let sound_sibling = root.join("SoundExtra.c4g");
    let nested_ordinary = pure.join("Assets");
    for path in [&child, &ordinary, &sound_sibling, &nested_ordinary] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(root.join("Root.wav"), b"root").test_value();
    fs::write(pure.join("Direct.ogg"), b"direct").test_value();
    fs::write(child.join("Child.mp3"), b"child").test_value();
    fs::write(ordinary.join("Leak.wav"), b"ordinary leak").test_value();
    fs::write(sound_sibling.join("Leak.ogg"), b"sound sibling leak").test_value();
    fs::write(nested_ordinary.join("Deep.mp3"), b"nested leak").test_value();

    let root = Group::open(&root).test_value();
    let mut sound_effect_groups = Vec::new();
    collect_definition_tree_sound_groups(&root, &mut sound_effect_groups);
    let mut resolver = SoundResolver::empty();
    resolver.configure_scenario_with_sound_effect_groups(None, &sound_effect_groups);

    main_assert_eq!(resolver.sample_names() => ["child.mp3", "direct.ogg", "root.wav"]);
    for rejected in ["Leak", "Leak.ogg", "Deep.mp3"] {
        main_assert!(resolver.resolve_entry(rejected).is_none(), "non-definition descendant `{rejected}` entered the sound bank");
    }
}

#[test]
fn sound_sample_rank_tracks_cpp_last_load_order_separately_from_precedence() {
    let dir = tempdir();
    let global = dir.path().join("Sound.c4g");
    let definition = dir.path().join("Objects.c4d");
    let scenario = dir.path().join("Order.c4s");
    for path in [&global, &definition, &scenario] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(global.join("A.wav"), b"global a").test_value();
    fs::write(global.join("B.wav"), b"global b").test_value();
    fs::write(definition.join("A.wav"), b"definition a").test_value();
    fs::write(definition.join("C.wav"), b"definition c").test_value();
    fs::write(scenario.join("B.wav"), b"scenario b").test_value();
    fs::write(scenario.join("D.wav"), b"scenario d").test_value();

    let global_group = Group::open(&global).test_value();
    let mut resolver = audio_fixture!(
        sound_resolver:
            collect_sound_libraries_for_path(&global),
            Vec::new(),
            None,
            direct_sound_sample_loads(&global_group),
    );
    resolver.rebuild_sample_ranks();
    let definition_group = Group::open(&definition).test_value();
    let scenario_group = Group::open(&scenario).test_value();
    let sound_effect_groups = [definition_group, scenario_group];
    main_assert!(resolver.configure_scenario_with_sound_effect_groups(Some(&scenario), &sound_effect_groups,));

    for definition_sample in ["a.wav", "c.wav"] {
        for scenario_sample in ["b.wav", "d.wav"] {
            main_assert!(resolver.sample_order(definition_sample) < resolver.sample_order(scenario_sample), "definition samples load before the scenario tree");
        }
    }
    let mut expected_wildcard_order = ["a.wav", "b.wav", "c.wav", "d.wav"];
    expected_wildcard_order.sort_by_key(|name| resolver.sample_order(name));
    let wildcard_order = (0..expected_wildcard_order.len())
        .map(|selected| {
            resolver
                .resolve_entry_with_random("*", |range| {
                    main_assert_eq!(range => expected_wildcard_order.len());
                    selected
                })
                .test_value()
                .file_name()
                .to_string()
        })
        .collect::<Vec<_>>();
    main_assert_eq!(wildcard_order => expected_wildcard_order);

    main_assert_eq!(resolver.resolve_entry("A").expect("definition override").load_audio().expect("read definition A") => b"definition a");
    main_assert_eq!(resolver.resolve_entry("B").expect("scenario override").load_audio().expect("read scenario B") => b"scenario b");
}

#[test]
fn sound_sample_rank_prebuilds_definition_trees_and_resets_between_scenarios() {
    let dir = tempdir();
    let definitions = dir.path().join("Objects.c4d");
    let definition_child = definitions.join("Child.c4d");
    let first_scenario = dir.path().join("First.c4s");
    let scenario_child = first_scenario.join("Local.c4d");
    let second_scenario = dir.path().join("Second.c4s");
    for path in [
        &definitions,
        &definition_child,
        &first_scenario,
        &scenario_child,
        &second_scenario,
    ] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(definitions.join("Def.ogg"), b"definition root").test_value();
    fs::write(definitions.join("Def.wav"), b"definition wav").test_value();
    fs::write(definition_child.join("Nested.wav"), b"nested def").test_value();
    fs::write(first_scenario.join("Root.wav"), b"scenario root").test_value();
    fs::write(scenario_child.join("Local.wav"), b"scenario child").test_value();
    fs::write(second_scenario.join("Next.wav"), b"next scenario").test_value();

    let definitions = Group::open(&definitions).test_value();
    let mut resolver = audio_fixture!(sound_resolver: Vec::new(), Vec::new(), None, Vec::new());
    resolver.rebuild_sample_ranks();
    let first_scenario_group = Group::open(&first_scenario).test_value();
    let mut first_sound_effect_groups = Vec::new();
    collect_definition_tree_sound_groups(&definitions, &mut first_sound_effect_groups);
    collect_definition_tree_sound_groups(&first_scenario_group, &mut first_sound_effect_groups);
    main_assert!(resolver.configure_scenario_with_sound_effect_groups(Some(&first_scenario), &first_sound_effect_groups,));

    let ordered = ["def.wav", "def.ogg", "nested.wav", "root.wav", "local.wav"];
    main_assert!(ordered.windows(2).all(|pair| resolver.sample_order(pair[0]) < resolver.sample_order(pair[1])));

    let second_scenario_group = Group::open(&second_scenario).test_value();
    main_assert!(resolver.configure_scenario_with_sound_effect_groups(Some(&second_scenario), std::slice::from_ref(&second_scenario_group),));
    main_assert_eq!(resolver.sample_order("next.wav") => 0);
    for stale in ordered {
        main_assert!(!resolver.sample_ranks.contains_key(stale));
    }
}

#[test]
fn loop_started_while_muted_gets_a_channel_after_unmute() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(1_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    audio.options.sound_enabled = false;
    snapshot.audio.push(test_sound_command(true));

    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    snapshot.audio.clear();
    main_assert!(audio.active_channels.contains_key(&key));
    main_assert!(audio.active_channels[&key].channel.is_none());
    let original_started_at = audio.active_channels[&key].started_at;

    snapshot.audio.push(test_sound_command(true));
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    snapshot.audio.clear();
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert_eq!(audio.active_channels[&key].started_at => original_started_at);

    audio.options.sound_enabled = true;
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    let channel = audio.active_channels[&key].channel.test_value();
    main_assert!(audio.system.channel_is_playing(channel));
}

#[test]
fn synchronous_deleted_target_rejects_loop_and_applies_cached_one_shot_mix() {
    struct DeletedTargetWorld(ObjectId);

    impl clonk_engine::LocalAudioWorld for DeletedTargetWorld {
        fn object_position(&self, object: ObjectId) -> Option<Vector2> {
            (object == self.0).then_some(Vector2::new(40, 50))
        }

        fn object_status_present(&self, object: ObjectId) -> bool {
            object != self.0
        }

        fn player_view(&self, _player: i32) -> Option<clonk_engine::LocalAudioPlayerView> {
            None
        }
    }

    let (_dir, mut audio, _) = test_audio_context_with_sound(1_000);
    let target = ObjectId::new(7);
    let world = DeletedTargetWorld(target);
    let request = |looped| clonk_engine::LocalSoundStart {
        name: "Loop".to_string(),
        target: Some(target),
        volume: 100,
        looped,
        multiple: true,
        custom_falloff: Some(350),
        target_position: None,
        position: None,
    };

    audio.set_synchronous_sound_state(&[audio_viewport(0, OWNER_NONE, Vector2::new(40, 50))], true);
    audio.rendered_object_audibility.insert(
        target,
        CachedObjectAudibilityMix {
            object_position: Vector2::new(40, 50),
            audibility: 50,
            pan: 25,
        },
    );
    let (initial_volume, initial_pan) =
        deleted_target_initial_execute_mix((1.0, 0.0), (50, 0.25), Some(1_400));
    main_assert!((initial_volume - 0.75).abs() < 1e-6, "volume={initial_volume}");
    main_assert!((initial_pan - 0.25).abs() < 1e-6, "pan={initial_pan}");

    // Fresh Instance::Execute caches the object before DetachObj, so a
    // one-shot first gets the final positional volume/pan and then still
    // applies that cached object's audibility, custom falloff and pan
    // (C4SoundSystem.cpp:153-170,190-202,351-355). A loop is rejected.
    main_assert!(!clonk_engine::SynchronousSoundHost::start_sound(
        &mut audio,
        &request(true),
        &world,
    ));
    main_assert!(audio.active_channels.is_empty());
    main_assert!(clonk_engine::SynchronousSoundHost::start_sound(
        &mut audio,
        &request(false),
        &world,
    ));
    let info = &audio.active_channels[&SoundInstanceKey::new("Loop", None)];
    let (volume, pan) = info.detached_mix.test_value();
    main_assert!((volume - 1.0).abs() < 1e-6, "volume={volume}");
    main_assert!(pan.abs() < 1e-6, "pan={pan}");
    main_assert!(info.channel.is_none(), "custom falloff silences Execute(true)");

    // The cached object pointer is local to Execute(true). On the next pass
    // the instance is position-only and restores at its detached raw mix.
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    audio.update_channels(
        &snapshot,
        &[audio_viewport(0, OWNER_NONE, Vector2::new(40, 50))],
        true,
    );
    main_assert!(audio.active_channels[&SoundInstanceKey::new("Loop", None)].channel.is_some());
}

#[test]
fn channel_restore_at_capacity_follows_sample_then_instance_order() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    for name in ["First.wav", "Second.wav", "Third.wav"] {
        fs::write(scenario.join(name), silent_pcm_wav(1_000)).test_value();
    }
    let mut audio = AudioContext::try_new(audio_fixture!(audio_max_channels: 1)).test_value();
    audio.configure_scenario(Some(&scenario));
    audio.options.sound_enabled = false;
    let snapshot = make_snapshot(Vec::new(), Vec::new());

    for name in ["First", "Second", "Third"] {
        audio
            .start_sound(name, None, 100, true, false, None, &snapshot, &[])
            .test_value();
    }
    main_assert_eq!(audio.active_channels.len() => 3);
    main_assert!(audio.active_channels.values().all(|info| info.channel.is_none()));

    // Assign synthetic native tuple ranks that deliberately disagree with
    // this map's iteration order. This keeps the regression deterministic
    // despite RandomState while exercising both ordering fields.
    let hash_order = audio.active_channels.keys().cloned().collect::<Vec<_>>();
    let winner = hash_order[2].clone();
    let same_sample_later = hash_order[1].clone();
    let later_sample = hash_order[0].clone();
    {
        let info = audio.active_channels.get_mut(&winner).test_value();
        info.sample_order = 0;
        info.instance_order = 1;
    }
    {
        let info = audio
            .active_channels
            .get_mut(&same_sample_later)
            .test_value();
        info.sample_order = 0;
        info.instance_order = 2;
    }
    {
        let info = audio.active_channels.get_mut(&later_sample).test_value();
        info.sample_order = 1;
        info.instance_order = 0;
    }

    audio.options.sound_enabled = true;
    audio.update_channels(&snapshot, &[], true);

    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert!(audio.active_channels.contains_key(&winner));
    let channel = audio.active_channels[&winner].channel.test_value();
    main_assert!(audio.system.channel_is_playing(channel));
    main_assert!(!audio.active_channels.contains_key(&same_sample_later));
    main_assert!(!audio.active_channels.contains_key(&later_sample));
}

#[test]
fn deleted_earlier_loop_frees_capacity_for_ordered_restore() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    for name in ["First.wav", "Second.wav"] {
        fs::write(scenario.join(name), silent_pcm_wav(1_000)).test_value();
    }
    let mut audio = AudioContext::try_new(audio_fixture!(audio_max_channels: 1)).test_value();
    audio.configure_scenario(Some(&scenario));

    let first = make_object(1, "SND1", Vector2::ZERO);
    let second = make_object(2, "SND2", Vector2::new(100, 0));
    let initial = make_snapshot(vec![first.clone(), second.clone()], Vec::new());
    audio
        .start_sound(
            "First",
            Some(first.id),
            100,
            true,
            false,
            None,
            &initial,
            &[audio_viewport(0, OWNER_NONE, first.position)],
        )
        .test_value();
    audio
        .start_sound(
            "Second",
            Some(second.id),
            100,
            true,
            false,
            None,
            &initial,
            &[],
        )
        .test_value();
    let first_key = SoundInstanceKey::new("First", Some(first.id));
    let second_key = SoundInstanceKey::new("Second", Some(second.id));
    main_assert!(audio.active_channels[&first_key].channel.is_some());
    main_assert!(audio.active_channels[&second_key].channel.is_none());
    audio
        .active_channels
        .get_mut(&first_key)
        .test_value()
        .sample_order = 0;
    audio
        .active_channels
        .get_mut(&second_key)
        .test_value()
        .sample_order = 1;

    let without_first = make_snapshot(vec![second.clone()], Vec::new());
    audio.update_channels(
        &without_first,
        &[audio_viewport(0, OWNER_NONE, second.position)],
        true,
    );

    main_assert!(!audio.active_channels.contains_key(&first_key));
    let restored = audio.active_channels.get(&second_key).test_value();
    let channel = restored.channel.test_value();
    main_assert!(audio.system.channel_is_playing(channel));
}

#[test]
fn channel_less_muted_loop_still_obeys_volume_and_stop_commands() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(1_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    audio.options.sound_enabled = false;
    snapshot.audio.push(test_sound_command(true));
    audio.process_audio(&snapshot, &mut runtime_music_enabled);

    snapshot.audio = vec![audio_fixture!(set_volume: "Loop".to_string(), 37)];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert_eq!(audio.active_channels[&key].volume => 37);
    main_assert!(audio.active_channels[&key].channel.is_none());

    snapshot.audio = vec![AudioCommand::StopSound {
        name: "Loop".to_string(),
        target: None,
    }];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(!audio.active_channels.contains_key(&key));

    snapshot.audio.clear();
    audio.options.sound_enabled = true;
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(!audio.active_channels.contains_key(&key));
}

#[test]
fn sound_level_revolumes_and_stops_a_prior_frame_one_shot() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    snapshot.audio.push(test_sound_command(false));
    audio.process_audio(&snapshot, &mut runtime_music_enabled);

    let original = &audio.active_channels[&key];
    let original_channel = original.channel.test_value();
    let original_started_at = original.started_at;
    snapshot.audio = vec![audio_fixture!(set_volume: "Loo?".to_string(), 50)];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);

    let updated = &audio.active_channels[&key];
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert_eq!(updated.channel => Some(original_channel));
    main_assert_eq!(updated.started_at => original_started_at);
    main_assert_eq!(updated.volume => 50);
    main_assert!(!updated.looped, "SoundLevel must not promote a one-shot");
    main_assert!(audio.system.channel_is_playing(original_channel));

    snapshot.audio = vec![AudioCommand::StopSound {
        name: "Loop.wav".to_string(),
        target: None,
    }];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(!audio.active_channels.contains_key(&key));
    main_assert!(!audio.system.channel_is_playing(original_channel));
}

#[test]
fn sound_level_starts_and_reuses_a_loop_when_no_instance_exists() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    snapshot.audio = vec![audio_fixture!(set_volume: "Loop".to_string(), 37)];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);

    let started = &audio.active_channels[&key];
    let original_channel = started.channel.test_value();
    let original_started_at = started.started_at;
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert!(started.looped);
    main_assert_eq!(started.volume => 37);
    main_assert!(audio.system.channel_is_playing(original_channel));

    snapshot.audio = vec![audio_fixture!(set_volume: "Loop".to_string(), 61)];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    let updated = &audio.active_channels[&key];
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert_eq!(updated.channel => Some(original_channel));
    main_assert_eq!(updated.started_at => original_started_at);
    main_assert!(updated.looped);
    main_assert_eq!(updated.volume => 61);

    snapshot.audio = vec![AudioCommand::StopSound {
        name: "Loop".to_string(),
        target: None,
    }];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(!audio.active_channels.contains_key(&key));
    main_assert!(!audio.system.channel_is_playing(original_channel));
}

#[test]
fn sound_level_above_100_reaches_app_instance_unchanged() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    snapshot.audio = vec![audio_fixture!(set_volume: "Loop".to_string(), 140)];

    audio.process_audio(&snapshot, &mut runtime_music_enabled);

    main_assert_eq!(audio.active_channels[&key].volume => 140);
    let (mix_volume, pan) = compute_mix_values(
        audio.active_channels.get_mut(&key).test_value(),
        &snapshot,
        &[],
    );
    main_assert!((mix_volume - 1.4).abs() < 1.0e-6);
    main_assert_eq!(pan => 0.0);
}

#[test]
fn sound_level_starts_a_loop_after_the_finished_one_shot_is_swept() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    snapshot.audio.push(test_sound_command(false));
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    let finished_channel = audio.active_channels[&key].channel.test_value();
    audio.system.halt_channel(finished_channel);

    snapshot.audio = vec![audio_fixture!(set_volume: "Loop".to_string(), 45)];
    audio.process_audio(&snapshot, &mut runtime_music_enabled);

    main_assert!(!audio.active_channels.contains_key(&key), "the unswept C++ instance counts as found, then cleanup removes it");
    audio.process_audio(&snapshot, &mut runtime_music_enabled);

    let fallback = &audio.active_channels[&key];
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert!(fallback.looped);
    main_assert_eq!(fallback.volume => 45);
    main_assert!(fallback.channel.is_some_and(|channel| audio.system.channel_is_playing(channel)));
}

#[test]
fn muted_one_shot_past_half_duration_is_culled_before_it_can_resume() {
    let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
    let key = SoundInstanceKey::new("Loop", None);
    let mut runtime_music_enabled = false;
    snapshot.audio.push(test_sound_command(false));
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    snapshot.audio.clear();
    audio.active_channels.get_mut(&key).test_value().started_at =
        Instant::now() - Duration::from_millis(6_000);

    audio.options.sound_enabled = false;
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(audio.active_channels.contains_key(&key));
    main_assert!(audio.active_channels[&key].channel.is_none());

    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(!audio.active_channels.contains_key(&key));

    audio.options.sound_enabled = true;
    audio.process_audio(&snapshot, &mut runtime_music_enabled);
    main_assert!(!audio.active_channels.contains_key(&key));
}

#[test]
fn inaudible_loops_leave_channels_free_for_a_nearby_sound() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    for name in ["FarA.wav", "FarB.wav", "Near.wav"] {
        fs::write(scenario.join(name), silent_pcm_wav(10_000)).test_value();
    }
    let mut audio = AudioContext::try_new(audio_fixture!(audio_max_channels: 2)).test_value();
    audio.configure_scenario(Some(&scenario));

    let listener = make_object(1, "LIST", Vector2::ZERO);
    let first = make_object(2, "SND1", Vector2::new(1_000, 0));
    let second = make_object(3, "SND2", Vector2::new(1_100, 0));
    let nearby = make_object(4, "SND3", Vector2::new(100, 0));
    let initial = make_snapshot(
        vec![
            listener.clone(),
            first.clone(),
            second.clone(),
            nearby.clone(),
        ],
        Vec::new(),
    );
    for (name, target) in [("FarA", first.id), ("FarB", second.id)] {
        audio
            .start_sound(
                name,
                Some(target),
                100,
                true,
                true,
                None,
                &initial,
                &[audio_viewport(0, OWNER_NONE, listener.position)],
            )
            .test_value();
    }
    let first_key = SoundInstanceKey::new("FarA", Some(first.id));
    let second_key = SoundInstanceKey::new("FarB", Some(second.id));
    main_assert!(audio.active_channels[&first_key].channel.is_none());
    main_assert!(audio.active_channels[&second_key].channel.is_none());

    audio
        .start_sound(
            "Near",
            Some(nearby.id),
            100,
            false,
            true,
            None,
            &initial,
            &[audio_viewport(0, OWNER_NONE, listener.position)],
        )
        .test_value();
    let nearby_channel = audio
        .active_channels
        .get(&SoundInstanceKey::new("Near", Some(nearby.id)))
        .test_value()
        .channel
        .test_value();
    main_assert!(audio.system.channel_is_playing(nearby_channel));
}

#[test]
fn inaudible_one_shot_is_culled_one_update_after_half_duration() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Impact.wav"), silent_pcm_wav(10_000)).test_value();
    let mut audio = AudioContext::try_new(audio_fixture!(audio_max_channels: 1)).test_value();
    audio.configure_scenario(Some(&scenario));

    let listener = make_object(1, "LIST", Vector2::ZERO);
    let source = make_object(2, "SNDS", Vector2::new(100, 0));
    let initial = make_snapshot(vec![listener.clone(), source.clone()], Vec::new());
    audio
        .start_sound(
            "Impact",
            Some(source.id),
            100,
            false,
            true,
            None,
            &initial,
            &[audio_viewport(0, OWNER_NONE, listener.position)],
        )
        .test_value();
    let key = SoundInstanceKey::new("Impact", Some(source.id));
    let channel = audio.active_channels[&key].channel.test_value();
    audio.active_channels.get_mut(&key).test_value().started_at =
        Instant::now() - Duration::from_millis(6_000);

    let moved = make_snapshot(
        vec![
            listener.clone(),
            make_object(source.id.as_u64(), "SNDS", Vector2::new(1_000, 0)),
        ],
        Vec::new(),
    );
    let viewports = [audio_viewport(0, OWNER_NONE, listener.position)];
    audio.update_channels(&moved, &viewports, true);
    main_assert!(audio.active_channels.contains_key(&key), "the release pass retains the logical one-shot instance");
    main_assert!(audio.active_channels[&key].channel.is_none());
    main_assert!(!audio.system.channel_is_playing(channel));

    audio.update_channels(&moved, &viewports, true);
    main_assert!(!audio.active_channels.contains_key(&key), "the next pass culls an inaudible one-shot past half duration");
}

#[test]
fn scenario_sound_resolver_loads_local_definition_folder_root_only() {
    // C4Game::FoldersWithLocalsDefs adds each .c4f ancestor containing a
    // direct *.c4d child as a definition resource (C4Game.cpp:3961-3994).
    // C4DefList::Load first tries that resource root, and even though it
    // has no DefCore, C4Def::Load still loads its direct sound effects
    // (C4Def.cpp:927-950, 591-596). It never descends into sibling .c4s
    // groups, because the recursive definition scan accepts only *.c4d.
    let dir = tempdir();
    let folder = dir.path().join("Tutorial.c4f");
    let definitions = folder.join("Objects.c4d");
    let scenario = folder.join("Tutorial01.c4s");
    let sibling = folder.join("Tutorial02.c4s");
    fs::create_dir_all(&definitions).test_value();
    fs::create_dir_all(&scenario).test_value();
    fs::create_dir_all(&sibling).test_value();
    fs::write(folder.join("Drop.wav"), b"parent drop").test_value();
    fs::write(definitions.join("Voice.wav"), b"definition voice").test_value();
    fs::write(sibling.join("Sibling.wav"), b"sibling sound").test_value();

    let mut resolver = audio_fixture!(sound_resolver: Vec::new(), Vec::new(), None, Vec::new());
    main_assert!(resolver.configure_scenario(Some(&scenario)));

    main_assert_eq!(resolver.resolve_entry("Drop").expect("parent-folder root sound").load_audio().expect("read parent-folder sound") => b"parent drop");
    main_assert!(resolver.resolve_entry("Sibling").is_none(), "sibling scenario sounds are not definition-folder root sounds");
    main_assert_eq!(resolver.sample_names() => vec!["drop.wav", "voice.wav"], "the engine-facing inventory follows the same admitted libraries");
    main_assert_eq!(resolver.resolve_entry("Voice").expect("local definition-tree sound").load_audio().expect("read local definition sound") => b"definition voice");
}

#[test]
fn compute_mix_values_matches_cxx_audibility() {
    let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
    let source = make_object(2, "Source", Vector2::new(1350, 1000));
    let snapshot = make_snapshot(
        vec![listener.clone(), source.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    let (volume, pan) = compute_mix_values_for(
        100,
        Some(source.id),
        None,
        &snapshot,
        &[audio_viewport(0, OWNER_NONE, listener.position)],
    );
    main_assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
    main_assert!((pan - 0.7).abs() < 1e-6, "pan={pan}");
}

#[test]
fn compute_mix_values_respects_custom_falloff() {
    let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
    let source = make_object(2, "Source", Vector2::new(1700, 1000));
    let snapshot = make_snapshot(
        vec![listener.clone(), source.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    let (volume, pan) = compute_mix_values_for(
        100,
        Some(source.id),
        Some(1400),
        &snapshot,
        &[audio_viewport(0, OWNER_NONE, listener.position)],
    );
    main_assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
    main_assert!((pan - 1.0).abs() < 1e-6, "pan={pan}");
}

#[test]
fn negative_custom_falloff_matches_cpp_transform() {
    // C4SoundSystem applies the signed integer transform for every
    // nonzero value. A negative denominator therefore clamps every raw
    // audibility to full volume, while pan remains positional.
    for audibility in [0, 50, 100] {
        main_assert_eq!(adjusted_audibility(audibility, Some(-700)) => 1.0);
    }
    main_assert_eq!(adjusted_audibility(0, Some(0)) => 0.0);

    let (_dir, mut audio, _) = test_audio_context_with_sound(1_000);
    let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
    let source = make_object(2, "Source", Vector2::new(1700, 1000));
    let snapshot = make_snapshot(
        vec![listener.clone(), source.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    let viewports = [audio_viewport(0, OWNER_NONE, listener.position)];

    audio
        .start_sound(
            "Loop",
            Some(source.id),
            100,
            true,
            false,
            Some(-700),
            &snapshot,
            &viewports,
        )
        .test_value();
    let key = SoundInstanceKey::new("Loop", Some(source.id));
    let info = audio.active_channels.get_mut(&key).test_value();
    main_assert_eq!(info.custom_falloff => Some(-700));
    let (volume, pan) = compute_mix_values(info, &snapshot, &viewports);
    main_assert_eq!(volume => 1.0);
    main_assert_eq!(pan => 1.0);
}

#[test]
fn compute_mix_values_for_global_sound_preserves_base_mix() {
    let listener = make_object(1, "Listener", Vector2::new(0, 0));
    let snapshot = make_snapshot(
        vec![listener.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    let (volume, pan) = compute_mix_values_for(80, None, None, &snapshot, &[]);
    main_assert!((volume - 0.8).abs() < 1e-6);
    main_assert_eq!(pan => 0.0);
}

#[test]
fn viewport_feedback_coalesces_global_sample_and_uses_initial_sound_gate() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("CloseViewport.wav"), silent_pcm_wav(1_000)).test_value();
    let mut audio = AudioContext::try_new(
        audio_fixture!(audio_sound_enabled_menu_sound_enabled_max_channels: false, true, 1),
    )
    .test_value();
    audio.configure_scenario(Some(&scenario));
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    let key = SoundInstanceKey::new("CloseViewport", None);

    main_assert!(audio.try_start_global_effect("CloseViewport", false, &snapshot).expect("pre-running feedback starts"));
    main_assert!(audio.active_channels[&key].channel.is_some());
    main_assert!(
        !audio
            .try_start_global_effect("CloseViewport", false, &snapshot)
            .expect("duplicate feedback is rejected"),
        "C++ keeps one global instance per resolved sample"
    );
    main_assert_eq!(audio.active_channels.len() => 1);

    audio.reset_sfx();
    main_assert!(audio.try_start_global_effect("CloseViewport", true, &snapshot).expect("running muted feedback keeps a logical instance"));
    main_assert!(audio.active_channels[&key].channel.is_none());
}

#[test]
fn running_gui_sound_requires_fe_samples_and_rx_sound() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("ArrowHit.wav"), silent_pcm_wav(1_000)).test_value();
    fs::write(scenario.join("Elevator.wav"), silent_pcm_wav(1_000)).test_value();
    let mut audio = AudioContext::try_new(
        audio_fixture!(audio_sound_enabled_menu_sound_enabled_max_channels: true, false, 2),
    )
    .test_value();
    audio.configure_scenario(Some(&scenario));
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    let key = SoundInstanceKey::new("ArrowHit", None);

    audio.play_gui_sound("ArrowHit", false, &snapshot);
    audio.play_gui_sound("ArrowHit", true, &snapshot);
    main_assert!(audio.active_channels.is_empty(), "FESamples rejects new GUI requests in every game state");
    main_assert!(
        audio
            .try_start_global_effect("ArrowHit", false, &snapshot)
            .expect("direct effect creates a muted logical instance"),
        "native direct StartSoundEffect has no outer GUI gate"
    );
    main_assert!(audio.active_channels[&key].channel.is_none());
    audio.reset_sfx();

    audio.options.menu_sound_enabled = true;
    audio.options.sound_enabled = false;
    audio.play_gui_sound("ArrowHit", true, &snapshot);
    main_assert!(audio.active_channels.contains_key(&key));
    main_assert!(audio.active_channels[&key].channel.is_none(), "RXSound mutes playback but retains the admitted GUI instance");
    audio.options.sound_enabled = true;
    audio.update_channels(&snapshot, &[], true);
    main_assert!(audio.active_channels[&key].channel.is_some(), "unmuting before half duration recreates the SDL channel");

    audio.reset_sfx();
    audio.options.sound_enabled = false;
    audio.play_gui_sound("ArrowHit", true, &snapshot);
    let muted = &audio.active_channels[&key];
    main_assert!(!muted.non_looping_past_half_duration(muted.started_at + Duration::from_millis(500)));
    main_assert!(muted.non_looping_past_half_duration(muted.started_at + Duration::from_millis(501)));
    audio.active_channels.get_mut(&key).test_value().started_at =
        Instant::now() - Duration::from_millis(600);
    audio.update_channels(&snapshot, &[], true);
    main_assert!(!audio.active_channels.contains_key(&key), "a channel-less one-shot expires strictly after half duration");

    audio.play_gui_sound("ArrowHit", false, &snapshot);
    let startup_channel = audio.active_channels[&key].channel.test_value();
    audio.update_channels(&snapshot, &[], true);
    main_assert!(audio.active_channels[&key].channel.is_none());
    main_assert!(!audio.system.channel_is_playing(startup_channel));

    audio.reset_sfx();
    audio.options.menu_sound_enabled = false;
    audio.start_lobby_elevator(&snapshot);
    let elevator = SoundInstanceKey::new("Elevator", None);
    main_assert!(audio.active_channels[&elevator].looped);
    main_assert!(audio.active_channels[&elevator].channel.is_none());
    audio.start_lobby_elevator(&snapshot);
    main_assert_eq!(audio.active_channels.len() => 1);
    audio.options.menu_sound_enabled = true;
    audio.update_channels(&snapshot, &[], false);
    main_assert!(audio.active_channels[&elevator].channel.is_some());
    audio.stop_lobby_elevator();
    main_assert!(audio.active_channels.is_empty());
}

#[test]
fn global_gui_instance_survives_transitions_until_its_sample_is_reloaded() {
    let dir = tempdir();
    let scenario = dir.path().join("First.c4s");
    let unrelated = dir.path().join("Unrelated.c4d");
    let invalid_replacement = dir.path().join("Invalid.c4d");
    let replacement = dir.path().join("Replacement.c4d");
    for path in [&scenario, &unrelated, &invalid_replacement, &replacement] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(scenario.join("Click.wav"), silent_pcm_wav(1_000)).test_value();
    fs::write(unrelated.join("Command.wav"), silent_pcm_wav(1_000)).test_value();
    fs::write(
        invalid_replacement.join("CLICK.WAV"),
        b"not an audio stream",
    )
    .test_value();
    fs::write(replacement.join("Click.wav"), silent_pcm_wav(1_000)).test_value();

    let mut audio = AudioContext::try_new(
        audio_fixture!(audio_sound_enabled_menu_sound_enabled_max_channels: false, true, 2),
    )
    .test_value();
    let scenario_group = Group::open(&scenario).test_value();
    audio.configure_scenario_with_resources(
        Some(&scenario),
        Some(&[]),
        Some(std::slice::from_ref(&scenario_group)),
    );
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    let key = SoundInstanceKey::new("Click", None);
    audio.play_gui_sound("Click", false, &snapshot);
    let original_sample_order = audio.active_channels[&key].sample_order;
    let frontend_channel = audio.active_channels[&key].channel.test_value();

    let unrelated_group = Group::open(&unrelated).test_value();
    audio.register_definition_sounds("UNRELATED", &unrelated_group);
    main_assert!(audio.active_channels.contains_key(&key));
    main_assert!(audio.system.channel_is_playing(frontend_channel));

    audio.update_channels(&snapshot, &[], true);
    main_assert!(audio.active_channels[&key].channel.is_none());
    main_assert!(!audio.system.channel_is_playing(frontend_channel));
    main_assert!(audio.active_channels.contains_key(&key), "the startup instance crosses into running RXSound control");
    audio.options.sound_enabled = true;
    audio.update_channels(&snapshot, &[], true);
    let restored_channel = audio.active_channels[&key].channel.test_value();

    let invalid_group = Group::open(&invalid_replacement).test_value();
    audio.register_definition_sounds("INVALID", &invalid_group);
    main_assert!(audio.active_channels.contains_key(&key), "an undecodable replacement leaves the prior sample instance alive");
    main_assert_eq!(audio.active_channels[&key].sample_order => original_sample_order, "a failed replacement does not move the prior sample in catalog order");
    main_assert!(audio.system.channel_is_playing(restored_channel));

    let replacement_group = Group::open(&replacement).test_value();
    audio.register_definition_sounds("TEST", &replacement_group);
    main_assert!(!audio.active_channels.contains_key(&key));
    main_assert!(!audio.system.channel_is_playing(restored_channel));

    audio.play_gui_sound("Click", false, &snapshot);
    main_assert!(audio.active_channels[&key].sample_order > original_sample_order, "a successful replacement appends the new sample at the catalog tail");
    let reloaded_channel = audio.active_channels[&key].channel.test_value();
    audio.register_definition_sounds("TEST", &replacement_group);
    main_assert!(!audio.active_channels.contains_key(&key), "reloading an already registered definition still replaces its sample");
    main_assert!(!audio.system.channel_is_playing(reloaded_channel));

    audio.play_gui_sound("Click", false, &snapshot);
    let generation_channel = audio.active_channels[&key].channel.test_value();
    audio.reset_sound_system_generation();
    main_assert!(audio.active_channels.is_empty());
    main_assert!(!audio.system.channel_is_playing(generation_channel));
    main_assert!(audio.resolver.scenario_root.is_none());
    main_assert_eq!(audio.resolver.definition_library_count => 0);
    main_assert!(audio.resolver.registered_definitions.is_empty());
}

#[test]
fn repeated_gui_sound_uses_global_instance_dedup() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Click.wav"), silent_pcm_wav(1_000)).test_value();
    let mut audio = AudioContext::try_new(audio_fixture!(audio_max_channels: 2)).test_value();
    audio.configure_scenario(Some(&scenario));
    let snapshot = make_snapshot(Vec::new(), Vec::new());
    let key = SoundInstanceKey::new("Click", None);

    audio.play_gui_sound("Click", false, &snapshot);
    let first = audio.active_channels[&key].clone();
    audio.play_gui_sound("Click.wav", false, &snapshot);
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert_eq!(audio.active_channels[&key].channel => first.channel);
    main_assert_eq!(audio.active_channels[&key].instance_order => first.instance_order, "the resolved sample's existing global instance is not retriggered");

    let channel = first.channel.test_value();
    audio.system.halt_channel(channel);
    audio.update_channels(&snapshot, &[], false);
    main_assert!(audio.active_channels.is_empty(), "frontend frames sweep completed shared instances");
    audio.play_gui_sound("Click", false, &snapshot);
    main_assert!(audio.active_channels[&key].instance_order > first.instance_order);
}

#[test]
fn positional_mix_takes_max_volume_and_sums_only_active_viewport_pans() {
    let source = make_object(1, "Source", Vector2::new(350, 100));
    let local_left = audio_fixture!(
        player_state_id_view_cursor_viewports:
            1,
            Some(source.id),
            vec![clonk_engine::PlayerViewport::new(Vector2::new(0, 100))],
    );
    let local_right = audio_fixture!(
        player_state_id_view_cursor_viewports:
            2,
            Some(source.id),
            vec![clonk_engine::PlayerViewport::new(Vector2::new(1000, 100))],
    );
    let remote = audio_fixture!(
        player_state_id_view_cursor_viewports:
            3,
            Some(source.id),
            vec![clonk_engine::PlayerViewport::new(Vector2::new(-1000, 100))],
    );
    let mut snapshot = make_snapshot(vec![source], Vec::new());
    snapshot.players = vec![local_left, local_right, remote];
    snapshot.hud.local_players = vec![1, 2];
    let viewports = [
        audio_viewport(0, 1, Vector2::new(0, 100)),
        audio_viewport(1, 2, Vector2::new(1000, 100)),
    ];

    main_assert_eq!(
        compute_positional_mix_values(Vector2::new(350, 100), &snapshot, &viewports) =>
        (100, -0.6),
        "pan is +70-130, not averaged; a player without an active viewport contributes nothing",
    );
}

#[test]
fn line_object_sound_uses_rendered_endpoints() {
    let mut line = make_object(1, "LINE", Vector2::new(2_000, 100));
    line.vertices = vec![
        clonk_engine::ObjectVertex::new(0, 100),
        clonk_engine::ObjectVertex::new(350, 100),
    ];
    let mut snapshot = make_snapshot(vec![line.clone()], Vec::new());
    snapshot.definition_lines.insert(
        line.definition_id.clone(),
        clonk_engine::DefinitionLineMetadata {
            line: 1,
            ..Default::default()
        },
    );
    let viewports = [audio_viewport(0, OWNER_NONE, Vector2::new(0, 100))];
    let mut calls = HashMap::from([(
        line.id,
        vec![
            RenderedAudibilityCall::World {
                point: Vector2::new(0, 100),
            },
            RenderedAudibilityCall::World {
                point: Vector2::new(350, 100),
            },
        ],
    )]);
    let mut audio = empty_test_audio_context();
    audio.cache_rendered_object_audibility(&calls, &snapshot, &viewports);

    main_assert_eq!(
        compute_mix_values_for_with_rendered_audibility(
            100,
            Some(line.id),
            None,
            &snapshot,
            &viewports,
            &audio.rendered_object_audibility,
        ) =>
        (0.5, 0.7),
        "the last absolute live vertex replaces the first SetAudibilityAt result",
    );

    calls.get_mut(&line.id).test_value().reverse();
    audio.cache_rendered_object_audibility(&calls, &snapshot, &viewports);
    main_assert_eq!(
        compute_mix_values_for_with_rendered_audibility(
            100,
            Some(line.id),
            None,
            &snapshot,
            &viewports,
            &audio.rendered_object_audibility,
        ) =>
        (1.0, 0.0),
        "reversing the endpoints proves native call order rather than max-volume mixing",
    );

    snapshot.definition_lines.clear();
    main_assert_eq!(
        compute_mix_values_for_with_rendered_audibility(
            100,
            Some(line.id),
            None,
            &snapshot,
            &viewports,
            &audio.rendered_object_audibility,
        ) =>
        (1.0, 0.0),
        "changing classification alone does not invalidate native's retained fields",
    );
    audio.cache_rendered_object_audibility(&HashMap::new(), &snapshot, &viewports);
    main_assert_eq!(
        compute_mix_values_for_with_rendered_audibility(
            100,
            Some(line.id),
            None,
            &snapshot,
            &viewports,
            &audio.rendered_object_audibility,
        ) =>
        (0.0, 1.0),
        "the next completed normal-object render resets the retained cache",
    );
}

#[test]
fn parallax_sound_uses_rendered_target_position() {
    let mut target = make_object(1, "PARA", Vector2::new(350, 100));
    target.category |= C4D_PARALLAX;
    let snapshot = make_snapshot(vec![target.clone()], Vec::new());
    let viewports = [
        audio_viewport(0, OWNER_NONE, Vector2::new(400, 100)),
        audio_viewport(1, OWNER_NONE, Vector2::new(0, 100)),
    ];
    let calls = HashMap::from([(
        target.id,
        vec![
            RenderedAudibilityCall::Parallax {
                point: target.position,
                rendered_center: Vector2::new(250, 100),
            },
            RenderedAudibilityCall::Parallax {
                point: target.position,
                rendered_center: Vector2::new(50, 100),
            },
        ],
    )]);
    let mut audio = empty_test_audio_context();
    audio.cache_rendered_object_audibility(&calls, &snapshot, &viewports);

    main_assert_eq!(
        compute_mix_values_for_with_rendered_audibility(
            100,
            Some(target.id),
            None,
            &snapshot,
            &viewports,
            &audio.rendered_object_audibility,
        ) =>
        (0.58, 0.8),
        "the last rendered viewport wins volume while both rendered pans accumulate",
    );
    main_assert_eq!(audio.rendered_object_audibility[&target.id] => CachedObjectAudibilityMix {object_position: target.position, audibility: 58, pan: 80,},);
    main_assert_eq!(
        compute_positional_mix_values(target.position, &snapshot, &viewports) =>
        (93, 0.6),
        "the ordinary world-origin mix remains observably different",
    );
}

#[test]
fn crew_death_sound_starts_from_the_dying_object_rendered_audibility() {
    // C4Object::GetAudibility (C4Object.cpp:5622-5628) returns the cached
    // Audible whenever it is not -1; only C4GraphicsSystem::Execute's
    // ResetAudibility (C4GraphicsSystem.cpp:158-159) invalidates it. The
    // object's own movement never does. A crew death is exactly that
    // case: AssignDeath moves the falling clonk, clears the cursor, and
    // the scenario relaunches a replacement elsewhere before the frame's
    // audio is mixed (EkeReloaded SFT.c4d/Script.c:702-721 -> HarpoonRace
    // RelaunchPlayer). The listener is therefore already across the map
    // while the dying clonk still carries the audibility drawn in the
    // frame it died in, so its Sound("SF_Die") must still get a channel.
    let dir = tempdir();
    let scenario = dir.path().join("Death.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("SF_Die.wav"), silent_pcm_wav(10_000)).test_value();
    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.configure_scenario(Some(&scenario));

    let clonk = make_object(1, "SF5B", Vector2::new(2_000, 100));
    let viewports = [audio_viewport(0, 1, clonk.position)];
    let mut drawn = make_snapshot(vec![clonk.clone()], Vec::new());
    drawn.players = vec![PlayerState {
        id: 1,
        cursor: Some(clonk.id),
        viewports: vec![clonk_engine::PlayerViewport::new(clonk.position)],
        ..Default::default()
    }];
    audio.cache_rendered_object_audibility(
        &HashMap::from([(
            clonk.id,
            vec![RenderedAudibilityCall::World {
                point: clonk.position,
            }],
        )]),
        &drawn,
        &viewports,
    );

    let mut corpse = clonk;
    corpse.position.y += 1;
    let replacement = make_object(2, "SF5B", Vector2::new(100, 100));
    let mut died = make_snapshot(vec![corpse.clone(), replacement.clone()], Vec::new());
    died.players = vec![PlayerState {
        id: 1,
        cursor: Some(replacement.id),
        viewports: vec![clonk_engine::PlayerViewport::new(replacement.position)],
        ..Default::default()
    }];
    main_assert_eq!(
        compute_mix_values_for(100, Some(corpse.id), None, &died, &viewports) =>
        (0.0, 0.0),
        "the relaunched listener alone would silence the death sound",
    );

    audio
        .start_sound(
            "SF_Die",
            Some(corpse.id),
            100,
            false,
            false,
            None,
            &died,
            &viewports,
        )
        .test_value();
    let key = SoundInstanceKey::new("SF_Die", Some(corpse.id));
    main_assert!(audio.active_channels[&key].channel.is_some(), "the retained draw audibility must keep the death sound audible",);
}

#[test]
fn post_render_attached_mix_releases_then_restores_channel_capacity() {
    let dir = tempdir();
    let scenario = dir.path().join("Audio.c4s");
    fs::create_dir_all(&scenario).test_value();
    for name in ["First.wav", "Second.wav"] {
        fs::write(scenario.join(name), silent_pcm_wav(10_000)).test_value();
    }
    let mut audio = AudioContext::try_new(audio_fixture!(audio_max_channels: 1)).test_value();
    audio.configure_scenario(Some(&scenario));

    let first = make_object(1, "SND1", Vector2::ZERO);
    let second = make_object(2, "SND2", Vector2::new(2_000, 0));
    let mut snapshot = make_snapshot(vec![first.clone(), second.clone()], Vec::new());
    snapshot.definition_lines.insert(
        second.definition_id.clone(),
        clonk_engine::DefinitionLineMetadata {
            line: 1,
            ..Default::default()
        },
    );
    let viewports = [audio_viewport(0, OWNER_NONE, Vector2::ZERO)];
    audio
        .start_sound(
            "First",
            Some(first.id),
            100,
            false,
            false,
            None,
            &snapshot,
            &viewports,
        )
        .test_value();
    audio
        .start_sound(
            "Second",
            Some(second.id),
            100,
            false,
            false,
            None,
            &snapshot,
            &viewports,
        )
        .test_value();
    let first_key = SoundInstanceKey::new("First", Some(first.id));
    let second_key = SoundInstanceKey::new("Second", Some(second.id));
    main_assert!(audio.active_channels[&first_key].channel.is_some());
    main_assert!(audio.active_channels[&second_key].channel.is_none());
    audio
        .active_channels
        .get_mut(&first_key)
        .test_value()
        .sample_order = 0;
    audio
        .active_channels
        .get_mut(&second_key)
        .test_value()
        .sample_order = 1;

    let rendered_viewports = [audio_viewport(0, OWNER_NONE, Vector2::new(1_000, 0))];
    let calls = HashMap::from([(
        second.id,
        vec![RenderedAudibilityCall::World {
            point: Vector2::new(1_000, 0),
        }],
    )]);
    audio.cache_rendered_object_audibility(&calls, &snapshot, &rendered_viewports);
    audio.refresh_attached_channel_mix_after_render(&snapshot, &rendered_viewports);

    main_assert!(audio.active_channels[&first_key].channel.is_none());
    main_assert!(audio.active_channels[&second_key].channel.is_some());
    main_assert_eq!(audio.active_channels[&first_key].detached_mix => Some((0.0, -1.0)), "the ordinary earlier sound releases its channel under the new viewport",);
    main_assert_eq!(
        audio.active_channels[&second_key].detached_mix =>
        Some((0.0, 1.0)),
        "a detached one-shot would retain the second object's origin, not its line endpoint",
    );

    let second_channel = audio.active_channels[&second_key].channel.test_value();
    audio.system.halt_channel(second_channel);
    audio.refresh_attached_channel_mix_after_render(&snapshot, &rendered_viewports);
    main_assert!(!audio.active_channels.contains_key(&second_key), "a special channel that finished during rendering is removed",);

    audio
        .start_sound(
            "Second", None, 100, false, false, None, &snapshot, &viewports,
        )
        .test_value();
    let blocker_key = SoundInstanceKey::new("Second", None);
    snapshot.definition_lines.insert(
        first.definition_id.clone(),
        clonk_engine::DefinitionLineMetadata {
            line: 1,
            ..Default::default()
        },
    );
    audio.cache_rendered_object_audibility(
        &HashMap::from([(
            first.id,
            vec![RenderedAudibilityCall::World {
                point: Vector2::new(1_000, 0),
            }],
        )]),
        &snapshot,
        &rendered_viewports,
    );
    audio.refresh_attached_channel_mix_after_render(&snapshot, &rendered_viewports);
    main_assert!(!audio.active_channels.contains_key(&first_key), "a special instance whose newly audible channel cannot be restored is removed",);
    main_assert!(audio.active_channels.contains_key(&blocker_key));
}

#[test]
fn positional_audio_handler_freezes_mix_and_rejects_second_global_instance() {
    let dir = tempdir();
    let scenario = dir.path().join("Goldrush.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Pshshsh.wav"), silent_pcm_wav(1_000)).test_value();

    let options = audio_fixture!(audio_max_channels: 2);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));

    let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
    let mut snapshot = make_snapshot(vec![listener.clone()], Vec::new());
    snapshot.players = vec![PlayerState {
        id: 7,
        view_cursor: Some(listener.id),
        ..Default::default()
    }];
    let viewports = [audio_viewport(0, 7, Vector2::new(800, 1000))];
    let event = AudioCommand::PlaySoundAt {
        name: "Pshshsh".to_string(),
        position: Vector2::new(1150, 1000),
    };
    let mut runtime_music_enabled = false;

    audio.handle_events(
        std::slice::from_ref(&event),
        &snapshot,
        &viewports,
        &mut runtime_music_enabled,
    );
    let key = SoundInstanceKey::new("Pshshsh", None);
    let first = audio.active_channels.get(&key).test_value().clone();
    main_assert!(!first.looped);
    main_assert_eq!(first.target => None);
    main_assert_eq!(first.volume => 79);
    let (frozen_volume, frozen_pan) = first.detached_mix.test_value();
    main_assert!((frozen_volume - 0.79).abs() < f32::EPSILON);
    main_assert!((frozen_pan - 0.7).abs() < f32::EPSILON);

    audio.handle_events(
        std::slice::from_ref(&event),
        &snapshot,
        &viewports,
        &mut runtime_music_enabled,
    );
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert_eq!(audio.active_channels[&key].channel => first.channel);
}

#[test]
fn sound_instance_lookup_matches_prepared_resolved_sample_names() {
    let dir = tempdir();
    let scenario = dir.path().join("Lookup.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000)).test_value();

    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.configure_scenario(Some(&scenario));
    let source = make_object(1, "FIRE", Vector2::new(100, 100));
    let snapshot = make_snapshot(vec![source.clone()], Vec::new());
    audio
        .start_sound(
            "Fire",
            Some(source.id),
            100,
            true,
            false,
            None,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, source.position)],
        )
        .test_value();
    let channel = audio
        .active_channels
        .values()
        .next()
        .and_then(|info| info.channel)
        .test_value();

    audio.stop_sound("Fire.wav", Some(source.id));

    main_assert!(audio.active_channels.is_empty());
    main_assert!(!audio.system.channel_is_playing(channel));
}

#[test]
fn wildcard_non_multiple_lookup_suppresses_before_reresolution() {
    let dir = tempdir();
    let first_scenario = dir.path().join("First.c4s");
    let second_scenario = dir.path().join("Second.c4s");
    fs::create_dir_all(&first_scenario).test_value();
    fs::create_dir_all(&second_scenario).test_value();
    fs::write(first_scenario.join("Blast1.wav"), silent_pcm_wav(10_000)).test_value();
    fs::write(second_scenario.join("Blast2.wav"), silent_pcm_wav(10_000)).test_value();

    let options = audio_fixture!(audio_max_channels: 2);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&first_scenario));
    let source = make_object(1, "BLST", Vector2::new(100, 100));
    let snapshot = make_snapshot(vec![source.clone()], Vec::new());
    main_assert!(audio
        .try_start_sound(
            "Blast1",
            Some(source.id),
            100,
            false,
            false,
            None,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, source.position)],
        )
        .expect("concrete blast starts"));

    audio.configure_scenario(Some(&second_scenario));
    main_assert!(
        !audio
            .try_start_sound(
                "Blast*",
                Some(source.id),
                100,
                false,
                false,
                None,
                &snapshot,
                &[audio_viewport(0, OWNER_NONE, source.position)],
            )
            .expect("wildcard lookup succeeds"),
        "the live Blast1 sample suppresses the request before Blast2 is resolved"
    );
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert_eq!(audio.active_channels.values().next().unwrap().sample_name => "blast1.wav");
}

#[test]
fn detached_one_shot_does_not_get_orphaned_by_an_identical_new_request() {
    let dir = tempdir();
    let scenario = dir.path().join("DetachCollision.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000)).test_value();

    let options = audio_fixture!(audio_max_channels: 2);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));
    let source = make_object(1, "FIRE", Vector2::new(100, 100));
    let snapshot = make_snapshot(vec![source.clone()], Vec::new());
    let start = |audio: &mut AudioContext| {
        audio.start_sound(
            "Fire",
            Some(source.id),
            100,
            false,
            false,
            None,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, source.position)],
        )
    };

    start(&mut audio).test_value();
    let first_channel = audio
        .active_channels
        .values()
        .next()
        .and_then(|info| info.channel)
        .test_value();
    audio.detach_object_sounds(
        source.id,
        source.position,
        &snapshot,
        &[audio_viewport(0, OWNER_NONE, source.position)],
    );
    start(&mut audio).test_value();

    main_assert_eq!(audio.active_channels.len() => 2);
    let second_channel = audio
        .active_channels
        .values()
        .find(|info| info.target == Some(source.id))
        .and_then(|info| info.channel)
        .test_value();
    main_assert!(audio.system.channel_is_playing(first_channel));
    main_assert!(audio.system.channel_is_playing(second_channel));

    audio.stop_sound("Fire.wav", Some(source.id));
    main_assert_eq!(audio.active_channels.len() => 1);
    main_assert!(audio.system.channel_is_playing(first_channel));
    main_assert!(!audio.system.channel_is_playing(second_channel));

    audio.stop_sound("Fir?", None);
    main_assert!(audio.active_channels.is_empty());
    main_assert!(!audio.system.channel_is_playing(first_channel));
}

#[test]
fn global_lookup_stops_the_oldest_detached_instance_of_one_sample() {
    let dir = tempdir();
    let scenario = dir.path().join("InstanceOrder.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000)).test_value();

    let options = audio_fixture!(audio_max_channels: 2);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));
    let first = make_object(1, "FIRE", Vector2::new(100, 100));
    let second = make_object(2, "FIRE", Vector2::new(300, 100));
    let snapshot = make_snapshot(vec![first.clone(), second.clone()], Vec::new());

    let mut detached_channels = Vec::new();
    for source in [&first, &second] {
        audio
            .start_sound(
                "Fire",
                Some(source.id),
                100,
                false,
                false,
                None,
                &snapshot,
                &[audio_viewport(0, OWNER_NONE, source.position)],
            )
            .test_value();
        let channel = audio
            .active_channels
            .values()
            .find(|info| info.target == Some(source.id))
            .and_then(|info| info.channel)
            .test_value();
        detached_channels.push(channel);
        audio.detach_object_sounds(
            source.id,
            source.position,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, source.position)],
        );
    }

    audio.stop_sound("Fire", None);

    main_assert!(!audio.system.channel_is_playing(detached_channels[0]));
    main_assert!(audio.system.channel_is_playing(detached_channels[1]));
    main_assert_eq!(audio.active_channels.len() => 1);
}

#[test]
fn wildcard_lookup_uses_sample_order_before_instance_order() {
    let dir = tempdir();
    let scenario = dir.path().join("SampleOrder.c4s");
    fs::create_dir_all(&scenario).test_value();
    for name in ["Tone1.wav", "Tone2.wav"] {
        fs::write(scenario.join(name), silent_pcm_wav(10_000)).test_value();
    }

    let options = audio_fixture!(audio_max_channels: 2);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));
    let source = make_object(1, "TONE", Vector2::new(100, 100));
    let snapshot = make_snapshot(vec![source.clone()], Vec::new());
    let (lower_name, higher_name) =
        if audio.resolver.sample_order("tone1.wav") < audio.resolver.sample_order("tone2.wav") {
            ("Tone1", "Tone2")
        } else {
            ("Tone2", "Tone1")
        };
    for name in [higher_name, lower_name] {
        audio
            .start_sound(
                name,
                Some(source.id),
                100,
                false,
                false,
                None,
                &snapshot,
                &[audio_viewport(0, OWNER_NONE, source.position)],
            )
            .test_value();
    }
    let lower_channel = audio
        .active_channels
        .values()
        .find(|info| info.sample_name == format!("{}.wav", lower_name.to_ascii_lowercase()))
        .and_then(|info| info.channel)
        .test_value();
    let higher_channel = audio
        .active_channels
        .values()
        .find(|info| info.sample_name == format!("{}.wav", higher_name.to_ascii_lowercase()))
        .and_then(|info| info.channel)
        .test_value();

    audio.stop_sound("Tone?", Some(source.id));

    main_assert!(!audio.system.channel_is_playing(lower_channel));
    main_assert!(audio.system.channel_is_playing(higher_channel));
    main_assert_eq!(audio.active_channels.len() => 1);
}

#[test]
fn object_removal_detach_stops_the_attached_loop_within_one_frame() {
    let dir = tempdir();
    let scenario = dir.path().join("SoundDetach.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000)).test_value();

    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.configure_scenario(Some(&scenario));
    let listener = make_object(1, "LIST", Vector2::new(1000, 1000));
    let source = make_object(2, "FIRE", Vector2::new(1350, 1000));
    let initial = make_snapshot(
        vec![listener.clone(), source.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    audio
        .start_sound(
            "Fire",
            Some(source.id),
            100,
            true,
            false,
            None,
            &initial,
            &[audio_viewport(0, OWNER_NONE, listener.position)],
        )
        .test_value();
    let key = SoundInstanceKey::new("Fire", Some(source.id));
    let channel = audio.active_channels[&key].channel.test_value();

    let mut removed = make_snapshot(
        vec![listener.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    removed.audio = vec![AudioCommand::DetachObjectSounds {
        target: source.id,
        position: source.position,
    }];
    let mut runtime_music_enabled = false;
    audio.process_audio_with_viewports(
        &removed,
        &[audio_viewport(0, OWNER_NONE, listener.position)],
        &mut runtime_music_enabled,
    );

    main_assert!(!audio.active_channels.contains_key(&key));
    main_assert!(!audio.system.channel_is_playing(channel));
}

#[test]
fn object_removal_detach_freezes_one_shot_at_last_positional_mix() {
    let dir = tempdir();
    let scenario = dir.path().join("SoundDetach.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Impact.wav"), silent_pcm_wav(10_000)).test_value();

    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.configure_scenario(Some(&scenario));
    let listener = make_object(1, "LIST", Vector2::new(1000, 1000));
    let source = make_object(2, "IMPT", Vector2::new(1350, 1000));
    let initial = make_snapshot(
        vec![listener.clone(), source.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    audio
        .start_sound(
            "Impact",
            Some(source.id),
            40,
            false,
            false,
            Some(1400),
            &initial,
            &[audio_viewport(0, OWNER_NONE, listener.position)],
        )
        .test_value();
    let key = SoundInstanceKey::new("Impact", Some(source.id));
    let channel = audio.active_channels[&key].channel.test_value();

    let mut removed = make_snapshot(
        vec![listener.clone()],
        vec![audio_fixture!(hud_player: vec![listener.id], Some(listener.id))],
    );
    removed.audio = vec![AudioCommand::DetachObjectSounds {
        target: source.id,
        position: source.position,
    }];
    let mut runtime_music_enabled = false;
    audio.process_audio_with_viewports(
        &removed,
        &[audio_viewport(0, OWNER_NONE, listener.position)],
        &mut runtime_music_enabled,
    );

    let info = &audio.active_channels[&key];
    main_assert_eq!(info.target => None);
    let (volume, pan) = info.detached_mix.test_value();
    let frozen_mix = (volume, pan);
    main_assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
    main_assert!((pan - 0.7).abs() < 1e-6, "pan={pan}");
    main_assert!(audio.system.channel_is_playing(channel));

    let moved_listener = make_object(1, "LIST", Vector2::new(2000, 1000));
    let moved = make_snapshot(vec![moved_listener.clone()], Vec::new());
    audio.update_channels(&moved, &[], true);
    main_assert_eq!(audio.active_channels[&key].detached_mix => Some(frozen_mix));
    main_assert!(!audio.try_start_sound("Impact", None, 100, false, true, None, &moved, &[],).expect("global near-dedup check succeeds"));

    audio.stop_sound("Impact", None);
    main_assert!(!audio.active_channels.contains_key(&key));
    main_assert!(!audio.system.channel_is_playing(channel));
}

#[test]
fn repeated_object_sound_reuses_the_cpp_instance_before_allocating_a_channel() {
    // FnSound returns before StartSoundEffect when the same wildcard is
    // already playing on the object (C4Script.cpp:2317-2319). This must
    // happen before SDL_mixer asks for another free channel.
    let dir = tempdir();
    let scenario = dir.path().join("Goldrush.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("HorseWalk1.wav"), silent_pcm_wav(1_000)).test_value();

    let options = audio_fixture!(audio_max_channels: 1);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));

    let horse = make_object(1, "HORS", Vector2::new(100, 100));
    let snapshot = make_snapshot(vec![horse.clone()], Vec::new());
    let play = |audio: &mut AudioContext| {
        audio.start_sound(
            "HorseWalk*",
            Some(horse.id),
            100,
            false,
            false,
            None,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, horse.position)],
        )
    };

    play(&mut audio).test_value();
    play(&mut audio).test_value();
}

#[test]
fn nearby_objects_share_the_cpp_sample_instance_even_when_multiple_is_requested() {
    // C4SoundSystem::NewInstance rejects another instance of the resolved
    // sample within NearSoundRadius=50, after FnSound's fMultiple check
    // (C4SoundSystem.cpp:341-350; C4SoundSystem.h:43).
    let dir = tempdir();
    let scenario = dir.path().join("Goldrush.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("HorseWalk1.wav"), silent_pcm_wav(1_000)).test_value();

    let options = audio_fixture!(audio_max_channels: 1);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));

    let left = make_object(1, "HORS", Vector2::new(100, 100));
    let right = make_object(2, "HORS", Vector2::new(149, 100));
    let snapshot = make_snapshot(vec![left.clone(), right.clone()], Vec::new());

    audio
        .start_sound(
            "HorseWalk*",
            Some(left.id),
            100,
            false,
            true,
            None,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, left.position)],
        )
        .test_value();
    audio
        .start_sound(
            "HorseWalk*",
            Some(right.id),
            100,
            false,
            true,
            None,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, left.position)],
        )
        .test_value();
}

#[test]
fn the_near_gate_survives_objects_removed_in_the_same_frame() {
    // clonk-org/clonk-rs#946: a dissipating force-field wall is SEVEN segments
    // (ForceFieldWall.c4d/Script.c spawns `i < 7`), stacked 20px apart, and each
    // one runs the shared `Destroy()` in ForceFieldAirShield.c4d/Script.c:68-72
    //
    //     private func Destroy() { Sound("DeEnergize"); RemoveObject(); }
    //
    // so all seven ask for the same sample and then remove themselves in the
    // same frame.
    //
    // C++ answers that with `NewInstance`'s "Already playing near?" gate, which
    // reads the position straight off the still-live `C4Object`
    // (`C4SoundSystem.cpp:341-348`, `IsNear` at :252-261) — `RemoveObject` has
    // not been processed yet when the script calls `Sound`. Segments within
    // NearSoundRadius=50 of a live instance are refused.
    //
    // The port applies the queued command against the snapshot the tick
    // produced, in which those objects are already gone. `zip` then yields None
    // and the gate reports "not near", so every segment gets its own instance
    // and the dissipation is as many times too loud as there are segments.
    let dir = tempdir();
    let scenario = dir.path().join("Goldrush.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("DeEnergize.wav"), silent_pcm_wav(1_000)).test_value();

    let options = audio_fixture!(audio_max_channels: 8);
    let mut audio = AudioContext::try_new(options).test_value();
    audio.configure_scenario(Some(&scenario));

    // Two of the seven, at the definition's 20px spacing — well inside 50.
    let upper = make_object(1, "FCWS", Vector2::new(100, 100));
    let lower = make_object(2, "FCWS", Vector2::new(100, 120));
    let alive = make_snapshot(vec![upper.clone(), lower.clone()], Vec::new());
    let viewports = [audio_viewport(0, OWNER_NONE, upper.position)];

    // Drive the real command path: the tick stamps each call with where its
    // object stood, which is the whole point of the fix.
    let segment_sound = |segment: &ObjectSnapshot| AudioCommand::PlaySound {
        name: "DeEnergize".to_string(),
        target: Some(segment.id),
        volume: 100,
        looped: false,
        multiple: false,
        custom_falloff: None,
        target_position: Some(segment.position),
    };

    let mut music_enabled = true;
    audio.handle_events(
        &[segment_sound(&upper)],
        &alive,
        &viewports,
        &mut music_enabled,
    );
    main_assert_eq!(audio.active_channels.len() => 1);

    // Both segments removed themselves this frame, so neither survives into the
    // snapshot the command is applied against.
    let removed = make_snapshot(Vec::new(), Vec::new());
    audio.handle_events(
        &[segment_sound(&lower)],
        &removed,
        &viewports,
        &mut music_enabled,
    );
    main_assert_eq!(
        audio.active_channels.len() => 1,
        "a segment 20px away is still near, whether or not it outlived the frame"
    );
}

#[test]
fn name_conflicts_use_cpp_raw_byte_case_folding() {
    let configured = LegacyCString::from_bytes(b"\xe4lpha".to_vec()).test_value();
    let active = [b"\xc4LPHA".as_slice()];
    let mut ranges = Vec::new();
    let selected = classic_script_player_name(&configured, &active, &mut |range| {
        ranges.push(range);
        0
    });

    main_assert_eq!(selected.as_bytes() => b"\xe4lpha");
    main_assert_eq!(ranges => vec![1]);
}

#[test]
fn random_color_wraps_cpp_channel_256_after_clamping() {
    let draws = [256, 301, 255];
    let mut draw = 0;
    let mut ranges = Vec::new();
    let color = classic_script_player_color(&mut |range| {
        ranges.push(range);
        let value = draws[draw];
        draw += 1;
        value
    });

    main_assert_eq!(color => 0x00ff_0000);
    main_assert_eq!(ranges => vec![302, 302, 302]);
}

#[test]
fn each_script_player_add_draws_three_fresh_random_color_channels() {
    let (mut app, mut commands) = script_player_add_fixture(b"Solo", &[], 2);
    let draws = [1, 2, 3, 4, 5, 6];
    let mut draw = 0;
    let mut ranges = Vec::new();
    {
        let mut next_random = |range| {
            ranges.push(range);
            let value = draws[draw];
            draw += 1;
            value
        };

        app.add_classic_lobby_script_player_with_random(&mut next_random);
        app.add_classic_lobby_script_player_with_random(&mut next_random);
    }

    let requests = commands.take_player_info_updates();
    main_assert_eq!(requests.len() => 2);
    main_assert_eq!(requests[0].players[0].color => 0x0003_0201);
    main_assert_eq!(requests[1].players[0].color => 0x0006_0504);
    main_assert_ne!(requests[0].players[0].color => 0x00f4_0000);
    main_assert_ne!(requests[1].players[0].color => 0x0000_c800);
    main_assert_eq!(ranges => vec![302; 6]);
}

#[test]
fn sandbox_music_is_decodable() {
    // Keep real installed-resource discovery and decoder coverage here;
    // the menu lifecycle regression uses an explicitly completed fixture
    // so thread scheduling cannot decide whether that state test passes.
    // Music discovery reads process env; hold the env lock so the
    // EnvGuard-based tests cannot redirect paths mid-load.
    let _lock = env_lock().lock();
    let audio = sandbox_music_bytes();
    let decoded = decode_audio(audio).test_value();
    main_assert_eq!(decoded.sample_rate => 44_100);
    main_assert!(decoded.frames.len() > 2_000);
}

#[test]
fn more_music_parser_ignores_comments_and_ascii_whitespace() {
    main_assert_eq!(
        parse_more_music(
            b" \r\n\t# comment with leading whitespace \r\n  #clear\t\r\n  Extra Music  \r\n"
        ) =>
        vec![
            MoreMusicDirective::Clear,
            MoreMusicDirective::Add(b"Extra Music".to_vec()),
        ]
    );
}

#[cfg(unix)]
#[test]
fn more_music_and_playlist_preserve_non_utf8_track_name_bytes() {
    const EXTRA_GROUP: &[u8] = b"Extra\xfe.c4g";
    const FIRST_TRACK: &[u8] = b"Tune\x80.ogg";
    const SECOND_TRACK: &[u8] = b"Tune\x81.ogg";

    let root = tempdir();
    let outer = root.path().join("Outer.c4g");
    let mut extra_group = MutableGroup::new_bytes(EXTRA_GROUP.to_vec());
    extra_group
        .add_file_bytes_with_metadata(FIRST_TRACK.to_vec(), b"first".to_vec(), 1, false)
        .test_value();
    extra_group
        .add_file_bytes_with_metadata(SECOND_TRACK.to_vec(), b"second".to_vec(), 1, false)
        .test_value();
    let mut outer_group = MutableGroup::new("Outer.c4g");
    outer_group
        .add_child_bytes(EXTRA_GROUP.to_vec(), extra_group)
        .test_value();
    fs::write(&outer, outer_group.pack().test_value()).test_value();

    let manifest = root.path().join("MoreMusic.txt");
    fs::write(
        &manifest,
        b"Outer.c4g/Extra\xfe.c4g/\0ignored-after-c-string\n".as_slice(),
    )
    .test_value();

    let mut catalog = MusicCatalog::empty();
    load_more_music(&mut catalog, &manifest).test_value();
    main_assert_eq!(catalog.assets.len() => 2);
    main_assert_eq!(
        catalog
            .resolve(&clonk_script::c4_string_from_bytes(FIRST_TRACK))
            .expect("resolve first raw basename")
            .load_audio()
            .expect("read first raw basename exactly") =>
        b"first"
    );
    main_assert_eq!(
        catalog
            .resolve(&clonk_script::c4_string_from_bytes(SECOND_TRACK))
            .expect("resolve second raw basename")
            .load_audio()
            .expect("read second raw basename exactly") =>
        b"second"
    );

    let mut first_full_path = outer.as_os_str().as_encoded_bytes().to_vec();
    first_full_path.push(std::path::MAIN_SEPARATOR as u8);
    first_full_path.extend_from_slice(EXTRA_GROUP);
    first_full_path.push(std::path::MAIN_SEPARATOR as u8);
    first_full_path.extend_from_slice(FIRST_TRACK);
    main_assert_eq!(
        catalog
            .resolve(&clonk_script::c4_string_from_bytes(&first_full_path))
            .expect("resolve raw full path")
            .file_name_bytes
            .as_slice() =>
        FIRST_TRACK
    );

    let mut resolver = MusicResolver::empty();
    resolver.global = catalog;
    resolver.set_playlist(Some(clonk_script::c4_string_from_bytes(b"Tune\x81.*")));
    main_assert_eq!(
        resolver
            .first_default()
            .map(|asset| asset.file_name_bytes.as_slice()) =>
        Some(SECOND_TRACK),
        "script playlist bytes distinguish names that share a lossy rendering"
    );
    main_assert!(music_playlist_matches(b"Tune?.ogg", FIRST_TRACK));
    main_assert!(music_playlist_matches(b"Tune?.ogg", SECOND_TRACK));
}

#[test]
fn more_music_directory_after_clear_replaces_global_catalog() {
    let root = tempdir();
    let global = root.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Default.ogg"), b"default").test_value();

    let extra = root.path().join("Extra Music");
    fs::create_dir_all(&extra).test_value();
    fs::write(extra.join("Added.ogg"), b"added").test_value();
    fs::write(extra.join("Effect.wav"), b"effect").test_value();
    fs::write(extra.join("Notes.txt"), b"notes").test_value();
    fs::write(root.path().join("Loose.mod"), b"loose").test_value();
    let manifest = root.path().join("MoreMusic.txt");
    fs::write(
        &manifest,
        b" \r\n\t# ignored comment \r\n  #clear\t\r\n  Extra Music  \r\nLoose.mod\r\nLoose.mod\r\n",
    )
    .test_value();

    let mut catalog = MusicCatalog::from_group(Group::open(&global).test_value()).test_value();
    load_more_music(&mut catalog, &manifest).test_value();

    main_assert_eq!(catalog.filenames() => ["Added.ogg", "Loose.mod", "Loose.mod"]);
    main_assert_eq!(catalog.resolve("Added.ogg").expect("MoreMusic directory track").load_audio().expect("read MoreMusic directory track") => b"added");
}

#[test]
fn more_music_mp3_wildcard_adds_only_matching_supported_files() {
    let root = tempdir();
    let global = root.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Default.ogg"), b"default").test_value();

    let extra = root.path().join("Extras");
    fs::create_dir_all(extra.join("Folder.mp3")).test_value();
    for name in [
        "Keep.mp3",
        "Upper.MP3",
        "Other.ogg",
        "LooksLike.mp3.bak",
        "Readme.txt",
    ] {
        fs::write(extra.join(name), name.as_bytes()).test_value();
    }
    fs::write(extra.join("Folder.mp3/Inside.mp3"), b"nested").test_value();
    let wildcard = PathBuf::from("Extras").join("*.mp3");
    let manifest = root.path().join("MoreMusic.txt");
    fs::write(&manifest, format!("{}\n", wildcard.display())).test_value();

    let mut catalog = MusicCatalog::from_group(Group::open(&global).test_value()).test_value();
    load_more_music(&mut catalog, &manifest).test_value();
    let mut filenames = catalog.filenames();
    filenames.sort_by_cached_key(|name| name.to_ascii_lowercase());

    main_assert_eq!(filenames => ["Default.ogg", "Keep.mp3", "Upper.MP3"]);
}

#[test]
fn music_catalog_resolves_exact_filename_and_cpp_stem_names() {
    // C4MusicSystem::FindSong tries exact filename, then the requested
    // stem plus every supported extension (C4MusicSystem.cpp:312-333).
    let dir = tempdir();
    let music = dir.path().join("Music.c4g");
    fs::create_dir_all(&music).test_value();
    fs::write(music.join("Frontend.ogg"), b"frontend").test_value();
    fs::write(music.join("Pizza Strings.ogg"), b"pizza").test_value();

    let group = Group::open(&music).test_value();
    let catalog = MusicCatalog::from_group(group).test_value();

    main_assert_eq!(catalog.resolve("Frontend.ogg").expect("exact filename").load_audio().expect("read exact filename") => b"frontend");
    main_assert_eq!(catalog.resolve("Frontend").expect("frontend stem").load_audio().expect("read frontend stem") => b"frontend");
    main_assert_eq!(catalog.resolve("Pizza Strings").expect("pizza stem").load_audio().expect("read pizza stem") => b"pizza");
}

#[test]
fn music_playlist_filter_uses_raw_semicolon_patterns_and_basename_matching() {
    main_assert!(music_playlist_matches(b"NoMatch;*.mId", b"Theme.MID"));
    main_assert!(music_playlist_matches(b"NoMatch;Ambient.*", b"Ambient.ogg"));
    main_assert!(!music_playlist_matches(b"NoMatch; Ambient.*", b"Ambient.ogg"), "C++ does not trim playlist sections");

    let dir = tempdir();
    let group = Group::open(dir.path()).test_value();
    let asset = MusicAsset::for_test_path(Arc::new(group), PathBuf::from("nested/Theme.MID"));
    let catalog = MusicCatalog {
        assets: vec![asset],
    };
    main_assert_eq!(
        catalog
            .first_enabled(Some("*.mid"))
            .map(|asset| asset.file_name_bytes.as_slice()) =>
        Some(b"Theme.MID".as_slice()),
        "playlist matching uses GetFilename rather than the full asset path"
    );
}

#[test]
fn music_playlist_explicit_filter_allows_frontend_and_default_excludes_special_tracks() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    for name in ["@Hidden.ogg", "Credits.ogg", "Frontend.ogg", "Theme.ogg"] {
        fs::write(global.join(name), name.as_bytes()).test_value();
    }

    let group = Group::open(&global).test_value();
    let mut resolver = MusicResolver::with_global_group(group).test_value();
    main_assert_eq!(resolver.first_default().map(|asset| asset.file_name_bytes.as_slice()) => Some(b"Theme.ogg".as_slice()));

    resolver.set_playlist(Some("Frontend.*".to_string()));
    main_assert_eq!(
        resolver
            .first_default()
            .map(|asset| asset.file_name_bytes.as_slice()) =>
        Some(b"Frontend.ogg".as_slice()),
        "an explicit playlist replaces the default exclusions"
    );

    resolver.set_playlist(None);
    main_assert_eq!(
        resolver
            .first_default()
            .map(|asset| asset.file_name_bytes.as_slice()) =>
        Some(b"Theme.ogg".as_slice()),
        "restoring the default playlist excludes frontend/credits/@ tracks again"
    );
}

#[test]
fn default_music_selection_uses_unsynced_choices_without_immediate_repeats() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    for name in ["A.ogg", "B.ogg", "C.ogg"] {
        fs::write(global.join(name), name.as_bytes()).test_value();
    }

    let resolver = MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    let engine = Engine::with_seed(0x1234_5678);
    let synced_rng_before = engine.snapshot().rng;
    let mut choices = VecDeque::from([0usize, 0, 1, 0]);
    let mut bounds = Vec::new();
    let mut recent = None;
    let mut selected_names = Vec::new();

    for _ in 0..4 {
        let selected = resolver
            .select_default_with(recent.as_ref(), |range| {
                bounds.push(range);
                choices.pop_front().expect("stubbed SafeRandom choice")
            })
            .test_value();
        selected_names.push(selected.file_name_bytes.clone());
        recent = Some(Arc::clone(&selected.identity));
    }

    main_assert_eq!(selected_names => [b"A.ogg".to_vec(), b"B.ogg".to_vec(), b"C.ogg".to_vec(), b"A.ogg".to_vec()]);
    main_assert_eq!(bounds => [3, 2, 2, 2]);
    main_assert!(selected_names.windows(2).all(|pair| pair[0] != pair[1]), "the most recently started track is excluded while alternatives exist");
    main_assert_eq!(engine.snapshot().rng => synced_rng_before, "music selection must not consume the engine's synchronized LCG");
}

#[test]
fn music_resolver_keeps_global_catalog_when_scenario_has_no_music_source() {
    // PlayScenarioMusic only clears the constructor-loaded global catalog
    // when it discovers at least one local music directory
    // (C4MusicSystem.cpp:139-165).
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Frontend.ogg"), b"frontend").test_value();
    fs::write(global.join("Pizza Strings.ogg"), b"pizza").test_value();
    let scenario = dir.path().join("Tutorial.c4f").join("Tutorial01.c4s");
    fs::create_dir_all(&scenario).test_value();

    let global = Group::open(&global).test_value();
    let mut resolver = MusicResolver::with_global_group(global).test_value();
    resolver.configure_scenario(Some(&scenario)).test_value();

    main_assert_eq!(resolver.resolve("Frontend").expect("global Frontend fallback").load_audio().expect("read global Frontend") => b"frontend");
    main_assert_eq!(resolver.resolve("Pizza Strings").expect("global Pizza Strings fallback").load_audio().expect("read global Pizza Strings") => b"pizza");
    main_assert_eq!(
        resolver
            .first_default()
            .expect("default global scenario track")
            .load_audio()
            .expect("read default global track") =>
        b"pizza",
        "Frontend is explicitly addressable but excluded from the default playlist"
    );
}

#[test]
fn music_resolver_replaces_global_catalog_when_parent_has_music_group() {
    // A discovered local music directory clears the global song list
    // before local tracks are loaded (C4MusicSystem.cpp:152-166).
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Frontend.ogg"), b"frontend").test_value();

    let folder = dir.path().join("Fantasy.c4f");
    let scenario = folder.join("Scenario.c4s");
    let local = folder.join("Music.c4g");
    fs::create_dir_all(&scenario).test_value();
    fs::create_dir_all(&local).test_value();
    fs::write(local.join("Local Theme.ogg"), b"local").test_value();

    let global = Group::open(&global).test_value();
    let mut resolver = MusicResolver::with_global_group(global).test_value();
    resolver.set_playlist(Some("Frontend.*".to_string()));
    resolver.configure_scenario(Some(&scenario)).test_value();

    main_assert!(resolver.resolve("Frontend").is_none(), "a local catalog replaces rather than extends global music");
    main_assert_eq!(resolver.resolve("Local Theme").expect("local theme").load_audio().expect("read local theme") => b"local");
    main_assert_eq!(
        resolver
            .first_default()
            .map(|asset| asset.file_name_bytes.as_slice()) =>
        Some(b"Local Theme.ogg".as_slice()),
        "loading a new scenario music catalog restores the default playlist"
    );
}

#[test]
fn definition_pack_music_is_enumerated_in_groupset_order_and_replaces_global_music() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Global.ogg"), b"global").test_value();

    let folder = dir.path().join("Fantasy.c4f");
    let scenario = folder.join("Scenario.c4s");
    let folder_music = folder.join("Music.c4g");
    fs::create_dir_all(&scenario).test_value();
    fs::create_dir_all(&folder_music).test_value();
    fs::write(folder_music.join("Shared.ogg"), b"folder").test_value();

    let first_pack = dir.path().join("First.c4d");
    let second_pack = dir.path().join("Second.c4d");
    let first_music = first_pack.join("Music.c4g");
    let second_music = second_pack.join("Music.c4g");
    fs::create_dir_all(&first_music).test_value();
    fs::create_dir_all(&second_music).test_value();
    fs::write(first_music.join("FirstOnly.ogg"), b"first only").test_value();
    fs::write(first_music.join("PackTie.ogg"), b"first").test_value();
    fs::write(first_music.join("Shared.ogg"), b"first shared").test_value();
    fs::write(second_music.join("SecondOnly.ogg"), b"second only").test_value();
    fs::write(second_music.join("PackTie.ogg"), b"second").test_value();

    let roots = [
        Group::open(&first_pack).test_value(),
        Group::open(&second_pack).test_value(),
    ];
    let mut resolver =
        MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    main_assert!(resolver.configure_scenario_with_definition_roots(Some(&scenario), &roots).expect("configure definition music"));

    main_assert!(resolver.resolve("Global").is_none());
    main_assert_eq!(resolver.resolve("Shared").expect("folder wins over definition roots").load_audio().expect("folder bytes") => b"folder");
    main_assert_eq!(
        resolver
            .resolve("PackTie")
            .expect("tied definition track")
            .load_audio()
            .expect("definition bytes") =>
        b"second",
        "FindGroup enumerates the later equal-priority definition root first"
    );
    main_assert!(resolver.resolve("FirstOnly").is_some());
    main_assert!(resolver.resolve("SecondOnly").is_some());
    main_assert!(
        !resolver
            .configure_scenario(Some(&scenario))
            .expect("same-path playback configure"),
        "the later path-only playback pass must retain definition-root music"
    );
    main_assert_eq!(resolver.resolve("PackTie").expect("definition catalog retained").load_audio().expect("retained definition bytes") => b"second");

    fs::write(second_music.join("Reloaded.ogg"), b"reloaded").test_value();
    resolver.set_playlist(Some("FirstOnly.*".to_string()));
    main_assert!(
        resolver
            .configure_scenario_with_definition_roots(Some(&scenario), &roots)
            .expect("resource-aware same-path reload"),
        "a real activation must rebuild even when path and roots are unchanged"
    );
    main_assert!(resolver.playlist.is_none());
    main_assert_eq!(resolver.resolve("Reloaded").expect("replacement track discovered").load_audio().expect("replacement bytes") => b"reloaded");
}

#[test]
fn extra_music_uses_later_activated_children_then_root_and_skips_inactive_children() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    let scenario = dir.path().join("Scenario.c4s");
    let first_pack = dir.path().join("First.c4d");
    let second_pack = dir.path().join("Second.c4d");
    let extra = dir.path().join("Extra.c4g");
    let extra_root_music = extra.join("Music.c4g");
    let first_music = extra.join("First.c4d/Music.c4g");
    let second_music = extra.join("Second.c4d/Music.c4g");
    let unused_music = extra.join("Unused.c4d/Music.c4g");
    for path in [
        &global,
        &scenario,
        &first_pack,
        &second_pack,
        &extra_root_music,
        &first_music,
        &second_music,
        &unused_music,
    ] {
        fs::create_dir_all(path).test_value();
    }
    fs::write(global.join("Global.ogg"), b"global").test_value();
    fs::write(extra_root_music.join("RootOnly.ogg"), b"root").test_value();
    fs::write(first_music.join("ChildTie.ogg"), b"first").test_value();
    fs::write(second_music.join("ChildTie.ogg"), b"second").test_value();
    fs::write(unused_music.join("Unused.ogg"), b"unused").test_value();

    let roots = [
        Group::open(&first_pack).test_value(),
        Group::open(&second_pack).test_value(),
    ];
    let mut resolver =
        MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    resolver.extra = Some(Group::open(&extra).test_value());
    resolver
        .configure_scenario_with_definition_roots(Some(&scenario), &roots)
        .test_value();

    main_assert!(resolver.resolve("Global").is_none());
    main_assert_eq!(
        resolver
            .resolve("ChildTie")
            .expect("activated child tie")
            .load_audio()
            .expect("second child bytes") =>
        b"second",
        "direct GroupSet iteration keeps the later activated Extra child first"
    );
    main_assert_eq!(resolver.resolve("RootOnly").expect("Extra root track").load_audio().expect("root bytes") => b"root");
    main_assert!(resolver.resolve("Unused").is_none());
}

#[test]
fn malformed_definition_pack_music_child_clears_global_without_aborting() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    let scenario = dir.path().join("Scenario.c4s");
    let definition = dir.path().join("Broken.c4d");
    fs::create_dir_all(&global).test_value();
    fs::create_dir_all(&scenario).test_value();
    fs::create_dir_all(&definition).test_value();
    fs::write(global.join("Global.ogg"), b"global").test_value();
    fs::write(definition.join("Music.c4g"), b"not a group").test_value();

    let mut resolver =
        MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    resolver
        .configure_scenario_with_definition_roots(
            Some(&scenario),
            &[Group::open(&definition).test_value()],
        )
        .test_value();

    main_assert!(resolver.resolve("Global").is_none());
    main_assert!(resolver.active_filenames().is_empty());
}

#[test]
fn malformed_scenario_music_child_clears_global_and_keeps_valid_root_tracks() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Global.ogg"), b"global").test_value();

    // LoadDir handles every path independently. A bad scenario child and
    // a bad inner .c4f child therefore cannot discard the scenario-root
    // track or the valid outer .c4f sibling that follows them.
    let outer = dir.path().join("Outer.c4f");
    let inner = outer.join("Inner.c4f");
    let scenario = inner.join("Scenario.c4s");
    let outer_music = outer.join("Music.c4g");
    fs::create_dir_all(&scenario).test_value();
    fs::create_dir_all(&outer_music).test_value();
    fs::write(scenario.join("Scenario Root.ogg"), b"scenario").test_value();
    fs::write(scenario.join("Music.c4g"), b"not a group").test_value();
    fs::write(inner.join("Music.c4g"), b"also not a group").test_value();
    fs::write(outer_music.join("Outer.ogg"), b"outer").test_value();

    let mut resolver =
        MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    resolver.configure_scenario(Some(&scenario)).test_value();

    main_assert!(resolver.resolve("Global").is_none());
    main_assert_eq!(resolver.active_filenames() => ["Scenario Root.ogg", "Outer.ogg"], "valid sources retain native scenario-to-parent order");
    main_assert_eq!(resolver.resolve("Scenario Root").expect("scenario-root track retained").load_audio().expect("scenario-root bytes") => b"scenario");
    main_assert_eq!(resolver.resolve("Outer").expect("outer parent track retained").load_audio().expect("outer parent bytes") => b"outer");

    // Presence, not successful opening, is the local-source signal for a
    // direct scenario child and for a registered .c4f parent child.
    let bad_scenario = dir.path().join("OnlyBroken.c4s");
    fs::create_dir_all(&bad_scenario).test_value();
    fs::write(bad_scenario.join("Music.c4g"), b"broken").test_value();
    resolver
        .configure_scenario(Some(&bad_scenario))
        .test_value();
    main_assert!(resolver.resolve("Global").is_none());
    main_assert!(resolver.active_filenames().is_empty());

    let bad_parent = dir.path().join("OnlyBroken.c4f");
    let child_scenario = bad_parent.join("Child.c4s");
    fs::create_dir_all(&child_scenario).test_value();
    fs::write(bad_parent.join("Music.c4g"), b"broken parent").test_value();
    resolver
        .configure_scenario(Some(&child_scenario))
        .test_value();
    main_assert!(resolver.resolve("Global").is_none());
    main_assert!(resolver.active_filenames().is_empty());
}

#[test]
fn local_music_source_enumeration_failure_is_isolated() {
    let dir = tempdir();
    let source_path = dir.path().join("Music.c4g");
    fs::create_dir_all(&source_path).test_value();
    fs::write(source_path.join("Track.ogg"), b"track").test_value();
    let source = Group::open(&source_path).test_value();
    fs::remove_dir_all(&source_path).test_value();
    main_assert!(source.entries().is_err(), "fixture must exercise lazy source-enumeration failure");

    let mut catalog = MusicCatalog::empty();
    extend_music_source(&mut catalog, source, "test source");
    main_assert!(catalog.filenames().is_empty());
}

#[test]
fn music_control_combines_config_and_scenario_volume() {
    // C4MusicSystem::UpdateVolume multiplies Config.Sound.MusicVolume by
    // Game.iMusicLevel only while a game is running
    // (C4MusicSystem.cpp:281-290).
    let mut control = MusicControlState::new(0.8);
    main_assert!((control.effective_volume() - 0.8).abs() < f32::EPSILON);

    control.set_scenario_level(Some(30));
    main_assert!((control.effective_volume() - 0.24).abs() < f32::EPSILON);

    control.set_scenario_level(Some(0));
    main_assert_eq!(control.effective_volume() => 0.0);

    control.set_configured_volume(0.5);
    control.set_scenario_level(Some(30));
    main_assert!((control.effective_volume() - 0.15).abs() < f32::EPSILON);

    control.set_scenario_level(None);
    main_assert!((control.effective_volume() - 0.5).abs() < f32::EPSILON);
}

#[test]
fn idle_music_fade_is_a_noop_and_does_not_suppress_a_later_play() {
    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    let initial_generation = lock_unpoisoned(&audio.music_control).generation;

    main_assert!(!audio.fade_out_music(GAME_MUSIC_FADE_OUT_MS));
    main_assert_eq!(lock_unpoisoned(&audio.music_control).generation => initial_generation, "an idle fade must not invalidate the next Play");

    audio.play_music(&silent_pcm_wav(1_000), true).test_value();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !audio.system.music_is_playing() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    main_assert!(audio.system.music_is_playing(), "a Play following an idle fade must still reach the mixer");
}

#[test]
fn script_stop_music_remains_an_immediate_halt() {
    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    let music = audio.system.load_music(&silent_pcm_wav(5_000)).test_value();
    audio.system.play_music(&music, true).test_value();
    let mut runtime_music_enabled = true;

    audio.handle_events(
        &[AudioCommand::StopMusic],
        &make_snapshot(Vec::new(), Vec::new()),
        &[],
        &mut runtime_music_enabled,
    );

    main_assert!(!runtime_music_enabled);
    main_assert!(!audio.system.music_is_playing());
}

#[test]
fn music_control_rejects_stale_decode_and_uses_latest_level() {
    // A replacement/stop invalidates an in-flight decode. A MusicLevel
    // call made while the replacement decodes must supply the volume used
    // when that generation finally starts.
    let mut control = MusicControlState::new(1.0);
    control.set_scenario_level(Some(100));
    let stale = control.advance_generation();
    let current = control.advance_generation();
    control.set_scenario_level(Some(30));

    main_assert_eq!(control.start_volume(stale) => None);
    main_assert_eq!(control.start_volume(current) => Some(0.3));
}

#[test]
fn missing_named_music_does_not_invalidate_current_generation() {
    // C4MusicSystem::Play returns before Stop when FindSong cannot resolve
    // the requested name (C4MusicSystem.cpp:65-97).
    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    let before = lock_unpoisoned(&audio.music_control).generation;

    main_assert!(!audio.play_named_music("__definitely_missing__", true).expect("missing lookup succeeds"));

    let after = lock_unpoisoned(&audio.music_control).generation;
    main_assert_eq!(after => before, "a miss must leave current playback intact");
}

#[test]
fn resolved_unreadable_music_stops_current_playback_before_failure() {
    for named in [true, false] {
        let dir = tempdir();
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).test_value();
        fs::write(global.join("Prior.ogg"), b"prior").test_value();
        fs::write(global.join("Gone.ogg"), b"gone").test_value();

        let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
        audio.music_resolver =
            MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
        let prior = Arc::clone(&audio.music_resolver.resolve("Prior").test_value().identity);
        lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&prior));
        audio.set_music_playlist(Some("Gone.*".to_string()));

        let current = audio.system.load_music(&silent_pcm_wav(5_000)).test_value();
        audio.system.play_music(&current, true).test_value();
        *lock_unpoisoned(&audio.pending_music) = Some(current);
        let before = lock_unpoisoned(&audio.music_control).generation;

        fs::remove_file(global.join("Gone.ogg")).test_value();
        let result = if named {
            audio.play_named_music("Gone", false)
        } else {
            audio.play_default_music(false)
        };
        main_assert!(result.is_err(), "the resolved {} replacement must reach its failed read", if named { "named" } else { "default" });
        main_assert!(!audio.system.music_is_playing());
        main_assert!(lock_unpoisoned(&audio.pending_music).is_none());
        main_assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire) => 0);
        main_assert!(audio.queued_music_starts.is_empty());
        main_assert_ne!(lock_unpoisoned(&audio.music_control).generation => before, "the resolved replacement invalidates the current generation");
        let recent = lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .clone()
            .test_value();
        main_assert!(Arc::ptr_eq(&recent, &prior));
    }
}

#[test]
fn deferred_unreadable_music_stops_predecessor_and_preserves_recent_marker() {
    audio_fixture!(music_pair: dir, global, audio);
    let a_identity = Arc::clone(&audio.music_resolver.resolve("A").test_value().identity);
    let fixture = audio.system.load_music(&silent_pcm_wav(5_000)).test_value();
    audio.control_music_loads_with(fixture);

    audio.play_named_music("A", false).test_value();
    audio.play_named_music("B", false).test_value();
    main_assert_eq!(audio.queued_music_starts.len() => 1);
    fs::remove_file(global.join("B.ogg")).test_value();

    main_assert!(audio.complete_next_controlled_music_load().expect("complete predecessor and pump replacement"));
    main_assert!(!audio.system.music_is_playing());
    main_assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire) => 0);
    main_assert!(audio.queued_music_starts.is_empty());
    main_assert!(audio.controlled_music_loads.as_ref().expect("controlled music loading").requests.is_empty());
    main_assert!(lock_unpoisoned(&audio.pending_music).is_none());
    let recent = lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .clone()
        .test_value();
    main_assert!(Arc::ptr_eq(&recent, &a_identity));
}

#[test]
fn back_to_back_music_commands_exclude_the_prior_selected_track() {
    audio_fixture!(music_pair: dir, global, audio);
    let a_identity = Arc::clone(&audio.music_resolver.resolve("A").test_value().identity);
    let b_identity = Arc::clone(&audio.music_resolver.resolve("B").test_value().identity);
    lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&a_identity));
    let fixture = audio.system.load_music(&silent_pcm_wav(20)).test_value();
    audio.control_music_loads_with(fixture);

    let mut runtime_music_enabled = false;
    audio.handle_events(
        &[
            AudioCommand::PlayMusic {
                name: String::new(),
                looped: false,
            },
            AudioCommand::PlayMusic {
                name: String::new(),
                looped: false,
            },
        ],
        &make_snapshot(Vec::new(), Vec::new()),
        &[],
        &mut runtime_music_enabled,
    );

    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    main_assert_eq!(audio.queued_music_starts.len() => 1);
    main_assert!(controlled.requests.front().and_then(|request| request.identity.as_ref()).is_some_and(|identity| Arc::ptr_eq(identity, &b_identity)));

    main_assert!(audio.complete_next_controlled_music_load().expect("complete first music start"));
    let first_started = lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .clone()
        .test_value();
    main_assert!(Arc::ptr_eq(&first_started, &b_identity));
    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    main_assert!(controlled.requests.front().and_then(|request| request.identity.as_ref()).is_some_and(|identity| Arc::ptr_eq(identity, &a_identity)));
    main_assert!(audio.queued_music_starts.is_empty());

    main_assert!(audio.complete_next_controlled_music_load().expect("complete second music start"));
    let second_started = lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .clone()
        .test_value();
    main_assert!(Arc::ptr_eq(&second_started, &a_identity));
}

#[test]
fn queued_default_selection_observes_a_failed_prior_start() {
    audio_fixture!(music_pair: dir, global, audio);
    let a_identity = Arc::clone(&audio.music_resolver.resolve("A").test_value().identity);
    let b_identity = Arc::clone(&audio.music_resolver.resolve("B").test_value().identity);
    lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&a_identity));
    let fixture = audio.system.load_music(&silent_pcm_wav(20)).test_value();
    audio.control_music_loads_with(fixture);

    audio.play_default_music(false).test_value();
    audio.play_default_music(false).test_value();
    main_assert!(!audio.fail_next_controlled_music_load().expect("fail first music start"));

    let recent = lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .clone()
        .test_value();
    main_assert!(Arc::ptr_eq(&recent, &a_identity));
    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    main_assert!(controlled.requests.front().and_then(|request| request.identity.as_ref()).is_some_and(|identity| Arc::ptr_eq(identity, &b_identity)));
}

#[test]
fn stop_music_cancels_deferred_starts_and_rejects_the_stale_worker() {
    audio_fixture!(music_pair: dir, global, audio);
    let prior = Arc::clone(&audio.music_resolver.resolve("A").test_value().identity);
    lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&prior));
    let fixture = audio.system.load_music(&silent_pcm_wav(20)).test_value();
    audio.control_music_loads_with(fixture);

    audio.play_default_music(false).test_value();
    audio.play_default_music(false).test_value();
    audio.stop_music();
    main_assert!(audio.queued_music_starts.is_empty());
    main_assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire) => 0);
    main_assert!(!audio.complete_next_controlled_music_load().expect("complete stale worker"));
    main_assert!(audio.controlled_music_loads.as_ref().expect("controlled music loading").requests.is_empty());
    let recent = lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .clone()
        .test_value();
    main_assert!(Arc::ptr_eq(&recent, &prior));
    main_assert!(!audio.system.music_is_playing());
}

#[test]
fn most_recent_music_changes_only_after_a_successful_start() {
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    fs::create_dir_all(&global).test_value();
    fs::write(global.join("Good.ogg"), silent_pcm_wav(100)).test_value();
    fs::write(global.join("Broken.ogg"), b"not audio").test_value();

    let mut audio = AudioContext::try_new(AudioOptions::default()).test_value();
    audio.music_resolver =
        MusicResolver::with_global_group(Group::open(&global).test_value()).test_value();
    let good_identity = Arc::clone(&audio.music_resolver.resolve("Good").test_value().identity);
    audio.set_music_playlist(Some("Broken.*".to_string()));
    audio.play_named_music("Good", false).test_value();

    let deadline = Instant::now() + Duration::from_secs(2);
    while lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .is_none()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    let started = lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .clone()
        .test_value();
    main_assert!(Arc::ptr_eq(&started, &good_identity));

    audio.play_named_music("Broken", false).test_value();
    let deadline = Instant::now() + Duration::from_secs(2);
    while audio.music_load_pending.load(AtomicOrdering::Acquire) != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    main_assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire) => 0, "malformed decode worker completed before the assertion");
    let after_failure = lock_unpoisoned(&audio.music_control)
        .most_recently_played
        .clone()
        .test_value();
    main_assert!(Arc::ptr_eq(&after_failure, &started), "a decode failure preserves the last successfully started asset");
}

#[test]
fn scenario_music_does_not_promote_nested_wav_effects() {
    // C4MusicSystem::PlayScenarioMusic only scans supported music files at
    // the scenario root and Music.c4g groups (C4MusicSystem.cpp:139-163).
    // In particular, Drachenfels' Princess.c4d/PrincessScream.wav is an
    // object sound effect, never scenario music.
    let dir = tempdir();
    let scenario = dir.path().join("Drachenfels.c4s");
    let princess = scenario.join("Princess.c4d");
    fs::create_dir_all(&princess).test_value();
    fs::write(princess.join("PrincessScream.wav"), b"scream").test_value();

    main_assert_eq!(load_scenario_music_bytes(&scenario).expect("inspect scenario music") => None);
}

#[test]
fn scenario_music_excludes_root_wav_effects() {
    // WAV is deliberately absent from C++ MusicFileExtensions
    // (C4MusicSystem.cpp:31-32), even when it sits at scenario root.
    let dir = tempdir();
    let scenario = dir.path().join("Effects.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Ambient.wav"), b"effect").test_value();

    main_assert_eq!(load_scenario_music_bytes(&scenario).expect("inspect scenario music") => None);
}

#[test]
fn scenario_music_accepts_supported_root_track_case_insensitively() {
    let dir = tempdir();
    let scenario = dir.path().join("Scenario.c4s");
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Local.OGG"), b"scenario track").test_value();

    main_assert_eq!(load_scenario_music_bytes(&scenario).expect("inspect scenario music") => Some(b"scenario track".to_vec()));
}

#[test]
fn scenario_music_uses_parent_music_group_when_root_has_none() {
    let dir = tempdir();
    let folder = dir.path().join("Fantasy.c4f");
    let scenario = folder.join("Drachenfels.c4s");
    let music = folder.join("Music.c4g");
    fs::create_dir_all(&scenario).test_value();
    fs::create_dir_all(&music).test_value();
    fs::write(music.join("Knightly Wonders.mid"), b"shared track").test_value();

    main_assert_eq!(load_scenario_music_bytes(&scenario).expect("inspect scenario music") => Some(b"shared track".to_vec()));
}

#[test]
fn scenario_music_uses_direct_music_group_without_scanning_definitions() {
    let dir = tempdir();
    let scenario = dir.path().join("Scenario.c4s");
    let music = scenario.join("Music.c4g");
    let definition = scenario.join("Actor.c4d");
    fs::create_dir_all(&music).test_value();
    fs::create_dir_all(&definition).test_value();
    fs::write(music.join("Theme.mp3"), b"scenario music").test_value();
    fs::write(definition.join("Scream.wav"), b"effect").test_value();

    main_assert_eq!(load_scenario_music_bytes(&scenario).expect("inspect scenario music") => Some(b"scenario music".to_vec()));
}

#[test]
fn dragon_rock_selects_fantasy_music_never_princess_scream() {
    let repository = test_repository_root().to_path_buf();
    let fantasy = repository.join("content/Fantasy.c4f");
    let scenario = fantasy.join("Drachenfels.c4s");
    main_assert!(scenario.is_dir(), "the initialized official content submodule must provide {}", scenario.display());

    let selected = load_scenario_music_bytes(&scenario)
        .expect("inspect Dragon Rock music")
        .test_value();
    let scream = fs::read(scenario.join("Princess.c4d/PrincessScream.wav")).test_value();
    let music_group = fantasy.join("Music.c4g");
    let expected_tracks: Vec<_> = [
        "Knightly Wonders.mid",
        "Medieval Waltz.mid",
        "Morning Dawn.mid",
    ]
    .into_iter()
    .map(|name| fs::read(music_group.join(name)).test_value())
    .collect();

    main_assert_ne!(selected => scream, "sound effect was promoted to music");
    main_assert!(expected_tracks.contains(&selected), "Dragon Rock did not select from Fantasy.c4f/Music.c4g");
}

#[test]
fn about_scrollbar_sounds_and_repeat_run_through_production_paths() {
    let mut app = new_real_classic_menu_app(320, 240);
    enter_about_licenses(&mut app);
    app.ui_sound_log.clear();

    let layout = clonk_frontend::startup_about_dlg::about_layout(320, 240);
    let text = layout.licenses.text;
    let bar = clonk_frontend::classic_gui::IntRect::new(
        text.x + text.w - 5 - 16,
        text.y + 8,
        16,
        text.h - 16,
    );
    let track = PhysicalPosition::new(f64::from(bar.x + 8), f64::from(bar.y + bar.h / 2));
    app.test_cursor(track);
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(app.ui_sound_log => vec!["Command"]);
    main_assert!(app.startup_about_dialog.as_ref().is_some_and(|dialog| dialog.license_scroll_offset() > 0));
    app.test_left_button(ElementState::Released);

    app.ui_sound_log.clear();
    let bottom_arrow = PhysicalPosition::new(f64::from(bar.x + 8), f64::from(bar.y + bar.h - 1));
    app.test_cursor(bottom_arrow);
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(app.ui_sound_log => vec!["ArrowHit"]);
    let before_frame = app
        .startup_about_dialog
        .as_ref()
        .test_value()
        .license_scroll_offset();
    let mut frame = vec![0_u8; 320 * 240 * 4];
    app.test_render(&mut frame);
    let after_frame = app
        .startup_about_dialog
        .as_ref()
        .test_value()
        .license_scroll_offset();
    main_assert!(after_frame > before_frame);

    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.ui_sound_log => vec!["ArrowHit", "ArrowHit"]);
    app.test_render(&mut frame);
    main_assert_eq!(app.startup_about_dialog.as_ref().unwrap().license_scroll_offset() => after_frame);
}

#[test]
fn modal_and_definition_overlays_restore_the_base_frame_when_closed() {
    let mut app = new_real_classic_menu_app(640, 480);
    let mut base = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut base);

    app.push_message_dialog(
        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
            "Message",
            "Caption",
            clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
        ),
        MessageDialogContinuation::None,
    )
    .test_value();
    let mut modal = vec![0_u8; 640 * 480 * 4];
    main_assert!(app.render(&mut modal).expect("render message overlay"));
    main_assert_ne!(modal => base);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    let mut closed = vec![0x77; 640 * 480 * 4];
    main_assert!(app.render(&mut closed).expect("render base after modal"));
    main_assert_eq!(closed => base);

    app.open_definition_selector(FrontendScenario::fallback())
        .test_value();
    let mut selector = vec![0_u8; 640 * 480 * 4];
    main_assert!(app.render(&mut selector).expect("render definition selector"));
    main_assert_ne!(selector => base);
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Cancelled,
    ])
    .test_value();
    let mut closed = vec![0x88; 640 * 480 * 4];
    main_assert!(app.render(&mut closed).expect("render base after selector"));
    main_assert_eq!(closed => base);
}

#[test]
fn options_sound_sheet_seeds_from_live_audio_and_applies_typed_actions() {
    use clonk_frontend::startup_options_dlg::{
        OptionsDlgAction, SoundCheckboxId, SoundSheetAction, SoundSheetState, SoundVolumeId,
    };

    let mut app = new_running_sandbox_app();
    app.return_to_menu();
    {
        let mut audio = app.test_audio_mut();
        audio.options.menu_music_enabled = true;
        audio.options.menu_sound_enabled = false;
        audio.options.music_enabled = true;
        audio.options.sound_enabled = false;
        audio.set_music_volume_percent(83);
        audio.set_sound_volume_percent(27);
        // The port-only voice row seeds from the same live options
        // (clonk-org/clonk-rs#452).
        audio.options.voice_enabled = true;
        audio.options.voice_volume = 1.42;
        audio.options.voice_push_to_talk = VirtualKeyCode::KeyT;
    }
    app.open_options_menu();
    main_assert_eq!(
        app.startup_options_dialog
            .as_ref()
            .expect("options dialog")
            .sound() =>
        &SoundSheetState::new(true, false, true, false, 83, 27).with_voice(true, 142, "T".into())
    );

    app.process_options_dialog_actions(vec![OptionsDlgAction::Sound(
        audio_fixture!(checkbox: SoundCheckboxId::FrontendMusic, false),
    )])
    .test_value();
    app.process_options_dialog_actions(vec![
        OptionsDlgAction::Sound(
            audio_fixture!(checkbox: SoundCheckboxId::FrontendSoundEffects, true),
        ),
        OptionsDlgAction::Sound(audio_fixture!(checkbox: SoundCheckboxId::GameMusic, false)),
        OptionsDlgAction::Sound(audio_fixture!(checkbox: SoundCheckboxId::GameSoundEffects, true)),
        OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
            id: SoundVolumeId::Music,
            value: 25,
        }),
        OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
            id: SoundVolumeId::SoundEffects,
            value: 75,
        }),
        OptionsDlgAction::Sound(audio_fixture!(checkbox: SoundCheckboxId::VoiceEnabled, false)),
        OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
            id: SoundVolumeId::Voice,
            value: 160,
        }),
    ])
    .test_value();

    let audio = app.test_audio_ref();
    main_assert!(!audio.options.menu_music_enabled);
    main_assert!(audio.options.menu_sound_enabled);
    main_assert!(!audio.options.music_enabled);
    main_assert!(audio.options.sound_enabled);
    main_assert_eq!(audio.options.music_volume_percent() => 25);
    main_assert_eq!(audio.options.sound_volume_percent() => 75);
    main_assert!(!audio.options.voice_enabled);
    main_assert_eq!(audio.options.voice_volume_percent() => 160);
    main_assert_eq!(lock_unpoisoned(&audio.music_control).effective_volume() => 0.25, "the live/pending music controller must update with the slider");
}

#[test]
fn options_sound_modifier_tabs_and_raw_gamepad_buttons_keep_classic_ownership() {
    use clonk_frontend::startup_options_dlg::SoundCheckboxId;

    let mut keyboard = new_running_sandbox_app();
    keyboard.return_to_menu();
    enter_unported_startup_subscreen(
        &mut keyboard,
        ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
    );
    keyboard.test_modifiers(ModifiersState::empty());
    keyboard.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(keyboard.startup_options_dialog.as_ref().expect("options dialog").focused_sound_checkbox() => Some(SoundCheckboxId::FrontendMusic));
    keyboard.test_key(VirtualKeyCode::Tab, ElementState::Released);
    keyboard.test_modifiers(ModifiersState::SHIFT);
    keyboard.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(keyboard.startup_options_dialog.as_ref().expect("options dialog").focused_sound_checkbox() => None);

    keyboard.test_modifiers(ModifiersState::empty());
    keyboard.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    keyboard.test_modifiers(ModifiersState::ALT);
    keyboard.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(
        keyboard
            .startup_options_dialog
            .as_ref()
            .expect("options dialog")
            .focused_sound_checkbox() =>
        Some(SoundCheckboxId::FrontendMusic),
        "modifier-blind fallback must not invent a plain Tab"
    );
    keyboard.test_modifiers(ModifiersState::CONTROL | ModifiersState::SHIFT);
    keyboard.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert_eq!(keyboard.startup_options_dialog.as_ref().expect("options dialog").active_sheet() => clonk_frontend::startup_options_dlg::OptionsSheet::Graphics);

    let mut gamepad = new_running_sandbox_app();
    gamepad.return_to_menu();
    enter_unported_startup_subscreen(
        &mut gamepad,
        ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
    );
    gamepad.test_gamepad_events([
        gamepad_direction_event(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        ),
        gamepad_direction_event(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        ),
    ]);
    main_assert_eq!(gamepad.startup_options_dialog.as_ref().expect("options dialog").focused_sound_checkbox() => Some(SoundCheckboxId::FrontendSoundEffects));
    main_assert!(!gamepad.test_audio_ref().options.menu_sound_enabled);
    gamepad.test_gamepad_events([
        gamepad_gui_button_event(
            GamepadSlot::new(0),
            GuiButtonClass::Low,
            ElementState::Pressed,
        ),
        gamepad_action_event(
            GamepadSlot::new(0),
            GamepadActionType::Select,
            ElementState::Pressed,
        ),
    ]);
    main_assert!(gamepad.test_audio_ref().options.menu_sound_enabled);

    let mut back = new_running_sandbox_app();
    back.return_to_menu();
    enter_unported_startup_subscreen(
        &mut back,
        ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
    );
    back.test_gamepad_events([gamepad_direction_event(
        GamepadSlot::new(0),
        ControlButton::Left,
        ElementState::Pressed,
    )]);
    back.test_gamepad_events([
        gamepad_gui_button_event(
            GamepadSlot::new(0),
            GuiButtonClass::Low,
            ElementState::Pressed,
        ),
        gamepad_action_event(
            GamepadSlot::new(0),
            GamepadActionType::Select,
            ElementState::Pressed,
        ),
        gamepad_gui_button_event(
            GamepadSlot::new(0),
            GuiButtonClass::Low,
            ElementState::Released,
        ),
        gamepad_action_event(
            GamepadSlot::new(0),
            GamepadActionType::Select,
            ElementState::Released,
        ),
    ]);
    main_assert_eq!(back.startup_view => StartupView::MainMenu);
}

#[test]
fn options_sound_held_arrow_advances_before_each_rendered_frame() {
    let mut app = new_classic_running_sandbox_app();
    app.return_to_menu();
    app.resize(800, 600).test_value();
    app.test_audio_mut().set_music_volume_percent(50);
    enter_unported_startup_subscreen(
        &mut app,
        ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
    );
    let slider = {
        let gui = app.assets.clonk_fonts.as_deref().test_value();
        let book = app.assets.options_book_fonts.as_deref().test_value();
        clonk_frontend::startup_options_dlg::options_dlg_layout(800, 600, gui, book)
            .sound
            .slider(clonk_frontend::startup_options_dlg::SoundVolumeId::Music)
    };
    let decrement =
        PhysicalPosition::new(f64::from(slider.x + 2), f64::from(slider.y + slider.h / 2));
    app.test_cursor(decrement);
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(app.test_audio_ref().options.music_volume_percent() => 50, "the arrow changes during DrawElement, not on pointer-down");

    let mut frame = vec![0_u8; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(app.test_audio_ref().options.music_volume_percent() < 50, "advance_frame must apply the slider callback before pixels");
    app.test_left_button(ElementState::Released);
}

#[test]
fn ingame_selection_frame_tracks_cpp_button_drag_lifecycle() {
    // Both mouse buttons enter C4MC_Drag_Selecting only after one axis
    // exceeds C4MC_DragSensitivity. Draw then anchors DownX/DownY in
    // world space, uses the current clamped viewport cursor as the other
    // endpoint, and ButtonUp removes the frame immediately
    // (C4MouseControl.cpp:203-316,406-414,893-980,1009-1037,1158-1170).
    let mut app = new_running_sandbox_app();
    let owner = app.local_owner;
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    let viewport = app.graphics.viewport_rect(owner).test_value();
    let (start, end) = (viewport.y + 12..viewport.y + viewport.height as i32 - 32)
        .step_by(4)
        .flat_map(|y| {
            (viewport.x + 12..viewport.x + viewport.width as i32 - 36)
                .step_by(4)
                .map(move |x| {
                    (
                        GuiPoint::new(x as f32, y as f32),
                        GuiPoint::new((x + 24) as f32, (y + 16) as f32),
                    )
                })
        })
        .find(|(start, end)| {
            let Some(first) = app.graphics.viewport_point_at(*start) else {
                return false;
            };
            let Some(second) = app.graphics.viewport_point_at(*end) else {
                return false;
            };
            first.owner == owner
                && second.owner == owner
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *start)
                    .is_none()
                && app
                    .graphics
                    .object_at_point(&app.snapshot, owner, *end)
                    .is_none()
                && app
                    .engine
                    .mouse_drag_crew_in_rect(
                        owner,
                        ingame_pointer_world_pixel(first),
                        ingame_pointer_world_pixel(second),
                    )
                    .is_empty()
                && app
                    .engine
                    .mouse_drag_carryables_in_rect(
                        ingame_pointer_world_pixel(first),
                        ingame_pointer_world_pixel(second),
                    )
                    .is_empty()
        })
        .test_value();

    app.test_cursor(PhysicalPosition::new(
        f64::from(start.x),
        f64::from(start.y),
    ));
    app.test_right_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(
        f64::from(start.x + 3.0),
        f64::from(start.y + 3.0),
    ));
    main_assert!(!app.ingame_right_mouse_state.expect("right-down remains live").motion.moved, "five logical pixels or less must not start C4MC_Drag_Selecting");
    app.test_render(&mut frame);

    app.test_cursor(PhysicalPosition::new(f64::from(end.x), f64::from(end.y)));
    let drag = app.ingame_right_mouse_state.test_value();
    main_assert!(drag.motion.moved);
    let down_world = ingame_pointer_world_pixel(drag.motion.start);
    app.test_render(&mut frame);
    let (down_x, _) = app.graphics.world_to_screen(owner, down_world).test_value();
    let current_x = end.x.round() as i32;
    let sample_x = (current_x + down_x.round() as i32) / 2;
    let sample_y = end.y.round() as i32;
    let expected = clonk_frontend::gamma_encode_fragment(
        clonk_frontend::MOUSE_SELECTION_FRAME_COLOR,
        &app.graphics
            .active_gamma_ramp(&app.snapshot.environment.gamma),
    );
    main_assert_eq!(
        app.graphics
            .surface()
            .get_pixel(sample_x as u32, sample_y as u32) =>
        Some(expected),
        "active C4MC_Drag_Selecting draws CRed above the viewport overlay"
    );

    app.test_right_button(ElementState::Released);
    main_assert!(app.ingame_right_mouse_state.is_none());
    app.test_render(&mut frame);
    main_assert_ne!(app.graphics.surface().get_pixel(sample_x as u32, sample_y as u32) => Some(expected), "ButtonUpDragSelecting removes the presentation frame");

    app.test_cursor(PhysicalPosition::new(
        f64::from(start.x),
        f64::from(start.y),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(f64::from(end.x), f64::from(end.y)));
    main_assert!(app.mouse_state.expect("left selection drag remains live").motion.selection_frame, "left and right landscape drags share C4MC_Drag_Selecting");
    app.test_render(&mut frame);
    let left_down_world = ingame_pointer_world_pixel(app.mouse_state.test_value().motion.start);
    let (left_down_x, _) = app
        .graphics
        .world_to_screen(owner, left_down_world)
        .test_value();
    let left_sample_x = (current_x + left_down_x.round() as i32) / 2;
    main_assert_eq!(
        app.graphics
            .surface()
            .get_pixel(left_sample_x as u32, sample_y as u32) =>
        Some(expected),
        "left C4MC_Drag_Selecting uses the same frame renderer"
    );
    app.test_left_button(ElementState::Released);
    main_assert!(app.mouse_state.is_none());
}

#[test]
fn ingame_options_sound_and_music_toggles_persist_to_config_file() {
    // Keep the process-global environment lock around only the tiny
    // isolated config writes; this state-only running fixture needs no
    // installed resources or user-data discovery.
    let mut app = new_state_only_lightweight_running_sandbox_app();
    {
        let mut audio = app.test_audio_mut();
        audio.options.sound_enabled = true;
        audio.options.music_enabled = true;
    }
    app.runtime_music_enabled = true;

    let user_data = tempdir();
    let repository = test_repository_root();
    let (_guard, paths) = guarded_test_app_paths(Some(repository), user_data.path());
    persist_config_value(&paths, "Sound", "Sound", "true").test_value();
    persist_config_value(&paths, "Sound", "Music", "true").test_value();
    persist_config_value(&paths, "Sound", "VendorExtension", "keep-me").test_value();
    app.app_paths = Some(paths.clone());

    app.apply_ingame_menu_action(MenuAction::ActivateOptions)
        .test_value();
    app.apply_ingame_menu_action(MenuAction::ToggleSound)
        .test_value();
    // `C4SoundSystem::ToggleOnOff` flips the flag in memory alone
    // (C4SoundSystem.cpp:138-142); the file is written when the Options
    // dialog closes or at clean shutdown. This test's subject is the write
    // *content*, so it flushes explicitly — the deferral itself is pinned by
    // `runtime_config_mutations_remain_process_local_until_shutdown_save`.
    app.flush_deferred_config();
    let after_sound = Config::load(paths.config_file()).test_value();
    main_assert_eq!(after_sound.get_in(Some("Sound"), "Sound") => Some("false"));
    main_assert_eq!(after_sound.get_in(Some("Sound"), "Music") => Some("true"), "the Sound action must not wait for or rewrite the Music action");

    app.apply_ingame_menu_action(MenuAction::ToggleMusic)
        .test_value();
    app.flush_deferred_config();
    let after_music = Config::load(paths.config_file()).test_value();
    main_assert_eq!(after_music.get_in(Some("Sound"), "Sound") => Some("false"));
    main_assert_eq!(after_music.get_in(Some("Sound"), "Music") => Some("false"));
    main_assert_eq!(after_music.get_in(Some("Sound"), "VendorExtension") => Some("keep-me"), "eager running toggles preserve unrelated classic config keys");

    let reloaded = AudioOptions::load(Some(&paths));
    main_assert!(!reloaded.sound_enabled, "next launch reloads RXSound off");
    main_assert!(!reloaded.music_enabled, "next launch reloads RXMusic off");
    main_assert_eq!(
        app.ingame_menu.as_ref().map(IngameMenuState::page) =>
        Some(ingame_menu::MenuPage::Options),
        "each native toggle reopens Options at the existing page"
    );
}

#[test]
fn music_toggle_tracks_actual_and_script_playback_and_missing_audio_fails_typed() {
    let mut ended = new_running_sandbox_app();
    let configured = ended.test_audio_ref().options.music_enabled;
    ended.runtime_music_enabled = true;
    ended.test_audio_mut().stop_music();
    let resources = ended.runtime_flash_resources().test_value().clone();
    ended.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(ended.runtime_music_enabled);
    main_assert_eq!(ended.test_audio_ref().options.music_enabled => configured);
    main_assert_eq!(ended.runtime_flash_message.as_ref().expect("On flash").text => resources.music_on_off(true));

    let mut scripted = new_running_sandbox_app();
    scripted.snapshot.audio = vec![AudioCommand::StopMusic];
    scripted.runtime_music_enabled = true;
    scripted.update_audio();
    main_assert!(!scripted.runtime_music_enabled);
    scripted.snapshot.audio = vec![AudioCommand::PlayMusic {
        name: "missing-script-track.ogg".to_string(),
        looped: false,
    }];
    scripted.update_audio();
    main_assert!(scripted.runtime_music_enabled);
    main_assert!(scripted.test_audio_ref().music_is_playing(), "MusicSystem::Execute analogue starts a replacement while enabled");
    scripted.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    main_assert!(!scripted.runtime_music_enabled);

    for modifiers in [ModifiersState::empty(), ModifiersState::CONTROL] {
        let mut missing = new_running_sandbox_app();
        missing.audio = None;
        missing.test_modifiers(modifiers);
        let error = missing
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect_err("missing audio must not fabricate a toggle");
        main_assert!(error.to_string().contains("classic audio system is unavailable"));
        main_assert!(missing.runtime_flash_message.is_none());
    }
    let mut startup_missing = new_running_sandbox_app();
    startup_missing.return_to_menu();
    startup_missing.audio = None;
    let error = startup_missing
        .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
        .expect_err("startup missing audio must fail typed");
    main_assert!(error.to_string().contains("classic audio system is unavailable"));
    main_assert!(startup_missing.runtime_flash_message.is_none());
}

#[test]
fn stale_music_worker_cannot_clear_successor_pending_generation() {
    let pending = Arc::new(AtomicU64::new(2));
    drop(PendingMusicLoadGuard(Arc::clone(&pending), 1));
    main_assert_eq!(pending.load(AtomicOrdering::Acquire) => 2);
    drop(PendingMusicLoadGuard(Arc::clone(&pending), 2));
    main_assert_eq!(pending.load(AtomicOrdering::Acquire) => 0);
}

#[test]
fn runtime_music_flash_reaches_every_nonexclusive_running_layer() {
    let assert_f3_renders = |app: &mut GameApp, layer: &str| {
        app.handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .unwrap_or_else(|error| panic!("F3 over {layer}: {error}"));
        let before = app
            .runtime_flash_message
            .as_ref()
            .unwrap_or_else(|| panic!("missing flash over {layer}"))
            .remaining_draws;
        let mut frame = vec![0_u8; 320 * 200 * 4];
        app.render(&mut frame)
            .unwrap_or_else(|error| panic!("render F3 over {layer}: {error:#}"));
        main_assert_eq!(app.runtime_flash_message.as_ref().expect("music text lasts more than one draw").remaining_draws => before - 1, "layer {layer}");
    };

    let mut message = new_running_sandbox_app();
    message
        .push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Music",
                "Remain open",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .test_value();
    assert_f3_renders(&mut message, "message dialog");
    main_assert_eq!(message.message_dialogs.len() => 1);

    let mut context = new_running_sandbox_app();
    context
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(24.0, 24.0),
        )
        .test_value();
    assert_f3_renders(&mut context, "context menu");
    main_assert!(context.context_menu.is_some());

    let mut scoreboard = new_scoreboard_test_app(
        r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
            }"#,
    );
    toggle_scoreboard(&mut scoreboard, ModifiersState::empty());
    assert_f3_renders(&mut scoreboard, "scoreboard");
    main_assert!(scoreboard.scoreboard_dialog.is_some());

    for mode in [
        AppObjectMenuMode::Inventory,
        AppObjectMenuMode::Container,
        AppObjectMenuMode::Context,
        AppObjectMenuMode::Build,
    ] {
        let mut object = new_running_sandbox_app();
        main_assert!(object.open_object_menu().expect("open defensive object state"));
        object.object_menu.test_mut().set_mode_for_parity_test(mode);
        object.test_key(VirtualKeyCode::F3, ElementState::Pressed);
        let flash_before = object.runtime_flash_message.clone();
        let mut frame = vec![0x4c; 320 * 200 * 4];
        let error = object
            .render(&mut frame)
            .expect_err("object menu must fail before generic pixels");
        main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&ClassicParityBoundary::AppObjectMenu(mode)));
        main_assert!(frame.iter().all(|byte| *byte == 0x4c));
        main_assert_eq!(object.runtime_flash_message => flash_before);
        main_assert!(object.object_menu.is_some());
    }

    let mut observer = new_running_sandbox_app();
    observer
        .engine
        .remove_player(observer.local_owner)
        .test_value();
    observer.engine.set_local_players([]);
    observer.snapshot = observer.engine.snapshot();
    observer.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    let mut frame = vec![0x7a; 320 * 200 * 4];
    observer.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0x7a));
}

/// C++ resolves every path through the live selected configuration object
/// (C4Config.cpp:1351-1357,1612-1627), so an explicit `/config` selection
/// has to reach the sound and music resolvers. Neither may rediscover
/// ambient defaults and read a different tree than the running app.
#[test]
fn explicit_config_paths_feed_audio_and_live_user_root() {
    let _lock = env_lock().lock();
    let install = tempdir();
    let selected = tempdir();
    let ambient = tempdir();
    fs::create_dir_all(install.path().join("planet/System.c4g")).test_value();

    // Two distinct trees: only the selected one holds the sample.
    let selected_sound = selected.path().join("Sound.c4g");
    let ambient_sound = ambient.path().join("Sound.c4g");
    fs::create_dir_all(&selected_sound).test_value();
    fs::create_dir_all(&ambient_sound).test_value();
    fs::write(selected_sound.join("Selected.wav"), silent_pcm_wav(1_000)).test_value();
    fs::write(ambient_sound.join("Ambient.wav"), silent_pcm_wav(1_000)).test_value();

    let names = |root: &Path| {
        let (libraries, _) = discover_global_sound_libraries_at(root);
        let mut resolver = SoundResolver::empty();
        resolver.global = libraries;
        resolver.sample_names()
    };
    main_assert_eq!(names(selected.path()) => vec!["selected.wav".to_string()]);
    main_assert_eq!(names(ambient.path()) => vec!["ambient.wav".to_string()]);

    // An explicit config selection whose UserPath points at the selected
    // tree must drive discovery, even while the ambient environment points
    // somewhere else.
    let config_file = install.path().join("explicit.ini");
    fs::write(
        &config_file,
        format!("[General]\nUserPath={}\n", selected.path().display()),
    )
    .test_value();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(install.path())),
        ("LC_USER_DATA_DIR", None),
        ("LC_CONFIG_FILE", None),
        ("LC_CONTENT_DIR", Some(selected.path())),
    ]);
    let paths = AppPaths::discover_with_config_file(Some(&config_file)).test_value();
    main_assert_eq!(paths.config_file() => config_file);
    main_assert_eq!(paths.user_data_dir() => selected.path());

    // The resolver built from those paths sees the selected tree only.
    let resolver = SoundResolver::discover_for_paths(Some(&paths));
    main_assert_eq!(resolver.sample_names() => vec!["selected.wav".to_string()]);

    // A pathless app walks no install media at all rather than guessing.
    main_assert!(SoundResolver::discover_for_paths(None).sample_names().is_empty());
}

/// When `GetScriptPlayerName` draws, and with what bound
/// (`src/C4Teams.cpp:912-923`, oracle `7d43b47`):
///
/// ```cpp
/// if (!sScriptPlayerNames.getLength()) return StdStrBuf::MakeRef(LoadResStr(C4ResStrTableKey::IDS_TEXT_COMPUTER));
/// int32_t iNameIdx = 0; StdStrBuf sOut;
/// while (sScriptPlayerNames.GetSection(iNameIdx++, &sOut, '|'))
///     if (!Game.PlayerInfos.GetActivePlayerInfoByName(sOut.getData()))
///         return sOut;
/// sScriptPlayerNames.GetSection(SafeRandom(iNameIdx - 1), &sOut, '|');
/// ```
///
/// `name_conflicts_use_cpp_raw_byte_case_folding` covers the one-name conflict,
/// so the draw itself is not new here. What is: the **bound** for more than one
/// name, and the two paths that must draw nothing at all.
///
/// The bound is off-by-one-prone by construction. `iNameIdx` is post-incremented
/// on every `GetSection` call *including the one that fails*, so after N names
/// it holds N+1, and `SafeRandom(iNameIdx - 1)` is `SafeRandom(N)` — a uniform
/// pick over exactly the N names. Reading the loop without noticing the failing
/// call is counted gives `SafeRandom(N-1)`, which can never return the last
/// name.
#[test]
fn script_player_name_draws_only_when_every_name_is_taken() {
    let configured =
        LegacyCString::from_bytes(b"Alpha|Beta|Gamma".to_vec()).test_value();

    // Nothing taken: the first name wins and no draw happens.
    let mut ranges = Vec::new();
    let selected = classic_script_player_name(&configured, &[], &mut |range| {
        ranges.push(range);
        0
    });
    main_assert_eq!(selected.as_bytes() => b"Alpha");
    main_assert_eq!(ranges => Vec::<usize>::new());

    // First taken: the next free name wins, still without drawing.
    let mut ranges = Vec::new();
    let selected =
        classic_script_player_name(&configured, &[b"Alpha".as_slice()], &mut |range| {
            ranges.push(range);
            0
        });
    main_assert_eq!(selected.as_bytes() => b"Beta");
    main_assert_eq!(ranges => Vec::<usize>::new());

    // All three taken: exactly one draw, bounded by the name count.
    let all_taken = [b"Alpha".as_slice(), b"Beta".as_slice(), b"Gamma".as_slice()];
    let mut ranges = Vec::new();
    let selected = classic_script_player_name(&configured, &all_taken, &mut |range| {
        ranges.push(range);
        2
    });
    main_assert_eq!(selected.as_bytes() => b"Gamma");
    main_assert_eq!(ranges => vec![3]);

    // The last index must be reachable — the whole point of the +1 above.
    let mut selections = Vec::new();
    for index in 0..3 {
        let selected = classic_script_player_name(&configured, &all_taken, &mut |_| index);
        selections.push(selected.as_bytes().to_vec());
    }
    main_assert_eq!(
        selections => vec![b"Alpha".to_vec(), b"Beta".to_vec(), b"Gamma".to_vec()]
    );

    // No configured names: the IDS_TEXT_COMPUTER default, drawn from nothing.
    let mut ranges = Vec::new();
    let selected =
        classic_script_player_name(&LegacyCString::default(), &all_taken, &mut |range| {
            ranges.push(range);
            0
        });
    main_assert_eq!(selected.as_bytes() => b"Computer");
    main_assert_eq!(ranges => Vec::<usize>::new());
}
