// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

    #[test]
    fn console_quit_is_global_and_headless_loop_exits_cleanly() {
        let mut app = new_state_only_menu_app(320, 200);
        for mode in [AppMode::Menu, AppMode::Loading, AppMode::Running] {
            app.mode = mode;
            app.process_console_command("/quit")
                .expect("dispatch global console quit");
            assert!(app.take_exit_request(), "quit must exit from {mode:?}");
        }

        let app = new_menu_app(320, 200);
        let (sender, receiver) = mpsc::channel();
        sender
            .send(ConsoleInputEvent::Command("/quit".to_string()))
            .expect("queue console quit");
        run_console_event_loop(app, receiver).expect("headless scheduler exits on /quit");

        let app = new_menu_app(320, 200);
        let (sender, receiver) = mpsc::channel();
        sender
            .send(ConsoleInputEvent::Error(io::Error::other(
                "fixture stdin failure",
            )))
            .expect("queue console reader failure");
        let error = run_console_event_loop(app, receiver)
            .expect_err("stdin read errors must fail the console process");
        assert!(error.to_string().contains("fixture stdin failure"));

        let mut boot = new_state_only_menu_app(320, 200);
        let (sender, receiver) = mpsc::channel();
        sender
            .send(BootLoadingEvent::Finished(None))
            .expect("finish console boot worker");
        boot.boot_loading = Some(BootLoadingState::new(receiver));
        boot.mode = AppMode::Loading;
        boot.console_mode = true;
        boot.loader_screen = None;
        boot.poll_boot_loading();
        assert_eq!(boot.mode, AppMode::Menu);
        assert!(boot.boot_loading.is_none());
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
            sender
                .send(BootLoadingEvent::Finished(None))
                .expect("finish headless boot worker");
            app.boot_loading = Some(BootLoadingState::new(receiver));
            app.mode = AppMode::Loading;
            app.loader_screen = None;
            app
        };

        // The gate is real: an ordinary windowed boot stays in Loading so the
        // next redraw can report the typed loader boundary.
        let mut windowed = finished_boot_app();
        windowed.poll_boot_loading();
        assert_eq!(windowed.mode, AppMode::Loading);

        let mut server = finished_boot_app();
        server.headless = true;
        server.poll_boot_loading();
        assert_eq!(server.mode, AppMode::Menu);
        assert!(server.boot_loading.is_none());
        assert!(
            !server.console_mode,
            "headless must not grant developer-console authority"
        );
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
        app.engine.tick().expect("tick arms clock");

        // Sub-second input accumulates without a pulse, and the phase is kept.
        for _ in 0..3 {
            advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(300))
                .expect("sub-second accumulation");
        }
        assert_eq!(app.game_time_seconds(), 0);
        assert_eq!(seconds, Duration::from_millis(900));

        // The exact one-second boundary pulses once and keeps the remainder.
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(250))
            .expect("boundary pulse");
        assert_eq!(app.game_time_seconds(), 1);
        assert_eq!(seconds, Duration::from_millis(150));

        // A five-second stall is one pulse, not five, and the sub-second phase
        // survives so the timer cannot drift.
        app.engine.tick().expect("re-arm the clock");
        let before = app.game_time_seconds();
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(5_400))
            .expect("coalesced stall");
        assert_eq!(
            app.game_time_seconds() - before,
            1,
            "a stall beyond the C++ reset threshold dispatches exactly one Sec1 callback"
        );
        assert_eq!(
            seconds,
            Duration::from_millis(550),
            "the accumulator reanchors to the sub-second phase instead of holding the backlog"
        );

        // The next ordinary second still pulses exactly once from there.
        app.engine.tick().expect("re-arm the clock");
        let before = app.game_time_seconds();
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(450))
            .expect("post-stall pulse");
        assert_eq!(app.game_time_seconds() - before, 1);
        assert_eq!(seconds, Duration::ZERO);
    }

    #[test]
    fn event_loop_second_accumulator_pulses_the_engine_clock() {
        // StdApp's one-second callback is independent from frame scheduling
        // (StdAppUnix.cpp:286-291); C4Game::Sec1Timer consumes TimeGo
        // (C4Game.cpp:1755-1759). Partial elapsed durations accumulate, but
        // headless Engine::tick calls alone never advance Game.Time.
        let mut app = new_menu_app(320, 200);
        let mut seconds = Duration::ZERO;
        app.engine.tick().expect("tick arms clock");
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(400))
            .expect("partial second pulse");
        assert_eq!(app.game_time_seconds(), 0);
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(600))
            .expect("completed second pulse");
        assert_eq!(app.game_time_seconds(), 1);

        // A gap longer than one second collapses to a single pulse. C++ fires
        // `pWindow->Sec1Timer()` at most once per Execute, on a plain
        // `seconds != LastExecute.tv_sec` comparison that cannot queue a
        // backlog (StdAppUnix.cpp:288-291), and Win32 never queues WM_TIMER
        // more than once (StdAppWin32.cpp:132). Replaying one pass per elapsed
        // second instead froze the app after any suspend or long load.
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_secs(2))
            .expect("coalesced timer pulse");
        assert_eq!(app.game_time_seconds(), 1);

        // The sub-second phase survives the coalescing, so the timer cannot
        // drift: 60.25s of backlog leaves exactly the 0.25s remainder pending.
        seconds = Duration::ZERO;
        advance_game_clock_from_elapsed(&mut app, &mut seconds, Duration::from_millis(60_250))
            .expect("long suspend coalesces");
        assert_eq!(seconds, Duration::from_millis(250));
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
        assert_eq!(frame_schedule_for_mode(AppMode::Menu, 28, 1, 16), startup);
        assert_eq!(frame_schedule_for_mode(AppMode::Loading, 28, 1, 16), startup);
        assert_eq!(
            frame_schedule_for_mode(AppMode::Running, 28, 1, 16),
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
        assert_eq!(schedule.simulation_interval, INGAME_FRAME_INTERVAL);
        assert_eq!(
            accumulator,
            Duration::ZERO,
            "elapsed time measured under the startup cadence must not leak into the game timer"
        );

        accumulate_frame_time_for_mode(
            AppMode::Running,
            28,
            1,
            16,
            &mut schedule,
            &mut accumulator,
            Duration::from_millis(27),
        );
        assert_eq!(accumulator, Duration::from_millis(27));

        assert!(synchronize_frame_schedule(
            AppMode::Running,
            28,
            2,
            16,
            &mut schedule,
            &mut accumulator,
        ));
        assert_eq!(accumulator, Duration::ZERO);

        assert!(synchronize_frame_schedule(
            AppMode::Running,
            1_000,
            3,
            16,
            &mut schedule,
            &mut accumulator,
        ));
        assert_eq!(schedule.simulation_interval, Duration::from_millis(1_000));
        assert_eq!(schedule.refresh_interval, Duration::from_millis(15));
        accumulate_frame_time_for_mode(
            AppMode::Running,
            1_000,
            3,
            16,
            &mut schedule,
            &mut accumulator,
            Duration::from_millis(1_000),
        );
        assert_eq!(accumulator, Duration::from_millis(1_000));

        accumulate_frame_time_for_mode(
            AppMode::Menu,
            1_000,
            3,
            16,
            &mut schedule,
            &mut accumulator,
            Duration::from_millis(1),
        );
        assert_eq!(schedule, startup);
        assert_eq!(accumulator, Duration::ZERO);
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
        assert_eq!(
            accumulator,
            Duration::from_millis(32),
            "ordinary frame gaps retain their timer debt"
        );

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
        assert_eq!(
            accumulator,
            MAX_ACCUMULATED_TIME,
            "exactly two seconds of timer debt keeps the bounded catch-up debt"
        );

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
        assert_eq!(
            accumulator,
            schedule.simulation_interval,
            "timer debt beyond two seconds resumes with one immediate normal-speed tick"
        );

        let mut app = new_running_sandbox_app();
        let frame_before = app.engine.frame();
        let outcome = advance_simulation_pass(&mut app, &mut schedule, &mut accumulator)
            .expect("execute the reanchored timer callback");
        assert_eq!(outcome.executed_frames, 1);
        assert_eq!(app.engine.frame(), frame_before + 1);
        assert_eq!(accumulator, Duration::ZERO);
    }

    #[test]
    fn focus_loss_clears_controls_repeat_tracking_and_pointer_state() {
        let mut app = GameApp::new(
            320,
            200,
            AudioOptions::default(),
            None,
            RuntimeConfig {
                player_owner: 1,
                player_name: "Focus tester".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app");
        install_classic_test_assets(&mut app);

        let mut definition =
            Definition::from_script("WLKR", "Walker", walker_script()).expect("crew definition");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([(
                "Walk".to_string(),
                ActionSpec::default().with_procedure("Walk"),
            )]),
        );
        definition.set_movement_profile(MovementProfile::default());
        definition.set_crew_member(true);
        app.engine
            .register_definition(definition)
            .expect("register crew definition");
        app.engine
            .set_player_starts(vec![clonk_engine::scenario::PlayerStart {
                ready_crew: vec![("WLKR".to_string(), 1)],
                ..Default::default()
            }]);
        app.join_local_player().expect("join fresh player");
        app.mode = AppMode::Running;
        app.ingame_pointer = Some(ViewportPointer {
            owner: app.local_owner,
            world: FloatVector2::new(10.0, 20.0),
            screen: GuiPoint::new(30.0, 40.0),
        });

        AppVirtualKeyboard::new(&mut app)
            .press(VirtualKeyCode::KeyX)
            .expect("press physical Down");
        assert!(!app.pressed_engine_keys.is_empty());
        assert_ne!(
            app.engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == app.local_owner)
                .expect("local player")
                .control
                .pressed_coms,
            0
        );

        app.handle_focus_lost().expect("handle focus loss");

        assert!(app.pressed_engine_keys.is_empty());
        assert_ne!(
            app.engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == app.local_owner)
                .expect("local player")
                .control
                .pressed_coms,
            0,
            "no native backend clears player controls on focus loss \
             (C4FullScreen.cpp:139-145,310-315,432-447)"
        );
        assert_eq!(
            app.ingame_pointer, None,
            "focus loss retains the old pointer_left lifecycle cleanup"
        );
    }

    #[test]
    fn invalid_sound_override_keeps_previous_decoded_sample() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Sound.c4g");
        let scenario = dir.path().join("Override.c4s");
        fs::create_dir_all(&global).expect("global sound group");
        fs::create_dir_all(&scenario).expect("scenario sound group");
        fs::write(global.join("Voice.wav"), silent_pcm_wav(1_000))
            .expect("valid global sample");
        fs::write(scenario.join("VOICE.WAV"), b"not an audio stream")
            .expect("invalid later override");

        let mut audio = empty_test_audio_context();
        audio.resolver.global = collect_sound_libraries_for_path(&global);
        audio.resolver.scenario = collect_sound_libraries_for_path(&scenario);
        audio.refresh_sound_catalog();

        assert_eq!(audio.available_sound_samples(), ["voice.wav"]);
        let resolved = audio
            .ensure_sound_with_key("Voice")
            .expect("validated catalog lookup")
            .expect("the earlier valid sample survives");
        assert_eq!(resolved.handle.duration_ms(), Some(1_000));
        assert!(
            resolved.sample_key.contains("sound.c4g"),
            "the undecodable scenario entry must not replace the global handle"
        );
        assert_eq!(resolved.sample_order, 0);
        assert!(
            audio
                .missing_sounds
                .iter()
                .any(|key| key.starts_with("asset::") && key.contains("voice.wav")),
            "the rejected decode is retained in diagnostics"
        );
    }

    #[test]
    fn unreadable_sound_override_keeps_previous_decoded_sample() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Sound.c4g");
        let scenario = dir.path().join("Unreadable.c4s");
        fs::create_dir_all(&global).expect("global sound group");
        fs::create_dir_all(&scenario).expect("scenario sound group");
        fs::write(global.join("Alert.wav"), silent_pcm_wav(750))
            .expect("valid global sample");
        let override_path = scenario.join("ALERT.WAV");
        fs::write(&override_path, silent_pcm_wav(1_500)).expect("temporary override sample");

        let global_libraries = collect_sound_libraries_for_path(&global);
        let scenario_libraries = collect_sound_libraries_for_path(&scenario);
        fs::remove_file(&override_path).expect("make catalogued override unreadable");

        let mut audio = empty_test_audio_context();
        audio.resolver.global = global_libraries;
        audio.resolver.scenario = scenario_libraries;
        audio.refresh_sound_catalog();

        let resolved = audio
            .ensure_sound_with_key("Alert")
            .expect("validated catalog lookup")
            .expect("the earlier readable sample survives");
        assert_eq!(resolved.handle.duration_ms(), Some(750));
        assert!(resolved.sample_key.contains("sound.c4g"));
        assert!(
            audio
                .missing_sounds
                .iter()
                .any(|key| key.starts_with("asset::") && key.contains("alert.wav")),
            "the rejected read is retained in diagnostics"
        );
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
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Sound.c4g");
        fs::create_dir_all(&global).expect("global sound group");
        fs::write(global.join("Door_Metal.wav"), silent_pcm_wav(500))
            .expect("the sample ClonkMars really ships");
        fs::write(global.join("Broken.wav"), b"not an audio stream")
            .expect("undecodable sample fixture");

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
                assert!(
                    audio
                        .ensure_sound_with_key("metaldoor")
                        .expect("validated catalog lookup")
                        .is_none(),
                    "the stale ActMap name resolves to nothing, exactly as in C++"
                );
            }
            assert!(
                audio
                    .ensure_sound_with_key("Door_Metal")
                    .expect("validated catalog lookup")
                    .is_some(),
                "the sample the door script really plays still resolves"
            );
        });

        let logged = capture.take();
        let name_lines = logged
            .lines()
            .filter(|line| line.contains("missing sound asset"))
            .collect::<Vec<_>>();
        assert_eq!(
            name_lines.len(),
            1,
            "the unresolved name is reported once, deduplicated: {logged}"
        );
        assert!(
            name_lines[0].contains("DEBUG"),
            "an unresolvable name is a content defect, not an engine fault: {}",
            name_lines[0]
        );
        assert!(
            logged
                .lines()
                .filter(|line| line.contains("sound candidate"))
                .all(|line| line.contains("WARN")),
            "a sample that fails to decode keeps C++'s error report: {logged}"
        );
        assert!(
            audio.missing_sounds.contains("request::metaldoor"),
            "the unresolved name stays in diagnostics whatever level it logged at"
        );
    }

    #[test]
    fn undecodable_speech_sample_does_not_suppress_text_fallback() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Speech.c4s");
        fs::create_dir_all(&scenario).expect("scenario sound group");
        fs::write(scenario.join("BrokenSpeech.wav"), b"corrupt speech")
            .expect("corrupt speech fixture");

        let mut audio = empty_test_audio_context();
        audio.resolver.scenario = collect_sound_libraries_for_path(&scenario);
        audio.refresh_sound_catalog();
        let advertised = audio.available_sound_samples();
        assert!(advertised.is_empty(), "undecodable speech is not advertised");

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
                    PlayerConfig::new(0, "Player").with_viewports([
                        clonk_engine::PlayerViewport::new(Vector2::ZERO),
                    ]),
                )
                .expect("speech fixture player");
            engine.configure_sound_samples(advertised.iter());
            let control = clonk_engine::ScriptControlData {
                script: LegacyCString::from_bytes(script.as_bytes().to_vec())
                    .expect("speech script has no NUL"),
                by_client: 0,
                ..clonk_engine::ScriptControlData::default()
            };
            engine
                .execute_script_control(&control, ScriptControlPolicy::live(false))
                .expect("message script executes")
                .expect("host script is accepted");

            assert!(engine.pending_audio.is_empty());
            let messages = engine.snapshot().hud.messages;
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].kind, expected_kind);
            assert_eq!(messages[0].lines, [expected]);
        }
    }

    #[test]
    fn matches_sound_pattern_uses_cpp_prepared_question_wildcards() {
        assert!(matches_sound_pattern("sound?.wav", "sound1.wav"));
        assert!(!matches_sound_pattern("sound?.wav", "sound12.wav"));
        assert!(matches_sound_pattern("mix???.ogg", "mix001.ogg"));
        assert!(!matches_sound_pattern("mix???.ogg", "mix01.ogg"));
    }

    #[test]
    fn sound_search_terms_converts_star_to_cpp_one_character_wildcard() {
        let terms = SoundSearchTerms::new("Sound*");
        assert_eq!(terms.wildcard_pattern.as_deref(), Some("sound?.wav"));
        assert!(terms.search_names.is_empty());

        let explicit_extension = SoundSearchTerms::new("Sound.*");
        assert_eq!(
            explicit_extension.wildcard_pattern.as_deref(),
            Some("sound.?")
        );
        assert!(explicit_extension.search_names.is_empty());
    }

    #[test]
    fn sound_search_terms_preserves_cpp_literal_whitespace_and_dotfile_extensions() {
        assert_eq!(
            SoundSearchTerms::new(" Fire ").search_names,
            [" fire .wav"]
        );
        assert_eq!(SoundSearchTerms::new(".wav").search_names, [".wav"]);
        assert_eq!(SoundSearchTerms::new("Fire.").search_names, ["fire..wav"]);
        let nested = format!("dir.name{}Fire", std::path::MAIN_SEPARATOR);
        let nested_wav = format!("dir.name{}fire.wav", std::path::MAIN_SEPARATOR);
        assert_eq!(
            SoundSearchTerms::new(&nested).search_names,
            [nested_wav]
        );
    }

    #[test]
    fn extensionless_sound_names_resolve_only_wav_across_libraries() {
        assert_eq!(SoundSearchTerms::new("Boom").search_names, ["boom.wav"]);
        assert_eq!(
            SoundSearchTerms::new("Boom.ogg").search_names,
            ["boom.ogg"]
        );
        assert_eq!(
            SoundSearchTerms::new("Boom.mp3").search_names,
            ["boom.mp3"]
        );
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Codec.c4s");
        let global = dir.path().join("Sound.c4g");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::create_dir_all(&global).expect("create global group");
        fs::write(scenario.join("OnlyOgg.ogg"), b"only ogg")
            .expect("write ogg-only sound");
        fs::write(scenario.join("OnlyMp3.mp3"), b"only mp3")
            .expect("write mp3-only sound");
        fs::write(scenario.join("Prefer.ogg"), b"scenario ogg")
            .expect("write scenario ogg");
        fs::write(scenario.join("Prefer.mp3"), b"scenario mp3")
            .expect("write scenario mp3");
        fs::write(global.join("Prefer.wav"), b"global wav").expect("write global wav");

        let resolver = SoundResolver {
            global: collect_sound_libraries_for_path(&global),
            scenario: collect_sound_libraries_for_path(&scenario),
            scenario_root: Some(scenario),
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: Vec::new(),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        };

        assert!(resolver.resolve_entry("OnlyOgg").is_none());
        assert!(resolver.resolve_entry("OnlyMp3").is_none());
        assert_eq!(
            resolver
                .resolve_entry("OnlyOgg.ogg")
                .expect("explicit ogg resolves")
                .load_audio()
                .expect("read ogg bytes"),
            b"only ogg"
        );
        assert_eq!(
            resolver
                .resolve_entry("OnlyMp3.mp3")
                .expect("explicit mp3 resolves")
                .load_audio()
                .expect("read mp3 bytes"),
            b"only mp3"
        );
        assert_eq!(
            resolver
                .resolve_entry("Prefer")
                .expect("extensionless request finds lower-priority wav")
                .load_audio()
                .expect("read wav bytes"),
            b"global wav"
        );
        assert_eq!(
            resolver
                .resolve_entry("Prefer.ogg")
                .expect("explicit ogg keeps scenario precedence")
                .load_audio()
                .expect("read ogg bytes"),
            b"scenario ogg"
        );
        assert_eq!(
            resolver
                .resolve_entry("Prefer.mp3")
                .expect("explicit mp3 keeps scenario precedence")
                .load_audio()
                .expect("read mp3 bytes"),
            b"scenario mp3"
        );
    }

    #[test]
    fn sound_resolver_star_matches_exactly_one_extra_character() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Wildcard.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Foo.wav"), b"no extra character")
            .expect("write zero-character candidate");
        fs::write(scenario.join("Foo12.wav"), b"two extra characters")
            .expect("write two-character candidate");

        let make_resolver = || SoundResolver {
            global: Vec::new(),
            scenario: Vec::new(),
            scenario_root: None,
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: Vec::new(),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        };
        let mut resolver = make_resolver();
        assert!(resolver.configure_scenario(Some(&scenario)));
        assert!(resolver.resolve_entry("Foo*").is_none());

        fs::write(scenario.join("Foo1.wav"), b"one extra character")
            .expect("write one-character candidate");
        let mut resolver = make_resolver();
        assert!(resolver.configure_scenario(Some(&scenario)));
        assert_eq!(
            resolver
                .resolve_entry("Foo*")
                .expect("one-character wildcard resolves")
                .load_audio()
                .expect("resolved sample loads"),
            b"one extra character"
        );
    }

    fn wildcard_sound_resolver_fixture() -> (tempfile::TempDir, SoundResolver) {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Scenario.c4s");
        let global = dir.path().join("Sound.c4g");
        let definition = dir.path().join("Blast.c4d");
        for path in [&scenario, &global, &definition] {
            fs::create_dir_all(path).expect("sound group");
        }
        fs::write(scenario.join("Blast1.wav"), b"scenario blast").expect("scenario sound");
        fs::write(global.join("Blast2.wav"), b"global blast").expect("global sound");
        fs::write(definition.join("Blast3.wav"), b"definition blast")
            .expect("definition sound");

        let mut resolver = SoundResolver {
            global: collect_sound_libraries_for_path(&global),
            scenario: collect_sound_libraries_for_path(&scenario),
            scenario_root: Some(scenario),
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: Vec::new(),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        };
        let definition_group = Group::open(&definition).expect("definition group");
        resolver.register_definition_group("BLST", &definition_group);
        (dir, resolver)
    }

    #[test]
    fn wildcard_sound_resolution_varies_without_advancing_synced_rng() {
        let (_dir, resolver) = wildcard_sound_resolver_fixture();
        let mut with_sound = clonk_engine::LcgRng::seed_from_u64(0xc4);
        let mut without_sound = with_sound.clone();
        let resolved = (0..100)
            .map(|_| {
                resolver
                    .resolve_entry("Blast*")
                    .expect("wildcard sound")
                    .cache_key()
            })
            .collect::<HashSet<_>>();

        assert!(
            resolved.len() > 1,
            "one hundred SafeRandom selections must not collapse to one sample"
        );
        assert_eq!(
            with_sound, without_sound,
            "sound resolution must not touch the synchronized LCG state"
        );
        let with_sound_draws = (0..16)
            .map(|_| with_sound.random(10_000))
            .collect::<Vec<_>>();
        let without_sound_draws = (0..16)
            .map(|_| without_sound.random(10_000))
            .collect::<Vec<_>>();
        assert_eq!(with_sound_draws, without_sound_draws);
    }

    #[test]
    fn wildcard_sound_resolution_spans_scenario_global_and_definitions() {
        let (_dir, resolver) = wildcard_sound_resolver_fixture();
        let resolved = (0..3)
            .map(|selected| {
                resolver
                    .resolve_entry_with_random("Blast*", |range| {
                        assert_eq!(range, 3, "every resolved library contributes a match");
                        selected
                    })
                    .expect("selected wildcard sound")
                    .load_audio()
                    .expect("sound bytes")
            })
            .collect::<HashSet<_>>();
        let expected = [
            b"scenario blast".to_vec(),
            b"global blast".to_vec(),
            b"definition blast".to_vec(),
        ]
        .into_iter()
        .collect::<HashSet<_>>();
        assert_eq!(resolved, expected);

        let exact = resolver
            .resolve_entry_with_random("Blast2", |_| {
                panic!("exact sound resolution must not consume SafeRandom")
            })
            .expect("exact global sound");
        assert_eq!(exact.load_audio().expect("exact sound bytes"), b"global blast");
    }

    #[test]
    fn definition_sound_overrides_global_while_scenario_remains_first() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Sound.c4g");
        let first_definition = dir.path().join("Objects.c4d");
        let second_definition = dir.path().join("MoreObjects.c4d");
        let scenario = dir.path().join("Override.c4s");
        for path in [&global, &first_definition, &second_definition, &scenario] {
            fs::create_dir_all(path).expect("create sound group");
        }
        fs::write(global.join("Clang.wav"), b"global clang").expect("write global sound");
        fs::write(first_definition.join("CLANG.WAV"), b"first definition clang")
            .expect("write first definition sound");
        fs::write(second_definition.join("clang.wav"), b"second definition clang")
            .expect("write second definition sound");
        fs::write(scenario.join("ClAnG.WaV"), b"scenario clang")
            .expect("write scenario sound");

        let mut resolver = SoundResolver {
            global: collect_sound_libraries_for_path(&global),
            scenario: Vec::new(),
            scenario_root: None,
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: direct_sound_sample_loads(
                &Group::open(&global).expect("open global sound group"),
            ),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        };
        resolver.rebuild_sample_ranks();
        assert_eq!(
            resolver
                .resolve_entry("Clang")
                .expect("global sample resolves")
                .load_audio()
                .expect("read global sample"),
            b"global clang"
        );

        let first_group = Group::open(&first_definition).expect("open first definition group");
        resolver.register_definition_group("CLNK", &first_group);
        assert_eq!(
            resolver
                .resolve_entry("Clang")
                .expect("definition sample overrides global")
                .load_audio()
                .expect("read definition sample"),
            b"first definition clang"
        );

        let second_group = Group::open(&second_definition).expect("open second definition group");
        resolver.register_definition_group("MORE", &second_group);
        assert_eq!(
            resolver
                .resolve_entry("Clang")
                .expect("later definition sample overrides earlier definition")
                .load_audio()
                .expect("read later definition sample"),
            b"second definition clang"
        );

        assert!(resolver.configure_scenario(Some(&scenario)));
        assert_eq!(
            resolver
                .resolve_entry("Clang")
                .expect("scenario sample overrides definition")
                .load_audio()
                .expect("read scenario sample"),
            b"scenario clang"
        );
        let wildcard = resolver
            .resolve_entry_with_random("Clang.???", |range| {
                assert_eq!(range, 1, "shadowed filenames are one C++ sample");
                0
            })
            .expect("scenario wildcard sample overrides shadowed definitions and global");
        assert_eq!(
            wildcard.load_audio().expect("read wildcard sample"),
            b"scenario clang"
        );
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

        let dir = tempdir().expect("tempdir");
        let definitions = dir.path().join("Objects.c4d");
        let valid = definitions.join("Valid.c4d");
        let pure_sounds = definitions.join("Potions.c4d");
        let scenario_path = dir.path().join("SoundTest.c4s");
        for path in [&valid, &pure_sounds, &scenario_path] {
            fs::create_dir_all(path).expect("create sound fixture group");
        }
        fs::write(
            valid.join("DefCore.txt"),
            "[DefCore]\nid=TEST\nName=Test\nCategory=1\n",
        )
        .expect("write valid definition core");
        write_test_definition_graphics(&valid);
        fs::write(pure_sounds.join("Drink.wav"), silent_pcm_wav(20))
            .expect("write pure-container sound");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Sound Test\n\n[Definitions]\nDefinition1=Objects.c4d\n",
        )
        .expect("write scenario core");

        let fixture_resolver = FixtureResolver {
            definitions: Group::open(&definitions).expect("open definition root"),
        };
        let scenario = Scenario::load_from_path_with(&scenario_path, &fixture_resolver)
            .expect("load scenario with pure sound child");
        assert!(
            scenario
                .sound_effect_groups()
                .iter()
                .any(|group| group.root() == pure_sounds),
            "the live C4DefList event stream retains the DefCore-less child"
        );

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.resolver = SoundResolver::empty();
        audio.refresh_sound_catalog();
        let advertised =
            configure_scenario_sound_samples(Some(&mut audio), &scenario, &scenario_path);
        assert!(advertised.iter().any(|name| name == "drink.wav"));
        assert!(
            audio
                .ensure_sound_with_key("Drink")
                .expect("decode pure-container sample")
                .is_some(),
            "the advertised pure-container sample must also be playable"
        );
    }

    #[test]
    fn nested_non_definition_audio_is_not_admitted() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Objects.c4d");
        let pure = root.join("Pure.c4d");
        let child = pure.join("Child.c4d");
        let ordinary = root.join("Ordinary");
        let sound_sibling = root.join("SoundExtra.c4g");
        let nested_ordinary = pure.join("Assets");
        for path in [&child, &ordinary, &sound_sibling, &nested_ordinary] {
            fs::create_dir_all(path).expect("create nested sound fixture");
        }
        fs::write(root.join("Root.wav"), b"root").expect("write root sound");
        fs::write(pure.join("Direct.ogg"), b"direct").expect("write direct c4d sound");
        fs::write(child.join("Child.mp3"), b"child").expect("write child c4d sound");
        fs::write(ordinary.join("Leak.wav"), b"ordinary leak").expect("write ordinary leak");
        fs::write(sound_sibling.join("Leak.ogg"), b"sound sibling leak")
            .expect("write sound-container leak");
        fs::write(nested_ordinary.join("Deep.mp3"), b"nested leak")
            .expect("write nested ordinary leak");

        let root = Group::open(&root).expect("open definition root");
        let mut sound_effect_groups = Vec::new();
        collect_definition_tree_sound_groups(&root, &mut sound_effect_groups);
        let mut resolver = SoundResolver::empty();
        resolver.configure_scenario_with_sound_effect_groups(None, &sound_effect_groups);

        assert_eq!(
            resolver.sample_names(),
            ["child.mp3", "direct.ogg", "root.wav"]
        );
        for rejected in ["Leak", "Leak.ogg", "Deep.mp3"] {
            assert!(
                resolver.resolve_entry(rejected).is_none(),
                "non-definition descendant `{rejected}` entered the sound bank"
            );
        }
    }

    #[test]
    fn sound_sample_rank_tracks_cpp_last_load_order_separately_from_precedence() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Sound.c4g");
        let definition = dir.path().join("Objects.c4d");
        let scenario = dir.path().join("Order.c4s");
        for path in [&global, &definition, &scenario] {
            fs::create_dir_all(path).expect("create sound group");
        }
        fs::write(global.join("A.wav"), b"global a").expect("write global A");
        fs::write(global.join("B.wav"), b"global b").expect("write global B");
        fs::write(definition.join("A.wav"), b"definition a").expect("write definition A");
        fs::write(definition.join("C.wav"), b"definition c").expect("write definition C");
        fs::write(scenario.join("B.wav"), b"scenario b").expect("write scenario B");
        fs::write(scenario.join("D.wav"), b"scenario d").expect("write scenario D");

        let global_group = Group::open(&global).expect("open global group");
        let mut resolver = SoundResolver {
            global: collect_sound_libraries_for_path(&global),
            scenario: Vec::new(),
            scenario_root: None,
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: direct_sound_sample_loads(&global_group),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        };
        resolver.rebuild_sample_ranks();
        let definition_group = Group::open(&definition).expect("open definition group");
        let scenario_group = Group::open(&scenario).expect("open scenario group");
        let sound_effect_groups = [definition_group, scenario_group];
        assert!(resolver.configure_scenario_with_sound_effect_groups(
            Some(&scenario),
            &sound_effect_groups,
        ));

        for definition_sample in ["a.wav", "c.wav"] {
            for scenario_sample in ["b.wav", "d.wav"] {
                assert!(
                    resolver.sample_order(definition_sample)
                        < resolver.sample_order(scenario_sample),
                    "definition samples load before the scenario tree"
                );
            }
        }
        let mut expected_wildcard_order = ["a.wav", "b.wav", "c.wav", "d.wav"];
        expected_wildcard_order.sort_by_key(|name| resolver.sample_order(name));
        let wildcard_order = (0..expected_wildcard_order.len())
            .map(|selected| {
                resolver
                    .resolve_entry_with_random("*", |range| {
                        assert_eq!(range, expected_wildcard_order.len());
                        selected
                    })
                    .expect("wildcard sample")
                    .file_name()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(wildcard_order, expected_wildcard_order);

        assert_eq!(
            resolver
                .resolve_entry("A")
                .expect("definition override")
                .load_audio()
                .expect("read definition A"),
            b"definition a"
        );
        assert_eq!(
            resolver
                .resolve_entry("B")
                .expect("scenario override")
                .load_audio()
                .expect("read scenario B"),
            b"scenario b"
        );
    }

    #[test]
    fn sound_sample_rank_prebuilds_definition_trees_and_resets_between_scenarios() {
        let dir = tempdir().expect("tempdir");
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
            fs::create_dir_all(path).expect("create definition tree");
        }
        fs::write(definitions.join("Def.ogg"), b"definition root").expect("write root def");
        fs::write(definitions.join("Def.wav"), b"definition wav").expect("write wav def");
        fs::write(definition_child.join("Nested.wav"), b"nested def")
            .expect("write nested def");
        fs::write(first_scenario.join("Root.wav"), b"scenario root")
            .expect("write scenario root");
        fs::write(scenario_child.join("Local.wav"), b"scenario child")
            .expect("write scenario child");
        fs::write(second_scenario.join("Next.wav"), b"next scenario")
            .expect("write next scenario");

        let definitions = Group::open(&definitions).expect("open definitions");
        let mut resolver = SoundResolver {
            global: Vec::new(),
            scenario: Vec::new(),
            scenario_root: None,
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: Vec::new(),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        };
        resolver.rebuild_sample_ranks();
        let first_scenario_group = Group::open(&first_scenario).expect("open first scenario");
        let mut first_sound_effect_groups = Vec::new();
        collect_definition_tree_sound_groups(&definitions, &mut first_sound_effect_groups);
        collect_definition_tree_sound_groups(&first_scenario_group, &mut first_sound_effect_groups);
        assert!(resolver.configure_scenario_with_sound_effect_groups(
            Some(&first_scenario),
            &first_sound_effect_groups,
        ));

        let ordered = [
            "def.wav",
            "def.ogg",
            "nested.wav",
            "root.wav",
            "local.wav",
        ];
        assert!(ordered
            .windows(2)
            .all(|pair| resolver.sample_order(pair[0]) < resolver.sample_order(pair[1])));

        let second_scenario_group = Group::open(&second_scenario).expect("open second scenario");
        assert!(resolver.configure_scenario_with_sound_effect_groups(
            Some(&second_scenario),
            std::slice::from_ref(&second_scenario_group),
        ));
        assert_eq!(resolver.sample_order("next.wav"), 0);
        for stale in ordered {
            assert!(!resolver.sample_ranks.contains_key(stale));
        }
    }

    #[test]
    fn loop_started_while_muted_gets_a_channel_after_unmute() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(1_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        audio.options.sound_enabled = false;
        snapshot.audio.push(test_sound_command(true));

        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        snapshot.audio.clear();
        assert!(audio.active_channels.contains_key(&key));
        assert!(audio.active_channels[&key].channel.is_none());
        let original_started_at = audio.active_channels[&key].started_at;

        snapshot.audio.push(test_sound_command(true));
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        snapshot.audio.clear();
        assert_eq!(audio.active_channels.len(), 1);
        assert_eq!(audio.active_channels[&key].started_at, original_started_at);

        audio.options.sound_enabled = true;
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        let channel = audio.active_channels[&key]
            .channel
            .expect("muted loop starts after unmute");
        assert!(audio.system.channel_is_playing(channel));
    }

    #[test]
    fn channel_restore_at_capacity_follows_sample_then_instance_order() {
        let dir = tempdir().expect("channel restore fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        for name in ["First.wav", "Second.wav", "Third.wav"] {
            fs::write(scenario.join(name), silent_pcm_wav(1_000))
                .expect("write sound fixture");
        }
        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        })
        .expect("audio context");
        audio.configure_scenario(Some(&scenario));
        audio.options.sound_enabled = false;
        let snapshot = make_snapshot(Vec::new(), Vec::new());

        for name in ["First", "Second", "Third"] {
            audio
                .start_sound(name, None, 100, true, false, None, &snapshot, &[])
                .expect("muted loop starts without a channel");
        }
        assert_eq!(audio.active_channels.len(), 3);
        assert!(audio
            .active_channels
            .values()
            .all(|info| info.channel.is_none()));

        // Assign synthetic native tuple ranks that deliberately disagree with
        // this map's iteration order. This keeps the regression deterministic
        // despite RandomState while exercising both ordering fields.
        let hash_order = audio.active_channels.keys().cloned().collect::<Vec<_>>();
        let winner = hash_order[2].clone();
        let same_sample_later = hash_order[1].clone();
        let later_sample = hash_order[0].clone();
        {
            let info = audio
                .active_channels
                .get_mut(&winner)
                .expect("winner instance");
            info.sample_order = 0;
            info.instance_order = 1;
        }
        {
            let info = audio
                .active_channels
                .get_mut(&same_sample_later)
                .expect("later same-sample instance");
            info.sample_order = 0;
            info.instance_order = 2;
        }
        {
            let info = audio
                .active_channels
                .get_mut(&later_sample)
                .expect("later sample instance");
            info.sample_order = 1;
            info.instance_order = 0;
        }

        audio.options.sound_enabled = true;
        audio.update_channels(&snapshot, &[], true);

        assert_eq!(audio.active_channels.len(), 1);
        assert!(audio.active_channels.contains_key(&winner));
        let channel = audio.active_channels[&winner]
            .channel
            .expect("first native instance reacquires the only channel");
        assert!(audio.system.channel_is_playing(channel));
        assert!(!audio.active_channels.contains_key(&same_sample_later));
        assert!(!audio.active_channels.contains_key(&later_sample));
    }

    #[test]
    fn deleted_earlier_loop_frees_capacity_for_ordered_restore() {
        let dir = tempdir().expect("ordered channel release fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        for name in ["First.wav", "Second.wav"] {
            fs::write(scenario.join(name), silent_pcm_wav(1_000))
                .expect("write sound fixture");
        }
        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        })
        .expect("audio context");
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
            .expect("first loop occupies the only channel");
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
            .expect("inaudible second loop starts without a channel");
        let first_key = SoundInstanceKey::new("First", Some(first.id));
        let second_key = SoundInstanceKey::new("Second", Some(second.id));
        assert!(audio.active_channels[&first_key].channel.is_some());
        assert!(audio.active_channels[&second_key].channel.is_none());
        audio
            .active_channels
            .get_mut(&first_key)
            .expect("first instance")
            .sample_order = 0;
        audio
            .active_channels
            .get_mut(&second_key)
            .expect("second instance")
            .sample_order = 1;

        let without_first = make_snapshot(vec![second.clone()], Vec::new());
        audio.update_channels(
            &without_first,
            &[audio_viewport(0, OWNER_NONE, second.position)],
            true,
        );

        assert!(!audio.active_channels.contains_key(&first_key));
        let restored = audio
            .active_channels
            .get(&second_key)
            .expect("later loop survives after the earlier release");
        let channel = restored
            .channel
            .expect("later loop reacquires the released channel");
        assert!(audio.system.channel_is_playing(channel));
    }

    #[test]
    fn channel_less_muted_loop_still_obeys_volume_and_stop_commands() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(1_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        audio.options.sound_enabled = false;
        snapshot.audio.push(test_sound_command(true));
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );

        snapshot.audio = vec![AudioCommand::SetSoundVolume {
            name: "Loop".to_string(),
            target: None,
            volume: 37,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert_eq!(audio.active_channels[&key].volume, 37);
        assert!(audio.active_channels[&key].channel.is_none());

        snapshot.audio = vec![AudioCommand::StopSound {
            name: "Loop".to_string(),
            target: None,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert!(!audio.active_channels.contains_key(&key));

        snapshot.audio.clear();
        audio.options.sound_enabled = true;
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert!(!audio.active_channels.contains_key(&key));
    }

    #[test]
    fn sound_level_revolumes_and_stops_a_prior_frame_one_shot() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        snapshot.audio.push(test_sound_command(false));
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );

        let original = &audio.active_channels[&key];
        let original_channel = original.channel.expect("one-shot mixer channel");
        let original_started_at = original.started_at;
        snapshot.audio = vec![AudioCommand::SetSoundVolume {
            name: "Loo?".to_string(),
            target: None,
            volume: 50,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );

        let updated = &audio.active_channels[&key];
        assert_eq!(audio.active_channels.len(), 1);
        assert_eq!(updated.channel, Some(original_channel));
        assert_eq!(updated.started_at, original_started_at);
        assert_eq!(updated.volume, 50);
        assert!(!updated.looped, "SoundLevel must not promote a one-shot");
        assert!(audio.system.channel_is_playing(original_channel));

        snapshot.audio = vec![AudioCommand::StopSound {
            name: "Loop.wav".to_string(),
            target: None,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert!(!audio.active_channels.contains_key(&key));
        assert!(!audio.system.channel_is_playing(original_channel));
    }

    #[test]
    fn sound_level_starts_and_reuses_a_loop_when_no_instance_exists() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        snapshot.audio = vec![AudioCommand::SetSoundVolume {
            name: "Loop".to_string(),
            target: None,
            volume: 37,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );

        let started = &audio.active_channels[&key];
        let original_channel = started.channel.expect("fallback loop mixer channel");
        let original_started_at = started.started_at;
        assert_eq!(audio.active_channels.len(), 1);
        assert!(started.looped);
        assert_eq!(started.volume, 37);
        assert!(audio.system.channel_is_playing(original_channel));

        snapshot.audio = vec![AudioCommand::SetSoundVolume {
            name: "Loop".to_string(),
            target: None,
            volume: 61,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        let updated = &audio.active_channels[&key];
        assert_eq!(audio.active_channels.len(), 1);
        assert_eq!(updated.channel, Some(original_channel));
        assert_eq!(updated.started_at, original_started_at);
        assert!(updated.looped);
        assert_eq!(updated.volume, 61);

        snapshot.audio = vec![AudioCommand::StopSound {
            name: "Loop".to_string(),
            target: None,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert!(!audio.active_channels.contains_key(&key));
        assert!(!audio.system.channel_is_playing(original_channel));
    }

    #[test]
    fn sound_level_above_100_reaches_app_instance_unchanged() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        snapshot.audio = vec![AudioCommand::SetSoundVolume {
            name: "Loop".to_string(),
            target: None,
            volume: 140,
        }];

        audio.process_audio(&snapshot, &mut runtime_music_enabled);

        assert_eq!(audio.active_channels[&key].volume, 140);
        let (mix_volume, pan) = compute_mix_values(
            audio.active_channels.get_mut(&key).expect("wind instance"),
            &snapshot,
            &[],
        );
        assert!((mix_volume - 1.4).abs() < 1.0e-6);
        assert_eq!(pan, 0.0);
    }

    #[test]
    fn sound_level_starts_a_loop_after_the_finished_one_shot_is_swept() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        snapshot.audio.push(test_sound_command(false));
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        let finished_channel = audio.active_channels[&key]
            .channel
            .expect("one-shot mixer channel");
        audio.system.halt_channel(finished_channel);

        snapshot.audio = vec![AudioCommand::SetSoundVolume {
            name: "Loop".to_string(),
            target: None,
            volume: 45,
        }];
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );

        assert!(
            !audio.active_channels.contains_key(&key),
            "the unswept C++ instance counts as found, then cleanup removes it"
        );
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );

        let fallback = &audio.active_channels[&key];
        assert_eq!(audio.active_channels.len(), 1);
        assert!(fallback.looped);
        assert_eq!(fallback.volume, 45);
        assert!(fallback
            .channel
            .is_some_and(|channel| audio.system.channel_is_playing(channel)));
    }

    #[test]
    fn muted_one_shot_past_half_duration_is_culled_before_it_can_resume() {
        let (_dir, mut audio, mut snapshot) = test_audio_context_with_sound(10_000);
        let key = SoundInstanceKey::new("Loop", None);
        let mut runtime_music_enabled = false;
        snapshot.audio.push(test_sound_command(false));
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        snapshot.audio.clear();
        audio
            .active_channels
            .get_mut(&key)
            .expect("one-shot instance")
            .started_at = Instant::now() - Duration::from_millis(6_000);

        audio.options.sound_enabled = false;
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert!(audio.active_channels.contains_key(&key));
        assert!(audio.active_channels[&key].channel.is_none());

        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert!(!audio.active_channels.contains_key(&key));

        audio.options.sound_enabled = true;
        audio.process_audio(
            &snapshot,
            &mut runtime_music_enabled,
        );
        assert!(!audio.active_channels.contains_key(&key));
    }

    #[test]
    fn inaudible_loops_leave_channels_free_for_a_nearby_sound() {
        let dir = tempdir().expect("inaudible loop fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        for name in ["FarA.wav", "FarB.wav", "Near.wav"] {
            fs::write(scenario.join(name), silent_pcm_wav(10_000))
                .expect("write sound fixture");
        }
        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels: 2,
            ..AudioOptions::default()
        })
        .expect("audio context");
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
                .expect("audible loop starts");
        }
        let first_key = SoundInstanceKey::new("FarA", Some(first.id));
        let second_key = SoundInstanceKey::new("FarB", Some(second.id));
        assert!(audio.active_channels[&first_key].channel.is_none());
        assert!(audio.active_channels[&second_key].channel.is_none());

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
            .expect("nearby sound uses a released mixer slot");
        let nearby_channel = audio
            .active_channels
            .get(&SoundInstanceKey::new("Near", Some(nearby.id)))
            .expect("nearby sound instance")
            .channel
            .expect("nearby sound channel");
        assert!(audio.system.channel_is_playing(nearby_channel));
    }

    #[test]
    fn inaudible_one_shot_is_culled_one_update_after_half_duration() {
        let dir = tempdir().expect("inaudible one-shot fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Impact.wav"), silent_pcm_wav(10_000))
            .expect("write sound fixture");
        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        })
        .expect("audio context");
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
            .expect("audible one-shot starts");
        let key = SoundInstanceKey::new("Impact", Some(source.id));
        let channel = audio.active_channels[&key]
            .channel
            .expect("one-shot channel");
        audio
            .active_channels
            .get_mut(&key)
            .expect("one-shot instance")
            .started_at = Instant::now() - Duration::from_millis(6_000);

        let moved = make_snapshot(
            vec![
                listener.clone(),
                make_object(source.id.as_u64(), "SNDS", Vector2::new(1_000, 0)),
            ],
            Vec::new(),
        );
        let viewports = [audio_viewport(0, OWNER_NONE, listener.position)];
        audio.update_channels(&moved, &viewports, true);
        assert!(
            audio.active_channels.contains_key(&key),
            "the release pass retains the logical one-shot instance"
        );
        assert!(audio.active_channels[&key].channel.is_none());
        assert!(!audio.system.channel_is_playing(channel));

        audio.update_channels(&moved, &viewports, true);
        assert!(
            !audio.active_channels.contains_key(&key),
            "the next pass culls an inaudible one-shot past half duration"
        );
    }

    #[test]
    fn scenario_sound_resolver_loads_local_definition_folder_root_only() {
        // C4Game::FoldersWithLocalsDefs adds each .c4f ancestor containing a
        // direct *.c4d child as a definition resource (C4Game.cpp:3961-3994).
        // C4DefList::Load first tries that resource root, and even though it
        // has no DefCore, C4Def::Load still loads its direct sound effects
        // (C4Def.cpp:927-950, 591-596). It never descends into sibling .c4s
        // groups, because the recursive definition scan accepts only *.c4d.
        let dir = tempdir().expect("tempdir");
        let folder = dir.path().join("Tutorial.c4f");
        let definitions = folder.join("Objects.c4d");
        let scenario = folder.join("Tutorial01.c4s");
        let sibling = folder.join("Tutorial02.c4s");
        fs::create_dir_all(&definitions).expect("create local definitions");
        fs::create_dir_all(&scenario).expect("create scenario");
        fs::create_dir_all(&sibling).expect("create sibling scenario");
        fs::write(folder.join("Drop.wav"), b"parent drop").expect("write parent sound");
        fs::write(definitions.join("Voice.wav"), b"definition voice")
            .expect("write definition sound");
        fs::write(sibling.join("Sibling.wav"), b"sibling sound").expect("write sibling sound");

        let mut resolver = SoundResolver {
            global: Vec::new(),
            scenario: Vec::new(),
            scenario_root: None,
            registered_definitions: HashSet::new(),
            definition_library_count: 0,
            base_sample_loads: Vec::new(),
            definition_sample_loads: Vec::new(),
            scenario_sample_loads: Vec::new(),
            sample_ranks: HashMap::new(),
            sample_ranks_prebuilt: false,
        };
        assert!(resolver.configure_scenario(Some(&scenario)));

        assert_eq!(
            resolver
                .resolve_entry("Drop")
                .expect("parent-folder root sound")
                .load_audio()
                .expect("read parent-folder sound"),
            b"parent drop"
        );
        assert!(
            resolver.resolve_entry("Sibling").is_none(),
            "sibling scenario sounds are not definition-folder root sounds"
        );
        assert_eq!(
            resolver.sample_names(),
            vec!["drop.wav", "voice.wav"],
            "the engine-facing inventory follows the same admitted libraries"
        );
        assert_eq!(
            resolver
                .resolve_entry("Voice")
                .expect("local definition-tree sound")
                .load_audio()
                .expect("read local definition sound"),
            b"definition voice"
        );
    }

    #[test]
    fn compute_mix_values_matches_cxx_audibility() {
        let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
        let source = make_object(2, "Source", Vector2::new(1350, 1000));
        let snapshot = make_snapshot(
            vec![listener.clone(), source.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
        );
        let (volume, pan) = compute_mix_values_for(
            100,
            Some(source.id),
            None,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, listener.position)],
        );
        assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
        assert!((pan - 0.7).abs() < 1e-6, "pan={pan}");
    }

    #[test]
    fn compute_mix_values_respects_custom_falloff() {
        let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
        let source = make_object(2, "Source", Vector2::new(1700, 1000));
        let snapshot = make_snapshot(
            vec![listener.clone(), source.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
        );
        let (volume, pan) = compute_mix_values_for(
            100,
            Some(source.id),
            Some(1400),
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, listener.position)],
        );
        assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
        assert!((pan - 1.0).abs() < 1e-6, "pan={pan}");
    }

    #[test]
    fn negative_custom_falloff_matches_cpp_transform() {
        // C4SoundSystem applies the signed integer transform for every
        // nonzero value. A negative denominator therefore clamps every raw
        // audibility to full volume, while pan remains positional.
        for audibility in [0, 50, 100] {
            assert_eq!(adjusted_audibility(audibility, Some(-700)), 1.0);
        }
        assert_eq!(adjusted_audibility(0, Some(0)), 0.0);

        let (_dir, mut audio, _) = test_audio_context_with_sound(1_000);
        let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
        let source = make_object(2, "Source", Vector2::new(1700, 1000));
        let snapshot = make_snapshot(
            vec![listener.clone(), source.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
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
            .expect("negative-falloff sound starts");
        let key = SoundInstanceKey::new("Loop", Some(source.id));
        let info = audio
            .active_channels
            .get_mut(&key)
            .expect("channel info retains the sound");
        assert_eq!(info.custom_falloff, Some(-700));
        let (volume, pan) = compute_mix_values(info, &snapshot, &viewports);
        assert_eq!(volume, 1.0);
        assert_eq!(pan, 1.0);
    }

    #[test]
    fn compute_mix_values_for_global_sound_preserves_base_mix() {
        let listener = make_object(1, "Listener", Vector2::new(0, 0));
        let snapshot = make_snapshot(
            vec![listener.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
        );
        let (volume, pan) = compute_mix_values_for(
            80,
            None,
            None,
            &snapshot,
            &[],
        );
        assert!((volume - 0.8).abs() < 1e-6);
        assert_eq!(pan, 0.0);
    }

    #[test]
    fn viewport_feedback_coalesces_global_sample_and_uses_initial_sound_gate() {
        let dir = tempdir().expect("viewport feedback fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(
            scenario.join("CloseViewport.wav"),
            silent_pcm_wav(1_000),
        )
        .expect("write viewport feedback sample");
        let mut audio = AudioContext::try_new(AudioOptions {
            sound_enabled: false,
            menu_sound_enabled: true,
            max_channels: 1,
            ..AudioOptions::default()
        })
        .expect("audio context");
        audio.configure_scenario(Some(&scenario));
        let snapshot = make_snapshot(Vec::new(), Vec::new());
        let key = SoundInstanceKey::new("CloseViewport", None);

        assert!(
            audio
                .try_start_global_effect("CloseViewport", false, &snapshot)
                .expect("pre-running feedback starts")
        );
        assert!(audio.active_channels[&key].channel.is_some());
        assert!(
            !audio
                .try_start_global_effect("CloseViewport", false, &snapshot)
                .expect("duplicate feedback is rejected"),
            "C++ keeps one global instance per resolved sample"
        );
        assert_eq!(audio.active_channels.len(), 1);

        audio.reset_sfx();
        assert!(
            audio
                .try_start_global_effect("CloseViewport", true, &snapshot)
                .expect("running muted feedback keeps a logical instance")
        );
        assert!(audio.active_channels[&key].channel.is_none());
    }

    #[test]
    fn running_gui_sound_requires_fe_samples_and_rx_sound() {
        let dir = tempdir().expect("GUI sound fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("ArrowHit.wav"), silent_pcm_wav(1_000))
            .expect("write GUI sound sample");
        fs::write(scenario.join("Elevator.wav"), silent_pcm_wav(1_000))
            .expect("write lobby loop sample");
        let mut audio = AudioContext::try_new(AudioOptions {
            sound_enabled: true,
            menu_sound_enabled: false,
            max_channels: 2,
            ..AudioOptions::default()
        })
        .expect("audio context");
        audio.configure_scenario(Some(&scenario));
        let snapshot = make_snapshot(Vec::new(), Vec::new());
        let key = SoundInstanceKey::new("ArrowHit", None);

        audio.play_gui_sound("ArrowHit", false, &snapshot);
        audio.play_gui_sound("ArrowHit", true, &snapshot);
        assert!(
            audio.active_channels.is_empty(),
            "FESamples rejects new GUI requests in every game state"
        );
        assert!(
            audio
                .try_start_global_effect("ArrowHit", false, &snapshot)
                .expect("direct effect creates a muted logical instance"),
            "native direct StartSoundEffect has no outer GUI gate"
        );
        assert!(audio.active_channels[&key].channel.is_none());
        audio.reset_sfx();

        audio.options.menu_sound_enabled = true;
        audio.options.sound_enabled = false;
        audio.play_gui_sound("ArrowHit", true, &snapshot);
        assert!(audio.active_channels.contains_key(&key));
        assert!(
            audio.active_channels[&key].channel.is_none(),
            "RXSound mutes playback but retains the admitted GUI instance"
        );
        audio.options.sound_enabled = true;
        audio.update_channels(&snapshot, &[], true);
        assert!(
            audio.active_channels[&key].channel.is_some(),
            "unmuting before half duration recreates the SDL channel"
        );

        audio.reset_sfx();
        audio.options.sound_enabled = false;
        audio.play_gui_sound("ArrowHit", true, &snapshot);
        let muted = &audio.active_channels[&key];
        assert!(
            !muted.non_looping_past_half_duration(muted.started_at + Duration::from_millis(500))
        );
        assert!(muted.non_looping_past_half_duration(muted.started_at + Duration::from_millis(501)));
        audio
            .active_channels
            .get_mut(&key)
            .expect("muted running GUI instance")
            .started_at = Instant::now() - Duration::from_millis(600);
        audio.update_channels(&snapshot, &[], true);
        assert!(
            !audio.active_channels.contains_key(&key),
            "a channel-less one-shot expires strictly after half duration"
        );

        audio.play_gui_sound("ArrowHit", false, &snapshot);
        let startup_channel = audio.active_channels[&key]
            .channel
            .expect("FESamples starts the pre-game channel despite RXSound=false");
        audio.update_channels(&snapshot, &[], true);
        assert!(audio.active_channels[&key].channel.is_none());
        assert!(!audio.system.channel_is_playing(startup_channel));

        audio.reset_sfx();
        audio.options.menu_sound_enabled = false;
        audio.start_lobby_elevator(&snapshot);
        let elevator = SoundInstanceKey::new("Elevator", None);
        assert!(audio.active_channels[&elevator].looped);
        assert!(audio.active_channels[&elevator].channel.is_none());
        audio.start_lobby_elevator(&snapshot);
        assert_eq!(audio.active_channels.len(), 1);
        audio.options.menu_sound_enabled = true;
        audio.update_channels(&snapshot, &[], false);
        assert!(audio.active_channels[&elevator].channel.is_some());
        audio.stop_lobby_elevator();
        assert!(audio.active_channels.is_empty());
    }

    #[test]
    fn global_gui_instance_survives_transitions_until_its_sample_is_reloaded() {
        let dir = tempdir().expect("GUI transition fixture");
        let scenario = dir.path().join("First.c4s");
        let unrelated = dir.path().join("Unrelated.c4d");
        let invalid_replacement = dir.path().join("Invalid.c4d");
        let replacement = dir.path().join("Replacement.c4d");
        for path in [&scenario, &unrelated, &invalid_replacement, &replacement] {
            fs::create_dir_all(path).expect("create sound group");
        }
        fs::write(scenario.join("Click.wav"), silent_pcm_wav(1_000))
            .expect("write initial GUI sample");
        fs::write(unrelated.join("Command.wav"), silent_pcm_wav(1_000))
            .expect("write unrelated sample");
        fs::write(invalid_replacement.join("CLICK.WAV"), b"not an audio stream")
            .expect("write invalid replacement sample");
        fs::write(replacement.join("Click.wav"), silent_pcm_wav(1_000))
            .expect("write replacement GUI sample");

        let mut audio = AudioContext::try_new(AudioOptions {
            sound_enabled: false,
            menu_sound_enabled: true,
            max_channels: 2,
            ..AudioOptions::default()
        })
        .expect("audio context");
        let scenario_group = Group::open(&scenario).expect("open scenario sound group");
        audio.configure_scenario_with_resources(
            Some(&scenario),
            Some(&[]),
            Some(std::slice::from_ref(&scenario_group)),
        );
        let snapshot = make_snapshot(Vec::new(), Vec::new());
        let key = SoundInstanceKey::new("Click", None);
        audio.play_gui_sound("Click", false, &snapshot);
        let original_sample_order = audio.active_channels[&key].sample_order;
        let frontend_channel = audio.active_channels[&key]
            .channel
            .expect("FESamples starts the frontend channel");

        let unrelated_group = Group::open(&unrelated).expect("open unrelated group");
        audio.register_definition_sounds("UNRELATED", &unrelated_group);
        assert!(audio.active_channels.contains_key(&key));
        assert!(audio.system.channel_is_playing(frontend_channel));

        audio.update_channels(&snapshot, &[], true);
        assert!(audio.active_channels[&key].channel.is_none());
        assert!(!audio.system.channel_is_playing(frontend_channel));
        assert!(
            audio.active_channels.contains_key(&key),
            "the startup instance crosses into running RXSound control"
        );
        audio.options.sound_enabled = true;
        audio.update_channels(&snapshot, &[], true);
        let restored_channel = audio.active_channels[&key]
            .channel
            .expect("RXSound restores the shared instance");

        let invalid_group =
            Group::open(&invalid_replacement).expect("open invalid replacement group");
        audio.register_definition_sounds("INVALID", &invalid_group);
        assert!(
            audio.active_channels.contains_key(&key),
            "an undecodable replacement leaves the prior sample instance alive"
        );
        assert_eq!(
            audio.active_channels[&key].sample_order,
            original_sample_order,
            "a failed replacement does not move the prior sample in catalog order"
        );
        assert!(audio.system.channel_is_playing(restored_channel));

        let replacement_group = Group::open(&replacement).expect("open replacement group");
        audio.register_definition_sounds("TEST", &replacement_group);
        assert!(!audio.active_channels.contains_key(&key));
        assert!(!audio.system.channel_is_playing(restored_channel));

        audio.play_gui_sound("Click", false, &snapshot);
        assert!(
            audio.active_channels[&key].sample_order > original_sample_order,
            "a successful replacement appends the new sample at the catalog tail"
        );
        let reloaded_channel = audio.active_channels[&key]
            .channel
            .expect("registered sample restarts");
        audio.register_definition_sounds("TEST", &replacement_group);
        assert!(
            !audio.active_channels.contains_key(&key),
            "reloading an already registered definition still replaces its sample"
        );
        assert!(!audio.system.channel_is_playing(reloaded_channel));

        audio.play_gui_sound("Click", false, &snapshot);
        let generation_channel = audio.active_channels[&key]
            .channel
            .expect("next sound-system generation has a live channel to clear");
        audio.reset_sound_system_generation();
        assert!(audio.active_channels.is_empty());
        assert!(!audio.system.channel_is_playing(generation_channel));
        assert!(audio.resolver.scenario_root.is_none());
        assert_eq!(audio.resolver.definition_library_count, 0);
        assert!(audio.resolver.registered_definitions.is_empty());
    }

    #[test]
    fn repeated_gui_sound_uses_global_instance_dedup() {
        let dir = tempdir().expect("GUI sound fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Click.wav"), silent_pcm_wav(1_000))
            .expect("write GUI sound sample");
        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels: 2,
            ..AudioOptions::default()
        })
        .expect("audio context");
        audio.configure_scenario(Some(&scenario));
        let snapshot = make_snapshot(Vec::new(), Vec::new());
        let key = SoundInstanceKey::new("Click", None);

        audio.play_gui_sound("Click", false, &snapshot);
        let first = audio.active_channels[&key].clone();
        audio.play_gui_sound("Click.wav", false, &snapshot);
        assert_eq!(audio.active_channels.len(), 1);
        assert_eq!(audio.active_channels[&key].channel, first.channel);
        assert_eq!(
            audio.active_channels[&key].instance_order, first.instance_order,
            "the resolved sample's existing global instance is not retriggered"
        );

        let channel = first.channel.expect("startup GUI channel");
        audio.system.halt_channel(channel);
        audio.update_channels(&snapshot, &[], false);
        assert!(
            audio.active_channels.is_empty(),
            "frontend frames sweep completed shared instances"
        );
        audio.play_gui_sound("Click", false, &snapshot);
        assert!(audio.active_channels[&key].instance_order > first.instance_order);
    }

    #[test]
    fn positional_mix_takes_max_volume_and_sums_only_active_viewport_pans() {
        let source = make_object(1, "Source", Vector2::new(350, 100));
        let local_left = PlayerState {
            id: 1,
            view_cursor: Some(source.id),
            viewports: vec![clonk_engine::PlayerViewport::new(Vector2::new(0, 100))],
            ..Default::default()
        };
        let local_right = PlayerState {
            id: 2,
            view_cursor: Some(source.id),
            viewports: vec![clonk_engine::PlayerViewport::new(Vector2::new(1000, 100))],
            ..Default::default()
        };
        let remote = PlayerState {
            id: 3,
            view_cursor: Some(source.id),
            viewports: vec![clonk_engine::PlayerViewport::new(Vector2::new(-1000, 100))],
            ..Default::default()
        };
        let mut snapshot = make_snapshot(vec![source], Vec::new());
        snapshot.players = vec![local_left, local_right, remote];
        snapshot.hud.local_players = vec![1, 2];
        let viewports = [
            audio_viewport(0, 1, Vector2::new(0, 100)),
            audio_viewport(1, 2, Vector2::new(1000, 100)),
        ];

        assert_eq!(
            compute_positional_mix_values(Vector2::new(350, 100), &snapshot, &viewports),
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

        assert_eq!(
            compute_mix_values_for_with_rendered_audibility(
                100,
                Some(line.id),
                None,
                &snapshot,
                &viewports,
                &audio.rendered_object_audibility,
            ),
            (0.5, 0.7),
            "the last absolute live vertex replaces the first SetAudibilityAt result",
        );

        calls.get_mut(&line.id).expect("line calls").reverse();
        audio.cache_rendered_object_audibility(&calls, &snapshot, &viewports);
        assert_eq!(
            compute_mix_values_for_with_rendered_audibility(
                100,
                Some(line.id),
                None,
                &snapshot,
                &viewports,
                &audio.rendered_object_audibility,
            ),
            (1.0, 0.0),
            "reversing the endpoints proves native call order rather than max-volume mixing",
        );

        snapshot.definition_lines.clear();
        assert_eq!(
            compute_mix_values_for_with_rendered_audibility(
                100,
                Some(line.id),
                None,
                &snapshot,
                &viewports,
                &audio.rendered_object_audibility,
            ),
            (1.0, 0.0),
            "changing classification alone does not invalidate native's retained fields",
        );
        audio.cache_rendered_object_audibility(&HashMap::new(), &snapshot, &viewports);
        assert_eq!(
            compute_mix_values_for_with_rendered_audibility(
                100,
                Some(line.id),
                None,
                &snapshot,
                &viewports,
                &audio.rendered_object_audibility,
            ),
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

        assert_eq!(
            compute_mix_values_for_with_rendered_audibility(
                100,
                Some(target.id),
                None,
                &snapshot,
                &viewports,
                &audio.rendered_object_audibility,
            ),
            (0.58, 0.8),
            "the last rendered viewport wins volume while both rendered pans accumulate",
        );
        assert_eq!(
            audio.rendered_object_audibility[&target.id],
            CachedObjectAudibilityMix {
                object_position: target.position,
                audibility: 58,
                pan: 80,
            },
        );
        assert_eq!(
            compute_positional_mix_values(target.position, &snapshot, &viewports),
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
        let dir = tempdir().expect("death sound fixture");
        let scenario = dir.path().join("Death.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("SF_Die.wav"), silent_pcm_wav(10_000))
            .expect("write death sample");
        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
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
        assert_eq!(
            compute_mix_values_for(100, Some(corpse.id), None, &died, &viewports),
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
            .expect("the death sound starts");
        let key = SoundInstanceKey::new("SF_Die", Some(corpse.id));
        assert!(
            audio.active_channels[&key].channel.is_some(),
            "the retained draw audibility must keep the death sound audible",
        );
    }

    #[test]
    fn post_render_attached_mix_releases_then_restores_channel_capacity() {
        let dir = tempdir().expect("post-render sound fixture");
        let scenario = dir.path().join("Audio.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        for name in ["First.wav", "Second.wav"] {
            fs::write(scenario.join(name), silent_pcm_wav(10_000))
                .expect("write sound fixture");
        }
        let mut audio = AudioContext::try_new(AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        })
        .expect("audio context");
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
            .expect("first ordinary sound starts");
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
            .expect("inaudible second line sound remains logical");
        let first_key = SoundInstanceKey::new("First", Some(first.id));
        let second_key = SoundInstanceKey::new("Second", Some(second.id));
        assert!(audio.active_channels[&first_key].channel.is_some());
        assert!(audio.active_channels[&second_key].channel.is_none());
        audio
            .active_channels
            .get_mut(&first_key)
            .expect("first instance")
            .sample_order = 0;
        audio
            .active_channels
            .get_mut(&second_key)
            .expect("second instance")
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

        assert!(audio.active_channels[&first_key].channel.is_none());
        assert!(audio.active_channels[&second_key].channel.is_some());
        assert_eq!(
            audio.active_channels[&first_key].detached_mix,
            Some((0.0, -1.0)),
            "the ordinary earlier sound releases its channel under the new viewport",
        );
        assert_eq!(
            audio.active_channels[&second_key].detached_mix,
            Some((0.0, 1.0)),
            "a detached one-shot would retain the second object's origin, not its line endpoint",
        );

        let second_channel = audio.active_channels[&second_key]
            .channel
            .expect("second sound owns the released channel");
        audio.system.halt_channel(second_channel);
        audio.refresh_attached_channel_mix_after_render(&snapshot, &rendered_viewports);
        assert!(
            !audio.active_channels.contains_key(&second_key),
            "a special channel that finished during rendering is removed",
        );

        audio
            .start_sound(
                "Second",
                None,
                100,
                false,
                false,
                None,
                &snapshot,
                &viewports,
            )
            .expect("global blocker occupies the only channel");
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
        assert!(
            !audio.active_channels.contains_key(&first_key),
            "a special instance whose newly audible channel cannot be restored is removed",
        );
        assert!(audio.active_channels.contains_key(&blocker_key));
    }

    #[test]
    fn positional_audio_handler_freezes_mix_and_rejects_second_global_instance() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Goldrush.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Pshshsh.wav"), silent_pcm_wav(1_000))
            .expect("write positional sound");

        let options = AudioOptions {
            max_channels: 2,
            ..AudioOptions::default()
        };
        let mut audio = AudioContext::try_new(options).expect("audio context");
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
        let first = audio
            .active_channels
            .get(&key)
            .expect("positional sound starts as a global instance")
            .clone();
        assert!(!first.looped);
        assert_eq!(first.target, None);
        assert_eq!(first.volume, 79);
        let (frozen_volume, frozen_pan) = first.detached_mix.expect("positional mix is frozen");
        assert!((frozen_volume - 0.79).abs() < f32::EPSILON);
        assert!((frozen_pan - 0.7).abs() < f32::EPSILON);

        audio.handle_events(
            std::slice::from_ref(&event),
            &snapshot,
            &viewports,
            &mut runtime_music_enabled,
        );
        assert_eq!(audio.active_channels.len(), 1);
        assert_eq!(audio.active_channels[&key].channel, first.channel);
    }

    #[test]
    fn sound_instance_lookup_matches_prepared_resolved_sample_names() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Lookup.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000))
            .expect("write fire sound");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
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
            .expect("extensionless loop starts");
        let channel = audio
            .active_channels
            .values()
            .next()
            .and_then(|info| info.channel)
            .expect("loop has a channel");

        audio.stop_sound("Fire.wav", Some(source.id));

        assert!(audio.active_channels.is_empty());
        assert!(!audio.system.channel_is_playing(channel));
    }

    #[test]
    fn wildcard_non_multiple_lookup_suppresses_before_reresolution() {
        let dir = tempdir().expect("tempdir");
        let first_scenario = dir.path().join("First.c4s");
        let second_scenario = dir.path().join("Second.c4s");
        fs::create_dir_all(&first_scenario).expect("create first scenario group");
        fs::create_dir_all(&second_scenario).expect("create second scenario group");
        fs::write(first_scenario.join("Blast1.wav"), silent_pcm_wav(10_000))
            .expect("write first blast");
        fs::write(second_scenario.join("Blast2.wav"), silent_pcm_wav(10_000))
            .expect("write second blast");

        let options = AudioOptions {
            max_channels: 2,
            ..AudioOptions::default()
        };
        let mut audio = AudioContext::try_new(options).expect("audio context");
        audio.configure_scenario(Some(&first_scenario));
        let source = make_object(1, "BLST", Vector2::new(100, 100));
        let snapshot = make_snapshot(vec![source.clone()], Vec::new());
        assert!(
            audio
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
                .expect("concrete blast starts")
        );

        audio.configure_scenario(Some(&second_scenario));
        assert!(
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
        assert_eq!(audio.active_channels.len(), 1);
        assert_eq!(
            audio.active_channels.values().next().unwrap().sample_name,
            "blast1.wav"
        );
    }

    #[test]
    fn detached_one_shot_does_not_get_orphaned_by_an_identical_new_request() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("DetachCollision.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000))
            .expect("write fire sound");

        let options = AudioOptions {
            max_channels: 2,
            ..AudioOptions::default()
        };
        let mut audio = AudioContext::try_new(options).expect("audio context");
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

        start(&mut audio).expect("first one-shot starts");
        let first_channel = audio
            .active_channels
            .values()
            .next()
            .and_then(|info| info.channel)
            .expect("first one-shot channel");
        audio.detach_object_sounds(
            source.id,
            source.position,
            &snapshot,
            &[audio_viewport(0, OWNER_NONE, source.position)],
        );
        start(&mut audio).expect("second one-shot starts");

        assert_eq!(audio.active_channels.len(), 2);
        let second_channel = audio
            .active_channels
            .values()
            .find(|info| info.target == Some(source.id))
            .and_then(|info| info.channel)
            .expect("second one-shot channel");
        assert!(audio.system.channel_is_playing(first_channel));
        assert!(audio.system.channel_is_playing(second_channel));

        audio.stop_sound("Fire.wav", Some(source.id));
        assert_eq!(audio.active_channels.len(), 1);
        assert!(audio.system.channel_is_playing(first_channel));
        assert!(!audio.system.channel_is_playing(second_channel));

        audio.stop_sound("Fir?", None);
        assert!(audio.active_channels.is_empty());
        assert!(!audio.system.channel_is_playing(first_channel));
    }

    #[test]
    fn global_lookup_stops_the_oldest_detached_instance_of_one_sample() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("InstanceOrder.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000))
            .expect("write fire sound");

        let options = AudioOptions {
            max_channels: 2,
            ..AudioOptions::default()
        };
        let mut audio = AudioContext::try_new(options).expect("audio context");
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
                .expect("one-shot starts");
            let channel = audio
                .active_channels
                .values()
                .find(|info| info.target == Some(source.id))
                .and_then(|info| info.channel)
                .expect("one-shot channel");
            detached_channels.push(channel);
            audio.detach_object_sounds(
                source.id,
                source.position,
                &snapshot,
                &[audio_viewport(0, OWNER_NONE, source.position)],
            );
        }

        audio.stop_sound("Fire", None);

        assert!(!audio.system.channel_is_playing(detached_channels[0]));
        assert!(audio.system.channel_is_playing(detached_channels[1]));
        assert_eq!(audio.active_channels.len(), 1);
    }

    #[test]
    fn wildcard_lookup_uses_sample_order_before_instance_order() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("SampleOrder.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        for name in ["Tone1.wav", "Tone2.wav"] {
            fs::write(scenario.join(name), silent_pcm_wav(10_000)).expect("write tone");
        }

        let options = AudioOptions {
            max_channels: 2,
            ..AudioOptions::default()
        };
        let mut audio = AudioContext::try_new(options).expect("audio context");
        audio.configure_scenario(Some(&scenario));
        let source = make_object(1, "TONE", Vector2::new(100, 100));
        let snapshot = make_snapshot(vec![source.clone()], Vec::new());
        let (lower_name, higher_name) = if audio.resolver.sample_order("tone1.wav")
            < audio.resolver.sample_order("tone2.wav")
        {
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
                .expect("tone starts");
        }
        let lower_channel = audio
            .active_channels
            .values()
            .find(|info| info.sample_name == format!("{}.wav", lower_name.to_ascii_lowercase()))
            .and_then(|info| info.channel)
            .expect("lower-rank tone channel");
        let higher_channel = audio
            .active_channels
            .values()
            .find(|info| info.sample_name == format!("{}.wav", higher_name.to_ascii_lowercase()))
            .and_then(|info| info.channel)
            .expect("higher-rank tone channel");

        audio.stop_sound("Tone?", Some(source.id));

        assert!(!audio.system.channel_is_playing(lower_channel));
        assert!(audio.system.channel_is_playing(higher_channel));
        assert_eq!(audio.active_channels.len(), 1);
    }

    #[test]
    fn object_removal_detach_stops_the_attached_loop_within_one_frame() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("SoundDetach.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Fire.wav"), silent_pcm_wav(10_000))
            .expect("write loop sound");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.configure_scenario(Some(&scenario));
        let listener = make_object(1, "LIST", Vector2::new(1000, 1000));
        let source = make_object(2, "FIRE", Vector2::new(1350, 1000));
        let initial = make_snapshot(
            vec![listener.clone(), source.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
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
            .expect("loop starts");
        let key = SoundInstanceKey::new("Fire", Some(source.id));
        let channel = audio.active_channels[&key]
            .channel
            .expect("enabled loop has a mixer channel");

        let mut removed = make_snapshot(
            vec![listener.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
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

        assert!(!audio.active_channels.contains_key(&key));
        assert!(!audio.system.channel_is_playing(channel));
    }

    #[test]
    fn object_removal_detach_freezes_one_shot_at_last_positional_mix() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("SoundDetach.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("Impact.wav"), silent_pcm_wav(10_000))
            .expect("write one-shot sound");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.configure_scenario(Some(&scenario));
        let listener = make_object(1, "LIST", Vector2::new(1000, 1000));
        let source = make_object(2, "IMPT", Vector2::new(1350, 1000));
        let initial = make_snapshot(
            vec![listener.clone(), source.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
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
            .expect("one-shot starts");
        let key = SoundInstanceKey::new("Impact", Some(source.id));
        let channel = audio.active_channels[&key]
            .channel
            .expect("enabled one-shot has a mixer channel");

        let mut removed = make_snapshot(
            vec![listener.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
                wealth: 0,
                score: 0,
            }],
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
        assert_eq!(info.target, None);
        let (volume, pan) = info.detached_mix.expect("detached mix is frozen");
        let frozen_mix = (volume, pan);
        assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
        assert!((pan - 0.7).abs() < 1e-6, "pan={pan}");
        assert!(audio.system.channel_is_playing(channel));

        let moved_listener = make_object(1, "LIST", Vector2::new(2000, 1000));
        let moved = make_snapshot(vec![moved_listener.clone()], Vec::new());
        audio.update_channels(&moved, &[], true);
        assert_eq!(audio.active_channels[&key].detached_mix, Some(frozen_mix));
        assert!(!audio
            .try_start_sound(
                "Impact",
                None,
                100,
                false,
                true,
                None,
                &moved,
                &[],
            )
            .expect("global near-dedup check succeeds"));

        audio.stop_sound("Impact", None);
        assert!(!audio.active_channels.contains_key(&key));
        assert!(!audio.system.channel_is_playing(channel));
    }

    #[test]
    fn repeated_object_sound_reuses_the_cpp_instance_before_allocating_a_channel() {
        // FnSound returns before StartSoundEffect when the same wildcard is
        // already playing on the object (C4Script.cpp:2317-2319). This must
        // happen before SDL_mixer asks for another free channel.
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Goldrush.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("HorseWalk1.wav"), silent_pcm_wav(1_000))
            .expect("write horse sound");

        let options = AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        };
        let mut audio = AudioContext::try_new(options).expect("audio context");
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

        play(&mut audio).expect("first horse step starts");
        play(&mut audio).expect("duplicate horse step is already playing");
    }

    #[test]
    fn nearby_objects_share_the_cpp_sample_instance_even_when_multiple_is_requested() {
        // C4SoundSystem::NewInstance rejects another instance of the resolved
        // sample within NearSoundRadius=50, after FnSound's fMultiple check
        // (C4SoundSystem.cpp:341-350; C4SoundSystem.h:43).
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Goldrush.c4s");
        fs::create_dir_all(&scenario).expect("create scenario group");
        fs::write(scenario.join("HorseWalk1.wav"), silent_pcm_wav(1_000))
            .expect("write horse sound");

        let options = AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        };
        let mut audio = AudioContext::try_new(options).expect("audio context");
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
            .expect("first nearby horse starts");
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
            .expect("nearby horse reuses the sample instance");
    }

    #[test]
    fn name_conflicts_use_cpp_raw_byte_case_folding() {
        let configured = LegacyCString::from_bytes(b"\xe4lpha".to_vec()).unwrap();
        let active = [b"\xc4LPHA".as_slice()];
        let mut ranges = Vec::new();
        let selected = classic_script_player_name(&configured, &active, &mut |range| {
            ranges.push(range);
            0
        });

        assert_eq!(selected.as_bytes(), b"\xe4lpha");
        assert_eq!(ranges, vec![1]);
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

        assert_eq!(color, 0x00ff_0000);
        assert_eq!(ranges, vec![302, 302, 302]);
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
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].players[0].color, 0x0003_0201);
        assert_eq!(requests[1].players[0].color, 0x0006_0504);
        assert_ne!(requests[0].players[0].color, 0x00f4_0000);
        assert_ne!(requests[1].players[0].color, 0x0000_c800);
        assert_eq!(ranges, vec![302; 6]);
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
        let decoded = decode_audio(audio).expect("sandbox music decodes");
        assert_eq!(decoded.sample_rate, 44_100);
        assert!(decoded.frames.len() > 2_000);
    }

    #[test]
    fn more_music_parser_ignores_comments_and_ascii_whitespace() {
        assert_eq!(
            parse_more_music(
                b" \r\n\t# comment with leading whitespace \r\n  #clear\t\r\n  Extra Music  \r\n"
            ),
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

        let root = tempdir().expect("legacy MoreMusic fixture");
        let outer = root.path().join("Outer.c4g");
        let mut extra_group = MutableGroup::new_bytes(EXTRA_GROUP.to_vec());
        extra_group
            .add_file_bytes_with_metadata(FIRST_TRACK.to_vec(), b"first".to_vec(), 1, false)
            .expect("add first raw-name track");
        extra_group
            .add_file_bytes_with_metadata(SECOND_TRACK.to_vec(), b"second".to_vec(), 1, false)
            .expect("add second raw-name track");
        let mut outer_group = MutableGroup::new("Outer.c4g");
        outer_group
            .add_child_bytes(EXTRA_GROUP.to_vec(), extra_group)
            .expect("add invalid-UTF-8 music child");
        fs::write(&outer, outer_group.pack().expect("pack outer music group"))
            .expect("write outer music group");

        let manifest = root.path().join("MoreMusic.txt");
        fs::write(
            &manifest,
            b"Outer.c4g/Extra\xfe.c4g/\0ignored-after-c-string\n".as_slice(),
        )
        .expect("write invalid-UTF-8 MoreMusic path");

        let mut catalog = MusicCatalog::empty();
        load_more_music(&mut catalog, &manifest).expect("load raw MoreMusic catalog");
        assert_eq!(catalog.assets.len(), 2);
        assert_eq!(
            catalog
                .resolve(&clonk_script::c4_string_from_bytes(FIRST_TRACK))
                .expect("resolve first raw basename")
                .load_audio()
                .expect("read first raw basename exactly"),
            b"first"
        );
        assert_eq!(
            catalog
                .resolve(&clonk_script::c4_string_from_bytes(SECOND_TRACK))
                .expect("resolve second raw basename")
                .load_audio()
                .expect("read second raw basename exactly"),
            b"second"
        );

        let mut first_full_path = outer.as_os_str().as_encoded_bytes().to_vec();
        first_full_path.push(std::path::MAIN_SEPARATOR as u8);
        first_full_path.extend_from_slice(EXTRA_GROUP);
        first_full_path.push(std::path::MAIN_SEPARATOR as u8);
        first_full_path.extend_from_slice(FIRST_TRACK);
        assert_eq!(
            catalog
                .resolve(&clonk_script::c4_string_from_bytes(&first_full_path))
                .expect("resolve raw full path")
                .file_name_bytes
                .as_slice(),
            FIRST_TRACK
        );

        let mut resolver = MusicResolver::empty();
        resolver.global = catalog;
        resolver.set_playlist(Some(clonk_script::c4_string_from_bytes(b"Tune\x81.*")));
        assert_eq!(
            resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(SECOND_TRACK),
            "script playlist bytes distinguish names that share a lossy rendering"
        );
        assert!(music_playlist_matches(b"Tune?.ogg", FIRST_TRACK));
        assert!(music_playlist_matches(b"Tune?.ogg", SECOND_TRACK));
    }

    #[test]
    fn more_music_directory_after_clear_replaces_global_catalog() {
        let root = tempdir().expect("MoreMusic directory fixture");
        let global = root.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("fixture global music group");
        fs::write(global.join("Default.ogg"), b"default").expect("default music fixture");

        let extra = root.path().join("Extra Music");
        fs::create_dir_all(&extra).expect("fixture extra music directory");
        fs::write(extra.join("Added.ogg"), b"added").expect("added music fixture");
        fs::write(extra.join("Effect.wav"), b"effect").expect("unsupported WAV fixture");
        fs::write(extra.join("Notes.txt"), b"notes").expect("non-music fixture");
        fs::write(root.path().join("Loose.mod"), b"loose")
            .expect("plain music file fixture");
        let manifest = root.path().join("MoreMusic.txt");
        fs::write(
            &manifest,
            b" \r\n\t# ignored comment \r\n  #clear\t\r\n  Extra Music  \r\nLoose.mod\r\nLoose.mod\r\n",
        )
        .expect("MoreMusic fixture");

        let mut catalog = MusicCatalog::from_group(
            Group::open(&global).expect("open global music fixture"),
        )
        .expect("build global music catalog");
        load_more_music(&mut catalog, &manifest).expect("load MoreMusic directory");

        assert_eq!(
            catalog.filenames(),
            ["Added.ogg", "Loose.mod", "Loose.mod"]
        );
        assert_eq!(
            catalog
                .resolve("Added.ogg")
                .expect("MoreMusic directory track")
                .load_audio()
                .expect("read MoreMusic directory track"),
            b"added"
        );
    }

    #[test]
    fn more_music_mp3_wildcard_adds_only_matching_supported_files() {
        let root = tempdir().expect("MoreMusic wildcard fixture");
        let global = root.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("fixture global music group");
        fs::write(global.join("Default.ogg"), b"default").expect("default music fixture");

        let extra = root.path().join("Extras");
        fs::create_dir_all(extra.join("Folder.mp3")).expect("matching directory fixture");
        for name in [
            "Keep.mp3",
            "Upper.MP3",
            "Other.ogg",
            "LooksLike.mp3.bak",
            "Readme.txt",
        ] {
            fs::write(extra.join(name), name.as_bytes()).expect("wildcard music fixture");
        }
        fs::write(extra.join("Folder.mp3/Inside.mp3"), b"nested")
            .expect("nested music fixture");
        let wildcard = PathBuf::from("Extras").join("*.mp3");
        let manifest = root.path().join("MoreMusic.txt");
        fs::write(&manifest, format!("{}\n", wildcard.display())).expect("MoreMusic fixture");

        let mut catalog = MusicCatalog::from_group(
            Group::open(&global).expect("open global music fixture"),
        )
        .expect("build global music catalog");
        load_more_music(&mut catalog, &manifest).expect("load MoreMusic wildcard");
        let mut filenames = catalog.filenames();
        filenames.sort_by_cached_key(|name| name.to_ascii_lowercase());

        assert_eq!(filenames, ["Default.ogg", "Keep.mp3", "Upper.MP3"]);
    }

    #[test]
    fn music_catalog_resolves_exact_filename_and_cpp_stem_names() {
        // C4MusicSystem::FindSong tries exact filename, then the requested
        // stem plus every supported extension (C4MusicSystem.cpp:312-333).
        let dir = tempdir().expect("tempdir");
        let music = dir.path().join("Music.c4g");
        fs::create_dir_all(&music).expect("create music group");
        fs::write(music.join("Frontend.ogg"), b"frontend").expect("write frontend");
        fs::write(music.join("Pizza Strings.ogg"), b"pizza").expect("write pizza");

        let group = Group::open(&music).expect("open music group");
        let catalog = MusicCatalog::from_group(group).expect("build music catalog");

        assert_eq!(
            catalog
                .resolve("Frontend.ogg")
                .expect("exact filename")
                .load_audio()
                .expect("read exact filename"),
            b"frontend"
        );
        assert_eq!(
            catalog
                .resolve("Frontend")
                .expect("frontend stem")
                .load_audio()
                .expect("read frontend stem"),
            b"frontend"
        );
        assert_eq!(
            catalog
                .resolve("Pizza Strings")
                .expect("pizza stem")
                .load_audio()
                .expect("read pizza stem"),
            b"pizza"
        );
    }

    #[test]
    fn music_playlist_filter_uses_raw_semicolon_patterns_and_basename_matching() {
        assert!(music_playlist_matches(b"NoMatch;*.mId", b"Theme.MID"));
        assert!(music_playlist_matches(
            b"NoMatch;Ambient.*",
            b"Ambient.ogg"
        ));
        assert!(
            !music_playlist_matches(b"NoMatch; Ambient.*", b"Ambient.ogg"),
            "C++ does not trim playlist sections"
        );

        let dir = tempdir().expect("tempdir");
        let group = Group::open(dir.path()).expect("open music fixture root");
        let asset =
            MusicAsset::for_test_path(Arc::new(group), PathBuf::from("nested/Theme.MID"));
        let catalog = MusicCatalog {
            assets: vec![asset],
        };
        assert_eq!(
            catalog
                .first_enabled(Some("*.mid"))
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"Theme.MID".as_slice()),
            "playlist matching uses GetFilename rather than the full asset path"
        );
    }

    #[test]
    fn music_playlist_explicit_filter_allows_frontend_and_default_excludes_special_tracks() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create music group");
        for name in ["@Hidden.ogg", "Credits.ogg", "Frontend.ogg", "Theme.ogg"] {
            fs::write(global.join(name), name.as_bytes()).expect("write music fixture");
        }

        let group = Group::open(&global).expect("open global music");
        let mut resolver = MusicResolver::with_global_group(group).expect("build resolver");
        assert_eq!(
            resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"Theme.ogg".as_slice())
        );

        resolver.set_playlist(Some("Frontend.*".to_string()));
        assert_eq!(
            resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"Frontend.ogg".as_slice()),
            "an explicit playlist replaces the default exclusions"
        );

        resolver.set_playlist(None);
        assert_eq!(
            resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"Theme.ogg".as_slice()),
            "restoring the default playlist excludes frontend/credits/@ tracks again"
        );
    }

    #[test]
    fn default_music_selection_uses_unsynced_choices_without_immediate_repeats() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        for name in ["A.ogg", "B.ogg", "C.ogg"] {
            fs::write(global.join(name), name.as_bytes()).expect("write music fixture");
        }

        let resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("open global music group"),
        )
        .expect("build music resolver");
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
                .expect("enabled music candidate");
            selected_names.push(selected.file_name_bytes.clone());
            recent = Some(Arc::clone(&selected.identity));
        }

        assert_eq!(
            selected_names,
            [
                b"A.ogg".to_vec(),
                b"B.ogg".to_vec(),
                b"C.ogg".to_vec(),
                b"A.ogg".to_vec()
            ]
        );
        assert_eq!(bounds, [3, 2, 2, 2]);
        assert!(
            selected_names.windows(2).all(|pair| pair[0] != pair[1]),
            "the most recently started track is excluded while alternatives exist"
        );
        assert_eq!(
            engine.snapshot().rng,
            synced_rng_before,
            "music selection must not consume the engine's synchronized LCG"
        );
    }

    #[test]
    fn music_resolver_keeps_global_catalog_when_scenario_has_no_music_source() {
        // PlayScenarioMusic only clears the constructor-loaded global catalog
        // when it discovers at least one local music directory
        // (C4MusicSystem.cpp:139-165).
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        fs::write(global.join("Frontend.ogg"), b"frontend").expect("write frontend");
        fs::write(global.join("Pizza Strings.ogg"), b"pizza").expect("write pizza");
        let scenario = dir.path().join("Tutorial.c4f").join("Tutorial01.c4s");
        fs::create_dir_all(&scenario).expect("create tutorial scenario");

        let global = Group::open(&global).expect("open global music");
        let mut resolver = MusicResolver::with_global_group(global).expect("build global resolver");
        resolver
            .configure_scenario(Some(&scenario))
            .expect("configure tutorial scenario");

        assert_eq!(
            resolver
                .resolve("Frontend")
                .expect("global Frontend fallback")
                .load_audio()
                .expect("read global Frontend"),
            b"frontend"
        );
        assert_eq!(
            resolver
                .resolve("Pizza Strings")
                .expect("global Pizza Strings fallback")
                .load_audio()
                .expect("read global Pizza Strings"),
            b"pizza"
        );
        assert_eq!(
            resolver
                .first_default()
                .expect("default global scenario track")
                .load_audio()
                .expect("read default global track"),
            b"pizza",
            "Frontend is explicitly addressable but excluded from the default playlist"
        );
    }

    #[test]
    fn music_resolver_replaces_global_catalog_when_parent_has_music_group() {
        // A discovered local music directory clears the global song list
        // before local tracks are loaded (C4MusicSystem.cpp:152-166).
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        fs::write(global.join("Frontend.ogg"), b"frontend").expect("write frontend");

        let folder = dir.path().join("Fantasy.c4f");
        let scenario = folder.join("Scenario.c4s");
        let local = folder.join("Music.c4g");
        fs::create_dir_all(&scenario).expect("create scenario");
        fs::create_dir_all(&local).expect("create local music group");
        fs::write(local.join("Local Theme.ogg"), b"local").expect("write local theme");

        let global = Group::open(&global).expect("open global music");
        let mut resolver = MusicResolver::with_global_group(global).expect("build global resolver");
        resolver.set_playlist(Some("Frontend.*".to_string()));
        resolver
            .configure_scenario(Some(&scenario))
            .expect("configure local scenario");

        assert!(
            resolver.resolve("Frontend").is_none(),
            "a local catalog replaces rather than extends global music"
        );
        assert_eq!(
            resolver
                .resolve("Local Theme")
                .expect("local theme")
                .load_audio()
                .expect("read local theme"),
            b"local"
        );
        assert_eq!(
            resolver
                .first_default()
                .map(|asset| asset.file_name_bytes.as_slice()),
            Some(b"Local Theme.ogg".as_slice()),
            "loading a new scenario music catalog restores the default playlist"
        );
    }

    #[test]
    fn definition_pack_music_is_enumerated_in_groupset_order_and_replaces_global_music() {
        let dir = tempdir().expect("definition music fixture");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("global music group");
        fs::write(global.join("Global.ogg"), b"global").expect("global track");

        let folder = dir.path().join("Fantasy.c4f");
        let scenario = folder.join("Scenario.c4s");
        let folder_music = folder.join("Music.c4g");
        fs::create_dir_all(&scenario).expect("scenario group");
        fs::create_dir_all(&folder_music).expect("folder music group");
        fs::write(folder_music.join("Shared.ogg"), b"folder").expect("folder track");

        let first_pack = dir.path().join("First.c4d");
        let second_pack = dir.path().join("Second.c4d");
        let first_music = first_pack.join("Music.c4g");
        let second_music = second_pack.join("Music.c4g");
        fs::create_dir_all(&first_music).expect("first definition music");
        fs::create_dir_all(&second_music).expect("second definition music");
        fs::write(first_music.join("FirstOnly.ogg"), b"first only")
            .expect("first-only track");
        fs::write(first_music.join("PackTie.ogg"), b"first")
            .expect("first tied track");
        fs::write(first_music.join("Shared.ogg"), b"first shared")
            .expect("first shared track");
        fs::write(second_music.join("SecondOnly.ogg"), b"second only")
            .expect("second-only track");
        fs::write(second_music.join("PackTie.ogg"), b"second")
            .expect("second tied track");

        let roots = [
            Group::open(&first_pack).expect("first definition root"),
            Group::open(&second_pack).expect("second definition root"),
        ];
        let mut resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("global music root"),
        )
        .expect("global resolver");
        assert!(
            resolver
                .configure_scenario_with_definition_roots(Some(&scenario), &roots)
                .expect("configure definition music")
        );

        assert!(resolver.resolve("Global").is_none());
        assert_eq!(
            resolver
                .resolve("Shared")
                .expect("folder wins over definition roots")
                .load_audio()
                .expect("folder bytes"),
            b"folder"
        );
        assert_eq!(
            resolver
                .resolve("PackTie")
                .expect("tied definition track")
                .load_audio()
                .expect("definition bytes"),
            b"second",
            "FindGroup enumerates the later equal-priority definition root first"
        );
        assert!(resolver.resolve("FirstOnly").is_some());
        assert!(resolver.resolve("SecondOnly").is_some());
        assert!(
            !resolver
                .configure_scenario(Some(&scenario))
                .expect("same-path playback configure"),
            "the later path-only playback pass must retain definition-root music"
        );
        assert_eq!(
            resolver
                .resolve("PackTie")
                .expect("definition catalog retained")
                .load_audio()
                .expect("retained definition bytes"),
            b"second"
        );

        fs::write(second_music.join("Reloaded.ogg"), b"reloaded")
            .expect("same-path replacement track");
        resolver.set_playlist(Some("FirstOnly.*".to_string()));
        assert!(
            resolver
                .configure_scenario_with_definition_roots(Some(&scenario), &roots)
                .expect("resource-aware same-path reload"),
            "a real activation must rebuild even when path and roots are unchanged"
        );
        assert!(resolver.playlist.is_none());
        assert_eq!(
            resolver
                .resolve("Reloaded")
                .expect("replacement track discovered")
                .load_audio()
                .expect("replacement bytes"),
            b"reloaded"
        );
    }

    #[test]
    fn extra_music_uses_later_activated_children_then_root_and_skips_inactive_children() {
        let dir = tempdir().expect("Extra music fixture");
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
            fs::create_dir_all(path).expect("music fixture group");
        }
        fs::write(global.join("Global.ogg"), b"global").expect("global track");
        fs::write(extra_root_music.join("RootOnly.ogg"), b"root").expect("root track");
        fs::write(first_music.join("ChildTie.ogg"), b"first").expect("first child track");
        fs::write(second_music.join("ChildTie.ogg"), b"second")
            .expect("second child track");
        fs::write(unused_music.join("Unused.ogg"), b"unused").expect("unused child track");

        let roots = [
            Group::open(&first_pack).expect("first definition root"),
            Group::open(&second_pack).expect("second definition root"),
        ];
        let mut resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("global music root"),
        )
        .expect("global resolver");
        resolver.extra = Some(Group::open(&extra).expect("Extra root"));
        resolver
            .configure_scenario_with_definition_roots(Some(&scenario), &roots)
            .expect("configure Extra music");

        assert!(resolver.resolve("Global").is_none());
        assert_eq!(
            resolver
                .resolve("ChildTie")
                .expect("activated child tie")
                .load_audio()
                .expect("second child bytes"),
            b"second",
            "direct GroupSet iteration keeps the later activated Extra child first"
        );
        assert_eq!(
            resolver
                .resolve("RootOnly")
                .expect("Extra root track")
                .load_audio()
                .expect("root bytes"),
            b"root"
        );
        assert!(resolver.resolve("Unused").is_none());
    }

    #[test]
    fn malformed_definition_pack_music_child_clears_global_without_aborting() {
        let dir = tempdir().expect("malformed definition music fixture");
        let global = dir.path().join("Music.c4g");
        let scenario = dir.path().join("Scenario.c4s");
        let definition = dir.path().join("Broken.c4d");
        fs::create_dir_all(&global).expect("global music group");
        fs::create_dir_all(&scenario).expect("scenario group");
        fs::create_dir_all(&definition).expect("definition group");
        fs::write(global.join("Global.ogg"), b"global").expect("global track");
        fs::write(definition.join("Music.c4g"), b"not a group")
            .expect("malformed music child");

        let mut resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("global music root"),
        )
        .expect("global resolver");
        resolver
            .configure_scenario_with_definition_roots(
                Some(&scenario),
                &[Group::open(&definition).expect("definition root")],
            )
            .expect("malformed child is a logged LoadDir miss");

        assert!(resolver.resolve("Global").is_none());
        assert!(resolver.active_filenames().is_empty());
    }

    #[test]
    fn malformed_scenario_music_child_clears_global_and_keeps_valid_root_tracks() {
        let dir = tempdir().expect("malformed scenario music fixture");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("global music group");
        fs::write(global.join("Global.ogg"), b"global").expect("global track");

        // LoadDir handles every path independently. A bad scenario child and
        // a bad inner .c4f child therefore cannot discard the scenario-root
        // track or the valid outer .c4f sibling that follows them.
        let outer = dir.path().join("Outer.c4f");
        let inner = outer.join("Inner.c4f");
        let scenario = inner.join("Scenario.c4s");
        let outer_music = outer.join("Music.c4g");
        fs::create_dir_all(&scenario).expect("scenario group");
        fs::create_dir_all(&outer_music).expect("outer music group");
        fs::write(scenario.join("Scenario Root.ogg"), b"scenario")
            .expect("scenario-root track");
        fs::write(scenario.join("Music.c4g"), b"not a group")
            .expect("malformed scenario child");
        fs::write(inner.join("Music.c4g"), b"also not a group")
            .expect("malformed parent child");
        fs::write(outer_music.join("Outer.ogg"), b"outer").expect("outer sibling track");

        let mut resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("global music root"),
        )
        .expect("global resolver");
        resolver
            .configure_scenario(Some(&scenario))
            .expect("malformed local children are isolated");

        assert!(resolver.resolve("Global").is_none());
        assert_eq!(
            resolver.active_filenames(),
            ["Scenario Root.ogg", "Outer.ogg"],
            "valid sources retain native scenario-to-parent order"
        );
        assert_eq!(
            resolver
                .resolve("Scenario Root")
                .expect("scenario-root track retained")
                .load_audio()
                .expect("scenario-root bytes"),
            b"scenario"
        );
        assert_eq!(
            resolver
                .resolve("Outer")
                .expect("outer parent track retained")
                .load_audio()
                .expect("outer parent bytes"),
            b"outer"
        );

        // Presence, not successful opening, is the local-source signal for a
        // direct scenario child and for a registered .c4f parent child.
        let bad_scenario = dir.path().join("OnlyBroken.c4s");
        fs::create_dir_all(&bad_scenario).expect("standalone scenario group");
        fs::write(bad_scenario.join("Music.c4g"), b"broken")
            .expect("standalone malformed child");
        resolver
            .configure_scenario(Some(&bad_scenario))
            .expect("standalone malformed scenario child");
        assert!(resolver.resolve("Global").is_none());
        assert!(resolver.active_filenames().is_empty());

        let bad_parent = dir.path().join("OnlyBroken.c4f");
        let child_scenario = bad_parent.join("Child.c4s");
        fs::create_dir_all(&child_scenario).expect("parent-child scenario group");
        fs::write(bad_parent.join("Music.c4g"), b"broken parent")
            .expect("standalone malformed parent child");
        resolver
            .configure_scenario(Some(&child_scenario))
            .expect("standalone malformed parent child");
        assert!(resolver.resolve("Global").is_none());
        assert!(resolver.active_filenames().is_empty());
    }

    #[test]
    fn local_music_source_enumeration_failure_is_isolated() {
        let dir = tempdir().expect("music enumeration fixture");
        let source_path = dir.path().join("Music.c4g");
        fs::create_dir_all(&source_path).expect("music source");
        fs::write(source_path.join("Track.ogg"), b"track").expect("music track");
        let source = Group::open(&source_path).expect("open music source");
        fs::remove_dir_all(&source_path).expect("invalidate opened directory source");
        assert!(
            source.entries().is_err(),
            "fixture must exercise lazy source-enumeration failure"
        );

        let mut catalog = MusicCatalog::empty();
        extend_music_source(&mut catalog, source, "test source");
        assert!(catalog.filenames().is_empty());
    }

    #[test]
    fn music_control_combines_config_and_scenario_volume() {
        // C4MusicSystem::UpdateVolume multiplies Config.Sound.MusicVolume by
        // Game.iMusicLevel only while a game is running
        // (C4MusicSystem.cpp:281-290).
        let mut control = MusicControlState::new(0.8);
        assert!((control.effective_volume() - 0.8).abs() < f32::EPSILON);

        control.set_scenario_level(Some(30));
        assert!((control.effective_volume() - 0.24).abs() < f32::EPSILON);

        control.set_scenario_level(Some(0));
        assert_eq!(control.effective_volume(), 0.0);

        control.set_configured_volume(0.5);
        control.set_scenario_level(Some(30));
        assert!((control.effective_volume() - 0.15).abs() < f32::EPSILON);

        control.set_scenario_level(None);
        assert!((control.effective_volume() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn idle_music_fade_is_a_noop_and_does_not_suppress_a_later_play() {
        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        let initial_generation = lock_unpoisoned(&audio.music_control).generation;

        assert!(!audio.fade_out_music(GAME_MUSIC_FADE_OUT_MS));
        assert_eq!(
            lock_unpoisoned(&audio.music_control).generation,
            initial_generation,
            "an idle fade must not invalidate the next Play"
        );

        audio
            .play_music(&silent_pcm_wav(1_000), true)
            .expect("schedule music after idle fade");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !audio.system.music_is_playing() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            audio.system.music_is_playing(),
            "a Play following an idle fade must still reach the mixer"
        );
    }

    #[test]
    fn script_stop_music_remains_an_immediate_halt() {
        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        let music = audio
            .system
            .load_music(&silent_pcm_wav(5_000))
            .expect("load test music");
        audio
            .system
            .play_music(&music, true)
            .expect("start test music");
        let mut runtime_music_enabled = true;

        audio.handle_events(
            &[AudioCommand::StopMusic],
            &make_snapshot(Vec::new(), Vec::new()),
            &[],
            &mut runtime_music_enabled,
        );

        assert!(!runtime_music_enabled);
        assert!(!audio.system.music_is_playing());
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

        assert_eq!(control.start_volume(stale), None);
        assert_eq!(control.start_volume(current), Some(0.3));
    }

    #[test]
    fn missing_named_music_does_not_invalidate_current_generation() {
        // C4MusicSystem::Play returns before Stop when FindSong cannot resolve
        // the requested name (C4MusicSystem.cpp:65-97).
        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        let before = lock_unpoisoned(&audio.music_control).generation;

        assert!(!audio
            .play_named_music("__definitely_missing__", true)
            .expect("missing lookup succeeds"));

        let after = lock_unpoisoned(&audio.music_control).generation;
        assert_eq!(after, before, "a miss must leave current playback intact");
    }

    #[test]
    fn resolved_unreadable_music_stops_current_playback_before_failure() {
        for named in [true, false] {
            let dir = tempdir().expect("tempdir");
            let global = dir.path().join("Music.c4g");
            fs::create_dir_all(&global).expect("create global music group");
            fs::write(global.join("Prior.ogg"), b"prior").expect("write prior fixture");
            fs::write(global.join("Gone.ogg"), b"gone").expect("write stale fixture");

            let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
            audio.music_resolver = MusicResolver::with_global_group(
                Group::open(&global).expect("open global music group"),
            )
            .expect("build music resolver");
            let prior = Arc::clone(
                &audio
                    .music_resolver
                    .resolve("Prior")
                    .expect("resolve prior music")
                    .identity,
            );
            lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&prior));
            audio.set_music_playlist(Some("Gone.*".to_string()));

            let current = audio
                .system
                .load_music(&silent_pcm_wav(5_000))
                .expect("load current music");
            audio
                .system
                .play_music(&current, true)
                .expect("start current music");
            *lock_unpoisoned(&audio.pending_music) = Some(current);
            let before = lock_unpoisoned(&audio.music_control).generation;

            fs::remove_file(global.join("Gone.ogg")).expect("remove catalogued replacement");
            let result = if named {
                audio.play_named_music("Gone", false)
            } else {
                audio.play_default_music(false)
            };
            assert!(
                result.is_err(),
                "the resolved {} replacement must reach its failed read",
                if named { "named" } else { "default" }
            );
            assert!(!audio.system.music_is_playing());
            assert!(lock_unpoisoned(&audio.pending_music).is_none());
            assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire), 0);
            assert!(audio.queued_music_starts.is_empty());
            assert_ne!(
                lock_unpoisoned(&audio.music_control).generation,
                before,
                "the resolved replacement invalidates the current generation"
            );
            let recent = lock_unpoisoned(&audio.music_control)
                .most_recently_played
                .clone()
                .expect("failed replacement preserves prior marker");
            assert!(Arc::ptr_eq(&recent, &prior));
        }
    }

    #[test]
    fn deferred_unreadable_music_stops_predecessor_and_preserves_recent_marker() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        fs::write(global.join("A.ogg"), b"A").expect("write A fixture");
        fs::write(global.join("B.ogg"), b"B").expect("write B fixture");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.music_resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("open global music group"),
        )
        .expect("build music resolver");
        let a_identity = Arc::clone(
            &audio
                .music_resolver
                .resolve("A")
                .expect("resolve A")
                .identity,
        );
        let fixture = audio
            .system
            .load_music(&silent_pcm_wav(5_000))
            .expect("predecode controlled music fixture");
        audio.control_music_loads_with(fixture);

        audio
            .play_named_music("A", false)
            .expect("start predecessor request");
        audio
            .play_named_music("B", false)
            .expect("queue resolved replacement");
        assert_eq!(audio.queued_music_starts.len(), 1);
        fs::remove_file(global.join("B.ogg")).expect("remove deferred catalogued replacement");

        assert!(audio
            .complete_next_controlled_music_load()
            .expect("complete predecessor and pump replacement"));
        assert!(!audio.system.music_is_playing());
        assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire), 0);
        assert!(audio.queued_music_starts.is_empty());
        assert!(audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading")
            .requests
            .is_empty());
        assert!(lock_unpoisoned(&audio.pending_music).is_none());
        let recent = lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .clone()
            .expect("predecessor remains the last successful music");
        assert!(Arc::ptr_eq(&recent, &a_identity));
    }

    #[test]
    fn back_to_back_music_commands_exclude_the_prior_selected_track() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        fs::write(global.join("A.ogg"), b"A").expect("write A fixture");
        fs::write(global.join("B.ogg"), b"B").expect("write B fixture");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.music_resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("open global music group"),
        )
        .expect("build music resolver");
        let a_identity = Arc::clone(
            &audio
                .music_resolver
                .resolve("A")
                .expect("resolve A")
                .identity,
        );
        let b_identity = Arc::clone(
            &audio
                .music_resolver
                .resolve("B")
                .expect("resolve B")
                .identity,
        );
        lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&a_identity));
        let fixture = audio
            .system
            .load_music(&silent_pcm_wav(20))
            .expect("predecode controlled music fixture");
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

        let controlled = audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading");
        assert_eq!(controlled.requests.len(), 1);
        assert_eq!(audio.queued_music_starts.len(), 1);
        assert!(controlled
            .requests
            .front()
            .and_then(|request| request.identity.as_ref())
            .is_some_and(|identity| Arc::ptr_eq(identity, &b_identity)));

        assert!(audio
            .complete_next_controlled_music_load()
            .expect("complete first music start"));
        let first_started = lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .clone()
            .expect("first request updates the recent marker");
        assert!(Arc::ptr_eq(&first_started, &b_identity));
        let controlled = audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading");
        assert_eq!(controlled.requests.len(), 1);
        assert!(controlled
            .requests
            .front()
            .and_then(|request| request.identity.as_ref())
            .is_some_and(|identity| Arc::ptr_eq(identity, &a_identity)));
        assert!(audio.queued_music_starts.is_empty());

        assert!(audio
            .complete_next_controlled_music_load()
            .expect("complete second music start"));
        let second_started = lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .clone()
            .expect("second request updates the recent marker");
        assert!(Arc::ptr_eq(&second_started, &a_identity));
    }

    #[test]
    fn queued_default_selection_observes_a_failed_prior_start() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        fs::write(global.join("A.ogg"), b"A").expect("write A fixture");
        fs::write(global.join("B.ogg"), b"B").expect("write B fixture");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.music_resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("open global music group"),
        )
        .expect("build music resolver");
        let a_identity = Arc::clone(
            &audio
                .music_resolver
                .resolve("A")
                .expect("resolve A")
                .identity,
        );
        let b_identity = Arc::clone(
            &audio
                .music_resolver
                .resolve("B")
                .expect("resolve B")
                .identity,
        );
        lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&a_identity));
        let fixture = audio
            .system
            .load_music(&silent_pcm_wav(20))
            .expect("predecode controlled music fixture");
        audio.control_music_loads_with(fixture);

        audio
            .play_default_music(false)
            .expect("queue first default");
        audio
            .play_default_music(false)
            .expect("queue second default");
        assert!(!audio
            .fail_next_controlled_music_load()
            .expect("fail first music start"));

        let recent = lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .clone()
            .expect("failed start preserves prior marker");
        assert!(Arc::ptr_eq(&recent, &a_identity));
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
    fn stop_music_cancels_deferred_starts_and_rejects_the_stale_worker() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        fs::write(global.join("A.ogg"), b"A").expect("write A fixture");
        fs::write(global.join("B.ogg"), b"B").expect("write B fixture");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.music_resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("open global music group"),
        )
        .expect("build music resolver");
        let prior = Arc::clone(
            &audio
                .music_resolver
                .resolve("A")
                .expect("resolve A")
                .identity,
        );
        lock_unpoisoned(&audio.music_control).most_recently_played = Some(Arc::clone(&prior));
        let fixture = audio
            .system
            .load_music(&silent_pcm_wav(20))
            .expect("predecode controlled music fixture");
        audio.control_music_loads_with(fixture);

        audio
            .play_default_music(false)
            .expect("queue first default");
        audio
            .play_default_music(false)
            .expect("queue second default");
        audio.stop_music();
        assert!(audio.queued_music_starts.is_empty());
        assert_eq!(audio.music_load_pending.load(AtomicOrdering::Acquire), 0);
        assert!(!audio
            .complete_next_controlled_music_load()
            .expect("complete stale worker"));
        assert!(audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading")
            .requests
            .is_empty());
        let recent = lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .clone()
            .expect("stale worker preserves prior marker");
        assert!(Arc::ptr_eq(&recent, &prior));
        assert!(!audio.system.music_is_playing());
    }

    #[test]
    fn most_recent_music_changes_only_after_a_successful_start() {
        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Music.c4g");
        fs::create_dir_all(&global).expect("create global music group");
        fs::write(global.join("Good.ogg"), silent_pcm_wav(100))
            .expect("write decodable music fixture");
        fs::write(global.join("Broken.ogg"), b"not audio")
            .expect("write malformed music fixture");

        let mut audio = AudioContext::try_new(AudioOptions::default()).expect("audio context");
        audio.music_resolver = MusicResolver::with_global_group(
            Group::open(&global).expect("open global music group"),
        )
        .expect("build music resolver");
        let good_identity = Arc::clone(
            &audio
                .music_resolver
                .resolve("Good")
                .expect("resolve valid named music")
                .identity,
        );
        audio.set_music_playlist(Some("Broken.*".to_string()));
        audio
            .play_named_music("Good", false)
            .expect("explicit named music bypasses the active playlist");

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
            .expect("successful worker records its asset");
        assert!(Arc::ptr_eq(&started, &good_identity));

        audio
            .play_named_music("Broken", false)
            .expect("schedule malformed named music");
        let deadline = Instant::now() + Duration::from_secs(2);
        while audio.music_load_pending.load(AtomicOrdering::Acquire) != 0
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            audio.music_load_pending.load(AtomicOrdering::Acquire),
            0,
            "malformed decode worker completed before the assertion"
        );
        let after_failure = lock_unpoisoned(&audio.music_control)
            .most_recently_played
            .clone()
            .expect("decode failure preserves prior marker");
        assert!(
            Arc::ptr_eq(&after_failure, &started),
            "a decode failure preserves the last successfully started asset"
        );
    }

    #[test]
    fn scenario_music_does_not_promote_nested_wav_effects() {
        // C4MusicSystem::PlayScenarioMusic only scans supported music files at
        // the scenario root and Music.c4g groups (C4MusicSystem.cpp:139-163).
        // In particular, Drachenfels' Princess.c4d/PrincessScream.wav is an
        // object sound effect, never scenario music.
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Drachenfels.c4s");
        let princess = scenario.join("Princess.c4d");
        fs::create_dir_all(&princess).expect("create definition directory");
        fs::write(princess.join("PrincessScream.wav"), b"scream").expect("write nested effect");

        assert_eq!(
            load_scenario_music_bytes(&scenario).expect("inspect scenario music"),
            None
        );
    }

    #[test]
    fn scenario_music_excludes_root_wav_effects() {
        // WAV is deliberately absent from C++ MusicFileExtensions
        // (C4MusicSystem.cpp:31-32), even when it sits at scenario root.
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Effects.c4s");
        fs::create_dir_all(&scenario).expect("create scenario directory");
        fs::write(scenario.join("Ambient.wav"), b"effect").expect("write root effect");

        assert_eq!(
            load_scenario_music_bytes(&scenario).expect("inspect scenario music"),
            None
        );
    }

    #[test]
    fn scenario_music_accepts_supported_root_track_case_insensitively() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Scenario.c4s");
        fs::create_dir_all(&scenario).expect("create scenario directory");
        fs::write(scenario.join("Local.OGG"), b"scenario track").expect("write local track");

        assert_eq!(
            load_scenario_music_bytes(&scenario).expect("inspect scenario music"),
            Some(b"scenario track".to_vec())
        );
    }

    #[test]
    fn scenario_music_uses_parent_music_group_when_root_has_none() {
        let dir = tempdir().expect("tempdir");
        let folder = dir.path().join("Fantasy.c4f");
        let scenario = folder.join("Drachenfels.c4s");
        let music = folder.join("Music.c4g");
        fs::create_dir_all(&scenario).expect("create scenario directory");
        fs::create_dir_all(&music).expect("create music group");
        fs::write(music.join("Knightly Wonders.mid"), b"shared track").expect("write shared track");

        assert_eq!(
            load_scenario_music_bytes(&scenario).expect("inspect scenario music"),
            Some(b"shared track".to_vec())
        );
    }

    #[test]
    fn scenario_music_uses_direct_music_group_without_scanning_definitions() {
        let dir = tempdir().expect("tempdir");
        let scenario = dir.path().join("Scenario.c4s");
        let music = scenario.join("Music.c4g");
        let definition = scenario.join("Actor.c4d");
        fs::create_dir_all(&music).expect("create music group");
        fs::create_dir_all(&definition).expect("create definition directory");
        fs::write(music.join("Theme.mp3"), b"scenario music").expect("write music");
        fs::write(definition.join("Scream.wav"), b"effect").expect("write effect");

        assert_eq!(
            load_scenario_music_bytes(&scenario).expect("inspect scenario music"),
            Some(b"scenario music".to_vec())
        );
    }

    #[test]
    fn dragon_rock_selects_fantasy_music_never_princess_scream() {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root")
            .to_path_buf();
        let fantasy = repository.join("content/Fantasy.c4f");
        let scenario = fantasy.join("Drachenfels.c4s");
        assert!(
            scenario.is_dir(),
            "the initialized official content submodule must provide {}",
            scenario.display()
        );

        let selected = load_scenario_music_bytes(&scenario)
            .expect("inspect Dragon Rock music")
            .expect("Fantasy music track");
        let scream = fs::read(scenario.join("Princess.c4d/PrincessScream.wav"))
            .expect("read Princess scream");
        let music_group = fantasy.join("Music.c4g");
        let expected_tracks: Vec<_> = [
            "Knightly Wonders.mid",
            "Medieval Waltz.mid",
            "Morning Dawn.mid",
        ]
        .into_iter()
        .map(|name| fs::read(music_group.join(name)).expect("read Fantasy music"))
        .collect();

        assert_ne!(selected, scream, "sound effect was promoted to music");
        assert!(
            expected_tracks.contains(&selected),
            "Dragon Rock did not select from Fantasy.c4f/Music.c4g"
        );
    }

    #[test]
    fn about_scrollbar_sounds_and_repeat_run_through_production_paths() {
        let mut app = new_real_classic_menu_app(320, 240);
        enter_about_licenses(&mut app);
        app.ui_sound_log.clear();

        let layout = clonk_frontend::startup_about_dlg::about_layout(320, 240);
        let text = layout.licenses.text;
        let bar = clonk_frontend::classic_gui::IntRect {
            x: text.x + text.w - 5 - 16,
            y: text.y + 8,
            w: 16,
            h: text.h - 16,
        };
        let track = PhysicalPosition::new(
            f64::from(bar.x + 8),
            f64::from(bar.y + bar.h / 2),
        );
        app.handle_cursor_moved(track).expect("hover license track");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("start license track drag");
        assert_eq!(app.ui_sound_log, vec!["Command"]);
        assert!(app
            .startup_about_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.license_scroll_offset() > 0));
        app.handle_mouse_button(ElementState::Released)
            .expect("release license track drag");

        app.ui_sound_log.clear();
        let bottom_arrow = PhysicalPosition::new(
            f64::from(bar.x + 8),
            f64::from(bar.y + bar.h - 1),
        );
        app.handle_cursor_moved(bottom_arrow)
            .expect("hover license bottom arrow");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("hold license bottom arrow");
        assert_eq!(app.ui_sound_log, vec!["ArrowHit"]);
        let before_frame = app
            .startup_about_dialog
            .as_ref()
            .unwrap()
            .license_scroll_offset();
        let mut frame = vec![0_u8; 320 * 240 * 4];
        app.render(&mut frame)
            .expect("present one held-arrow frame");
        let after_frame = app
            .startup_about_dialog
            .as_ref()
            .unwrap()
            .license_scroll_offset();
        assert!(after_frame > before_frame);

        app.handle_mouse_button(ElementState::Released)
            .expect("release license bottom arrow");
        assert_eq!(app.ui_sound_log, vec!["ArrowHit", "ArrowHit"]);
        app.render(&mut frame)
            .expect("present after arrow release");
        assert_eq!(
            app.startup_about_dialog
                .as_ref()
                .unwrap()
                .license_scroll_offset(),
            after_frame
        );
    }

    #[test]
    fn modal_and_definition_overlays_restore_the_base_frame_when_closed() {
        let mut app = new_real_classic_menu_app(640, 480);
        let mut base = vec![0_u8; 640 * 480 * 4];
        app.render(&mut base).expect("compose main-menu base");

        app.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Message",
                "Caption",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .expect("open message overlay");
        let mut modal = vec![0_u8; 640 * 480 * 4];
        assert!(app.render(&mut modal).expect("render message overlay"));
        assert_ne!(modal, base);
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("close message overlay");
        let mut closed = vec![0x77; 640 * 480 * 4];
        assert!(app.render(&mut closed).expect("render base after modal"));
        assert_eq!(closed, base);

        app.open_definition_selector(FrontendScenario::fallback())
            .expect("open definition selector");
        let mut selector = vec![0_u8; 640 * 480 * 4];
        assert!(app
            .render(&mut selector)
            .expect("render definition selector"));
        assert_ne!(selector, base);
        app.process_definition_selector_actions(vec![
            clonk_frontend::definition_sel::DefinitionSelAction::Cancelled,
        ])
        .expect("close definition selector");
        let mut closed = vec![0x88; 640 * 480 * 4];
        assert!(app.render(&mut closed).expect("render base after selector"));
        assert_eq!(closed, base);
    }

    #[test]
    fn options_sound_sheet_seeds_from_live_audio_and_applies_typed_actions() {
        use clonk_frontend::startup_options_dlg::{
            OptionsDlgAction, SoundCheckboxId, SoundSheetAction, SoundSheetState, SoundVolumeId,
        };

        let mut app = new_running_sandbox_app();
        app.return_to_menu();
        {
            let audio = app.audio.as_mut().expect("test audio");
            audio.options.menu_music_enabled = true;
            audio.options.menu_sound_enabled = false;
            audio.options.music_enabled = true;
            audio.options.sound_enabled = false;
            audio.set_music_volume_percent(83);
            audio.set_sound_volume_percent(27);
        }
        app.open_options_menu();
        assert_eq!(
            app.startup_options_dialog
                .as_ref()
                .expect("options dialog")
                .sound(),
            &SoundSheetState::new(true, false, true, false, 83, 27)
        );

        app.process_options_dialog_actions(vec![OptionsDlgAction::Sound(
            SoundSheetAction::CheckboxChanged {
                id: SoundCheckboxId::FrontendMusic,
                checked: false,
            },
        )])
        .expect("disable frontend music");
        app.process_options_dialog_actions(vec![
            OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                id: SoundCheckboxId::FrontendSoundEffects,
                checked: true,
            }),
            OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                id: SoundCheckboxId::GameMusic,
                checked: false,
            }),
            OptionsDlgAction::Sound(SoundSheetAction::CheckboxChanged {
                id: SoundCheckboxId::GameSoundEffects,
                checked: true,
            }),
            OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                id: SoundVolumeId::Music,
                value: 25,
            }),
            OptionsDlgAction::Sound(SoundSheetAction::VolumeChanged {
                id: SoundVolumeId::SoundEffects,
                value: 75,
            }),
        ])
        .expect("apply Sound-sheet actions");

        let audio = app.audio.as_ref().expect("test audio");
        assert!(!audio.options.menu_music_enabled);
        assert!(audio.options.menu_sound_enabled);
        assert!(!audio.options.music_enabled);
        assert!(audio.options.sound_enabled);
        assert_eq!(audio.options.music_volume_percent(), 25);
        assert_eq!(audio.options.sound_volume_percent(), 75);
        assert_eq!(
            lock_unpoisoned(&audio.music_control).effective_volume(),
            0.25,
            "the live/pending music controller must update with the slider"
        );
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
        keyboard
            .handle_modifiers_changed(ModifiersState::empty())
            .expect("clear modifiers");
        keyboard
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Tab enters the first Sound checkbox");
        assert_eq!(
            keyboard
                .startup_options_dialog
                .as_ref()
                .expect("options dialog")
                .focused_sound_checkbox(),
            Some(SoundCheckboxId::FrontendMusic)
        );
        keyboard
            .handle_key(VirtualKeyCode::Tab, ElementState::Released)
            .expect("release Tab");
        keyboard
            .handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("hold Shift");
        keyboard
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Shift+Tab reverses to the tabular");
        assert_eq!(
            keyboard
                .startup_options_dialog
                .as_ref()
                .expect("options dialog")
                .focused_sound_checkbox(),
            None
        );

        keyboard
            .handle_modifiers_changed(ModifiersState::empty())
            .expect("clear Shift");
        keyboard
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("focus the first Sound checkbox again");
        keyboard
            .handle_modifiers_changed(ModifiersState::ALT)
            .expect("hold Alt");
        keyboard
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Alt+Tab has no classic Options binding");
        assert_eq!(
            keyboard
                .startup_options_dialog
                .as_ref()
                .expect("options dialog")
                .focused_sound_checkbox(),
            Some(SoundCheckboxId::FrontendMusic),
            "modifier-blind fallback must not invent a plain Tab"
        );
        keyboard
            .handle_modifiers_changed(ModifiersState::CONTROL | ModifiersState::SHIFT)
            .expect("hold Ctrl+Shift");
        keyboard
            .handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Ctrl+Shift+Tab cycles the sheet despite child focus");
        assert_eq!(
            keyboard
                .startup_options_dialog
                .as_ref()
                .expect("options dialog")
                .active_sheet(),
            clonk_frontend::startup_options_dlg::OptionsSheet::Graphics
        );

        let mut gamepad = new_running_sandbox_app();
        gamepad.return_to_menu();
        enter_unported_startup_subscreen(
            &mut gamepad,
            ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
        );
        gamepad
            .process_gamepad_event_batch([
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
                GamepadEvent::Direction {
                    slot: GamepadSlot::new(0),
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
            ])
            .expect("focus FE sound effects");
        assert_eq!(
            gamepad
                .startup_options_dialog
                .as_ref()
                .expect("options dialog")
                .focused_sound_checkbox(),
            Some(SoundCheckboxId::FrontendSoundEffects)
        );
        assert!(!gamepad.audio.as_ref().unwrap().options.menu_sound_enabled);
        gamepad
            .process_gamepad_event_batch([
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
            ])
            .expect("raw AnyLowButton owns checkbox activation");
        assert!(gamepad.audio.as_ref().unwrap().options.menu_sound_enabled);

        let mut back = new_running_sandbox_app();
        back.return_to_menu();
        enter_unported_startup_subscreen(
            &mut back,
            ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
        );
        back.process_gamepad_event_batch([GamepadEvent::Direction {
            slot: GamepadSlot::new(0),
            button: ControlButton::Left,
            state: ElementState::Pressed,
        }])
        .expect("focus Back");
        back.process_gamepad_event_batch([
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
        ])
        .expect("raw low release closes exactly once");
        assert_eq!(back.startup_view, StartupView::MainMenu);
    }

    #[test]
    fn options_sound_held_arrow_advances_before_each_rendered_frame() {
        let mut app = new_classic_running_sandbox_app();
        app.return_to_menu();
        app.resize(800, 600).expect("resize menu");
        app.audio
            .as_mut()
            .expect("test audio")
            .set_music_volume_percent(50);
        enter_unported_startup_subscreen(
            &mut app,
            ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
        );
        let slider = {
            let gui = app.assets.clonk_fonts.as_deref().expect("GUI fonts");
            let book = app
                .assets
                .options_book_fonts
                .as_deref()
                .expect("options book fonts");
            clonk_frontend::startup_options_dlg::options_dlg_layout(800, 600, gui, book)
                .sound
                .slider(clonk_frontend::startup_options_dlg::SoundVolumeId::Music)
        };
        let decrement =
            PhysicalPosition::new(f64::from(slider.x + 2), f64::from(slider.y + slider.h / 2));
        app.handle_cursor_moved(decrement)
            .expect("hover decrement arrow");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("hold decrement arrow");
        assert_eq!(
            app.audio
                .as_ref()
                .expect("test audio")
                .options
                .music_volume_percent(),
            50,
            "the arrow changes during DrawElement, not on pointer-down"
        );

        let mut frame = vec![0_u8; 800 * 600 * 4];
        app.render(&mut frame).expect("render held-arrow frame");
        assert!(
            app.audio
                .as_ref()
                .expect("test audio")
                .options
                .music_volume_percent()
                < 50,
            "advance_frame must apply the slider callback before pixels"
        );
        app.handle_mouse_button(ElementState::Released)
            .expect("release decrement arrow");
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
        app.render(&mut frame).expect("establish sandbox viewport");
        let viewport = app
            .graphics
            .viewport_rect(owner)
            .expect("local sandbox viewport");
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
            .expect("sandbox viewport has an empty selection-frame rectangle");

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(start.x),
            f64::from(start.y),
        ))
        .expect("move to frame start");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("right-down stores the frame origin");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(start.x + 3.0),
            f64::from(start.y + 3.0),
        ))
        .expect("remain below drag sensitivity");
        assert!(
            !app.ingame_right_mouse_state
                .expect("right-down remains live")
                .motion
                .moved,
            "five logical pixels or less must not start C4MC_Drag_Selecting"
        );
        app.render(&mut frame).expect("render below threshold");

        app.handle_cursor_moved(PhysicalPosition::new(f64::from(end.x), f64::from(end.y)))
            .expect("cross drag sensitivity");
        let drag = app
            .ingame_right_mouse_state
            .expect("selection drag remains live");
        assert!(drag.motion.moved);
        let down_world = ingame_pointer_world_pixel(drag.motion.start);
        app.render(&mut frame)
            .expect("render active selection frame");
        let (down_x, _) = app
            .graphics
            .world_to_screen(owner, down_world)
            .expect("stored world origin remains projectable");
        let current_x = end.x.round() as i32;
        let sample_x = (current_x + down_x.round() as i32) / 2;
        let sample_y = end.y.round() as i32;
        let expected = clonk_frontend::gamma_encode_fragment(
            clonk_frontend::MOUSE_SELECTION_FRAME_COLOR,
            &app.graphics
                .active_gamma_ramp(&app.snapshot.environment.gamma),
        );
        assert_eq!(
            app.graphics
                .surface()
                .get_pixel(sample_x as u32, sample_y as u32),
            Some(expected),
            "active C4MC_Drag_Selecting draws CRed above the viewport overlay"
        );

        app.handle_right_mouse_button(ElementState::Released)
            .expect("right-up finishes empty selection frame");
        assert!(app.ingame_right_mouse_state.is_none());
        app.render(&mut frame).expect("render after button-up");
        assert_ne!(
            app.graphics
                .surface()
                .get_pixel(sample_x as u32, sample_y as u32),
            Some(expected),
            "ButtonUpDragSelecting removes the presentation frame"
        );

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(start.x),
            f64::from(start.y),
        ))
        .expect("return to left-frame start");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("left-down stores the same landscape frame origin");
        app.handle_cursor_moved(PhysicalPosition::new(f64::from(end.x), f64::from(end.y)))
            .expect("left drag crosses sensitivity");
        assert!(
            app.mouse_state
                .expect("left selection drag remains live")
                .motion
                .selection_frame,
            "left and right landscape drags share C4MC_Drag_Selecting"
        );
        app.render(&mut frame).expect("render active left frame");
        let left_down_world = ingame_pointer_world_pixel(
            app.mouse_state
                .expect("left selection drag remains live")
                .motion
                .start,
        );
        let (left_down_x, _) = app
            .graphics
            .world_to_screen(owner, left_down_world)
            .expect("left stored origin remains projectable");
        let left_sample_x = (current_x + left_down_x.round() as i32) / 2;
        assert_eq!(
            app.graphics
                .surface()
                .get_pixel(left_sample_x as u32, sample_y as u32),
            Some(expected),
            "left C4MC_Drag_Selecting uses the same frame renderer"
        );
        app.handle_mouse_button(ElementState::Released)
            .expect("left-up finishes selection frame");
        assert!(app.mouse_state.is_none());
    }

    #[test]
    fn ingame_options_sound_and_music_toggles_persist_to_config_file() {
        // Keep the process-global environment lock around only the tiny
        // isolated config writes; this state-only running fixture needs no
        // installed resources or user-data discovery.
        let mut app = new_state_only_lightweight_running_sandbox_app();
        let audio = app.audio.as_mut().expect("state-only test audio");
        audio.options.sound_enabled = true;
        audio.options.music_enabled = true;
        app.runtime_music_enabled = true;

        let user_data = tempdir().expect("isolated audio config");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover isolated app paths");
        persist_config_value(&paths, "Sound", "Sound", "true")
            .expect("seed RXSound config");
        persist_config_value(&paths, "Sound", "Music", "true")
            .expect("seed RXMusic config");
        persist_config_value(&paths, "Sound", "VendorExtension", "keep-me")
            .expect("seed preserved sound extension");
        app.app_paths = Some(paths.clone());

        app.apply_ingame_menu_action(MenuAction::ActivateOptions)
            .expect("open in-game Options");
        app.apply_ingame_menu_action(MenuAction::ToggleSound)
            .expect("toggle in-game Sound");
        // `C4SoundSystem::ToggleOnOff` flips the flag in memory alone
        // (C4SoundSystem.cpp:138-142); the file is written when the Options
        // dialog closes or at clean shutdown. This test's subject is the write
        // *content*, so it flushes explicitly — the deferral itself is pinned by
        // `runtime_config_mutations_remain_process_local_until_shutdown_save`.
        app.flush_deferred_config();
        let after_sound = Config::load(paths.config_file()).expect("reload Sound toggle");
        assert_eq!(
            after_sound.get_in(Some("Sound"), "Sound"),
            Some("false")
        );
        assert_eq!(
            after_sound.get_in(Some("Sound"), "Music"),
            Some("true"),
            "the Sound action must not wait for or rewrite the Music action"
        );

        app.apply_ingame_menu_action(MenuAction::ToggleMusic)
            .expect("toggle in-game Music");
        app.flush_deferred_config();
        let after_music = Config::load(paths.config_file()).expect("reload Music toggle");
        assert_eq!(
            after_music.get_in(Some("Sound"), "Sound"),
            Some("false")
        );
        assert_eq!(
            after_music.get_in(Some("Sound"), "Music"),
            Some("false")
        );
        assert_eq!(
            after_music.get_in(Some("Sound"), "VendorExtension"),
            Some("keep-me"),
            "eager running toggles preserve unrelated classic config keys"
        );

        let reloaded = AudioOptions::load(Some(&paths));
        assert!(!reloaded.sound_enabled, "next launch reloads RXSound off");
        assert!(!reloaded.music_enabled, "next launch reloads RXMusic off");
        assert_eq!(
            app.ingame_menu.as_ref().map(IngameMenuState::page),
            Some(ingame_menu::MenuPage::Options),
            "each native toggle reopens Options at the existing page"
        );
    }

    #[test]
    fn music_toggle_tracks_actual_and_script_playback_and_missing_audio_fails_typed() {
        let mut ended = new_running_sandbox_app();
        let configured = ended
            .audio
            .as_ref()
            .expect("test audio")
            .options
            .music_enabled;
        ended.runtime_music_enabled = true;
        ended.audio.as_mut().expect("test audio").stop_music();
        let resources = ended
            .runtime_flash_resources()
            .expect("flash resources")
            .clone();
        ended
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("F3 restarts ended playback");
        assert!(ended.runtime_music_enabled);
        assert_eq!(
            ended
                .audio
                .as_ref()
                .expect("test audio")
                .options
                .music_enabled,
            configured
        );
        assert_eq!(
            ended.runtime_flash_message.as_ref().expect("On flash").text,
            resources.music_on_off(true)
        );

        let mut scripted = new_running_sandbox_app();
        scripted.snapshot.audio = vec![AudioCommand::StopMusic];
        scripted.runtime_music_enabled = true;
        scripted.update_audio();
        assert!(!scripted.runtime_music_enabled);
        scripted.snapshot.audio = vec![AudioCommand::PlayMusic {
            name: "missing-script-track.ogg".to_string(),
            looped: false,
        }];
        scripted.update_audio();
        assert!(scripted.runtime_music_enabled);
        assert!(
            scripted
                .audio
                .as_ref()
                .expect("test audio")
                .music_is_playing(),
            "MusicSystem::Execute analogue starts a replacement while enabled"
        );
        scripted
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("actual pending playback toggles off");
        assert!(!scripted.runtime_music_enabled);

        for modifiers in [ModifiersState::empty(), ModifiersState::CONTROL] {
            let mut missing = new_running_sandbox_app();
            missing.audio = None;
            missing
                .handle_modifiers_changed(modifiers)
                .expect("set missing-audio modifier");
            let error = missing
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect_err("missing audio must not fabricate a toggle");
            assert!(error
                .to_string()
                .contains("classic audio system is unavailable"));
            assert!(missing.runtime_flash_message.is_none());
        }
        let mut startup_missing = new_running_sandbox_app();
        startup_missing.return_to_menu();
        startup_missing.audio = None;
        let error = startup_missing
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect_err("startup missing audio must fail typed");
        assert!(error
            .to_string()
            .contains("classic audio system is unavailable"));
        assert!(startup_missing.runtime_flash_message.is_none());
    }

    #[test]
    fn stale_music_worker_cannot_clear_successor_pending_generation() {
        let pending = Arc::new(AtomicU64::new(2));
        drop(PendingMusicLoadGuard(Arc::clone(&pending), 1));
        assert_eq!(pending.load(AtomicOrdering::Acquire), 2);
        drop(PendingMusicLoadGuard(Arc::clone(&pending), 2));
        assert_eq!(pending.load(AtomicOrdering::Acquire), 0);
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
            assert_eq!(
                app.runtime_flash_message
                    .as_ref()
                    .expect("music text lasts more than one draw")
                    .remaining_draws,
                before - 1,
                "layer {layer}"
            );
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
            .expect("push running message");
        assert_f3_renders(&mut message, "message dialog");
        assert_eq!(message.message_dialogs.len(), 1);

        let mut context = new_running_sandbox_app();
        context
            .open_context_menu_at(
                vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                    "Remain open",
                )],
                GuiPoint::new(24.0, 24.0),
            )
            .expect("open context");
        assert_f3_renders(&mut context, "context menu");
        assert!(context.context_menu.is_some());

        let mut scoreboard = new_scoreboard_test_app(
            r#"global func Initialize()
            {
                SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores");
            }"#,
        );
        toggle_scoreboard(&mut scoreboard, ModifiersState::empty());
        assert_f3_renders(&mut scoreboard, "scoreboard");
        assert!(scoreboard.scoreboard_dialog.is_some());

        for mode in [
            SaveBrowserMode::Save {
                suggested_label: "Slot".to_string(),
            },
            SaveBrowserMode::Load,
        ] {
            let mut save_browser = new_running_sandbox_app();
            save_browser.save_browser = Some(SaveBrowserState::new(mode.clone(), Vec::new()));
            save_browser
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("F3 reaches typed save/load state");
            let flash_before = save_browser.runtime_flash_message.clone();
            let mut frame = vec![0x6d; 320 * 200 * 4];
            let error = save_browser
                .render(&mut frame)
                .expect_err("save/load must fail before generic pixels");
            assert_eq!(
                error.downcast_ref::<ClassicParityBoundary>(),
                Some(&ClassicParityBoundary::SaveBrowser(mode.clone()))
            );
            assert!(frame.iter().all(|byte| *byte == 0x6d));
            assert_eq!(save_browser.runtime_flash_message, flash_before);
        }

        for mode in [
            AppObjectMenuMode::Inventory,
            AppObjectMenuMode::Container,
            AppObjectMenuMode::Context,
            AppObjectMenuMode::Build,
        ] {
            let mut object = new_running_sandbox_app();
            assert!(object
                .open_object_menu()
                .expect("open defensive object state"));
            object
                .object_menu
                .as_mut()
                .expect("defensive object state")
                .set_mode_for_parity_test(mode);
            object
                .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
                .expect("F3 reaches typed app-owned object-menu state");
            let flash_before = object.runtime_flash_message.clone();
            let mut frame = vec![0x4c; 320 * 200 * 4];
            let error = object
                .render(&mut frame)
                .expect_err("object menu must fail before generic pixels");
            assert_eq!(
                error.downcast_ref::<ClassicParityBoundary>(),
                Some(&ClassicParityBoundary::AppObjectMenu(mode))
            );
            assert!(frame.iter().all(|byte| *byte == 0x4c));
            assert_eq!(object.runtime_flash_message, flash_before);
            assert!(object.object_menu.is_some());
        }

        let mut observer = new_running_sandbox_app();
        observer
            .engine
            .remove_player(observer.local_owner)
            .expect("remove local player");
        observer.engine.set_local_players([]);
        observer.snapshot = observer.engine.snapshot();
        observer
            .handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .expect("Generic F3 reaches ownerless observer state");
        let mut frame = vec![0x7a; 320 * 200 * 4];
        observer
            .render(&mut frame)
            .expect("ownerless observer viewport renders");
        assert!(frame.iter().any(|byte| *byte != 0x7a));
    }

    /// C++ resolves every path through the live selected configuration object
    /// (C4Config.cpp:1351-1357,1612-1627), so an explicit `/config` selection
    /// has to reach the sound and music resolvers. Neither may rediscover
    /// ambient defaults and read a different tree than the running app.
    #[test]
    fn explicit_config_paths_feed_audio_and_live_user_root() {
        let _lock = env_lock().lock();
        let install = tempdir().expect("audio provenance install root");
        let selected = tempdir().expect("selected user root");
        let ambient = tempdir().expect("ambient user root");
        fs::create_dir_all(install.path().join("planet/System.c4g")).expect("System group");

        // Two distinct trees: only the selected one holds the sample.
        let selected_sound = selected.path().join("Sound.c4g");
        let ambient_sound = ambient.path().join("Sound.c4g");
        fs::create_dir_all(&selected_sound).expect("selected sound group");
        fs::create_dir_all(&ambient_sound).expect("ambient sound group");
        fs::write(selected_sound.join("Selected.wav"), silent_pcm_wav(1_000))
            .expect("selected sample");
        fs::write(ambient_sound.join("Ambient.wav"), silent_pcm_wav(1_000))
            .expect("ambient sample");

        let names = |root: &Path| {
            let (libraries, _) = discover_global_sound_libraries_at(root);
            let mut resolver = SoundResolver::empty();
            resolver.global = libraries;
            resolver.sample_names()
        };
        assert_eq!(names(selected.path()), vec!["selected.wav".to_string()]);
        assert_eq!(names(ambient.path()), vec!["ambient.wav".to_string()]);

        // An explicit config selection whose UserPath points at the selected
        // tree must drive discovery, even while the ambient environment points
        // somewhere else.
        let config_file = install.path().join("explicit.ini");
        fs::write(
            &config_file,
            format!(
                "[General]\nUserPath={}\n",
                selected.path().display()
            ),
        )
        .expect("write explicit config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install.path())),
            ("LC_USER_DATA_DIR", None),
            ("LC_CONFIG_FILE", None),
            ("LC_CONTENT_DIR", Some(selected.path())),
        ]);
        let paths = AppPaths::discover_with_config_file(Some(&config_file))
            .expect("discover from the explicit config");
        assert_eq!(paths.config_file(), config_file);
        assert_eq!(paths.user_data_dir(), selected.path());

        // The resolver built from those paths sees the selected tree only.
        let resolver = SoundResolver::discover_for_paths(Some(&paths));
        assert_eq!(resolver.sample_names(), vec!["selected.wav".to_string()]);

        // A pathless app walks no install media at all rather than guessing.
        assert!(SoundResolver::discover_for_paths(None)
            .sample_names()
            .is_empty());
    
}
