// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! startup_fixture {
    (loader_registrations: $scenario:expr, $scenario_group:expr, $head:expr, $paths:expr $(,)?) => {
        classic_loader_registrations(
            &$scenario,
            &$scenario_group,
            &$head,
            &loader_fixture_definition_load(),
            &$paths,
        )
        .test_value()
    };
    (player_selection_name_activated_color_dw_score_rounds_rounds_won_rounds_lost_total_playing_time: $name:expr, $activated:expr, $color_dw:expr, $score:expr, $rounds:expr, $rounds_won:expr, $rounds_lost:expr, $total_playing_time:expr $(,)?) => {
        clonk_frontend::startup_plrsel::PlrSelPlayer {
            name: $name,
            activated: $activated,
            big_icon: None,
            portrait: None,
            color_dw: $color_dw,
            score: $score,
            rounds: $rounds,
            rounds_won: $rounds_won,
            rounds_lost: $rounds_lost,
            total_playing_time: $total_playing_time,
            comment: String::new(),
        }
    };
    (startup_player: $path:expr, $file_name:expr, $player_file:expr, $render_model:expr $(,)?) => {
        StartupPlayerFile {
            path: $path,
            file_name: $file_name,
            player_file: $player_file,
            render_model: $render_model,
        }
    };
    (loader_group: $priority:expr, $registration_order:expr, $group:expr $(,)?) => {
        LoaderGroupRegistration {
            priority: $priority,
            registration_order: $registration_order,
            group: $group,
        }
    };
}

#[test]
fn two_line_product_logo_keeps_classic_startup_footprint() {
    let (_, logo_y, logo_width, logo_height) = startup_main_logo_geometry(800, 600, 972, 440);
    main_assert_eq!((logo_width, logo_height) => (282, 128), "the two-line logo keeps the classic 960x320 logo's 0.4x height",);

    let first_button = clonk_frontend::main_menu_layout(800, 600).buttons[0];
    main_assert!(logo_y + logo_height < first_button.y, "the startup logo must end above the first main-menu button",);
}

#[test]
fn hud_graphics_receive_canonical_transparent_pixels_from_the_shared_loader() {
    let directory = tempdir();
    let graphics_dir = directory.path().join("Graphics.c4g");
    fs::create_dir(&graphics_dir).test_value();
    let source =
        image::RgbaImage::from_raw(2, 1, vec![255, 127, 63, 0, 200, 100, 50, 1]).test_value();
    for name in [
        "Player.png",
        "Crew.png",
        "Rank.png",
        "Menu.png",
        "Energy.png",
    ] {
        source
            .save(graphics_dir.join(name))
            .unwrap_or_else(|error| panic!("write {name}: {error}"));
    }

    let graphics = GraphicsResource::open(&graphics_dir).test_value();
    let hud = FrontendAssets::load_hud_graphics(&graphics);
    for (name, image) in [
        ("Player.png", hud.player.as_ref()),
        ("Crew.png", hud.crew.as_ref()),
        ("Rank.png", hud.rank.as_ref()),
        ("Menu.png", hud.menu.as_ref()),
        ("Energy.png", hud.energy.as_ref()),
    ] {
        main_assert_eq!(
            image.unwrap_or_else(|| panic!("{name} loaded")).pixels() =>
            &[0, 0, 0, 0, 200, 100, 50, 1],
            "{name} reaches HudGraphics with only exact alpha-zero RGB cleared"
        );
    }
}

fn wait_for_menu_preserving_first_player_dialog(app: &mut GameApp) {
    wait_for_menu_impl(app, false);
}

#[test]
fn positional_mix_uses_player_listener_for_volume_and_viewport_for_pan() {
    let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
    let mut snapshot = make_snapshot(vec![listener.clone()], Vec::new());
    snapshot.players = vec![PlayerState {
        id: 7,
        view_cursor: Some(listener.id),
        // This is the requested camera center, not the live smoothed
        // C4Viewport center used by GetAudibility.
        viewports: vec![clonk_engine::PlayerViewport::new(Vector2::new(5000, 5000))],
        ..Default::default()
    }];
    snapshot.hud.local_players = vec![7];
    let viewports = [audio_viewport(0, 7, Vector2::new(800, 1000))];

    // Volume listens at ViewCursor (150px away), while pan uses the
    // physical viewport center (350px away): 79% and +0.70.
    main_assert_eq!(compute_positional_mix_values(Vector2::new(1150, 1000), &snapshot, &viewports) => (79, 0.7),);
    main_assert_eq!(
        compute_positional_mix_values(Vector2::new(1700, 1000), &snapshot, &viewports) =>
        (0, 1.0),
        "an event at the audibility radius is silent and fully right-panned",
    );
}

#[test]
fn frontend_preinit_reloads_changed_music_and_more_music_catalog() {
    // QuitGame enters C4AS_PreInit, whose unconditional MusicSystem.emplace()
    // destroys the local catalog and reconstructs Music.c4g + MoreMusic.txt
    // before startup or the next full scenario (C4Application.cpp:232-293,
    // 373-400; C4MusicSystem.cpp:37-45,391-434).
    let dir = tempdir();
    let global = dir.path().join("Music.c4g");
    let extras = dir.path().join("Extras");
    fs::create_dir_all(dir.path().join("planet/System.c4g")).test_value();
    fs::create_dir_all(&global).test_value();
    fs::create_dir_all(&extras).test_value();
    fs::write(global.join("Old Base.ogg"), b"old base").test_value();
    fs::write(global.join("Removed.mod"), b"removed").test_value();
    fs::write(extras.join("Old Match.mp3"), b"old wildcard").test_value();
    // \x2A is the byte for '*'; writing it as an escape keeps cloc's Rust
    // filter from misreading the glob's slash-star as a block-comment open
    // and stalling (the bytes are unchanged).
    fs::write(dir.path().join("MoreMusic.txt"), b"Extras/\x2A.mp3\n").test_value();

    let local_scenario = dir.path().join("Local.c4s");
    fs::create_dir_all(&local_scenario).test_value();
    fs::write(local_scenario.join("Local Theme.ogg"), b"local").test_value();

    let user_data = dir.path().join("user");
    let _env_lock = env_lock().lock();
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(dir.path())),
        ("LC_CONTENT_DIR", Some(dir.path())),
        ("LC_USER_DATA_DIR", Some(user_data.as_path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    let mut audio = AudioContext::try_new_with_paths(
        AudioOptions {
            max_channels: 1,
            ..AudioOptions::default()
        },
        Some(&paths),
    )
    .test_value();

    let mut initial = audio.music_resolver.global.filenames();
    initial.sort();
    main_assert_eq!(initial => ["Old Base.ogg", "Old Match.mp3", "Removed.mod"]);
    audio.configure_scenario(Some(&local_scenario));
    let stale_recent = Arc::clone(
        &audio
            .music_resolver
            .resolve("Local Theme")
            .test_value()
            .identity,
    );
    audio
        .music_resolver
        .set_playlist(Some("Local.*".to_string()));
    lock_unpoisoned(&audio.music_control).most_recently_played = Some(stale_recent);
    audio.set_scenario_music_level(Some(25));

    fs::remove_file(global.join("Old Base.ogg")).test_value();
    fs::remove_file(global.join("Removed.mod")).test_value();
    fs::remove_file(extras.join("Old Match.mp3")).test_value();
    fs::write(global.join("New Base.ogg"), b"new base").test_value();
    fs::write(global.join("Frontend.ogg"), b"new frontend").test_value();
    fs::write(extras.join("New Match.ogg"), b"new wildcard").test_value();
    // \x2A is '*' (see note above); bytes identical, dodges the same cloc stall.
    fs::write(
        dir.path().join("MoreMusic.txt"),
        b"Extras/\x2A.ogg\n#clear\nMusic.c4g\nExtras/\x2A.ogg\n",
    )
    .test_value();

    main_assert!(audio.music_resolver.global.resolve("New Base").is_none(), "external edits remain invisible until the next PreInit");
    audio.reset_music_system_generation(Some(&paths));

    let mut reloaded = audio.music_resolver.active_filenames();
    reloaded.sort();
    main_assert_eq!(reloaded => ["Frontend.ogg", "New Base.ogg", "New Match.ogg"]);
    main_assert_eq!(audio.music_resolver.resolve("New Base").expect("reloaded Music.c4g addition").load_audio().expect("read reloaded global track") => b"new base");
    main_assert_eq!(
        audio
            .music_resolver
            .resolve("New Match")
            .expect("changed wildcard match")
            .load_audio()
            .expect("read changed wildcard track") =>
        b"new wildcard"
    );
    for removed in ["Old Base", "Removed", "Old Match", "Local Theme"] {
        main_assert!(audio.music_resolver.resolve(removed).is_none(), "{removed} must not leak into the reconstructed catalog");
    }
    main_assert!(!audio.music_resolver.scenario_has_local_sources);
    main_assert!(audio.music_resolver.scenario_root.is_none());
    main_assert!(audio.music_resolver.playlist.is_none());
    let control = lock_unpoisoned(&audio.music_control);
    main_assert!(control.most_recently_played.is_none());
    main_assert!(control.scenario_level.is_none());
    drop(control);

    audio.prepare_frontend_music();
    main_assert_eq!(audio.music_resolver.playlist.as_deref() => Some("Frontend.*"));
    main_assert_eq!(
        audio
            .music_resolver
            .first_default()
            .map(|asset| asset.file_name_bytes.as_slice()) =>
        Some(b"Frontend.ogg".as_slice()),
        "the ensuing frontend selection must use the rediscovered catalog"
    );

    // A failed startup host/join also runs QuitGame -> PreInit ->
    // DoStartup. Prove that shortcut performs a third discovery before it
    // requests frontend music, rather than retaining this second catalog.
    let controlled_fixture = audio.system.load_music(&silent_pcm_wav(20)).test_value();
    audio.control_music_loads_with(controlled_fixture);
    fs::remove_file(global.join("New Base.ogg")).test_value();
    fs::remove_file(extras.join("New Match.ogg")).test_value();
    fs::write(global.join("Final Base.ogg"), b"final base").test_value();
    fs::write(extras.join("Final Match.ogg"), b"final wildcard").test_value();

    let mut app = new_real_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    app.audio = Some(audio);
    app.resume_frontend_music_after_fade = true;
    app.startup_restart_diagnostics.mark_quit_with_error();
    app.startup_restart_diagnostics
        .add_fatal_error("fixture startup failure");
    app.finish_startup_network_restart(StartupNetworkPurpose::Join)
        .test_value();

    let audio = app.audio.test_ref();
    let mut final_catalog = audio.music_resolver.active_filenames();
    final_catalog.sort();
    main_assert_eq!(final_catalog => ["Final Base.ogg", "Final Match.ogg", "Frontend.ogg"]);
    main_assert_eq!(audio.music_resolver.playlist.as_deref() => Some("Frontend.*"));
    main_assert!(!app.resume_frontend_music_after_fade);
    main_assert!(app.frontend_music_attempted_for_entry);
    let expected_frontend = audio
        .music_resolver
        .first_default()
        .test_value()
        .identity
        .clone();
    let controlled = audio.controlled_music_loads.test_ref();
    main_assert_eq!(controlled.requests.len() => 1);
    let request = controlled.requests.front().test_value();
    main_assert!(!request.looped);
    main_assert!(request.identity.as_ref().is_some_and(|identity| Arc::ptr_eq(identity, &expected_frontend)));

    let pre_console_generation = lock_unpoisoned(&audio.music_control).generation;
    fs::remove_file(global.join("Final Base.ogg")).test_value();
    fs::remove_file(extras.join("Final Match.ogg")).test_value();
    fs::write(global.join("Console Only.ogg"), b"console").test_value();
    app.console_mode = true;
    app.resume_frontend_music_after_fade = true;
    app.startup_restart_diagnostics.mark_quit_with_error();
    app.startup_restart_diagnostics
        .add_fatal_error("fixture console failure");
    app.finish_startup_network_restart(StartupNetworkPurpose::Join)
        .test_value();

    let audio = app.audio.test_ref();
    main_assert_eq!(lock_unpoisoned(&audio.music_control).generation => pre_console_generation);
    let mut retained_catalog = audio.music_resolver.active_filenames();
    retained_catalog.sort();
    main_assert_eq!(retained_catalog => ["Final Base.ogg", "Final Match.ogg", "Frontend.ogg"]);
    main_assert!(audio.music_resolver.resolve("Console Only").is_none());
    main_assert_eq!(
        audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading")
            .requests
            .len() =>
        1,
        "console failure must not run C4Startup::DoStartup again"
    );
    main_assert!(app.resume_frontend_music_after_fade);

    app.console_mode = false;
    app.classic_command_line.scenario = Some(dir.path().join("Explicit.c4s"));
    app.startup_restart_diagnostics.mark_quit_with_error();
    app.startup_restart_diagnostics
        .add_fatal_error("fixture command-line failure");
    app.finish_startup_network_restart(StartupNetworkPurpose::Join)
        .test_value();
    let audio = app.audio.test_ref();
    main_assert_eq!(lock_unpoisoned(&audio.music_control).generation => pre_console_generation);
    main_assert!(audio.music_resolver.resolve("Console Only").is_none());
    main_assert_eq!(audio.controlled_music_loads.as_ref().expect("controlled music loading").requests.len() => 1);

    app.classic_command_line.scenario = None;
    app.classic_command_line.record_stream = Some(PathBuf::from("record.example:11114"));
    main_assert!(!app.failed_open_game_returns_to_startup());
    app.classic_command_line.record_stream = Some(PathBuf::new());
    app.classic_command_line.direct_join = Some(String::new());
    main_assert!(app.failed_open_game_returns_to_startup(), "native suppresses startup only for nonempty command-line buffers");

    // User-aborting the pre-game lobby also makes OpenGame return false.
    // Its ordinary startup lineage must run the same QuitGame -> PreInit
    // reconstruction, while the earlier console edit is still pending.
    app.classic_command_line = ClassicCommandLine::default();
    let pre_lobby_generation = lock_unpoisoned(&app.audio.test_ref().music_control).generation;
    install_test_classic_host_lobby(&mut app);
    app.process_classic_lobby_actions(vec![ClassicLobbyAction::ExitRequested])
        .test_value();
    let audio = app.audio.test_ref();
    main_assert!(lock_unpoisoned(&audio.music_control).generation > pre_lobby_generation);
    main_assert!(audio.music_resolver.resolve("Console Only").is_some());
    main_assert_eq!(
        audio
            .controlled_music_loads
            .as_ref()
            .expect("controlled music loading")
            .requests
            .len() =>
        2,
        "lobby abort requests frontend music exactly once after rediscovery"
    );
}

/// `C4StartupAboutDlg`'s constructor resolves its title and the Back, Check for
/// updates and Licenses captions through `LoadResStr`
/// (C4StartupAboutDlg.cpp:262,279-284). The mnemonic markers travel with the
/// resolved text, because `Button::SetText` is what turns them into hotkeys.
#[test]
fn about_chrome_uses_runtime_resource_strings() {
    let mut app = new_real_classic_menu_app(640, 480);
    for (key, value) in [
        ("IDS_DLG_ABOUT", "&Programminfo"),
        ("IDS_BTN_BACK", "Zurueck"),
        ("IDS_BTN_CHECKFORUPDATES", "Nach &Updates suchen"),
        ("IDS_BTN_LICENSES", "&Lizenzen"),
    ] {
        app.startup_tooltip_resources
            .insert(key.to_string(), value.to_string());
    }
    app.open_about_dialog();
    main_assert_eq!(
        app.startup_about_dialog
            .as_ref()
            .expect("about dialog")
            .labels()
            .buttons =>
        [
            "Zurueck".to_string(),
            "Nach &Updates suchen".to_string(),
            "&Lizenzen".to_string(),
        ]
    );

    // The title strip still reads the same key it always did.
    let about = clonk_frontend::startup_about_dlg::about_layout(640, 480);
    let at_anchor = GuiPoint::new(
        about.title_anchor.0 as f32,
        about.title_anchor.1 as f32 + 1.0,
    );
    main_assert_eq!(app.about_tooltip_target_at(at_anchor) => Some(StartupTooltip::text("&Programminfo")));

    // The relocated mnemonic activates, and the old English one does not.
    app.keyboard_modifiers = ModifiersState::ALT;
    app.test_key(VirtualKeyCode::KeyU, ElementState::Pressed);
    main_assert_eq!(app.message_dialogs.len() => 1);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    app.test_key(VirtualKeyCode::KeyL, ElementState::Pressed);
    main_assert_eq!(app.startup_about_dialog.as_ref().expect("about dialog").current_page() => clonk_frontend::startup_about_dlg::AboutPage::Licenses);
}

/// `C4StartupOptionsDlg`'s constructor resolves every caption, label, button
/// and checkbox text through `LoadResStr` (C4StartupOptionsDlg.cpp:609-1033),
/// so the whole subtree follows the active language - including the widths it
/// measures from that text (`:687-697`).
#[test]
fn startup_options_visible_labels_follow_runtime_resources() {
    let mut app = new_real_classic_menu_app(640, 480);
    for (key, value) in [
        ("IDS_DLG_OPTIONS", "&Einstellungen"),
        ("IDS_DLG_PROGRAM", "Programm"),
        ("IDS_DLG_GRAPHICS", "Grafik"),
        ("IDS_DLG_SOUND", "Ton"),
        ("IDS_DLG_AUDIO", "Klang"),
        ("IDS_DLG_KEYBOARD", "Tastatur"),
        ("IDS_DLG_GAMEPAD", "Gamepad"),
        ("IDS_DLG_NETWORK", "Netzwerk"),
        ("IDS_BTN_BACK", "Zurueck"),
        ("IDS_CTL_LANGUAGE", "Sprache"),
        ("IDS_CTL_FONT", "Schriftart"),
        ("IDS_MNU_WHITECHAT", "Weisser Chat"),
        ("IDS_CTL_WHITECHAT_INGAME", "Im Spiel"),
        ("IDS_CTL_WHITECHAT_LOBBY", "Lobby"),
        ("IDS_CTL_TIMESTAMPS", "Zeitstempel"),
        ("IDS_CTL_PRELOADING", "Spieldaten vorladen"),
        ("IDS_BTN_RESETCONFIG", "Konfiguration zuruecksetzen"),
        ("IDS_DLG_ADVANCED_SETTINGS", "Erweiterte Einstellungen"),
        ("IDS_CTL_MUSIC", "Musik"),
        ("IDS_CTL_SOUNDFX", "Klaenge"),
        ("IDS_CTL_DISPLAYMODE", "Anzeigemodus"),
        ("IDS_CTL_GRAPHICSSCALE", "Skalierung"),
        ("IDS_CTL_SMOKELOW", "Niedrig"),
        ("IDS_CTL_SMOKEHI", "Hoch"),
        ("IDS_BTN_TESTGRAPHICSSCALE", "Anwenden"),
        ("IDS_BTN_RESETKEYBOARD", "Alle zuruecksetzen"),
        ("IDS_NET_PORT_REFERENCE", "Referenzport"),
        ("IDS_NET_PORT_DISCOVERY", "Suchport"),
        ("IDS_CTL_ACTIVE", "Aktiv"),
        ("IDS_CTL_USEOTHERSERVER", "Anderen Internetserver verwenden"),
        (
            "IDS_CTL_AUTOMATICUPDATES",
            "Automatische Updates aktivieren",
        ),
        ("IDS_CTL_UPNP", "UPnP verwenden"),
        ("IDS_NET_COMPUTERNAME", "Computername:"),
        ("IDS_NET_USERNAME", "Chatname:"),
        (
            "IDS_CTL_FAIRCREWSTRENGTH",
            "Staerke der \"Fairen Mannschaft\"",
        ),
        ("IDS_CTL_FAIRCREWWEAK", "Schwach"),
        ("IDS_CTL_FAIRCREWSTRONG", "Stark"),
        ("IDS_CTL_DIG", "Graben"),
        ("IDS_NET_PORT_TCP", "TCP-Anschluss"),
        ("IDS_NET_PORT_UDP", "UDP-Anschluss"),
        ("IDS_CTL_NOLANGINFO", "Sprachpaket nicht verfuegbar."),
    ] {
        app.startup_tooltip_resources
            .insert(key.to_string(), value.to_string());
    }
    app.open_options_menu();
    let labels = app.startup_options_dialog.test_ref().labels().clone();

    // The caption drops its mnemonic marker like every FullscreenDialog title.
    main_assert_eq!(labels.title => "Einstellungen");
    // The third sheet reads the port-only `IDS_DLG_AUDIO`, not the C++
    // `IDS_DLG_SOUND` the ingame menu still uses for its own "Sound" entry:
    // the port hosts the voice-chat group there too (clonk-org/clonk-rs#452).
    main_assert_eq!(labels.sheets => ["Programm", "Grafik", "Klang", "Tastatur", "Gamepad", "Netzwerk"].map(str::to_string));
    main_assert_eq!(labels.back => "Zurueck");
    main_assert_eq!(labels.language => "Sprache");
    main_assert_eq!(labels.reset_config => "Konfiguration zuruecksetzen");
    main_assert_eq!(labels.port_reference => "Referenzport");
    main_assert_eq!(labels.active => "Aktiv");
    main_assert_eq!(labels.chat_name => "Chatname:");
    // The fair-crew child captions and network port group titles are resolved
    // at construction (`C4StartupOptionsDlg.cpp:768-773,996-999`), not only
    // when a control is activated.
    main_assert_eq!(labels.fair_crew_weak => "Schwach");
    main_assert_eq!(labels.fair_crew_strong => "Stark");
    main_assert_eq!(labels.port_tcp => "TCP-Anschluss");
    main_assert_eq!(labels.port_udp => "UDP-Anschluss");
    main_assert_eq!(labels.fair_crew_strength => "Staerke der \"Fairen Mannschaft\"");

    // A key absent from the table falls back to the shipped US text, which is
    // what C4ResStrTable itself yields.
    app.startup_tooltip_resources.remove("IDS_CTL_LANGUAGE");
    app.open_options_menu();
    main_assert_eq!(app.startup_options_dialog.as_ref().expect("options dialog").labels().language => "Language");

    // KeySelDialog uses the same localized action name as the control button
    // (`C4StartupOptionsDlg.cpp:160-177`).
    for (key, value) in [
        (
            "IDS_MSG_PRESSKEY",
            "Taste fuer \"%s\" auf Tastaturblock %d druecken.",
        ),
        ("IDS_MSG_DEFINEKEY", "Taste zuweisen"),
    ] {
        app.startup_tooltip_resources
            .insert(key.to_string(), value.to_string());
    }
    app.process_options_dialog_actions(vec![
        clonk_frontend::startup_options_dlg::OptionsDlgAction::BeginControlCapture(
            clonk_frontend::startup_options_controls::ControlCaptureTarget {
                device: clonk_frontend::startup_options_controls::ControlDevice::Keyboard,
                set: 2,
                control: 5,
            },
        ),
    ])
    .test_value();
    let capture = app.message_dialogs.last().test_value();
    main_assert_eq!(capture.state.message() => "Taste fuer \"Graben\" auf Tastaturblock 3 druecken.");
    main_assert_eq!(capture.state.caption() => "Taste zuweisen");

    // The nested key-capture and resolution-confirm dialogs are resources too,
    // including their positional `%s`/`%d`/`%u` arguments.
    for (key, value) in [
        ("IDS_MNU_SWITCHRESOLUTION", "Aufloesung wechseln"),
        (
            "IDS_MNU_SWITCHRESOLUTION_TEXT",
            "Neue Aufloesung. Gefaellt sie?|Wird in %u Sekunden zurueckgesetzt...",
        ),
    ] {
        app.startup_tooltip_resources
            .insert(key.to_string(), value.to_string());
    }
    app.begin_options_scale_test(100, 150).test_value();
    let confirm = app.message_dialogs.last().test_value();
    main_assert_eq!(confirm.state.caption() => "Aufloesung wechseln");
    main_assert_eq!(confirm.state.message() => "Neue Aufloesung. Gefaellt sie?|Wird in 12 Sekunden zurueckgesetzt...");
    app.tick_options_scale_test_prompt();
    main_assert_eq!(app.message_dialogs.last().expect("confirmation").state.message() => "Neue Aufloesung. Gefaellt sie?|Wird in 11 Sekunden zurueckgesetzt...");

    // Layout measures the resolved text, so a longer label widens its column.
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let book = app.assets.options_book_fonts.as_deref().test_value();
    let wide = clonk_frontend::startup_options_dlg::OptionsLabels {
        language: "Sprachauswahl fuer das gesamte Programm".to_string(),
        fair_crew_weak: "Sehr schwache Mannschaft".to_string(),
        fair_crew_strong: "Aussergewoehnlich starke Mannschaft".to_string(),
        ..Default::default()
    };
    let narrow = clonk_frontend::startup_options_dlg::options_dlg_layout_with_labels(
        640,
        480,
        fonts,
        book,
        &clonk_frontend::startup_options_dlg::OptionsLabels::default(),
    );
    let widened = clonk_frontend::startup_options_dlg::options_dlg_layout_with_labels(
        640, 480, fonts, book, &wide,
    );
    main_assert!(
        widened.language_combo.x > narrow.language_combo.x,
        "a longer resolved label must push its combo right: {} vs {}",
        widened.language_combo.x,
        narrow.language_combo.x
    );
    main_assert!(
        widened.weak_label.w > narrow.weak_label.w
            && widened.strong_label.w > narrow.strong_label.w,
        "fair-crew labels must measure resolved child captions: weak {} vs {}, strong {} vs {}",
        widened.weak_label.w,
        narrow.weak_label.w,
        widened.strong_label.w,
        narrow.strong_label.w
    );
}

#[test]
fn startup_fullscreen_title_tooltips_follow_active_language_amp_rules() {
    let mut app = new_real_classic_menu_app(640, 480);
    for (key, value) in [
        ("IDS_DLG_NETSTART", "&Netzwerkstart"),
        ("IDS_DLG_ABOUT", "&Programminfo"),
        ("IDS_DLG_OPTIONS", "&Einstellungen"),
        ("IDS_DLG_PLAYERSELECTION", "&Spielerauswahl"),
        ("IDS_CTL_CREW", "&Mannschaft:"),
        ("IDS_DLG_STARTGAME", "&Lokales Spiel"),
    ] {
        app.startup_tooltip_resources
            .insert(key.to_string(), value.to_string());
    }
    let at_anchor = |anchor: (i32, i32)| GuiPoint::new(anchor.0 as f32, anchor.1 as f32 + 1.0);
    let fonts = Arc::clone(app.assets.clonk_fonts.test_ref());

    let net_metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts(fonts.as_ref());
    let mut network = clonk_frontend::startup_netdlg::NetDlgController::new(
        clonk_frontend::startup_netdlg::NetDlgConfig::default(),
        net_metrics,
    );
    network.resize(640, 480);
    app.startup_network_dialog = Some(network);
    let net_layout = clonk_frontend::startup_netdlg::net_dlg_layout(640, 480, &net_metrics);
    main_assert_eq!(app.network_game_tooltip_target_at(at_anchor(net_layout.title_anchor)) => Some(StartupTooltip::text("&Netzwerkstart")));

    app.open_about_dialog();
    let about = clonk_frontend::startup_about_dlg::about_layout(640, 480);
    main_assert_eq!(app.about_tooltip_target_at(at_anchor(about.title_anchor)) => Some(StartupTooltip::text("&Programminfo")));

    app.open_options_menu();
    let options_book = app.assets.options_book_fonts.as_deref().test_value();
    let options = clonk_frontend::startup_options_dlg::options_dlg_layout(
        640,
        480,
        fonts.as_ref(),
        options_book,
    );
    main_assert_eq!(app.options_tooltip_target_at(at_anchor(options.title_center)) => Some(StartupTooltip::text("Einstellungen")));

    app.open_player_selection_dialog();
    let player_layout = clonk_frontend::startup_plrsel::plrsel_layout(640, 480);
    main_assert_eq!(app.player_selection_tooltip_target_at(at_anchor(player_layout.title_anchor)) => Some(StartupTooltip::text("Spielerauswahl")));
    let player_dialog = app.startup_player_dialog.test_mut();
    player_dialog.set_player_count(1);
    main_assert!(player_dialog.enter_crew_mode(0, "Ada", vec![true]));
    main_assert_eq!(app.player_selection_tooltip_target_at(at_anchor(player_layout.title_anchor)) => Some(StartupTooltip::text("Mannschaft: Ada")));
    app.startup_tooltip
        .note_pointer_move(GuiPoint::new(10.0, 10.0));
    app.leave_startup_crew_mode();
    main_assert_eq!(app.startup_tooltip.pointer_position() => None);

    app.open_scenario_browser();
    let scenario = clonk_frontend::startup_scensel::scen_sel_layout(640, 480, fonts.as_ref());
    main_assert_eq!(app.scenario_browser_tooltip_target_at(at_anchor(scenario.title_anchor)) => Some(StartupTooltip::text("Lokales Spiel")));
}

#[test]
fn startup_irc_snapshot_projects_legacy_bytes_without_utf8_reinterpretation() {
    let raw = vec![0xc3, 0xa9];
    let mut channel_name = b"#".to_vec();
    channel_name.extend_from_slice(&raw);
    let snapshot = clonk_network::IrcClientSnapshot {
        connection_state: clonk_network::IrcConnectionState::Connected,
        nick: raw.clone(),
        prefixes: b"(ov)@+".to_vec(),
        channels: vec![clonk_network::IrcChannel {
            name: channel_name.clone(),
            topic: raw.clone(),
            users: vec![clonk_network::IrcUser {
                prefix: b"@".to_vec(),
                name: raw.clone(),
            }],
            receiving_users: false,
        }],
        messages: vec![clonk_network::IrcMessage {
            timestamp: std::time::SystemTime::UNIX_EPOCH,
            message_type: clonk_network::IrcMessageType::Message,
            source: raw.clone(),
            target: channel_name,
            data: raw.clone(),
        }],
        unread_index: 0,
        last_error: None,
    };

    let projected = project_startup_irc_snapshot("irc.example.test", snapshot);
    let presented = "\u{00c3}\u{00a9}";
    main_assert_eq!(projected.nick => presented);
    main_assert_eq!(projected.channels[0].name => format!("#{presented}"));
    main_assert_eq!(projected.channels[0].topic => presented);
    main_assert_eq!(projected.channels[0].users[0].prefix => "@");
    main_assert_eq!(projected.channels[0].users[0].name => presented);
    main_assert_eq!(projected.messages[0].source => presented);
    main_assert_eq!(projected.messages[0].target => format!("#{presented}"));
    main_assert_eq!(projected.messages[0].text => presented);
    main_assert!(projected.messages[0].is_channel);
    for text in [
        &projected.nick,
        &projected.channels[0].topic,
        &projected.channels[0].users[0].name,
        &projected.messages[0].source,
        &projected.messages[0].text,
    ] {
        main_assert_eq!(encode_startup_irc_text(text) => Some(raw.clone()));
    }
}

#[test]
fn startup_irc_warning_persists_login_and_checkbox_on_cancel_then_connects_on_ok() {
    use clonk_frontend::message_dialog::{
        MessageDialogButton, MessageDialogButtons, MessageDialogIcon, MessageDialogResult,
    };

    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "Startup", "HideMsgIRCDangerous", "0").test_value();
    persist_config_value(&paths, "IRC", "Server2", "irc.configured.test").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    install_classic_test_assets(&mut app);
    let cancelled = clonk_frontend::startup_netdlg::NetDlgChatLogin {
        server: "irc.cancelled.test".into(),
        nick: "SavedNick".into(),
        password: "transient-secret".into(),
        real_name: "Saved Name".into(),
        channel: "#saved".into(),
    };

    app.request_startup_irc_connection(cancelled.clone())
        .test_value();
    let warning = app.message_dialogs.last_mut().test_value();
    main_assert_eq!(warning.state.caption() => "Chat - Disclaimer");
    main_assert!(warning.state.message().contains("irc.cancelled.test"));
    main_assert_eq!(warning.state.buttons() => MessageDialogButtons::OK_CANCEL);
    main_assert_eq!(warning.state.icon() => MessageDialogIcon::NOTIFY);
    main_assert_eq!(warning.state.focused_button() => Some(MessageDialogButton::Ok));
    main_assert!(matches!(&warning.continuation, MessageDialogContinuation::StartupIrcConnectWarning { login } if login == &cancelled));
    main_assert_eq!(warning.state.handle_hotkey('d') => None);
    app.persist_top_message_dialog_checkbox_changes();
    app.finish_message_dialog(MessageDialogResult::Cancel)
        .test_value();
    main_assert!(app.startup_irc_client.is_none());

    let persisted = Config::load(paths.config_file()).test_value();
    main_assert_eq!(persisted.get_in(Some("IRC"), "Nick") => Some("SavedNick"));
    main_assert_eq!(persisted.get_in(Some("IRC"), "RealName") => Some("Saved Name"));
    main_assert_eq!(persisted.get_in(Some("IRC"), "Channel") => Some("#saved"));
    main_assert_eq!(persisted.get_in(Some("IRC"), "Password") => None);
    main_assert_eq!(persisted.get_in(Some("IRC"), "Server2") => Some("irc.configured.test"), "Connect persists form fields without replacing configured Server2");
    main_assert_eq!(persisted.get_in(Some("Startup"), "HideMsgIRCDangerous") => Some("1"), "the don't-show choice persists even when the connection is cancelled");

    persist_config_value(&paths, "Startup", "HideMsgIRCDangerous", "0").test_value();
    let (address, server) = spawn_loopback_irc_server();
    let accepted = clonk_frontend::startup_netdlg::NetDlgChatLogin {
        server: address,
        nick: "AcceptedNick".into(),
        password: "another-secret".into(),
        real_name: "Accepted Name".into(),
        channel: "#accepted".into(),
    };
    app.request_startup_irc_connection(accepted).test_value();
    main_assert!(app.startup_irc_client.is_none());
    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    let client = app.startup_irc_client.test_ref();
    main_assert!(matches!(client.recv_event_timeout(Duration::from_secs(2)), Ok(clonk_network::IrcClientEvent::Connected)));
    drop(app);
    server.test_join();
    reset_cached_app_paths();
}

#[test]
fn startup_irc_frontend_switches_and_renders_without_a_fail_closed_boundary() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").test_value();
    let address = listener.local_addr().test_value();
    let server = thread::spawn(move || {
        use std::io::Read as _;

        let (mut stream, _) = listener.accept().test_value();
        let mut buffer = [0_u8; 512];
        while stream.read(&mut buffer).is_ok_and(|read| read != 0) {}
    });
    let handle = clonk_network::IrcClientHandle::connect_with_timeout(
        clonk_network::IrcConnectConfig::new(
            address.to_string(),
            b"Clonker".to_vec(),
            b"Clonker".to_vec(),
        ),
        Duration::from_secs(2),
    )
    .test_value();
    main_assert!(matches!(handle.recv_event_timeout(Duration::from_secs(2)), Ok(clonk_network::IrcClientEvent::Connected)));

    let mut app = new_real_classic_menu_app(640, 480);
    app.startup_irc_server = address.to_string();
    app.startup_irc_client = Some(handle);
    app.open_network_game_dialog();
    let browser_status = app.status_text.clone();
    activate_startup_network_chat(&mut app);
    main_assert!(app.network.is_none());
    main_assert_eq!(app.status_text => browser_status);
    main_assert_eq!(app.startup_network_dialog.as_ref().unwrap().mode() => clonk_frontend::startup_netdlg::NetDlgMode::Chat);
    main_assert_eq!(app.startup_network_dialog.as_ref().unwrap().chat_connection_state() => clonk_frontend::startup_netdlg::NetDlgChatConnectionState::Connected);

    let mut frame = vec![0xa5; 640 * 480 * 4];
    app.test_render(&mut frame);
    main_assert!(frame.iter().any(|byte| *byte != 0xa5));

    let metrics = clonk_frontend::startup_netdlg::NetDlgFontMetrics {
        caption_back_extent: 51,
        text_ip_extent: 18,
        text_line_height: 22,
        caption_line_height: 25,
        title_line_height: 34,
    };
    let button = clonk_frontend::startup_netdlg::net_dlg_layout(640, 480, &metrics).btn_game_list;
    let point = PhysicalPosition::new(
        f64::from(button.x + button.w / 2),
        f64::from(button.y + button.h / 2),
    );
    app.test_cursor(point);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.startup_network_dialog.as_ref().unwrap().mode() => clonk_frontend::startup_netdlg::NetDlgMode::GameList);

    app.show_main_menu();
    app.open_network_game_dialog();
    activate_startup_network_chat(&mut app);
    main_assert!(app.startup_irc_client.is_some());
    main_assert_eq!(
        app.startup_network_dialog
            .as_ref()
            .unwrap()
            .chat_connection_state() =>
        clonk_frontend::startup_netdlg::NetDlgChatConnectionState::Connected,
        "the process-global IRC client survives startup-screen replacement"
    );
    drop(app);
    server.test_join();
}

#[test]
fn startup_irc_command_projection_covers_the_frontend_command_language() {
    use clonk_frontend::startup_netdlg::NetDlgChatCommand as Frontend;
    use clonk_network::IrcCommand as Backend;

    let cases = [
        (
            Frontend::Quit {
                reason: "bye".into(),
            },
            Backend::Quit {
                reason: b"bye".to_vec(),
            },
        ),
        (
            Frontend::Join {
                channel: "#clonk".into(),
            },
            Backend::Join {
                channel: b"#clonk".to_vec(),
            },
        ),
        (
            Frontend::Part {
                channel: "#clonk".into(),
            },
            Backend::Part {
                channel: b"#clonk".to_vec(),
            },
        ),
        (
            Frontend::Message {
                target: "#clonk".into(),
                text: "\u{00c3}\u{00a9}".into(),
            },
            Backend::Message {
                target: b"#clonk".to_vec(),
                text: vec![0xc3, 0xa9],
            },
        ),
        (
            Frontend::Notice {
                target: "Clonker".into(),
                text: "notice".into(),
            },
            Backend::Notice {
                target: b"Clonker".to_vec(),
                text: b"notice".to_vec(),
            },
        ),
        (
            Frontend::Action {
                target: "#clonk".into(),
                text: "waves".into(),
            },
            Backend::Action {
                target: b"#clonk".to_vec(),
                text: b"waves".to_vec(),
            },
        ),
        (
            Frontend::Raw("WHOIS Clonker".into()),
            Backend::Raw(b"WHOIS Clonker".to_vec()),
        ),
        (
            Frontend::ChangeNick {
                nick: "Clonker_".into(),
            },
            Backend::ChangeNick {
                nick: b"Clonker_".to_vec(),
            },
        ),
    ];
    for (frontend, backend) in cases {
        main_assert_eq!(project_startup_irc_command(frontend) => Some(backend));
    }
    main_assert_eq!(project_startup_irc_command(Frontend::OpenQuery {nick: "Clonker".into(),}) => None, "query tabs are a frontend-only operation");
    main_assert_eq!(
        project_startup_irc_command(Frontend::Raw("snowman \u{2603}".into())) =>
        None,
        "unrepresentable presentation text must not reach the byte transport"
    );
}

#[test]
fn missing_startup_models_precede_status_and_leave_pixels_untouched() {
    let mut app = new_real_classic_menu_app(320, 200);
    let cases = [
        (StartupView::NetworkGame, "C4StartupNetDlg"),
        (StartupView::PlayerSelection, "C4StartupPlrSelDlg"),
        (StartupView::Options, "C4StartupOptionsDlg"),
        (StartupView::About, "C4StartupAboutDlg"),
    ];

    for (view, missing) in cases {
        app.startup_view = view;
        app.startup_network_dialog = None;
        app.startup_player_dialog = None;
        app.startup_options_dialog = None;
        app.startup_about_dialog = None;
        app.status_text = "model boundary wins".to_string();
        let expected = ClassicParityBoundary::StartupModel { view, missing };
        let mut frame = vec![0x7c; 320 * 200 * 4];

        let error = app.render(&mut frame).expect_err("missing startup model");
        main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&expected));
        main_assert!(frame.iter().all(|byte| *byte == 0x7c));

        let mut native = vec![0x48; 640 * 400 * 4];
        let error = app
            .render_native_main_menu_text(&mut native, 640, 400)
            .expect_err("native pass must reject missing model");
        main_assert_eq!(error.downcast_ref::<ClassicParityBoundary>() => Some(&expected));
        main_assert!(native.iter().all(|byte| *byte == 0x48));
    }
}

/// The message dialog a finished check left behind.
fn update_result_dialog(app: &GameApp) -> &PendingMessageDialog {
    app.message_dialogs.last().test_value()
}

#[test]
fn an_available_update_opens_the_localized_yes_no_prompt() {
    // `C4UpdateDlg.cpp:383-385`: IDS_MSG_ANUPDATETOVERSIONISAVAILA under
    // btnYesNo and Ico_Ex_Update. C++ captions it with the update server
    // address; this port captions the whole check flow with the command that
    // opened it (see `game_app::update`).
    use crate::update_check::test_support::{manifest_for, FakeTransport, OFFERED_VERSION};

    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths);
    let transport = FakeTransport::serving(&manifest_for(
        OFFERED_VERSION,
        clonk_core::version::ENGINE_VERSION,
    ));

    app.check_for_updates_with(false, &transport).test_value();

    let prompt = update_result_dialog(&app);
    main_assert_eq!(prompt.state.message() => "An update to version 99.0.0 is available. \
             Do you want to download and install this update?");
    main_assert_eq!(prompt.state.caption() => "Check for Updates");
    main_assert_eq!(prompt.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::YES_NO);
    main_assert_eq!(prompt.state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Extended(14));
    main_assert!(matches!(prompt.continuation, MessageDialogContinuation::UpdatePrompt { .. }));

    // Declining is silent, exactly as C++'s ShowMessageModal returning
    // false is (`C4UpdateDlg.cpp:385-394`).
    let mut declined = new_classic_menu_app(640, 480);
    declined
        .check_for_updates_with(false, &transport)
        .test_value();
    declined
        .finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();
    main_assert!(declined.message_dialogs.is_empty());

    // Accepting starts the cancellable component download. The test build
    // parks its network worker, so this pins the hand-off without touching
    // the network.
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();
    let download = update_result_dialog(&app);
    main_assert_eq!(download.state.message() => "Downloading update 99.0.0...");
    main_assert_eq!(download.state.caption() => "Check for Updates");
    main_assert_eq!(download.state.progress() => Some(0));
    main_assert_eq!(download.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::CANCEL);
    main_assert!(app.update_download.is_some());

    app.check_for_updates_at(false, 1_000).test_value();
    main_assert!(app.update_check.is_none());
    main_assert_eq!(update_result_dialog(&app).state.message() => "Update still in progress. Please wait.");
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    main_assert!(app.update_download.is_none());
    main_assert!(app.message_dialogs.is_empty());
}

#[test]
fn only_a_manual_check_reports_that_there_is_no_update() {
    // `C4UpdateDlg.cpp:396-400`: the "no update" message is suppressed for
    // an automatic check, so a daily background check is silent.
    use crate::update_check::test_support::{manifest_for, FakeTransport};

    let transport = FakeTransport::serving(&manifest_for(
        clonk_core::version::PORT_VERSION,
        clonk_core::version::ENGINE_VERSION,
    ));

    let mut manual = new_classic_menu_app(640, 480);
    manual
        .check_for_updates_with(false, &transport)
        .test_value();
    main_assert_eq!(update_result_dialog(&manual).state.message() => format!("Clonk Rust {} is the latest version.", clonk_core::version::PORT_VERSION));
    main_assert_eq!(update_result_dialog(&manual).state.caption() => "Check for Updates");
    main_assert_eq!(update_result_dialog(&manual).state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::OK);

    let mut automatic = new_classic_menu_app(640, 480);
    automatic
        .check_for_updates_with(true, &transport)
        .test_value();
    main_assert!(automatic.message_dialogs.is_empty());
}

#[test]
fn a_failed_check_reports_the_transport_error_after_the_localized_prefix() {
    // `C4UpdateDlg.cpp:308-322` appends the client's own message to
    // IDS_MSG_UPDATEFAILED, and does so for automatic checks too.
    use crate::update_check::test_support::FakeTransport;

    let mut app = new_classic_menu_app(640, 480);
    app.check_for_updates_with(true, &FakeTransport::failing("https://mirror.invalid/u"))
        .test_value();

    let failure = update_result_dialog(&app);
    main_assert!(failure.state.message().starts_with("Update failed.: "), "{}", failure.state.message());
    main_assert!(failure.state.message().contains("503"));
    // Never "Error" in a title bar: the caption names the command, and the
    // failure itself is the body.
    main_assert_eq!(failure.state.caption() => "Check for Updates");
    main_assert_eq!(failure.state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Extended(14));
}

#[test]
fn a_release_built_against_another_engine_asks_for_a_manual_install() {
    // C++ hides this behind IsValidUpdate and says "no update available"
    // (`C4UpdateDlg.cpp:248`). The release does exist here, and installing
    // its content under the running engine would silently prune the game's
    // definitions, so it is reported as an update to install by hand.
    use crate::update_check::test_support::{manifest_for, FakeTransport, OFFERED_VERSION};

    let [major, minor, objects, revision, build] = clonk_core::version::ENGINE_VERSION;
    let mut app = new_classic_menu_app(640, 480);
    app.check_for_updates_with(
        false,
        &FakeTransport::serving(&manifest_for(
            OFFERED_VERSION,
            [major, minor, objects + 1, revision, build],
        )),
    )
    .test_value();

    main_assert_eq!(
        update_result_dialog(&app).state.message() =>
        "Version 99.0.0 cannot be installed from within the game. \
             Please install it manually."
    );
}

#[test]
fn an_incoming_update_package_is_refused_instead_of_executed() {
    // C++'s ApplyUpdate extracts the update program out of the handed-in
    // group and runs it (`C4UpdateDlg.cpp:171-215`). A file argument is
    // not a reason to execute a program, so this port refuses.
    let mut app = new_classic_menu_app(640, 480);

    app.refuse_incoming_update(Path::new("Patch.c4u"))
        .test_value();

    let refusal = update_result_dialog(&app);
    main_assert_eq!(refusal.state.caption() => "Update");
    main_assert_eq!(refusal.state.message() => "Update failed.");
    main_assert!(app.update_check.is_none(), "nothing is fetched for a package");
}

#[test]
fn the_automatic_check_is_throttled_to_once_a_day_and_records_every_attempt() {
    // `C4UpdateDlg.cpp:264-268`: an automatic check runs at most daily; a
    // manual one ignores the gate; and the attempt is recorded either way,
    // successful or not.
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "Network", "LastUpdateTime", "1000").test_value();

    let mut app = new_classic_menu_app(640, 480);
    app.app_paths = Some(paths.clone());
    // C4UpdateDlg.cpp:264-268 records the attempt before the check; this
    // port's immediate config rewrite must also carry the in-memory Display
    // values that C++ would retain in its global Config.
    app.display_flags.player_names = false;
    app.display_flags.clonk_names = false;
    app.display_flags.clock = true;
    app.display_flags.fps = true;
    app.display_flags.upper_board = UpperBoardMode::Small;
    app.defer_display_toggle(DisplayToggle::PlayerNames);
    app.defer_display_toggle(DisplayToggle::ClonkNames);
    app.defer_display_toggle(DisplayToggle::Clock);
    app.defer_display_toggle(DisplayToggle::Fps);
    app.defer_display_toggle(DisplayToggle::UpperBoard);

    app.check_for_updates_at(true, 1000 + 60 * 60 * 24 - 1)
        .test_value();
    main_assert!(app.update_check.is_none(), "an automatic check is daily");
    main_assert!(app.message_dialogs.is_empty());

    app.check_for_updates_at(false, 1000 + 60 * 60 * 24 - 1)
        .test_value();
    main_assert!(app.update_check.is_some());
    main_assert_eq!(
        Config::load(paths.config_file())
            .expect("reload config")
            .get_in(Some("Network"), "LastUpdateTime")
            .map(str::to_string) =>
        Some((1000 + 60 * 60 * 24 - 1).to_string()),
        "the attempt is stored before the result is known"
    );
    let saved = Config::load(paths.config_file()).test_value();
    main_assert_eq!(saved.get_in(Some("Graphics"), "ShowCrewNames") => Some("false"));
    main_assert_eq!(saved.get_in(Some("Graphics"), "ShowCrewCNames") => Some("false"));
    main_assert_eq!(saved.get_in(Some("Graphics"), "ShowClock") => Some("true"));
    main_assert_eq!(saved.get_in(Some("General"), "FPS") => Some("true"));
    main_assert_eq!(saved.get_in(Some("Graphics"), "UpperBoard") => Some("Small"));
    main_assert_eq!(app.deferred_config.len() => 0);

    // A second request while one is in flight says so rather than starting
    // another; C++ cannot reach this because its check blocks.
    app.check_for_updates_at(false, 1000 + 60 * 60 * 24)
        .test_value();
    main_assert_eq!(update_result_dialog(&app).state.message() => "Update still in progress. Please wait.");

    app.abort_update_check();
    app.check_for_updates_at(true, 1000 + 2 * 60 * 60 * 24)
        .test_value();
    main_assert!(app.update_check.is_some());
}

#[test]
fn about_update_action_runs_a_manual_check_and_retains_about() {
    // `C4StartupAboutDlg::OnUpdateBtn` runs `C4UpdateDlg::CheckForUpdates`
    // with fAutomatic unset (C4StartupAboutDlg.cpp:377-380), which opens
    // the cancellable wait dialog at C4UpdateDlg.cpp:275-279.
    let mut app = new_classic_menu_app(640, 480);

    app.open_about_dialog();
    app.process_about_dialog_actions(vec![
        clonk_frontend::startup_about_dlg::AboutDlgAction::CheckForUpdates,
    ])
    .test_value();
    main_assert_eq!(app.startup_view => StartupView::About);
    main_assert!(app.startup_about_dialog.is_some());
    main_assert!(app.update_check.is_some(), "the check must be in flight");
    let wait = app.message_dialogs.last().test_value();
    main_assert_eq!(wait.state.caption() => "Check for Updates");
    main_assert_eq!(wait.state.message() => "Checking for updates...");
    main_assert_eq!(wait.state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::Extended(14));
    main_assert_eq!(wait.state.buttons() => clonk_frontend::message_dialog::MessageDialogButtons::CANCEL);
    main_assert_eq!(wait.state.button_label(clonk_frontend::message_dialog::MessageDialogButton::Cancel) => "Abort");

    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
        .test_value();
    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.update_check.is_none(), "closing the wait dialog abandons the check");
    main_assert_eq!(app.startup_view => StartupView::About);

    for key in [VirtualKeyCode::Enter, VirtualKeyCode::Space] {
        let mut app = new_classic_menu_app(640, 480);
        app.open_about_dialog();
        for _ in 0..2 {
            app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
            app.test_key(VirtualKeyCode::Tab, ElementState::Released);
        }
        app.test_key(key, ElementState::Pressed);
        main_assert!(app.message_dialogs.is_empty());
        app.test_key(key, ElementState::Released);
        main_assert_eq!(app.message_dialogs.len() => 1);
        main_assert_eq!(app.startup_view => StartupView::About);
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Cancel)
            .test_value();
        main_assert!(app.message_dialogs.is_empty());
        main_assert_eq!(app.startup_view => StartupView::About);
    }
}

#[test]
fn about_shift_tab_reverses_buttons_and_license_tabs() {
    use clonk_frontend::startup_about_dlg::AboutPage;

    let mut app = new_classic_menu_app(640, 480);
    app.open_about_dialog();
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);

    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Space, ElementState::Released);
    main_assert_eq!(app.startup_about_dialog.as_ref().expect("About dialog").current_page() => AboutPage::Licenses);

    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);

    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.message_dialogs.len() => 1);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();

    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.startup_about_dialog.as_ref().expect("About dialog").current_page() => AboutPage::Credits);
    main_assert_eq!(app.startup_view => StartupView::About);
}

#[test]
fn unsupported_startup_actions_fail_before_status_or_domain_mutation() {
    let mut app = new_classic_menu_app(640, 480);

    app.open_player_selection_dialog();
    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::NewPlayer,
    ])
    .test_value();
    main_assert!(app.startup_player_properties_dialog.is_some());
    app.startup_player_properties_dialog = None;
}

#[test]
fn player_selection_widget_sounds_reach_the_production_audio_route() {
    let mut app = new_classic_menu_app(640, 480);
    let mut dialog = clonk_frontend::startup_plrsel::PlrSelController::new(2);
    dialog.resize(640, 480);
    app.startup_player_dialog = Some(dialog);
    app.ui_sound_log.clear();

    let actions = app
        .startup_player_dialog
        .as_mut()
        .test_value()
        .handle_key_down(KeyCode::Down);
    app.process_player_dialog_actions(actions).test_value();
    main_assert_eq!(app.ui_sound_log => ["Command"]);

    let back = clonk_frontend::startup_plrsel::plrsel_layout(640, 480).buttons[0];
    let back = GuiPoint::new((back.x + back.w / 2) as f32, (back.y + back.h / 2) as f32);
    let actions = app
        .startup_player_dialog
        .as_mut()
        .test_value()
        .handle_pointer_down(back);
    app.process_player_dialog_actions(actions).test_value();
    main_assert_eq!(app.ui_sound_log => ["Command", "ArrowHit"]);

    let actions = app
        .startup_player_dialog
        .as_mut()
        .test_value()
        .handle_pointer_up(back);
    app.process_player_dialog_actions(actions).test_value();
    main_assert_eq!(app.ui_sound_log => ["Command", "ArrowHit", "Click"]);
}

#[test]
fn startup_crew_mode_replaces_typed_boundary_and_crewless_stays_in_player_mode() {
    let directory = tempdir();
    let player_path = directory.path().join("Ada.c4p");
    fs::create_dir(&player_path).test_value();
    fs::write(
        player_path.join("Player.txt"),
        "[Player]\nName=Ada\n\n[Preferences]\nColorDw=255\n",
    )
    .test_value();
    for (file_name, name, id, experience) in [
        ("Low.c4i", "Low", "CLNK", 20),
        ("High.c4i", "High", "WIPF", 200),
    ] {
        let crew = player_path.join(file_name);
        fs::create_dir(&crew).test_value();
        fs::write(crew.join("ObjectInfo.txt"), format!(
            "[ObjectInfo]\nid={id}\nName={name}\nRankName=Clonk\nExperience={experience}\nParticipation=1\n"
        )).test_value();
    }
    let player_file = PlayerFile::load_from_path(&player_path).test_value();
    let player_model = startup_fixture!(
        player_selection_name_activated_color_dw_score_rounds_rounds_won_rounds_lost_total_playing_time:
            "Ada".to_string(),
            false,
            255,
            0,
            0,
            0,
            0,
            0,
    );
    let mut app = new_classic_menu_app(640, 480);
    app.startup_player_files.push(startup_fixture!(
        startup_player:
            player_path.clone(),
            "Ada.c4p".to_string(),
            player_file,
            player_model.clone(),
    ));
    app.startup_player_models.push(player_model);
    app.open_player_selection_dialog();

    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::ShowCrew(0),
    ])
    .test_value();
    let controller = app.startup_player_dialog.test_ref();
    main_assert!(controller.is_crew_mode());
    main_assert_eq!(controller.dialog_title() => "Crew: Ada");
    main_assert_eq!(controller.selected_index() => Some(0));
    main_assert_eq!(app.startup_crew_models.iter().map(|crew| crew.name.as_str()).collect::<Vec<_>>() => ["High", "Low"]);
    main_assert!(app.message_dialogs.is_empty());

    let selected_crew_file = app.startup_crew_files[0].file_name.clone();
    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::CrewParticipationChanged {
            index: 0,
            participating: false,
        },
    ])
    .test_value();
    let persisted = Group::open(player_path.join(&selected_crew_file))
        .and_then(|group| {
            clonk_engine::player_file::CrewInfo::load(&group)
                .map_err(|error| GroupError::InvalidGroup(error.to_string()))
        })
        .test_value();
    main_assert_eq!(persisted.participation => 0);

    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::SetCrewDeathMessage(0),
    ])
    .test_value();
    main_assert_eq!(app.game_option_input_dialog.as_ref().expect("crew death-message input").controller.max_text() => 75);
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "Farewell".to_string(),
    )])
    .test_value();
    let persisted = Group::open(player_path.join(&selected_crew_file))
        .and_then(|group| {
            clonk_engine::player_file::CrewInfo::load(&group)
                .map_err(|error| GroupError::InvalidGroup(error.to_string()))
        })
        .test_value();
    main_assert_eq!(persisted.death_message => "Farewell");

    let layout = clonk_frontend::startup_plrsel::plrsel_layout(640, 480);
    app.startup_player_dialog
        .test_mut()
        .set_pointer_position(Some(GuiPoint::new(
            (layout.list_client.x + layout.item_height * 2) as f32,
            (layout.list_client.y + layout.item_height / 2) as f32,
        )));
    main_assert!(app.open_startup_player_context_menu(false).expect("open crew context menu"));
    main_assert_eq!(app.context_menu.as_ref().expect("crew context menu").layout().panels[0].rows.len() => 3);
    app.close_context_menu_silently();

    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::LeaveCrew,
    ])
    .test_value();
    let controller = app.startup_player_dialog.test_ref();
    main_assert!(!controller.is_crew_mode());
    main_assert_eq!(controller.selected_index() => Some(0));

    fs::remove_dir_all(player_path.join("Low.c4i")).test_value();
    fs::remove_dir_all(player_path.join("High.c4i")).test_value();
    app.process_player_dialog_actions(vec![
        clonk_frontend::startup_plrsel::PlrSelAction::ShowCrew(0),
    ])
    .test_value();
    let controller = app.startup_player_dialog.test_ref();
    main_assert!(!controller.is_crew_mode());
    main_assert_eq!(controller.selected_index() => Some(0));
    main_assert_eq!(app.message_dialogs.len() => 1);
    main_assert_eq!(app.message_dialogs[0].state.caption() => "Crew: Ada");
    main_assert_eq!(app.message_dialogs[0].state.message() => "Ada does not have a crew yet!");
    main_assert_eq!(app.message_dialogs[0].state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::PLAYER);
}

#[test]
fn startup_player_resize_matches_cpp_copyfrom_sfc_max_size() {
    let source = ImageData::new(
        4,
        2,
        [
            [0, 0, 0, 255],
            [64, 0, 0, 255],
            [128, 0, 0, 255],
            [255, 0, 0, 255],
            [0, 64, 0, 255],
            [0, 128, 0, 255],
            [0, 192, 0, 255],
            [0, 255, 0, 255],
        ]
        .into_iter()
        .flatten()
        .collect(),
    );
    // This offscreen helper intentionally has no display configuration:
    // application scale and PointFiltering cannot affect Blit8 sampling.
    let resized = resize_startup_player_image(&source, 2);
    main_assert_eq!((resized.width(), resized.height()) => (2, 1));
    main_assert_eq!(resized.pixels() => &[0, 0, 0, 255, 128, 0, 0, 255], "offscreen Blit8 samples source-pixel left edges");

    let aspect = ImageData::new(5, 3, vec![255; 5 * 3 * 4]);
    main_assert_eq!(
        {
            let resized = resize_startup_player_image(&aspect, 4);
            (resized.width(), resized.height())
        } =>
        (4, 2),
        "the minor axis uses truncating integer aspect math"
    );
    let no_scale = ImageData::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    main_assert_eq!(resize_startup_player_image(&no_scale, 2) => no_scale);

    let loaded = startup_player_image_from_rgba(2, 1, vec![90, 80, 70, 0, 10, 20, 30, 255]);
    main_assert_eq!(loaded.pixels() => &[0, 0, 0, 0, 10, 20, 30, 255], "C4Surface load blackens hidden RGB");
    let owner_source = ImageData::new(2, 1, vec![0, 0, 255, 255, 200, 30, 30, 255]);
    let icon = startup_player_big_icon(&owner_source, 0x00ff_ffff).test_value();
    main_assert_eq!(icon.pixels() => &[254, 254, 254, 255, 200, 30, 30, 255], "software ModulateClr divides owner RGB by 256");

    let extreme = ImageData::new(1, 151, vec![255; 151 * 4]);
    let collapsed = resize_startup_player_image(&extreme, 150);
    main_assert_eq!((collapsed.width(), collapsed.height()) => (0, 0));
    main_assert!(collapsed.pixels().is_empty());
    main_assert_eq!(materialize_startup_player_image(&extreme, 150) => None);
}

#[test]
fn startup_player_existence_scan_stops_after_the_first_visible_file() {
    let _lock = env_lock().lock();
    let install_root = tempdir();
    let user_data = tempdir();
    fs::create_dir_all(install_root.path().join("planet/System.c4g")).test_value();
    fs::create_dir_all(install_root.path().join("Players")).test_value();
    fs::write(
        install_root.path().join("Players/Visible.C4P"),
        b"filename-only player marker",
    )
    .test_value();
    fs::create_dir_all(install_root.path().join("build")).test_value();
    fs::write(
        install_root.path().join("build/Players"),
        b"not a directory",
    )
    .test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install_root.path()), user_data.path());
    persist_config_value(&paths, "General", "PlayerPath", "Players").test_value();

    main_assert!(startup_player_file_exists(&paths).expect("short-circuit existence scan"));

    fs::remove_file(install_root.path().join("Players/Visible.C4P")).test_value();
    main_assert!(startup_player_file_exists(&paths).is_err());

    fs::remove_dir(install_root.path().join("Players")).test_value();
    fs::write(install_root.path().join("Players"), b"not a directory").test_value();
    fs::remove_file(install_root.path().join("build/Players")).test_value();
    fs::create_dir_all(install_root.path().join("build/Players")).test_value();
    fs::write(
        install_root.path().join("build/Players/Later.c4p"),
        b"later filename-only player marker",
    )
    .test_value();
    main_assert!(startup_player_file_exists(&paths).expect("continue after an earlier scan error"));

    fs::remove_file(install_root.path().join("build/Players/Later.c4p")).test_value();
    main_assert!(startup_player_file_exists(&paths).is_err());
    reset_cached_app_paths();
}

#[test]
fn main_menu_without_visible_player_forces_creation_and_overwrites_participants() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let player_root = user_data.path().join("Players");
    fs::create_dir_all(player_root.join(".Private.c4p")).test_value();
    fs::create_dir_all(player_root.join("Nested/Deep.c4p")).test_value();
    let mut config = Config::new();
    config.set_in(Some("General"), "PlayerPath", player_root.to_string_lossy());
    config.set_in(Some("General"), "Participants", "Stale.c4p;Other.c4p");
    config.set_in(Some("General"), "FirstStart", "0");
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    config.save(paths.config_file()).test_value();

    let mut app = GameApp::new(
        640,
        480,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Player".to_string(), false),
    )
    .test_value();
    wait_for_menu_preserving_first_player_dialog(&mut app);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    main_assert!(matches!(
        app.startup_player_properties_dialog
            .as_ref()
            .map(|pending| pending.controller.mode()),
        Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::New)
    ));
    main_assert!(app.startup_player_properties_dialog.as_ref().is_some_and(|pending| matches!(&pending.origin, StartupPlayerPropertiesOrigin::MainMenuFirstPlayer)));

    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Cancel,
    ]);
    main_assert!(app.startup_player_properties_dialog.is_none());
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    app.handle_main_menu_activation(MainMenuItem::PlayerSelection)
        .test_value();
    main_assert_eq!(app.startup_view => StartupView::PlayerSelection);
    app.process_player_dialog_actions(vec![clonk_frontend::startup_plrsel::PlrSelAction::Back])
        .test_value();
    main_assert!(app.startup_player_properties_dialog.is_some());
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Cancel,
    ]);

    let broken = player_root.join("Broken.C4P");
    fs::write(&broken, b"not a player group").test_value();
    app.show_main_menu();
    main_assert!(app.startup_player_properties_dialog.is_none(), "the native scan matches names without opening player groups");
    fs::remove_file(&broken).test_value();
    app.show_main_menu();
    main_assert!(app.startup_player_properties_dialog.as_ref().is_some_and(|pending| matches!(&pending.origin, StartupPlayerPropertiesOrigin::MainMenuFirstPlayer)));

    let raced = player_root.join("Racer.c4p");
    fs::create_dir_all(&raced).test_value();
    fs::write(
        raced.join("Player.txt"),
        "[Player]\nName=Racer\n\n[Preferences]\nColorDw=255\n",
    )
    .test_value();
    let mut raced_config = Config::load(paths.config_file()).test_value();
    raced_config.set_in(Some("General"), "Participants", raced.to_string_lossy());
    raced_config.save(paths.config_file()).test_value();

    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("First");
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);
    let created = player_root.join("First.c4p");
    main_assert!(created.is_file());
    main_assert!(app.startup_player_properties_dialog.is_none());
    main_assert_eq!(app.startup_player_files.len() => 2);
    main_assert!(app.startup_player_files.iter().any(|player| {
        player
            .file_name
            .eq_ignore_ascii_case(created.to_string_lossy().as_ref())
            && player.render_model.activated
    }));
    main_assert!(
        app.startup_player_files.iter().any(|player| {
            player
                .file_name
                .eq_ignore_ascii_case(raced.to_string_lossy().as_ref())
                && !player.render_model.activated
        }),
        "standalone creation deactivates a player that appears while its modal is open"
    );
    // The label reads the in-memory value, so a config file written behind the
    // modal cannot change it — `C4StartupMainDlg::UpdateParticipants` reads
    // `Config.General.Participants` directly (C4StartupMainDlg.cpp:174-200).
    main_assert_eq!(app.main_menu_state.participants_label => "Players: First");
    // `C4StartupPlrSelDlg` never saves (no `Config.Save()` in that file), so the
    // new participant reaches the file at the next save surface — and still
    // overwrites the raced value when it does.
    app.flush_deferred_config();
    main_assert_eq!(
        Config::load(paths.config_file())
            .expect("reload config")
            .get_in(Some("General"), "Participants") =>
        Some(created.to_string_lossy().as_ref()),
        "forced creation overwrites stale participants with the new file"
    );

    app.show_main_menu();
    main_assert!(app.startup_player_properties_dialog.is_none(), "a visible player prevents another forced dialog");
    app.handle_main_menu_activation(MainMenuItem::PlayerSelection)
        .test_value();
    fs::remove_dir_all(&raced).test_value();
    app.delete_startup_player_and_refresh(&created).test_value();
    app.process_player_dialog_actions(vec![clonk_frontend::startup_plrsel::PlrSelAction::Back])
        .test_value();
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    main_assert!(app.startup_player_properties_dialog.is_some(), "every main-menu show rechecks the physical player directory");
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Cancel,
    ]);
    main_assert!(app.startup_player_properties_dialog.is_none());
    app.handle_main_menu_activation(MainMenuItem::Options)
        .test_value();
    main_assert_eq!(app.startup_view => StartupView::Options);
    reset_cached_app_paths();
}

fn set_distinct_player_properties_fields(app: &mut GameApp, name: &str) -> (PlayerFile, String) {
    let controller = &mut app.startup_player_properties_dialog.test_mut().controller;
    controller.set_name(name);
    controller.set_comment("Retained validation comment");
    let player = controller.player_mut();
    player.pref_color = 3;
    player.pref_color_dw = 0x12_34_56;
    player.pref_control = 5;
    player.pref_mouse = true;
    player.pref_control_style = false;
    player.pref_auto_context_menu = false;
    (
        controller.player().clone(),
        controller.comment().to_string(),
    )
}

fn assert_player_properties_validation_modal(
    app: &GameApp,
    expected_message: &str,
    expected_player: &PlayerFile,
    expected_comment: &str,
) {
    use clonk_frontend::message_dialog::{
        MessageDialogButtons, MessageDialogIcon, MessageDialogSize,
    };

    let modal = app.message_dialogs.last().test_value();
    main_assert_eq!(modal.state.message() => expected_message);
    main_assert_eq!(modal.state.caption() => "");
    main_assert_eq!(modal.state.buttons() => MessageDialogButtons::OK);
    main_assert_eq!(modal.state.icon() => MessageDialogIcon::ERROR);
    main_assert_eq!(modal.state.size() => MessageDialogSize::Regular);
    main_assert!(matches!(modal.continuation, MessageDialogContinuation::None));
    let form = app.startup_player_properties_dialog.test_ref();
    main_assert_eq!(form.controller.player() => expected_player);
    main_assert_eq!(form.controller.comment() => expected_comment);
    main_assert_eq!(form.controller.validation_error() => None);
    main_assert!(app.status_text.is_empty());
}

#[test]
fn startup_player_properties_empty_name_shows_modal_message_dialog() {
    let user_data = tempdir();
    let (_guard, _paths, _player_root, mut app) =
        startup_player_properties_validation_app(user_data.path());
    let (expected_player, expected_comment) = set_distinct_player_properties_fields(&mut app, "");

    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    assert_player_properties_validation_modal(
        &app,
        "You must specify a player name!",
        &expected_player,
        &expected_comment,
    );
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.message_dialogs.is_empty());
    let form = app.startup_player_properties_dialog.as_ref().test_value();
    main_assert_eq!(form.controller.player() => &expected_player);
    main_assert_eq!(form.controller.comment() => expected_comment);
}

#[test]
fn startup_player_properties_duplicate_name_shows_modal_message_dialog() {
    let user_data = tempdir();
    let (_guard, _paths, player_root, mut app) =
        startup_player_properties_validation_app(user_data.path());
    let occupied = player_root.join("Taken.c4p");
    fs::create_dir(&occupied).test_value();
    let (expected_player, expected_comment) =
        set_distinct_player_properties_fields(&mut app, "Taken");

    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    assert_player_properties_validation_modal(
        &app,
        "Taken is already taken",
        &expected_player,
        &expected_comment,
    );
    main_assert!(occupied.is_dir());
    main_assert!(app.startup_player_files.is_empty());
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.message_dialogs.is_empty());
    let form = app.startup_player_properties_dialog.as_ref().test_value();
    main_assert_eq!(form.controller.player() => &expected_player);
    main_assert_eq!(form.controller.comment() => expected_comment);
}

#[test]
fn startup_player_properties_rename_step_failure_opens_classic_error_dialog() {
    use clonk_frontend::message_dialog::{
        MessageDialogButtons, MessageDialogIcon, MessageDialogResult, MessageDialogSize,
    };

    let user_data = tempdir();
    let (_guard, paths, player_root, mut app) =
        startup_player_properties_validation_app(user_data.path());
    app.startup_player_properties_dialog = None;

    let old = player_root.join("Old.c4p");
    fs::create_dir(&old).test_value();
    fs::write(old.join("Player.txt"), b"[Player]\nName=Old\n").test_value();
    persist_config_value(&paths, "General", "Participants", old.to_string_lossy()).test_value();
    app.refresh_startup_player_list();
    main_assert_eq!(app.startup_player_files.len() => 1);

    app.open_existing_startup_player_properties(0);
    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("Renamed");

    // Fail the rename step itself: the source group vanishes behind the
    // open form, so the move onto the new filename has nothing to rename.
    fs::remove_dir_all(&old).test_value();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    main_assert!(!old.exists());
    main_assert!(!player_root.join("Renamed.c4p").exists());
    main_assert!(app.startup_player_properties_dialog.is_none(), "the properties form closes before the screen-owned error dialog");
    main_assert!(app.startup_player_files.is_empty());
    main_assert!(app.status_text.is_empty());
    main_assert_eq!(Config::load(paths.config_file()).expect("reload reconciled config").get_in(Some("General"), "Participants") => Some(""));

    let modal = app.message_dialogs.last().test_value();
    main_assert_eq!(modal.state.caption() => "Error");
    main_assert!(!modal.state.message().is_empty());
    main_assert_eq!(modal.state.buttons() => MessageDialogButtons::OK);
    main_assert_eq!(modal.state.icon() => MessageDialogIcon::ERROR);
    main_assert_eq!(modal.state.size() => MessageDialogSize::Regular);
    main_assert!(matches!(modal.continuation, MessageDialogContinuation::None));

    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.message_dialogs.is_empty());
    main_assert_eq!(app.startup_player_dialog.as_ref().and_then(|dialog| dialog.selected_index()) => None);
}

#[test]
fn startup_player_properties_new_player_create_failure_opens_classic_error_dialog() {
    use clonk_frontend::message_dialog::{
        MessageDialogButtons, MessageDialogIcon, MessageDialogResult, MessageDialogSize,
    };

    let user_data = tempdir();
    let (_guard, _paths, player_root, mut app) =
        startup_player_properties_validation_app(user_data.path());
    app.startup_player_properties_dialog
        .test_mut()
        .controller
        .set_name("Fresh");

    // Break the configured player root under the open creation form: the
    // occupancy scan and the new group write both need a directory there.
    fs::remove_dir_all(&player_root).test_value();
    fs::write(&player_root, b"not a directory").test_value();
    app.process_startup_player_properties_actions(vec![
        clonk_frontend::startup_plrproperties::PlayerPropertiesAction::Submit,
    ]);

    main_assert!(app.startup_player_properties_dialog.is_none(), "the creation form closes before the screen-owned error dialog");
    main_assert!(app.startup_player_files.is_empty());
    main_assert!(app.status_text.is_empty());

    let modal = app.message_dialogs.last().test_value();
    main_assert_eq!(modal.state.caption() => "Error");
    main_assert!(!modal.state.message().is_empty());
    main_assert_eq!(modal.state.buttons() => MessageDialogButtons::OK);
    main_assert_eq!(modal.state.icon() => MessageDialogIcon::ERROR);
    main_assert_eq!(modal.state.size() => MessageDialogSize::Regular);
    main_assert!(matches!(modal.continuation, MessageDialogContinuation::None));

    app.finish_message_dialog(MessageDialogResult::Ok)
        .test_value();
    main_assert!(app.message_dialogs.is_empty());
}

#[test]
fn about_routes_wheel_to_credits_and_license_textwindows() {
    let mut credits = new_classic_menu_app(1280, 720);
    credits.open_about_dialog();
    let credits_layout = clonk_frontend::startup_about_dlg::about_layout(1280, 720);
    let scripting = credits_layout.sections[2].textbox;
    credits.test_cursor(PhysicalPosition::new(
        f64::from(scripting.x + 1),
        f64::from(scripting.y + 9),
    ));
    credits.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert_eq!(credits.startup_about_dialog.as_ref().and_then(|dialog| dialog.credit_scroll_offset(2)) => Some(28));

    let mut licenses = new_classic_menu_app(320, 240);
    enter_about_licenses(&mut licenses);
    let licenses_layout = clonk_frontend::startup_about_dlg::about_layout(320, 240);
    let text = licenses_layout.licenses.text;
    licenses.test_cursor(PhysicalPosition::new(
        f64::from(text.x + 12),
        f64::from(text.y + 10),
    ));
    licenses.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert!(licenses.startup_about_dialog.as_ref().is_some_and(|dialog| dialog.license_scroll_offset() > 0));
}

#[test]
fn startup_override_shortcuts_require_exact_unmodified_keys() {
    let mut app = new_classic_menu_app(640, 480);
    app.open_network_game_dialog();
    for modifiers in [
        ModifiersState::ALT,
        ModifiersState::CONTROL,
        ModifiersState::SHIFT,
    ] {
        app.keyboard_modifiers = modifiers;
        app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
        app.test_key(VirtualKeyCode::F5, ElementState::Released);
    }
    app.keyboard_modifiers = ModifiersState::SUPER;
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F5, ElementState::Released);

    let shortcut_model = startup_fixture!(
        player_selection_name_activated_color_dw_score_rounds_rounds_won_rounds_lost_total_playing_time:
            "Shortcut Player".to_string(),
            false,
            0xff,
            0,
            0,
            0,
            0,
            0,
    );
    app.startup_player_files.push(startup_fixture!(
        startup_player:
            PathBuf::from("Shortcut Player.c4p"),
            "Shortcut Player.c4p".to_string(),
            PlayerFile::default(),
            shortcut_model.clone(),
    ));
    app.startup_player_models.push(shortcut_model);
    app.open_player_selection_dialog();
    for (modifiers, key) in [
        (ModifiersState::ALT, VirtualKeyCode::Insert),
        (ModifiersState::CONTROL, VirtualKeyCode::F2),
        (ModifiersState::SHIFT, VirtualKeyCode::Delete),
    ] {
        app.keyboard_modifiers = modifiers;
        app.test_key(key, ElementState::Pressed);
        app.test_key(key, ElementState::Released);
    }
    main_assert!(app.message_dialogs.is_empty());
    main_assert!(app.status_text.is_empty());
    app.keyboard_modifiers = ModifiersState::SUPER;
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    main_assert!(matches!(
        app.startup_player_properties_dialog
            .as_ref()
            .map(|pending| pending.controller.mode()),
        Some(clonk_frontend::startup_plrproperties::PlayerPropertiesMode::Edit { index: 0 })
    ));
}

#[test]
fn startup_bootstrap_issues_are_typed_and_aggregated_in_cpp_init_order() {
    let mut app = new_real_classic_menu_app(320, 200);
    let assets = Arc::get_mut(&mut app.assets).test_value();

    // Remove in deliberately non-oracle order. The boundary must still
    // report C4StartupGraphics::Init order, followed by its font order.
    assets.startup_dialog_images.remove("StartupNetGetRef.png");
    assets.startup_dialog_images.remove("StartupPlrPropBG.png");
    assets.startup_dialog_images.remove("StartupScenSelBG.png");
    assets.startup_dialog_images.insert(
        "StartupBookScroll.png".to_string(),
        ImageData::new(0, 48, Vec::new()),
    );
    assets
        .startup_dialog_images
        .remove("StartupPlrCtrlType.png");
    assets.startup_bootstrap_image_failures.insert(
        "StartupPlrCtrlType.png".to_string(),
        "failed to decode image".to_string(),
    );
    let (caption, text, small) = {
        let fonts = assets.book_fonts.as_deref().test_value();
        (
            fonts.caption.clone(),
            fonts.text.clone(),
            fonts.small.clone(),
        )
    };
    assets.book_fonts = Some(Arc::new(clonk_frontend::startup_scensel::BookFontSet {
        title: clonk_graphics::clonk_font::ClonkFont::new(0),
        caption,
        text,
        small,
    }));

    let error = assets
        .require_classic_startup_bootstrap_resources()
        .expect_err("incomplete bootstrap must fail as one aggregate");
    main_assert_eq!(
        error =>
        ClassicParityBoundary::StartupBootstrapResources {
            issues: vec![
                ClassicStartupBootstrapIssue::missing("StartupScenSelBG.png"),
                ClassicStartupBootstrapIssue::missing("StartupPlrPropBG.png"),
                ClassicStartupBootstrapIssue::malformed(
                    "StartupBookScroll.png",
                    "a non-empty decoded RGBA surface",
                    "0x48 with 0 bytes",
                ),
                ClassicStartupBootstrapIssue::malformed(
                    "StartupPlrCtrlType.png",
                    "a non-empty decoded RGBA surface",
                    "failed to decode image",
                ),
                ClassicStartupBootstrapIssue::missing("StartupNetGetRef.png"),
                ClassicStartupBootstrapIssue::malformed(
                    "BookFontTitle",
                    "an initialized shadowless RX font",
                    "line_height=0, cell_height=1, h_space=-1",
                ),
            ],
        }
    );
}

#[test]
fn startup_bootstrap_precedes_recursive_startup_children() {
    let mut app = new_real_classic_menu_app(640, 480);
    for child in [
        RetainedStartupChild::Unported(ClassicStartupSubscreen::Options(
            clonk_frontend::startup_options_dlg::OptionsSheet::Graphics,
        )),
        RetainedStartupChild::OptionsSound,
        RetainedStartupChild::AboutLicenses,
    ] {
        enter_retained_startup_child(&mut app, child);
        let removed = Arc::get_mut(&mut app.assets)
            .test_value()
            .startup_dialog_images
            .remove("StartupPlrCtrlType.png")
            .test_value();
        let mut frame = vec![0xc7; 640 * 480 * 4];
        let error = app
            .render(&mut frame)
            .expect_err("bootstrap must precede the recursive child boundary");
        assert_startup_bootstrap_boundary(
            &error,
            vec![ClassicStartupBootstrapIssue::missing(
                "StartupPlrCtrlType.png",
            )],
        );
        main_assert!(frame.iter().all(|byte| *byte == 0xc7));
        Arc::get_mut(&mut app.assets)
            .test_value()
            .startup_dialog_images
            .insert("StartupPlrCtrlType.png".to_string(), removed);
        app.status_text.clear();
        app.show_main_menu();
    }
}

#[test]
fn startup_status_boundary_precedes_supported_view_pixels() {
    let mut app = new_real_classic_menu_app(320, 200);

    for view in [
        StartupView::MainMenu,
        StartupView::ScenarioBrowser,
        StartupView::NetworkGame,
        StartupView::PlayerSelection,
        StartupView::Options,
        StartupView::About,
    ] {
        match view {
            StartupView::MainMenu => app.show_main_menu(),
            StartupView::ScenarioBrowser => app.open_scenario_browser(),
            StartupView::NetworkGame => app.open_network_game_dialog(),
            StartupView::PlayerSelection => app.open_player_selection_dialog(),
            StartupView::Options => app.open_options_menu(),
            StartupView::About => app.open_about_dialog(),
            StartupView::NetworkLobby => unreachable!(),
        }
        let status = format!("diagnostic status for {view:?}");
        app.status_text = status.clone();
        let mut frame = vec![0x5a; 320 * 200 * 4];

        let error = app
            .render(&mut frame)
            .expect_err("generic startup status must never reach a frame");
        match error.downcast_ref::<ClassicParityBoundary>() {
            Some(ClassicParityBoundary::StartupStatusOverlay {
                view: boundary_view,
                status: boundary_status,
            }) => {
                main_assert_eq!(*boundary_view => view);
                main_assert_eq!(boundary_status => &status);
            }
            other => panic!("unexpected startup status boundary: {other:?}"),
        }
        main_assert_eq!(app.status_text => status, "diagnostic state is retained");
        main_assert!(frame.iter().all(|byte| *byte == 0x5a), "{view:?} must fail before copying newly rendered pixels");
    }
}

#[test]
fn main_menu_team_switch_reads_live_gate_and_dispatches_offline_control() {
    let mut app = new_state_only_running_sandbox_app();
    let owner = app.local_owner;
    app.engine.set_teams(vec![
        clonk_engine::TeamInfo::new(1, "Red", 0x00f4_0000),
        clonk_engine::TeamInfo::new(2, "Blue", 0x0000_00f4),
    ]);
    app.engine.test_player_mut(owner).set_team(Some(1));
    app.snapshot = app.engine.snapshot();

    let mut teams = app.engine.team_configuration();
    teams.allow_team_switch = false;
    app.engine.set_team_configuration(teams);
    let conditions = app.main_menu_conditions();
    main_assert!(!conditions.team_switch_allowed);
    main_assert!(
        !IngameMenuState::main_menu(&conditions, &IngameMenuLabels::default())
            .expect("main menu")
            .items()
            .iter()
            .any(|item| item.action == MenuAction::ActivateTeamSelection)
    );

    teams.allow_team_switch = true;
    app.engine.set_team_configuration(teams);
    main_assert!(app.main_menu_conditions().team_switch_allowed);

    app.engine
        .set_player_status(owner, PlayerStatus::TeamSelection)
        .test_value();
    app.apply_ingame_menu_action(MenuAction::ActivateTeamSelection)
        .test_value();
    let initial = app.ingame_menu.get(owner).test_value();
    main_assert_eq!(initial.close_action() => Some(&MenuAction::ActivateMain));
    main_assert!(initial.items().iter().all(|item| matches!(&item.action, MenuAction::SelectTeam(_))));
    app.ingame_menu.clear();
    app.engine
        .set_player_status(owner, PlayerStatus::Active)
        .test_value();

    app.apply_ingame_menu_action(MenuAction::ActivateTeamSelection)
        .test_value();
    let menu = app.ingame_menu.get_mut(owner).test_value();
    main_assert_eq!(menu.close_action() => Some(&MenuAction::ActivateMain));
    main_assert_eq!(menu.items().iter().map(|item| item.action.clone()).collect::<Vec<_>>() => [MenuAction::SwitchTeam(1), MenuAction::SwitchTeam(2)]);
    menu.set_selection(1);
    let outcome = menu
        .handle_command(ControlCommand::MenuEnter, CommandKind::Press)
        .test_value();

    app.execute_ingame_menu_outcome_for_player(owner, outcome)
        .test_value();

    let player = app.engine.test_player(owner);
    main_assert_eq!(player.status() => PlayerStatus::Active);
    main_assert_eq!(player.team() => Some(2));
    main_assert!(app.ingame_menu.get(owner).is_none());
}

#[test]
fn main_menu_hides_abort_and_display_fullscreen_only_entries_in_windowed_mode() {
    let mut app = new_state_only_running_sandbox_app();
    app.set_display_mode(DisplayMode::Window);

    let main =
        IngameMenuState::main_menu(&app.main_menu_conditions(), &IngameMenuLabels::default())
            .test_value();
    main_assert!(!main.items().iter().any(|item| item.action == MenuAction::Abort));
    let display =
        IngameMenuState::display_menu(&app.display_flags, 0, &IngameMenuLabels::default());
    main_assert_eq!(
        display
            .items()
            .iter()
            .map(|item| item.caption.as_str())
            .collect::<Vec<_>>() =>
        [
            "Player names",
            "Clonk names",
            "Portraits",
            "Commands",
            "Keys"
        ]
    );

    app.set_display_mode(DisplayMode::Fullscreen);
    let main =
        IngameMenuState::main_menu(&app.main_menu_conditions(), &IngameMenuLabels::default())
            .test_value();
    main_assert!(main.items().iter().any(|item| item.action == MenuAction::Abort));
    main_assert_eq!(IngameMenuState::display_menu(&app.display_flags, 0, &IngameMenuLabels::default()).items().len() => 9);
}

#[test]
fn crew_name_label_respects_display_flags() {
    let mut app = new_state_only_running_sandbox_app();
    let viewer = app.local_owner;
    let focus = app.engine.test_crew_cursor(viewer);
    let focus_state = app.engine.test_object_snapshot(focus);
    let remote = viewer + 1;
    app.engine
        .register_player(PlayerConfig::new(remote, "Remote Player"))
        .test_value();
    app.engine.spawn_test_object(
        SpawnConfig::new(focus_state.definition_id)
            .with_position(focus_state.position)
            .with_owner(remote)
            .with_crew_member(true)
            .with_custom_name("Remote Clonk"),
    );
    app.snapshot = app.engine.snapshot();

    let labels = |app: &GameApp| {
        let focus = app.snapshot.object(focus).test_value();
        app.crew_name_overlays(&[ViewportInput::new(viewer, focus.position, 1.0, focus)])
    };

    main_assert_eq!(labels(&app).iter().map(|label| label.text.as_str()).collect::<Vec<_>>() => ["Remote Clonk (Remote Player)"]);
    app.display_flags.player_names = false;
    main_assert_eq!(labels(&app)[0].text => "Remote Clonk");
    app.display_flags.player_names = true;
    app.display_flags.clonk_names = false;
    main_assert_eq!(labels(&app)[0].text => "Remote Player");
    app.display_flags.player_names = false;
    main_assert!(labels(&app).is_empty());
}

fn loader_origin_fixture_scenario(
    path: &Path,
    origin: &str,
) -> (FrontendScenario, Group, ScenarioLoaderHead) {
    fs::create_dir_all(path).test_value();
    fs::write(
        path.join("Scenario.txt"),
        format!("[Head]\nOrigin={origin}\n"),
    )
    .test_value();
    let group = Group::open(path).test_value();
    let head = ScenarioLoaderHead::load_from_group(&group).test_value();
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Fixture.c4s")
        .to_string();
    scenario.title = "Origin fixture".to_string();
    scenario.kind = ScenarioKind::Scenario;
    scenario.path = Some(path.to_path_buf());
    (scenario, group, head)
}

fn loader_fixture_definition_load() -> ScenarioDefinitionLoad {
    ScenarioDefinitionLoad::Fixed {
        modules: Vec::new(),
        definition_root: None,
    }
}

#[test]
fn extra_definition_names_follow_definition_path_and_local_folder_vector_order() {
    let root = tempdir();
    let outer = root.path().join("Outer.c4f");
    let inner = outer.join("Inner.c4f");
    let scenario = inner.join("Scenario.c4s");
    fs::create_dir_all(outer.join("OuterDefs.c4d")).test_value();
    fs::create_dir_all(inner.join("InnerDefs.c4d")).test_value();
    fs::create_dir_all(&scenario).test_value();
    fs::write(scenario.join("Scenario.txt"), "[Head]\nTitle=Vector\n").test_value();
    let scenario_group = Group::open(&scenario).test_value();
    let head = ScenarioLoaderHead::load_from_group(&scenario_group).test_value();
    let names = extra_definition_group_names(
        &head,
        &ScenarioDefinitionLoad::Fixed {
            modules: vec!["Packs/Objects.c4d".to_string()],
            definition_root: Some(root.path().join("Definitions")),
        },
        &scenario,
    )
    .test_value();
    main_assert_eq!(names => ["Objects.c4d", "Objects.c4d", "Outer.c4f", "Inner.c4f"]);
    main_assert_eq!(
        extra_definition_filename(r"Packs\Windows.c4d") =>
        Some(if cfg!(windows) {
            "Windows.c4d"
        } else {
            r"Packs\Windows.c4d"
        })
    );
}

#[test]
fn loader_origin_registers_existing_parent_when_final_scenario_is_missing() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let origin_parent = content.join("Parent.c4f");
    fs::create_dir(&origin_parent).test_value();
    let scenario_path = content.join("Actual.c4s");
    let (scenario, scenario_group, head) =
        loader_origin_fixture_scenario(&scenario_path, "Parent.c4f/Missing.c4s");

    let registrations =
        startup_fixture!(loader_registrations: scenario, scenario_group, head, paths);
    main_assert_eq!(registrations.len() => 2);
    main_assert_eq!(registrations[0].priority => 200);
    main_assert_eq!(registrations[1].priority => 100);
    main_assert_eq!(registrations[1].group.root() => origin_parent.as_path());
}

/// clonk-org/clonk-rs#186. C++ has one `ExePath`, so an `Origin` is read back
/// against the directory it was written against: `C4GroupSet.cpp:297` opens it
/// unprefixed, and `C4Config.cpp:1320-1321` forces the working directory to
/// `ExePath`. This port spells that root two ways, and savegames already on
/// disk carry the install-root spelling, so both must name the same group.
#[test]
fn loader_origin_relative_to_the_install_root_registers_the_same_parent() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let origin_parent = content.join("Parent.c4f");
    fs::create_dir(&origin_parent).test_value();
    let scenario_path = root.path().join("Savegames.c4f").join("Saved.c4s");
    let (scenario, scenario_group, head) =
        loader_origin_fixture_scenario(&scenario_path, "content/Parent.c4f/Original.c4s");

    let registrations =
        startup_fixture!(loader_registrations: scenario, scenario_group, head, paths);
    main_assert!(
        registrations
            .iter()
            .any(|registration| registration.group.root() == origin_parent.as_path()),
        "the Origin parent is registered, not doubled below the content root"
    );
}

#[test]
fn loader_origin_explicit_empty_value_is_a_valid_no_op() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let scenario_path = content.join("Actual.c4s");
    let (scenario, scenario_group, head) = loader_origin_fixture_scenario(&scenario_path, "");
    main_assert_eq!(head.origin() => Some("empty"));

    let registrations =
        startup_fixture!(loader_registrations: scenario, scenario_group, head, paths);
    main_assert_eq!(registrations.len() => 1);
    main_assert_eq!(registrations[0].group.root() => scenario_path.as_path());
}

#[test]
fn loader_origin_identical_existing_scenario_does_not_duplicate_parent() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let parent = content.join("Parent.c4f");
    let scenario_path = parent.join("Actual.c4s");
    let (scenario, scenario_group, head) =
        loader_origin_fixture_scenario(&scenario_path, "Parent.c4f/Actual.c4s");

    let registrations =
        startup_fixture!(loader_registrations: scenario, scenario_group, head, paths);
    main_assert_eq!(registrations.len() => 2);
    main_assert_eq!(
        registrations
            .iter()
            .filter(|registration| registration.group.root() == parent.as_path())
            .count() =>
        1,
        "ItemIdentical suppresses the duplicate Origin parent"
    );
}

#[test]
fn loader_origin_opens_packed_parent_chain_outer_to_inner() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let inner = packed_test_group(&[]);
    let outer = packed_test_file_group(&[("INNER.C4F", true, inner.as_slice())]);
    let outer_path = content.join("Outer.c4f");
    fs::write(&outer_path, outer).test_value();
    let scenario_path = content.join("Actual.c4s");
    let (scenario, scenario_group, head) =
        loader_origin_fixture_scenario(&scenario_path, "Outer.c4f/inner.c4f/Missing.c4s");

    let registrations =
        startup_fixture!(loader_registrations: scenario, scenario_group, head, paths);
    main_assert_eq!(registrations.len() => 3);
    main_assert_eq!(registrations[1].group.root() => outer_path.as_path());
    main_assert_eq!(registrations[1].priority => 100);
    // Packed C4Group traversal propagates the selected entry's stored spelling.
    main_assert_eq!(registrations[2].group.root() => outer_path.join("INNER.C4F").as_path());
    main_assert_eq!(registrations[2].priority => 101);
}

/// `C4GroupSet::RegisterParentFolders` registers each parent as it opens it
/// (`C4GroupSet.cpp:310`) and only stops at the first it cannot open
/// (`C4GroupSet.cpp:291-295`), so the groups above a missing child stay in the
/// set. Partial registration is the C++ shape, not a boundary.
#[test]
fn unresolvable_packed_loader_origin_keeps_the_parents_already_registered() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let outer_path = content.join("Outer.c4f");
    fs::write(&outer_path, packed_test_file_group(&[])).test_value();
    let scenario_path = content.join("Actual.c4s");
    let (_, _, head) =
        loader_origin_fixture_scenario(&scenario_path, "Outer.c4f/MissingInner.c4f/Missing.c4s");
    let origin = resolve_loader_origin(
        head.origin().expect("configured Origin"),
        &scenario_path,
        &paths,
    )
    .expect("representable ItemIdentical paths")
    .test_value();
    let mut registrations = Vec::new();
    let mut registration_order = 0;

    register_loader_origin_parents(&origin, &mut registrations, &mut registration_order);
    main_assert_eq!(registrations.len() => 1, "outer registration happens first");
    main_assert_eq!(registrations[0].group.root() => outer_path.as_path());
}

/// `C4Game::OpenScenario` discards the `RegisterParentFolders` result
/// (`C4Game.cpp:177-178`), so an Origin naming a parent the install no longer
/// holds costs those groups and nothing else — the scenario still loads.
#[test]
fn loader_origin_naming_an_absent_parent_still_registers_the_scenario() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let scenario_path = content.join("Actual.c4s");
    let (scenario, scenario_group, head) =
        loader_origin_fixture_scenario(&scenario_path, "Gone.c4f/Original.c4s");

    let registrations =
        startup_fixture!(loader_registrations: scenario, scenario_group, head, paths);
    main_assert_eq!(registrations.len() => 1);
    main_assert_eq!(registrations[0].group.root() => scenario_path.as_path());
}

#[test]
fn selected_loader_title_cross_loads_from_origin_and_local_candidate_wins() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    fs::create_dir_all(paths.config_dir()).test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();
    let scenario_path = content.join("Actual.c4s");
    let (_, scenario_group, _) =
        loader_origin_fixture_scenario(&scenario_path, "Archive.c4f/Original.c4s");
    let packed_title = paths
        .planet_dir()
        .join("Language.c4g")
        .join("Test.c4g")
        .join("Archive.c4f/Original.c4s");
    fs::create_dir_all(&packed_title).test_value();
    fs::write(packed_title.join("TitleUS.txt"), "US:Pack title\n").test_value();

    let packed = load_classic_scenario_loader_head(&scenario_group, &paths).test_value();
    main_assert_eq!(packed.scenario_title() => "Pack title");
    let mut staged = FrontendScenario::fallback();
    staged.title = "Catalog fallback".to_string();
    retain_selected_scenario_title(&mut staged, Some(packed.scenario_title()));
    main_assert_eq!(staged.title => "Pack title");

    fs::write(scenario_path.join("TitleUS.txt"), "US:Local title\n").test_value();
    let local = load_classic_scenario_loader_head(&scenario_group, &paths).test_value();
    main_assert_eq!(local.scenario_title() => "Local title");
}

#[test]
fn frontend_discovery_cross_loads_pack_title_and_local_candidate_wins() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguageEx=US\n").test_value();
    let scenario_path = content.join("Actual.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Core fallback\n",
    )
    .test_value();
    let pack_scenario = paths.planet_dir().join("Language.c4g/Test.c4g/Actual.c4s");
    fs::create_dir_all(&pack_scenario).test_value();
    fs::write(pack_scenario.join("TitleUS.txt"), "US:Pack catalog title\n").test_value();

    let packed = load_frontend_scenarios();
    let packed = packed
        .iter()
        .find(|entry| entry.identifier.ends_with("Actual.c4s"))
        .test_value();
    main_assert_eq!(packed.title => "Pack catalog title");

    fs::write(
        scenario_path.join("TitleUS.txt"),
        "US:Local catalog title\n",
    )
    .test_value();
    let local = load_frontend_scenarios();
    let local = local
        .iter()
        .find(|entry| entry.identifier.ends_with("Actual.c4s"))
        .test_value();
    main_assert_eq!(local.title => "Local catalog title");
}

#[test]
fn loader_title_uses_configured_language_ex_without_ui_fallbacks() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    fs::create_dir_all(paths.config_dir()).test_value();
    fs::write(
        paths.config_file(),
        "[General]\nLanguageEx=FR\nLanguage=FR - French\n",
    )
    .test_value();
    let scenario_path = content.join("Actual.c4s");
    fs::create_dir(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Head fallback\n",
    )
    .test_value();
    fs::write(scenario_path.join("TitleUS.txt"), "US:Wrong UI fallback\n").test_value();
    let scenario_group = Group::open(&scenario_path).test_value();

    let head = load_classic_scenario_loader_head(&scenario_group, &paths).test_value();
    main_assert_eq!(head.scenario_title() => "Head fallback");
}

#[test]
fn frontend_and_loader_titles_share_fresh_primary_language_sequence() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    paths.ensure_user_dirs().test_value();
    fs::write(paths.config_file(), "[General]\nLanguage=DE - Deutsch\n").test_value();
    let scenario_path = content.join("FreshLanguage.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Head fallback\n",
    )
    .test_value();
    fs::write(scenario_path.join("Title.txt"), "US:English title\n").test_value();
    let scenario_group = Group::open(&scenario_path).test_value();

    let loader_title = load_classic_scenario_loader_head(&scenario_group, &paths)
        .test_value()
        .scenario_title()
        .to_string();
    let frontend_title = load_frontend_scenarios()
        .into_iter()
        .find(|scenario| scenario.identifier.ends_with("FreshLanguage.c4s"))
        .test_value()
        .title;

    main_assert_eq!(
        (
            startup_language_sequence(Some(&paths)),
            classic_loader_language_sequence(&paths).expect("loader language sequence"),
            frontend_title,
            loader_title,
        ) =>
        (
            vec!["DE".to_string()],
            vec!["DE".to_string()],
            "Head fallback".to_string(),
            "Head fallback".to_string(),
        )
    );
}

#[test]
fn frontend_and_loader_titles_share_persisted_language_ex_sequence() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    paths.ensure_user_dirs().test_value();
    fs::write(
        paths.config_file(),
        "[General]\nLanguage=DE - Deutsch\nLanguageEx=DE,US\n",
    )
    .test_value();
    let scenario_path = content.join("PersistedLanguage.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Head fallback\n",
    )
    .test_value();
    fs::write(scenario_path.join("Title.txt"), "US:English title\n").test_value();
    let scenario_group = Group::open(&scenario_path).test_value();

    let loader_title = load_classic_scenario_loader_head(&scenario_group, &paths)
        .test_value()
        .scenario_title()
        .to_string();
    let frontend_title = load_frontend_scenarios()
        .into_iter()
        .find(|scenario| scenario.identifier.ends_with("PersistedLanguage.c4s"))
        .test_value()
        .title;

    main_assert_eq!(
        (
            startup_language_sequence(Some(&paths)),
            classic_loader_language_sequence(&paths).expect("loader language sequence"),
            frontend_title,
            loader_title,
        ) =>
        (
            vec!["DE".to_string(), "US".to_string()],
            vec!["DE".to_string(), "US".to_string()],
            "English title".to_string(),
            "English title".to_string(),
        )
    );
}

#[test]
fn loader_language_ex_keeps_raw_two_byte_segments_and_duplicates() {
    let root = tempdir();
    let (_guard, paths, _) = loader_origin_fixture_paths(root.path());
    fs::create_dir_all(paths.config_dir()).test_value();
    fs::write(paths.config_file(), "LanguageEx=DE, DE,DE\n").test_value();

    main_assert_eq!(classic_loader_language_sequence(&paths).expect("raw LanguageEx sequence") => vec!["DE".to_string(), " D".to_string(), "DE".to_string()]);
}

#[test]
fn loader_config_strings_fail_beyond_cpp_byte_capacity() {
    let oversized = "X".repeat(1025);
    for key in ["Language", "LanguageEx", "FontName", "LanguageCharset"] {
        let mut config = Config::new();
        config.set(key, oversized.clone());
        let error = classic_loader_bounded_config_value(&config, key)
            .expect_err("over-capacity loader string must fail closed");
        main_assert!(error.to_string().contains(key));
        main_assert!(error.to_string().contains("1024-byte"));
    }

    let mut config = Config::new();
    config.set("LanguageEx", "US\0,DE");
    let error = classic_loader_bounded_config_value(&config, "LanguageEx")
        .expect_err("embedded NUL must not expose a suffix C++ truncates");
    main_assert!(error.to_string().contains("embedded NUL"));
}

#[test]
fn raw_loader_config_nul_is_rejected_before_field_parsing() {
    let root = tempdir();
    let (_guard, paths, _) = loader_origin_fixture_paths(root.path());
    fs::create_dir_all(paths.config_dir()).test_value();
    fs::write(paths.config_file(), b"[Graphics]\nScale=100\0\nScale=500\n").test_value();

    let error = load_classic_loader_config(&paths)
        .expect_err("raw config suffix hidden from C++ must not be parsed");
    main_assert!(error.to_string().contains("embedded NUL"));
}

#[test]
fn planet_extra_registers_root_and_only_activated_definition_children() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let extra_path = paths.planet_dir().join("Extra.c4g");
    fs::create_dir(&extra_path).test_value();
    let extra_graphics = extra_path.join("Graphics.c4g");
    let objects = extra_path.join("Objects.c4d");
    let objects_graphics = objects.join("Graphics.c4g");
    let objects_materials = objects.join("Material.c4g");
    let second = extra_path.join("Second.c4d");
    let second_graphics = second.join("Graphics.c4g");
    let unused = extra_path.join("Unused.c4d");
    let unused_graphics = unused.join("Graphics.c4g");
    for path in [
        &extra_graphics,
        &objects_graphics,
        &second_graphics,
        &unused_graphics,
    ] {
        fs::create_dir_all(path).test_value();
    }
    write_preview_png(
        &extra_graphics.join("ChildWins.png"),
        [0x11, 0x22, 0x33, 0xff],
    );
    write_preview_png(
        &objects_graphics.join("ChildWins.png"),
        [0x44, 0x55, 0x66, 0xff],
    );
    write_preview_png(
        &objects_graphics.join("ScenarioWins.png"),
        [0x77, 0x88, 0x99, 0xff],
    );
    write_preview_png(
        &objects_graphics.join("ChildTie.png"),
        [0x12, 0x34, 0x56, 0xff],
    );
    write_preview_png(
        &second_graphics.join("ChildTie.png"),
        [0x65, 0x43, 0x21, 0xff],
    );
    write_preview_png(
        &unused_graphics.join("UnusedWins.png"),
        [0xaa, 0xbb, 0xcc, 0xff],
    );
    write_preview_png(&extra_path.join("LoaderRoot.png"), [0x11, 0x22, 0x33, 0xff]);
    write_preview_png(&objects.join("LoaderObjects.png"), [0x44, 0x55, 0x66, 0xff]);
    write_preview_png(&second.join("LoaderSecond.png"), [0x65, 0x43, 0x21, 0xff]);
    write_preview_png(&unused.join("LoaderUnused.png"), [0xaa, 0xbb, 0xcc, 0xff]);
    let extra_materials = extra_path.join("Material.c4g");
    let global_materials = paths.planet_dir().join("Material.c4g");
    for (path, marker) in [
        (&objects_materials, b"child".as_slice()),
        (&extra_materials, b"root".as_slice()),
        (&global_materials, b"global".as_slice()),
    ] {
        fs::create_dir_all(path).test_value();
        fs::write(path.join("Source.txt"), marker).test_value();
    }

    let startup = startup_loader_registrations(&paths).test_value();
    main_assert_eq!(startup.len() => 1);
    main_assert_eq!(startup[0].priority => 2);
    main_assert_eq!(startup[0].group.root() => extra_path.as_path());

    let scenario_path = content.join("Actual.c4s");
    let (scenario, scenario_group, head) = loader_origin_fixture_scenario(&scenario_path, "");
    let scenario_graphics = scenario_path.join("Graphics.c4g");
    fs::create_dir(&scenario_graphics).test_value();
    write_preview_png(
        &scenario_graphics.join("ScenarioWins.png"),
        [0xdd, 0xee, 0xff, 0xff],
    );
    let definitions = ScenarioDefinitionLoad::Fixed {
        modules: vec!["Objects.c4d".to_string(), "Second.c4d".to_string()],
        definition_root: None,
    };
    let registrations =
        classic_loader_registrations(&scenario, &scenario_group, &head, &definitions, &paths)
            .test_value();
    main_assert_eq!(
        registrations
            .iter()
            .map(|registration| (registration.priority, registration.group.root()))
            .collect::<Vec<_>>() =>
        [
            (200, scenario_path.as_path()),
            (2, extra_path.as_path()),
            (3, objects.as_path()),
            (3, second.as_path()),
        ]
    );
    main_assert!(registrations.iter().all(|registration| registration.group.root() != unused.as_path()));

    let loader_tier = highest_loader_tier(&registrations).test_value();
    main_assert_eq!(
        loader_tier
            .iter()
            .map(|group| group.root())
            .collect::<Vec<_>>() =>
        [second.as_path(), objects.as_path()],
        "priority-3 children pool later-first and exclude the priority-2 Extra root"
    );

    let graphics = loader_graphics_registrations(&registrations).test_value();
    let base_graphics = paths.planet_dir().join("Graphics.c4g");
    fs::create_dir(&base_graphics).test_value();
    let base = Group::open(&base_graphics).test_value();
    main_assert_eq!(
        load_named_graphics_image("ChildWins", &graphics, &base)
            .expect("activated child graphic")
            .pixels() =>
        [0x44, 0x55, 0x66, 0xff],
        "the activated child sits above the Extra root"
    );
    main_assert_eq!(
        load_named_graphics_image("ScenarioWins", &graphics, &base)
            .expect("scenario graphic")
            .pixels() =>
        [0xdd, 0xee, 0xff, 0xff],
        "scenario graphics remain above activated Extra children"
    );
    main_assert_eq!(
        load_named_graphics_image("ChildTie", &graphics, &base)
            .expect("equal-priority Extra child graphic")
            .pixels() =>
        [0x12, 0x34, 0x56, 0xff],
        "RegisterMainGroups reverses the child tie a second time"
    );
    main_assert!(select_named_graphics_image_source("UnusedWins", &graphics, &base).is_err());

    let first_definition_root = content.join("Objects.c4d");
    let second_definition_root = content.join("Second.c4d");
    fs::create_dir(&first_definition_root).test_value();
    fs::create_dir(&second_definition_root).test_value();
    let definition_roots = [
        Group::open(&first_definition_root).test_value(),
        Group::open(&second_definition_root).test_value(),
    ];
    let resolver = InstallDefinitionResolver::new(Some(Arc::new(paths.clone())));
    let resolved_graphics = resolver
        .resolve_graphics_groups_with_definition_roots(&scenario_group, &definition_roots)
        .test_value();
    main_assert_eq!(
        resolved_graphics
            .iter()
            .map(|group| group.root())
            .collect::<Vec<_>>() =>
        [
            scenario_graphics.as_path(),
            objects_graphics.as_path(),
            second_graphics.as_path(),
            extra_graphics.as_path(),
            base_graphics.as_path(),
        ]
    );

    let materials = resolver
        .resolve_material_groups(&scenario_group)
        .test_value();
    main_assert_eq!(
        materials
            .iter()
            .map(|group| group.root())
            .collect::<Vec<_>>() =>
        [extra_materials.as_path(), global_materials.as_path()],
        "Extra child materials register after C4GameParameters snapshots the material chain"
    );
}

#[test]
fn classic_loader_setup_accepts_an_activated_cosmetic_extra_child() {
    let root = tempdir();
    install_global_gui_and_loader_test_root(root.path());
    let content = root.path().join("content");
    let scenario_path = content.join("Scenario.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Cosmetic Extra\nLoader=LoaderExtra\n",
    )
    .test_value();
    fs::create_dir(content.join("Objects.c4d")).test_value();
    let extra_child = root.path().join("planet/Extra.c4g/Objects.c4d");
    fs::create_dir_all(&extra_child).test_value();
    write_preview_png(
        &extra_child.join("LoaderExtra.png"),
        [0x12, 0x34, 0x56, 0xff],
    );
    fs::write(extra_child.join("Graphics.c4g"), b"not a group").test_value();
    let user = root.path().join("user");
    let _guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(root.path())),
        ("LC_CONTENT_DIR", Some(content.as_path())),
        ("LC_USER_DATA_DIR", Some(user.as_path())),
    ]);
    let paths = test_app_paths();
    paths.ensure_user_dirs().test_value();
    persist_config_value(&paths, "General", "LanguageEx", "US").test_value();
    let assets = FrontendAssets::load(Some(&paths));
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "Scenario.c4s".to_string();
    scenario.title = "Cosmetic Extra".to_string();
    scenario.kind = ScenarioKind::Scenario;
    scenario.path = Some(scenario_path);
    let setup = build_scenario_loader(
        &scenario,
        &ScenarioDefinitionLoad::Fixed {
            modules: vec!["Objects.c4d".to_string()],
            definition_root: None,
        },
        &paths,
        &assets,
    )
    .test_value();
    main_assert_eq!(setup.screen.selection().selected_filename() => "LoaderExtra.png");
}

#[test]
fn distinct_global_extra_hits_take_ambiguity_boundary() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    fs::create_dir(paths.planet_dir().join("Extra.c4g")).test_value();
    fs::create_dir(content.join("Extra.c4g")).test_value();

    let error = startup_loader_registrations(&paths).err().test_value();
    main_assert!(error.to_string().contains("mapping is ambiguous"));
    main_assert!(error.to_string().contains("planet"));
    main_assert!(error.to_string().contains("content"));
}

#[test]
fn content_only_extra_cannot_enter_mapped_global_loader_set() {
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let extra_path = content.join("Extra.c4g");
    fs::create_dir(&extra_path).test_value();
    write_preview_png(
        &extra_path.join("LoaderUnmapped.png"),
        [0x12, 0x34, 0x56, 0xff],
    );

    let error = startup_loader_registrations(&paths).err().test_value();
    main_assert!(error.to_string().contains("outside the mapped global-data namespace"));
    main_assert!(error.to_string().contains(&extra_path.display().to_string()));
}

#[test]
fn invalid_present_loader_graphics_booleans_take_typed_boundary() {
    let root = tempdir();
    let (_guard, paths, _) = loader_origin_fixture_paths(root.path());
    paths.ensure_user_dirs().test_value();
    for key in ["PointFiltering", "DisableGamma"] {
        let mut config = Config::new();
        config.set_in(Some("Graphics"), key, "on");
        config.save(paths.config_file()).test_value();
        let error = validate_classic_loader_graphics_config(&paths)
            .expect_err("invalid-present legacy boolean must not default silently");
        main_assert!(error.to_string().contains(key));
        main_assert!(error.to_string().contains("expected 1, 0, true, or false"));
    }
}

#[test]
fn classic_loader_scale_numbers_require_full_decimal_i32() {
    main_assert_eq!(parse_classic_loader_i32(" 2147483647 ") => Some(i32::MAX));
    main_assert_eq!(parse_classic_loader_i32("-1") => Some(-1));
    main_assert_eq!(parse_classic_loader_i32("0x808080") => None);
    main_assert_eq!(parse_classic_loader_i32("2147483648") => None);
    main_assert_eq!(parse_classic_loader_i32("123tail") => None);
}

#[test]
fn assetless_loading_mode_fails_instead_of_drawing_generic_loader() {
    let mut app = new_menu_app(320, 200);
    app.mode = AppMode::Loading;
    let mut frame = vec![0_u8; 320 * 200 * 4];
    let error = app
        .render(&mut frame)
        .expect_err("generic loader approximation must not render");
    main_assert!(matches!(
        error.downcast_ref::<ClassicParityBoundary>(),
        Some(ClassicParityBoundary::LoaderScreen { context, detail })
            if *context == "startup loading"
                && detail.contains("application paths are unavailable")
    ));
}

#[test]
fn abandoning_the_network_lobby_reinitializes_the_startup_loader_screen() {
    // A left lobby unwinds through C4Application::QuitGame, which re-enters
    // PreInit; PreInit re-runs InitLoaderScreen(C4CFN_StartupBackgroundMain)
    // before DoStartup reconstructs the startup dialog
    // (src/C4Application.cpp:242-247,373-389,418-421). C4Game::Init's join
    // branch creates a loader only when none exists
    // (src/C4Game.cpp:371-381), so the next network join must still find
    // the startup loader installed instead of the loader boundary.
    let install = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let user_data = tempdir();
    let (_guard, paths) = isolated_test_app_paths(install.path(), user_data.path());
    paths.ensure_user_dirs().test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let installed = app
        .loader_screen
        .test_ref()
        .selection()
        .selected_filename()
        .to_string();

    app.replace_startup_view(StartupView::NetworkLobby);
    app.show_main_menu();

    main_assert_eq!(
        app.loader_screen
            .as_ref()
            .map(|loader| loader.selection().selected_filename().to_string()) =>
        Some(installed),
        "returning to the startup menu re-enters PreInit, which reinstalls the loader"
    );
    main_assert!(app.loader_error.is_none());

    // The join that follows draws behind that retained loader instead of
    // taking the loader boundary and killing the process.
    app.mode = AppMode::Loading;
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);
}

/// `PreInit` asks for `C4CFN_StartupBackgroundMain`, but `C4LoaderScreen::Init`
/// falls through to the general `Loader*` wildcard when that named loader is
/// absent, so a classic pack without `LoaderGoldmine1.png` still boots
/// (src/C4Application.cpp:240-248; src/C4LoaderScreen.cpp:75-87).
#[test]
fn startup_main_uses_classic_loader_wildcard_when_goldmine_is_absent() {
    let install = tempdir();
    install_global_gui_test_root(install.path(), None);
    let repository = test_repository_root();
    // The pack ships a loader, just not the requested one.
    for name in [
        "LoaderWatercave1.png",
        "Logo.png",
        "StartupBigButton.png",
        "StartupBigButtonDown.png",
    ] {
        fs::copy(
            repository.join("planet/Graphics.c4g").join(name),
            install.path().join("planet/Graphics.c4g").join(name),
        )
        .unwrap_or_else(|error| panic!("copy fixture {name}: {error}"));
    }
    let user_data = tempdir();
    let (_guard, paths) = isolated_test_app_paths(install.path(), user_data.path());
    paths.ensure_user_dirs().test_value();
    let app = new_menu_app_with_paths(640, 480, &paths);

    main_assert!(app.assets.menu_background.is_some(), "the wildcard fallback must supply the startup background");
    main_assert!(app.assets.require_classic_startup_main_resources().is_ok(), "preflight must not demand the named loader once a wildcard match exists");
    main_assert_eq!(app.loader_screen.as_ref().map(|loader| loader.selection().selected_filename().to_string()) => Some("LoaderWatercave1.png".to_string()));

    // A pack with no eligible loader at all still fails the preflight, so
    // the boundary is the absent wildcard rather than the absent name.
    drop(_guard);
    let bare = tempdir();
    install_global_gui_test_root(bare.path(), None);
    let (_bare_guard, bare_paths) = isolated_test_app_paths(bare.path(), user_data.path());
    bare_paths.ensure_user_dirs().test_value();
    let bare_assets = FrontendAssets::load(Some(&bare_paths));
    main_assert!(bare_assets.menu_background.is_none());
    main_assert!(matches!(
        bare_assets.require_classic_startup_main_resources(),
        Err(ClassicParityBoundary::StartupMainResources { missing })
            if missing.contains(&"LoaderGoldmine1.png")
    ));
}

#[test]
fn failed_startup_network_restart_reinitializes_the_startup_loader_screen() {
    // A failed host/join runs the same QuitGame -> PreInit -> DoStartup
    // shortcut, so PreInit re-initializes the loader screen before the
    // remembered dialog returns (src/C4Application.cpp:242-247,373-389).
    let install = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let user_data = tempdir();
    let (_guard, paths) = isolated_test_app_paths(install.path(), user_data.path());
    paths.ensure_user_dirs().test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    main_assert!(app.loader_screen.is_some());

    app.startup_restart_diagnostics.mark_quit_with_error();
    app.startup_restart_diagnostics
        .add_fatal_error("fixture join failure");
    app.finish_startup_network_restart(StartupNetworkPurpose::Join)
        .test_value();

    main_assert_eq!(
        app.loader_screen
            .as_ref()
            .map(|loader| loader.selection().selected_filename().to_string()) =>
        Some("LoaderGoldmine1.png".to_string()),
        "PreInit reinstalls the startup background loader for the next game"
    );
    main_assert!(app.loader_error.is_none());
}

#[test]
fn failed_local_scenario_load_reinitializes_the_startup_loader_screen() {
    // C4Application::OpenGame's failed ordinary fullscreen start also
    // unwinds through QuitGame -> PreInit, which re-initializes the loader
    // before the startup dialog returns (src/C4Application.cpp:242-247,
    // 373-389,442-451).
    let install = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let user_data = tempdir();
    let (_guard, paths) = isolated_test_app_paths(install.path(), user_data.path());
    paths.ensure_user_dirs().test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    main_assert!(app.loader_screen.is_some());
    app.mode = AppMode::Loading;

    app.finish_scenario_loading_failure("controlled local load failure".to_string(), false)
        .test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(
        app.loader_screen
            .as_ref()
            .map(|loader| loader.selection().selected_filename().to_string()) =>
        Some("LoaderGoldmine1.png".to_string()),
        "PreInit reinstalls the startup background loader for the next game"
    );
    main_assert!(app.loader_error.is_none());
}

#[test]
fn missing_explicit_definition_during_scenario_start_returns_to_startup() {
    // C4Game::OpenScenario loads every explicit definition before the loader
    // screen (C4Game.cpp:181-239; C4GameParameters.cpp:192-207). A missing
    // definition is therefore an ordinary failed OpenGame, which
    // C4Application::Execute recovers through QuitGame instead of propagating
    // a click-handler error (C4Application.cpp:442-450).
    let _lock = env_lock().lock();
    let install = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let user_data = tempdir();
    let (_guard, paths) = isolated_test_app_paths(install.path(), user_data.path());
    let scenario_path = install.path().join("content/MissingDefinition.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Missing definition\nLoader=LoaderGoldmine1*\n",
    )
    .test_value();

    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.last_startup_dialog = StartupDialog::ScenarioBrowser(ScenarioSelectorMode::Local);
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "MissingDefinition.c4s".to_string();
    scenario.title = "Missing definition".to_string();
    scenario.kind = ScenarioKind::Scenario;
    scenario.path = Some(scenario_path);

    let result = app.start_scenario_with_definition_load(
        scenario,
        ScenarioDefinitionLoad::Fixed {
            modules: vec!["Missing.c4d".to_string()],
            definition_root: None,
        },
    );
    main_assert!(
        result.is_ok(),
        "a missing explicit definition must return through startup recovery: {result:?}"
    );
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert!(!app.take_exit_request());
    main_assert!(app.loader_screen.is_some());
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.scenario_selector_mode => ScenarioSelectorMode::Local);
    main_assert!(
        app.message_dialogs
            .iter()
            .any(|dialog| dialog.state.message().contains("Missing.c4d")),
        "the missing definition is presented through startup diagnostics"
    );
}

#[test]
fn missing_explicit_definition_under_definition_path_returns_to_startup() {
    let _lock = env_lock().lock();
    let install = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let user_data = tempdir();
    let (_guard, paths) = isolated_test_app_paths(install.path(), user_data.path());
    let scenario_path = install.path().join("content/MissingDefinition.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Missing definition\nLoader=LoaderGoldmine1*\n",
    )
    .test_value();

    let definition_root = tempdir();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.last_startup_dialog = StartupDialog::ScenarioBrowser(ScenarioSelectorMode::Local);
    let mut scenario = FrontendScenario::fallback();
    scenario.identifier = "MissingDefinition.c4s".to_string();
    scenario.title = "Missing definition".to_string();
    scenario.kind = ScenarioKind::Scenario;
    scenario.path = Some(scenario_path);

    let result = app.start_scenario_with_definition_load(
        scenario,
        ScenarioDefinitionLoad::Fixed {
            modules: vec!["Missing.c4d".to_string()],
            definition_root: Some(path_with_trailing_native_separator(definition_root.path())),
        },
    );
    main_assert!(
        result.is_ok(),
        "a missing rooted explicit definition must return through startup recovery: {result:?}"
    );
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert!(!app.take_exit_request());
    main_assert_eq!(app.startup_view => StartupView::ScenarioBrowser);
    main_assert!(
        app.message_dialogs
            .iter()
            .any(|dialog| dialog.state.message().contains("Missing.c4d")),
        "the missing rooted definition is presented through startup diagnostics"
    );
}

#[test]
fn a_command_line_scenario_that_fails_to_load_ends_the_process() {
    // `ParseCommandLine` clears `UseStartupDialog` for an explicit scenario
    // (C4Game.cpp:3299), so the failed `OpenGame` unwinds `QuitGame` into
    // `Quit()` rather than reconstructing a startup dialog
    // (C4Application.cpp:373-405,438-450). Settling into `AppMode::Menu`
    // instead leaves a headless server running with nothing to run and no
    // menu it could ever draw.
    let mut app = new_state_only_menu_app(320, 200);
    app.headless = true;
    app.classic_command_line.scenario = Some(PathBuf::from("Broken.c4s"));
    app.mode = AppMode::Loading;

    app.finish_scenario_loading_failure("controlled command-line load failure".to_string(), false)
        .test_value();

    main_assert!(app.take_exit_request(), "there is no startup generation to return to");

    // A console `/open` failure is the other branch: `/open` sets
    // `UseStartupDialog` back (C4Application.cpp:598-612), so the engine
    // returns to `C4AS_Startup` and waits for the operator's next command.
    let mut opened = new_state_only_menu_app(320, 200);
    opened.headless = true;
    opened.classic_command_line.scenario = Some(PathBuf::from("Broken.c4s"));
    opened.console_restored_startup_dialog = true;
    opened.mode = AppMode::Loading;

    opened
        .finish_scenario_loading_failure("controlled /open load failure".to_string(), false)
        .test_value();

    main_assert_eq!(opened.mode => AppMode::Menu);
    main_assert!(!opened.take_exit_request(), "a console-opened failure waits for the next command instead");
}

#[test]
fn pathless_startup_skips_boot_worker_without_bypassing_loader_failure() {
    let mut app = test_game_app(320, 200, AudioOptions::default(), None).test_value();
    install_classic_test_assets(&mut app);
    main_assert!(app.boot_loading.is_none(), "pathless app skips boot worker");
    app.test_update();
    main_assert_eq!(app.mode => AppMode::Loading);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    main_assert!(app.render(&mut frame).is_err());
}

#[test]
fn loader_selector_repeats_explicit_extension_passes_and_cpp_wildcards() {
    let directory = tempdir();
    fs::write(directory.path().join("LoaderOne.png"), b"not decoded here").test_value();
    let group = Group::open(directory.path()).test_value();
    let graphics = group.clone();
    let mut ranges = Vec::new();
    let selected = select_loader_source(&[group], &graphics, "LoaderOne.png", |range| {
        ranges.push(range);
        if range == 1 {
            0
        } else {
            range - 1
        }
    })
    .test_value();
    main_assert_eq!(selected.entry.relative_path => PathBuf::from("LoaderOne.png"));
    main_assert_eq!(ranges => [1, 2, 3, 7]);
    main_assert!(classic_wildcard_match(b"*.*", b"extensionless"));
    main_assert_eq!(loader_patterns("LoaderTrailing.").expect("patterns").png => "LoaderTrailing..png");
    main_assert!(loader_patterns("Loader\0Hidden").is_err());
}

#[test]
fn raw_graphics_and_loader_lookup_ignores_unrelated_opaque_names() {
    let directory = tempdir();
    let image_path = directory.path().join("pixel.png");
    write_preview_png(&image_path, [11, 22, 33, 255]);
    let image_bytes = fs::read(image_path).test_value();

    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let opaque_path = PathBuf::from(OsStr::from_bytes(b"Opaque\xfe.bin"));
        main_assert!(opaque_path.to_str().is_none());
        let entry = |relative_path: PathBuf, name_bytes: &[u8]| GroupEntry {
            relative_path,
            name_bytes: name_bytes.to_vec(),
            is_directory: false,
            size: 1,
            time: 0,
            executable: false,
            crc_state: 0,
            stored_crc: 0,
        };
        let raw_entries = vec![
            entry(opaque_path, b"Opaque\xfe.bin"),
            entry(PathBuf::from("LoaderGood.png"), b"LoaderGood.png"),
            entry(PathBuf::from("Player.png"), b"Player.png"),
        ];
        main_assert!(loader_entries_have_content(&raw_entries));
        let graphic =
            find_classic_named_entry_from_entries(raw_entries, b"player.PNG").test_value();
        main_assert_eq!(graphic.name_bytes => b"Player.png");

        #[cfg(not(target_os = "macos"))]
        {
            let raw_directory = directory.path().join("Directory.c4g");
            fs::create_dir(&raw_directory).test_value();
            fs::write(
                raw_directory.join(OsStr::from_bytes(b"Opaque\xfe.bin")),
                b"unrelated",
            )
            .test_value();
            fs::write(raw_directory.join("LoaderGood.png"), &image_bytes).test_value();
            fs::write(raw_directory.join("Player.png"), &image_bytes).test_value();
            let directory_group = Group::open(&raw_directory).test_value();

            main_assert!(loader_group_has_content(&directory_group).expect("opaque sibling does not abort loader classification"));
            let selected = select_loader_source(
                std::slice::from_ref(&directory_group),
                &directory_group,
                "LoaderGood",
                |_| 0,
            )
            .test_value();
            main_assert_eq!(selected.filename_bytes() => b"LoaderGood.png");
            let graphic = find_classic_named_entry(&directory_group, "player.PNG")
                .expect("opaque sibling does not abort named graphics lookup")
                .test_value();
            main_assert_eq!(graphic.name_bytes => b"Player.png");
        }
    }

    let mut fixture = MutableGroup::new_bytes(b"Fixture.bin".to_vec());
    fixture
        .add_file_bytes_with_metadata(b"Opaque\xfe.bin".to_vec(), b"unrelated".to_vec(), 1, false)
        .test_value();
    fixture
        .add_file("LoaderGood.png", image_bytes.clone())
        .test_value();
    fixture
        .add_file_bytes_with_metadata(b"Loader\xff.png".to_vec(), image_bytes.clone(), 1, false)
        .test_value();
    fixture
        .add_file("Player.png", image_bytes.clone())
        .test_value();
    let group = Group::from_raw_memory(
        PathBuf::from("Fixture.bin"),
        fixture.pack_raw().expect("pack raw lookup fixture"),
    )
    .test_value();

    main_assert!(loader_group_has_content(&group).expect("classify loader group"));
    let selected = select_loader_source(std::slice::from_ref(&group), &group, "LoaderGood", |_| 0)
        .test_value();
    main_assert_eq!(selected.filename_bytes() => b"LoaderGood.png");
    main_assert_eq!(decode_selected_loader(&selected).expect("decode selected ASCII loader").pixels() => [11, 22, 33, 255]);

    let graphic = find_classic_named_entry(&group, "player.PNG")
        .expect("enumerate named graphics")
        .test_value();
    main_assert_eq!(graphic.name_bytes => b"Player.png");

    let opaque_specification = clonk_script::c4_string_from_bytes(b"Loader\xff.png");
    let selected = select_loader_source(
        std::slice::from_ref(&group),
        &group,
        &opaque_specification,
        |_| 0,
    )
    .test_value();
    main_assert_eq!(selected.filename_bytes() => b"Loader\xff.png");
    main_assert_eq!(decode_selected_loader(&selected).expect("selected opaque loader is read by exact entry identity").pixels() => [11, 22, 33, 255]);
}

#[test]
fn loader_tier_keeps_later_equal_priority_registration_first() {
    let directory = tempdir();
    let lower = directory.path().join("lower.c4f");
    let actual = directory.path().join("actual.c4f");
    let origin = directory.path().join("origin.c4f");
    for path in [&lower, &actual, &origin] {
        fs::create_dir(path).test_value();
        fs::write(path.join("LoaderTier.png"), b"candidate").test_value();
    }
    let registrations = vec![
        startup_fixture!(loader_group: 100, 0, Group::open(&lower).expect("lower group")),
        startup_fixture!(loader_group: 101, 1, Group::open(&actual).expect("actual group")),
        startup_fixture!(loader_group: 101, 2, Group::open(&origin).expect("origin group")),
    ];
    let tier = highest_loader_tier(&registrations).test_value();
    main_assert_eq!(tier.len() => 2);
    main_assert_eq!(tier[0].root() => origin.as_path());
    main_assert_eq!(tier[1].root() => actual.as_path());
}

#[test]
fn loader_selector_includes_child_group_names_then_decode_fails() {
    let directory = tempdir();
    fs::create_dir(directory.path().join("LoaderChild.png")).test_value();
    let group = Group::open(directory.path()).test_value();
    let selected = select_loader_source(std::slice::from_ref(&group), &group, "LoaderChild", |_| 0)
        .test_value();
    main_assert!(decode_selected_loader(&selected).is_err());
}

#[test]
fn loader_decoder_uses_selected_filename_extension_instead_of_magic() {
    let directory = tempdir();
    write_preview_png(
        &directory.path().join("LoaderRenamed.jpg"),
        [0x11, 0x22, 0x33, 0xff],
    );
    let group = Group::open(directory.path()).test_value();
    let selected =
        select_loader_source(std::slice::from_ref(&group), &group, "LoaderRenamed", |_| 0)
            .test_value();
    main_assert_eq!(selected.entry.relative_path => PathBuf::from("LoaderRenamed.jpg"));
    main_assert!(decode_selected_loader(&selected).is_err(), "C4Surface dispatches to JPEG for a .jpg entry and rejects PNG bytes");
}

#[test]
fn selected_loader_decoder_uses_the_same_transparent_pixel_invariant() {
    let directory = tempdir();
    image::RgbaImage::from_raw(2, 1, vec![17, 34, 51, 0, 68, 85, 102, 1])
        .expect("rgba image")
        .save(directory.path().join("Loader.png"))
        .test_value();
    let group = Group::open(directory.path()).test_value();
    let entry = find_classic_named_entry(&group, "Loader.png")
        .expect("enumerate loader group")
        .test_value();
    let selected = SelectedLoaderSource { group, entry };

    main_assert_eq!(decode_selected_loader(&selected).expect("decode loader").pixels() => &[0, 0, 0, 0, 68, 85, 102, 1]);
}

#[test]
fn startup_loader_render_uses_configured_user_gamma() {
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    paths.ensure_user_dirs().test_value();
    let mut config = Config::new();
    config.set_in(Some("Graphics"), "Gamma1", "0");
    config.set_in(Some("Graphics"), "Gamma2", "6579300");
    config.set_in(Some("Graphics"), "Gamma3", "13158600");
    config.save(paths.config_file()).test_value();
    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    main_assert_eq!(app.loader_gamma => Some(clonk_graphics::GammaRamp::from_control_points([0, 0x646464, 0xc8c8c8,])));
    let mut corrected = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut corrected);
    app.loader_gamma = Some(clonk_graphics::GammaRamp::standard());
    let mut standard = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut standard);
    main_assert_ne!(corrected => standard);

    config.set_in(Some("Graphics"), "DisableGamma", "true");
    config.save(paths.config_file()).test_value();
    main_assert_eq!(load_classic_loader_gamma(Some(&paths)) => None);
    app.loader_gamma = None;
    let current_renderer_config = app.graphics.advanced_renderer_config();
    app.graphics
        .set_advanced_renderer_config(clonk_frontend::AdvancedRendererConfig {
            disable_gamma: true,
            ..current_renderer_config
        });
    let mut disabled = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut disabled);
    main_assert!(disabled.chunks_exact(4).zip(standard.chunks_exact(4)).any(|(raw, corrected)| raw[..3].contains(&0) && raw != corrected));
}

#[test]
fn app_loader_keeps_progress_monotonic_and_retains_phase_status() {
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let mut app = test_game_app(320, 200, AudioOptions::default(), Some(&paths)).test_value();
    let resources = app.loader_screen.test_ref().resources().clone();
    let (sender, receiver) = mpsc::channel();
    app.loading_state = Some(ScenarioLoadingState::new(
        FrontendScenario::fallback(),
        resources,
        HashMap::new(),
        Vec::new(),
        receiver,
    ));
    let mut reporter = ScenarioLoadingReporter::new(sender);
    reporter.report(42, "Exact phase-status line");
    reporter.report(40, "Definition metadata and sources collected");
    reporter.send(ScenarioLoadingEvent::RefreshResources);
    app.poll_loading().test_value();
    let state = app.loader_screen.test_ref().state();
    main_assert_eq!(state.progress() => 42);
    main_assert_eq!(state.process() => None);
    main_assert!(matches!(
        state.log(),
        clonk_frontend::loader_screen::LoaderLog::Visible(lines)
            if lines.last().map(String::as_str)
                == Some("Definition metadata and sources collected")
    ));
}

#[test]
fn real_legacy_worker_updates_live_loader_through_activation() {
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    configure_test_startup_participant(&paths, user_data.path());
    let mut app = GameApp::new(
        320,
        200,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with("Loader parity".to_string(), false),
    )
    .test_value();
    wait_for_menu(&mut app);
    let scenario =
        resolve_next_mission_scenario(&app.scenario_catalog, "Tutorial.c4f/Tutorial01.c4s")
            .test_value();
    main_assert!(scenario.path.as_ref().expect("scenario path").join("Scenario.txt").is_file());

    app.start_scenario(scenario).test_value();
    wait_for_running_with_attempts(&mut app, 2_400);

    let state = app.loader_screen.test_ref().state();
    main_assert_eq!(state.progress() => 100);
    let clonk_frontend::loader_screen::LoaderLog::Visible(lines) = state.log() else {
        panic!("worker phase status must make the live loader log visible");
    };
    let mut previous = None;
    for expected in [
        "Scenario manifest and components decoded",
        "Definition metadata and sources collected",
        "Scenario script sources loaded",
        "Landscape data generated or decoded",
        "Object records decoded",
        "Players initialized",
        "Scenario activation complete",
    ] {
        let index = lines
            .iter()
            .position(|line| line == expected)
            .unwrap_or_else(|| panic!("missing cumulative loader status `{expected}`: {lines:?}"));
        if let Some(previous) = previous {
            main_assert!(index > previous, "loader status order regressed: {lines:?}");
        }
        previous = Some(index);
    }
}

#[test]
fn player_selection_wheel_and_held_arrow_route_through_app() {
    use clonk_frontend::startup_plrsel::{plrsel_layout, PlrSelController, PlrSelPlayer};

    let mut app = new_real_classic_menu_app(640, 480);
    app.startup_player_models = (0..20)
        .map(|index| PlrSelPlayer {
            name: format!("Player {index:02}"),
            activated: false,
            big_icon: None,
            portrait: None,
            color_dw: 0xff,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            comment: String::new(),
        })
        .collect();
    let mut controller = PlrSelController::new(app.startup_player_models.len());
    controller.resize(640, 480);
    main_assert_eq!(controller.list_max_scroll() => 300);
    app.startup_view = StartupView::PlayerSelection;
    app.startup_player_dialog = Some(controller);

    let layout = plrsel_layout(640, 480);
    app.test_cursor(PhysicalPosition::new(
        f64::from(layout.list_viewport.x + 4),
        f64::from(layout.list_viewport.y + 4),
    ));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").list_scroll_offset() => 60);
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0);
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").list_scroll_offset() => 0);

    app.test_cursor(PhysicalPosition::new(
        f64::from(layout.list_scrollbar.x + 8),
        f64::from(layout.list_scrollbar.y + layout.list_scrollbar.h - 8),
    ));
    app.test_left_button(ElementState::Pressed);
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);
    let first = app.startup_player_dialog.test_ref().list_scroll_offset();
    app.test_render(&mut frame);
    let second = app.startup_player_dialog.test_ref().list_scroll_offset();
    main_assert_eq!((first, second) => (1, 3));

    app.test_left_button(ElementState::Released);
    app.test_render(&mut frame);
    main_assert_eq!(app.startup_player_dialog.as_ref().expect("player dialog").list_scroll_offset() => second);

    app.test_cursor(PhysicalPosition::new(
        f64::from(layout.list_scrollbar.x + 8),
        f64::from(layout.list_scrollbar.y + layout.list_scrollbar.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    let first_row_name = PhysicalPosition::new(
        f64::from(layout.list_viewport.x + layout.item_height * 3),
        f64::from(layout.list_viewport.y + layout.item_height / 2),
    );
    app.test_cursor(first_row_name);
    app.test_left_button(ElementState::Released);
    main_assert!(app.plrsel_last_click.is_none(), "scrollbar release must not seed row double-click bookkeeping");

    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.plrsel_last_click.map(|(index, _)| index) => Some(0), "the first genuine row click must remain a single click");
    main_assert!(app.startup_player_properties_dialog.is_none());
}

#[test]
fn startup_dialog_fade_uses_classic_ten_presentation_ramp() {
    main_assert_eq!(
        (1..=STARTUP_DIALOG_FADE_STEPS)
            .map(|step| startup_dialog_fade_opacity(step * 10))
            .collect::<Vec<_>>() =>
        vec![26, 51, 77, 102, 128, 153, 179, 204, 230, 255]
    );

    let underlay = [10_u8, 20, 30, 40];
    let outgoing = [110_u8, 120, 130, 140];
    let incoming = [210_u8, 220, 230, 240];
    for (percent, expected) in [
        (10, [111, 121, 131, 141]),
        (50, [135, 145, 155, 165]),
        (90, [191, 201, 211, 221]),
        (100, incoming),
    ] {
        let mut actual = incoming;
        blend_startup_dialog_frames(&underlay, Some(&outgoing), &mut actual, percent);
        main_assert_eq!(actual => expected, "independent source-over at {percent}%");
    }
    let mut incoming_only = incoming;
    blend_startup_dialog_frames(&underlay, None, &mut incoming_only, 10);
    main_assert_eq!(incoming_only => [30, 40, 50, 60]);

    let mut app = new_real_classic_menu_app(320, 200);
    let mut main = vec![0_u8; 320 * 200 * 4];
    main_assert!(app.render(&mut main).expect("present stable Main dialog"));

    app.handle_main_menu_activation(MainMenuItem::About)
        .test_value();
    let fade = app.startup_dialog_fade.test_ref();
    main_assert_eq!(fade.outgoing => Some(StartupDialog::MainMenu));
    main_assert_eq!(fade.incoming => StartupDialog::About);
    main_assert_eq!(fade.step => 0);
    let fade_underlay = fade.underlay.clone();
    let fade_outgoing = fade.outgoing_frame.clone().test_value();
    app.test_update();
    main_assert_eq!(app.startup_dialog_fade.as_ref().unwrap().step => 0);

    let mut presented = Vec::new();
    for expected_step in 1..=STARTUP_DIALOG_FADE_STEPS {
        let mut frame = vec![0xa5; 320 * 200 * 4];
        main_assert!(app.render(&mut frame).expect("present fade frame"));
        if expected_step < STARTUP_DIALOG_FADE_STEPS {
            main_assert_eq!(app.startup_dialog_fade.as_ref().map(|fade| fade.step) => Some(expected_step));
        } else {
            main_assert!(app.startup_dialog_fade.is_none());
        }
        presented.push(frame);
    }

    let about = presented.last().test_value().clone();
    main_assert_ne!(main => about);
    for (index, actual) in presented.iter().take(9).enumerate() {
        let mut expected = about.clone();
        blend_startup_dialog_frames(
            &fade_underlay,
            Some(&fade_outgoing),
            &mut expected,
            (index as u8 + 1) * 10,
        );
        main_assert_eq!(actual => &expected, "fade presentation {}", index + 1);
    }
    main_assert_eq!(presented[9] => about, "frame ten is fully incoming");

    let mut settled = vec![0_u8; 320 * 200 * 4];
    main_assert!(app.render(&mut settled).expect("present settled About"));
    main_assert_eq!(settled => about);
    let mut again = vec![0_u8; 320 * 200 * 4];
    main_assert!(app.render(&mut again).expect("re-present settled About"));
    main_assert_eq!(again => about);
}

#[test]
fn startup_dialog_fade_suppresses_input_until_frame_ten_and_reverses() {
    let mut app = new_real_classic_menu_app(320, 200);
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);
    app.handle_main_menu_activation(MainMenuItem::About)
        .test_value();

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    app.test_gamepad_events([gamepad_action_event(
        GamepadSlot::new(0),
        GamepadActionType::Cancel,
        ElementState::Pressed,
    )]);
    main_assert_eq!(app.startup_view => StartupView::About);
    main_assert_eq!(app.startup_dialog_fade.as_ref().unwrap().step => 0);

    for expected_step in 1..=9 {
        app.test_render(&mut frame);
        main_assert_eq!(app.startup_dialog_fade.as_ref().unwrap().step => expected_step);
    }
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::About);

    app.test_render(&mut frame);
    main_assert!(app.startup_dialog_fade.is_none());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    let reverse = app.startup_dialog_fade.test_ref();
    main_assert_eq!(reverse.outgoing => Some(StartupDialog::About));
    main_assert_eq!(reverse.incoming => StartupDialog::MainMenu);
    main_assert_eq!(reverse.step => 0);

    let mut ninth = None;
    for expected_step in 1..=9 {
        app.test_render(&mut frame);
        main_assert_eq!(app.startup_dialog_fade.as_ref().unwrap().step => expected_step);
        if expected_step == 9 {
            ninth = Some(frame.clone());
        }
    }
    app.test_render(&mut frame);
    main_assert!(app.startup_dialog_fade.is_none());
    let frame_ten = frame.clone();
    let mut settled = vec![0_u8; frame.len()];
    app.test_render(&mut settled);
    main_assert_eq!(frame_ten => settled, "frame ten must already draw active focus");
    main_assert_ne!(ninth.expect("ninth frame") => frame_ten);

    app.handle_main_menu_activation(MainMenuItem::Options)
        .test_value();
    for _ in 0..STARTUP_DIALOG_FADE_STEPS {
        app.test_render(&mut frame);
    }
    app.process_options_dialog_actions(vec![
        clonk_frontend::startup_options_dlg::OptionsDlgAction::Back,
    ])
    .test_value();
    let back = app.startup_dialog_fade.test_ref();
    main_assert_eq!(back.outgoing => Some(StartupDialog::Options));
    main_assert_eq!(back.incoming => StartupDialog::MainMenu);
    main_assert_eq!(back.step => 0);
    for _ in 0..STARTUP_DIALOG_FADE_STEPS {
        app.test_render(&mut frame);
    }
    main_assert!(app.startup_dialog_fade.is_none());
}

#[test]
fn startup_dialog_fade_in_without_outgoing_suppresses_input_for_ten_frames() {
    let mut app = new_real_classic_menu_app(320, 200);
    app.begin_startup_dialog_fade_in();
    let fade = app.startup_dialog_fade.test_ref();
    main_assert_eq!(fade.outgoing => None);
    main_assert_eq!(fade.incoming => StartupDialog::MainMenu);

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(!app.take_exit_request());
    let mut frame = vec![0_u8; 320 * 200 * 4];
    for expected_step in 1..STARTUP_DIALOG_FADE_STEPS {
        app.test_render(&mut frame);
        main_assert_eq!(app.startup_dialog_fade.as_ref().unwrap().step => expected_step);
    }
    app.test_render(&mut frame);
    main_assert!(app.startup_dialog_fade.is_none());

    let mut modal_app = new_real_classic_menu_app(320, 200);
    modal_app.open_new_startup_player_properties_from(
        StartupPlayerPropertiesOrigin::MainMenuFirstPlayer,
    );
    modal_app.begin_startup_dialog_fade_in();
    for _ in 0..STARTUP_DIALOG_FADE_STEPS {
        modal_app.test_render(&mut frame);
    }
    let frame_ten = frame.clone();
    modal_app.test_render(&mut frame);
    main_assert_eq!(frame_ten => frame, "first-player modal remains visible after fade");
    main_assert!(modal_app.startup_player_properties_dialog.is_some());
}

#[test]
fn boot_loading_resize_reflows_main_menu_to_final_fullscreen_size() {
    // This is the macOS startup sequence from the reported Scale=300
    // capture: the initial logical framebuffer is 1152x644, then deferred
    // fullscreen supplies 1152x723 before boot leaves Loading.
    let mut app = new_real_classic_menu_app(1152, 644);
    app.mode = AppMode::Loading;
    app.resize(1152, 723).test_value();
    main_assert_eq!(app.mode => AppMode::Loading);
    app.mode = AppMode::Menu;
    app.show_main_menu();
    main_assert_eq!(app.startup_view => StartupView::MainMenu);

    app.graphics.set_runtime_sprite_filtering(3.0, false);
    app.configure_native_startup_fonts(3.0, false);
    let mut logical_frame = vec![0_u8; 1152 * 723 * 4];
    app.render_ordered_native_base(&mut logical_frame)
        .test_value();
    let commands = app
        .pending_native_presentation
        .test_ref()
        .batches
        .iter()
        .flat_map(|batch| batch.text.iter())
        .collect::<Vec<_>>();

    let start = commands
        .iter()
        .find(|command| command.text.ends_with("</c>tart Game"))
        .test_value();
    main_assert_eq!(
        (start.role, start.align, start.x, start.y) =>
        (
            clonk_graphics::clonk_font::ClonkFontRole::GuiTitle,
            clonk_graphics::clonk_font::TextAlign::Center,
            944,
            203
        ),
        "the whole retained button column must use final fullscreen geometry"
    );

    let participants = commands
        .iter()
        .find(|command| command.text == app.main_menu_state.participants_label)
        .test_value();
    main_assert_eq!(
        (
            participants.role,
            participants.align,
            participants.x,
            participants.y
        ) =>
        (
            clonk_graphics::clonk_font::ClonkFontRole::GuiTitle,
            clonk_graphics::clonk_font::TextAlign::Right,
            1101,
            640
        )
    );

    let fanproject = commands
        .iter()
        .find(|command| command.text.starts_with("Clonk Rust is a fan project"))
        .test_value();
    main_assert_eq!(
        (
            fanproject.role,
            fanproject.align,
            fanproject.x,
            fanproject.y
        ) =>
        (
            clonk_graphics::clonk_font::ClonkFontRole::GuiMini,
            clonk_graphics::clonk_font::TextAlign::Right,
            1129,
            695
        )
    );

    // CStdGL installs a 3456x2169 viewport into the 2168-row framebuffer,
    // producing the one-row top crop seen in the reference capture.
    let projection =
        clonk_graphics::ClipperProjection::new(3.0, (1152, 723), 2168, Rect::new(0, 0, 1152, 723));
    main_assert_eq!(projection.physical_clip() => Rect::new(0, -1, 3456, 2169));
    main_assert_eq!(projection.logical_to_physical(participants.x.into(), participants.y.into()) => (3303.0, 1919.0));
    main_assert_eq!(projection.logical_to_physical(fanproject.x.into(), fanproject.y.into()) => (3387.0, 2084.0));
}

#[test]
fn assigned_mouse_viewport_routes_only_its_player_main_menu_clicks() {
    // C++ forwards mouse input only for C4MouseControl's assigned viewport,
    // filters external dialogs to that exact viewport and its output rect,
    // then resolves C4MainMenu through the viewport's associated player
    // (pristine src/C4Viewport.cpp:505-529,546-563;
    // src/C4GraphicsSystem.cpp:445-459; src/C4GUI.cpp:802-845;
    // src/C4Menu.cpp:1114-1121; src/C4Viewport.cpp:1549-1563).
    let mut app = new_classic_running_sandbox_app();
    let primary = app.local_owner;
    let secondary = primary + 1;
    let primary_crew = app.engine.test_crew_cursor(primary);
    let primary_crew_state = app.engine.test_object_snapshot(primary_crew);

    app.engine
        .register_player(PlayerConfig::new(secondary, "Secondary"))
        .test_value();
    let secondary_position = Vector2::new(
        primary_crew_state.position.x.saturating_add(24),
        primary_crew_state.position.y,
    );
    let secondary_crew = app.engine.spawn_test_object(
        SpawnConfig::new(primary_crew_state.definition_id)
            .with_position(secondary_position)
            .with_owner(secondary)
            .with_crew_member(true),
    );
    app.engine
        .select_crew(secondary, [secondary_crew])
        .test_value();
    app.engine
        .set_crew_cursor(secondary, Some(secondary_crew))
        .test_value();
    app.engine
        .replace_player_viewports(
            secondary,
            vec![clonk_engine::PlayerViewport::new(secondary_position)
                .with_focus(Some(secondary_crew))],
        )
        .test_value();
    app.engine.set_local_players([primary, secondary]);
    app.local_controls = LocalControlRegistry::default();
    for (owner, preferred_set, prefers_mouse) in [(primary, 0, true), (secondary, 1, false)] {
        app.local_controls.initialize(test_local_control_init(
            owner,
            preferred_set,
            prefers_mouse,
            false,
        ));
    }
    app.snapshot = app.engine.snapshot();
    app.open_ingame_menu_for_player(primary).test_value();
    app.open_ingame_menu_for_player(secondary).test_value();
    let mut frame = vec![0_u8; 320 * 200 * 4];
    app.test_render(&mut frame);

    let item_point = |app: &GameApp, owner: i32, caption: &str| {
        let menu = app.ingame_menu.get(owner).test_value();
        let index = menu
            .items()
            .iter()
            .position(|item| item.caption == caption)
            .test_value();
        let area = app.graphics.viewport_rect(owner).test_value();
        let fallback = app.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            app.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let item_height = 16.max(font.line_height());
        let mut item_width = font.text_width(menu.caption()) + item_height + 16;
        for item in menu.items() {
            item_width = item_width.max(font.text_width(&item.caption) + item_height);
        }
        item_width += 3;
        let lines = (menu.items().len() as i32)
            .min(((area.height as i32 - 100) / item_height.max(1)).max(1))
            .max(1);
        let title_height = font.line_height().max(23);
        let extra_height = if app.display_flags.show_commands {
            16
        } else {
            0
        };
        let width = item_width + 4;
        let height = lines * item_height + title_height + extra_height + 2;
        let mut x = 35;
        let mut y = area.height as i32 - 35 - height;
        if width > area.width as i32 - 70 {
            x = (area.width as i32 - width) / 2;
        }
        if height > area.height as i32 - 70 {
            y = (area.height as i32 - height) / 2;
        }
        let visible = lines as usize;
        let scroll = index.saturating_add(1).saturating_sub(visible);
        let row = index - scroll;
        let item_left = (area.x + x + 2).max(area.x);
        let item_right = (area.x + x + 2 + item_width).min(area.x + area.width as i32);
        PhysicalPosition::new(
            f64::from((item_left + item_right) / 2),
            f64::from(area.y + y + title_height + row as i32 * item_height + item_height / 2),
        )
    };
    // Each half-height viewport exposes one line. Native intentionally
    // disables AdjustPosition for one-line menus, so exercise the first
    // visible row instead of relying on the removed selection-pinning.
    let primary_item = item_point(&app, primary, "Goals");
    let secondary_item = item_point(&app, secondary, "Goals");
    main_assert_eq!(app.local_controls.mouse_owner() => Some(primary));

    app.test_cursor(secondary_item);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(
        app.ingame_menu.get(primary).map(IngameMenuState::page) =>
        Some(ingame_menu::MenuPage::Main),
        "the unassigned viewport point must not clamp into the primary menu"
    );
    main_assert_eq!(
        app.ingame_menu.get(secondary).map(IngameMenuState::page) =>
        Some(ingame_menu::MenuPage::Main),
        "the unassigned viewport must not receive the click"
    );

    app.test_cursor(primary_item);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.ingame_menu.get(primary).map(IngameMenuState::page) => Some(ingame_menu::MenuPage::Goals));
    main_assert_eq!(
        app.ingame_menu.get(secondary).map(IngameMenuState::page) =>
        Some(ingame_menu::MenuPage::Main),
        "the primary action must not cross-route to the secondary menu"
    );

    app.local_controls = LocalControlRegistry::default();
    for (owner, preferred_set, prefers_mouse) in [(secondary, 1, true), (primary, 0, false)] {
        app.local_controls.initialize(test_local_control_init(
            owner,
            preferred_set,
            prefers_mouse,
            false,
        ));
    }
    main_assert_eq!(app.local_controls.mouse_owner() => Some(secondary));
    let secondary_close = {
        let area = app.graphics.viewport_rect(secondary).test_value();
        let fallback = app.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            app.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let close = app
            .ingame_menu
            .get(secondary)
            .test_value()
            .close_button_rect(
                area,
                &font,
                &IngameMenuGraphics {
                    show_commands: app.display_flags.show_commands,
                    show_close_button: true,
                    ..IngameMenuGraphics::default()
                },
            );
        GuiPoint::new(
            (close.x + close.width as i32 / 2) as f32,
            (close.y + close.height as i32 / 2) as f32,
        )
    };
    main_assert_eq!(
        app.ingame_menu_pointer_target(secondary_close) =>
        Some((secondary, IngameMenuPointerTarget::Close)),
        "the assigned secondary mouse owner's close button must hit-test"
    );
    let secondary_target = app.ingame_menu_pointer_target(gui_point_from_position(secondary_item));
    main_assert!(
            matches!(secondary_target, Some((owner, IngameMenuPointerTarget::Item(_))) if owner == secondary),
            "assigned secondary menu item must hit-test: target={secondary_target:?}, viewport={:?}, point={secondary_item:?}",
            app.graphics.viewport_rect(secondary),
        );
    app.test_cursor(secondary_item);
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.ingame_menu.get(secondary).map(IngameMenuState::page) => Some(ingame_menu::MenuPage::Goals));
    main_assert_eq!(
        app.ingame_menu.get(primary).map(IngameMenuState::page) =>
        Some(ingame_menu::MenuPage::Goals),
        "the secondary action must not cross-route to the primary menu"
    );
}

#[test]
fn activate_new_player_reoffers_an_eliminated_startup_file() {
    // ActivateNewPlayer checks Game.Players.FileInUse, not the startup
    // selection flag; a removed C4Player therefore makes its file eligible
    // again (src/C4MainMenu.cpp:59-121; src/C4PlayerList.cpp:433-451).
    let player_path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../clonk-engine/tests/fixtures/embedded_player.c4p"
    ));
    let player_file = PlayerFile::load_from_path(player_path).test_value();
    let mut app = new_running_sandbox_app();
    app.startup_player_files.push(startup_fixture!(
    startup_player:
        player_path.to_path_buf(),
        "embedded_player.c4p".to_string(),
        player_file.clone(),
        startup_fixture!(
            player_selection_name_activated_color_dw_score_rounds_rounds_won_rounds_lost_total_playing_time:
                player_file.name.clone(),
                true,
                player_file.normalized_preferred_color(),
                player_file.score,
                player_file.rounds,
                player_file.rounds_won,
                player_file.rounds_lost,
                player_file.total_playing_time,
        ),
));

    app.apply_ingame_menu_action(MenuAction::ActivateNewPlayer)
        .test_value();

    main_assert_eq!(
        app.ingame_menu
            .as_ref()
            .expect("new-player menu opens")
            .items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>() =>
        [MenuAction::JoinPlayer(
            player_path.to_string_lossy().into_owned()
        )]
    );
}

#[test]
fn activate_new_player_lists_cpp_eligible_files_in_source_order_and_closes_when_full() {
    // ActivateNewPlayer preserves DirectoryIterator order, skips directory
    // groups and files already used by a joined player, and refuses to
    // open once Game.Parameters.MaxPlayers is reached
    // (pristine 9ffa0a5d src/C4MainMenu.cpp:59-121;
    // src/C4PlayerList.cpp:433-451).
    let directory = tempdir();
    let write_packed_player = |filename: &str, name: &str| {
        let path = directory.path().join(filename);
        let mut group = clonk_resources::MutableGroup::new(filename);
        group
            .add_file_with_metadata(
                "Player.txt",
                format!("[Player]\nName={name}\n[Preferences]\nColorDw=255\n").into_bytes(),
                1,
                false,
            )
            .test_value();
        fs::write(&path, group.pack().test_value()).test_value();
        path
    };
    let zulu = write_packed_player("Zulu.c4p", "Zulu");
    let active = write_packed_player("Active.c4p", "Active");
    let alpha = write_packed_player("Alpha.c4p", "Alpha");
    let folder = directory.path().join("Folder.c4p");
    fs::create_dir_all(&folder).test_value();
    fs::write(
        folder.join("Player.txt"),
        "[Player]\nName=Folder\n[Preferences]\nColorDw=255\n",
    )
    .test_value();

    let mut config = Config::new();
    config.set_in(
        Some("General"),
        "PlayerPath",
        directory.path().to_string_lossy(),
    );
    config.set_in(Some("General"), "Participants", active.to_string_lossy());
    let mut players =
        startup_player_files::discover_player_files_in(directory.path(), &config).test_value();
    players.sort_by_key(|player| match player.player_file.name.as_str() {
        "Zulu" => 0,
        "Active" => 1,
        "Folder" => 2,
        "Alpha" => 3,
        _ => 4,
    });

    let mut app = new_running_sandbox_app();
    app.startup_player_files = players;
    app.apply_ingame_menu_action(MenuAction::ActivateNewPlayer)
        .test_value();

    let menu = app.ingame_menu.get(app.local_owner).test_value();
    main_assert_eq!(
        menu.items()
            .iter()
            .map(|item| item.caption.as_str())
            .collect::<Vec<_>>() =>
        [
            "Join player: Zulu",
            "Join player: Active",
            "Join player: Alpha",
        ]
    );
    main_assert_eq!(
        menu.items()
            .iter()
            .map(|item| item.action.clone())
            .collect::<Vec<_>>() =>
        [
            MenuAction::JoinPlayer(zulu.to_string_lossy().into_owned()),
            MenuAction::JoinPlayer(active.to_string_lossy().into_owned()),
            MenuAction::JoinPlayer(alpha.to_string_lossy().into_owned()),
        ]
    );

    app.snapshot.players = (0..12)
        .map(|id| clonk_engine::PlayerState {
            id,
            ..Default::default()
        })
        .collect();
    app.ingame_menu.clear();
    app.apply_ingame_menu_action(MenuAction::ActivateNewPlayer)
        .test_value();
    main_assert!(app.ingame_menu.is_none(), "a full game keeps the submenu closed");
}

#[test]
fn frontend_f3_and_ctrl_f3_persist_menu_audio_keys_in_startup_and_loading() {
    let mut app = new_menu_app(320, 200);
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    for (key, value) in [
        ("MenuMusic", "true"),
        ("MenuSound", "true"),
        ("Music", "true"),
        ("Sound", "true"),
        ("VendorExtension", "keep-me"),
    ] {
        persist_config_value(&paths, "Sound", key, value)
            .unwrap_or_else(|error| panic!("seed Sound.{key}: {error}"));
    }
    app.app_paths = Some(paths.clone());
    app.audio.test_mut().options.menu_music_enabled = true;
    app.audio.test_mut().options.menu_sound_enabled = true;

    let press_frontend_f3 = |app: &mut GameApp, modifiers: ModifiersState, label: &str| {
        app.handle_modifiers_changed(modifiers)
            .unwrap_or_else(|error| panic!("set modifiers for {label}: {error}"));
        app.handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .unwrap_or_else(|error| panic!("press {label}: {error}"));
        app.handle_key(VirtualKeyCode::F3, ElementState::Released)
            .unwrap_or_else(|error| panic!("release {label}: {error}"));
    };

    press_frontend_f3(&mut app, ModifiersState::empty(), "startup F3");
    let after_startup_music = {
        app.flush_deferred_config();
        Config::load(paths.config_file()).test_value()
    };
    main_assert_eq!(after_startup_music.get_in(Some("Sound"), "MenuMusic") => Some("false"));
    main_assert_eq!(after_startup_music.get_in(Some("Sound"), "MenuSound") => Some("true"), "bare F3 must not rewrite FESamples");

    press_frontend_f3(&mut app, ModifiersState::CONTROL, "startup Ctrl+F3");
    let after_startup_sound = {
        app.flush_deferred_config();
        Config::load(paths.config_file()).test_value()
    };
    main_assert_eq!(after_startup_sound.get_in(Some("Sound"), "MenuSound") => Some("false"));
    main_assert_eq!(after_startup_sound.get_in(Some("Sound"), "Music") => Some("true"));
    main_assert_eq!(after_startup_sound.get_in(Some("Sound"), "Sound") => Some("true"));
    main_assert_eq!(after_startup_sound.get_in(Some("Sound"), "VendorExtension") => Some("keep-me"));
    let startup_reload = AudioOptions::load(Some(&paths));
    main_assert!(!startup_reload.menu_music_enabled);
    main_assert!(!startup_reload.menu_sound_enabled);

    app.mode = AppMode::Loading;
    press_frontend_f3(&mut app, ModifiersState::empty(), "loading F3");
    press_frontend_f3(&mut app, ModifiersState::CONTROL, "loading Ctrl+F3");
    let after_loading = {
        app.flush_deferred_config();
        Config::load(paths.config_file()).test_value()
    };
    main_assert_eq!(after_loading.get_in(Some("Sound"), "MenuMusic") => Some("true"));
    main_assert_eq!(after_loading.get_in(Some("Sound"), "MenuSound") => Some("true"));
    main_assert_eq!(after_loading.get_in(Some("Sound"), "Music") => Some("true"));
    main_assert_eq!(after_loading.get_in(Some("Sound"), "Sound") => Some("true"));
    main_assert_eq!(after_loading.get_in(Some("Sound"), "VendorExtension") => Some("keep-me"));
    let loading_reload = AudioOptions::load(Some(&paths));
    main_assert!(loading_reload.menu_music_enabled);
    main_assert!(loading_reload.menu_sound_enabled);
}

#[test]
fn frontend_audio_toggle_write_failure_keeps_live_state() {
    let mut app = new_menu_app(320, 200);
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    app.app_paths = Some(paths.clone());
    let music_before = app.audio.test_ref().options.menu_music_enabled;
    let sound_before = app.audio.test_ref().options.menu_sound_enabled;

    fs::remove_file(paths.config_file()).test_value();
    fs::create_dir(paths.config_file()).test_value();

    app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F3, ElementState::Released);
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::F3, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F3, ElementState::Released);

    let audio = app.audio.test_ref();
    main_assert_eq!(audio.options.menu_music_enabled => !music_before);
    main_assert_eq!(audio.options.menu_sound_enabled => !sound_before);
}

#[test]
fn frontend_f3_and_ctrl_f3_recurse_through_every_startup_root_and_loading() {
    let exercise = |app: &mut GameApp, label: &str| {
        main_assert!(!matches!(app.mode, AppMode::Running), "{label}");
        let options_was_active = app.startup_options_dialog_is_active();
        let before_visual_music = app
            .startup_options_dialog
            .as_ref()
            .map(|dialog| dialog.sound().frontend_music);
        let before_music = app.audio.test_ref().options.menu_music_enabled;
        app.handle_modifiers_changed(ModifiersState::empty())
            .unwrap_or_else(|error| panic!("clear modifiers for {label}: {error}"));
        app.handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .unwrap_or_else(|error| panic!("frontend F3 over {label}: {error}"));
        main_assert_eq!(app.audio.as_ref().expect("test audio").options.menu_music_enabled => !before_music, "{label}");
        if let Some(before_visual_music) = before_visual_music {
            main_assert_eq!(
                app.startup_options_dialog
                    .as_ref()
                    .expect("retained options dialog")
                    .sound()
                    .frontend_music =>
                if options_was_active {
                    !before_music
                } else {
                    before_visual_music
                },
                "bare F3 synchronizes only the active Options dialog: {label}"
            );
        }
        main_assert!(app.runtime_flash_message.is_none(), "{label}");
        app.handle_key(VirtualKeyCode::F3, ElementState::Released)
            .unwrap_or_else(|error| panic!("frontend F3 release over {label}: {error}"));

        let before_sound = app.audio.test_ref().options.menu_sound_enabled;
        let before_visual_sound = app
            .startup_options_dialog
            .as_ref()
            .map(|dialog| dialog.sound().frontend_sound_effects);
        app.handle_modifiers_changed(ModifiersState::CONTROL)
            .unwrap_or_else(|error| panic!("set Ctrl for {label}: {error}"));
        app.handle_key(VirtualKeyCode::F3, ElementState::Pressed)
            .unwrap_or_else(|error| panic!("frontend Ctrl+F3 over {label}: {error}"));
        main_assert_eq!(app.audio.as_ref().expect("test audio").options.menu_sound_enabled => !before_sound, "{label}");
        if let Some(before_visual_sound) = before_visual_sound {
            main_assert_eq!(
                app.startup_options_dialog
                    .as_ref()
                    .expect("retained options dialog")
                    .sound()
                    .frontend_sound_effects =>
                before_visual_sound,
                "Ctrl+F3 deliberately leaves the classic checkbox stale: {label}"
            );
        }
        main_assert!(app.runtime_flash_message.is_none(), "{label}");
    };

    for view in StartupView::ALL {
        let mut app = new_running_sandbox_app();
        app.return_to_menu();
        match view {
            StartupView::MainMenu => app.show_main_menu(),
            StartupView::ScenarioBrowser => app.open_scenario_browser(),
            StartupView::NetworkLobby => {
                app.startup_view = StartupView::NetworkLobby;
                app.classic_host_lobby = None;
            }
            StartupView::NetworkGame => app.open_network_game_dialog(),
            StartupView::Options => app.open_options_menu(),
            StartupView::About => app.open_about_dialog(),
            StartupView::PlayerSelection => app.open_player_selection_dialog(),
        }
        exercise(&mut app, &format!("startup root {view:?}"));
    }

    let mut exact_lobby = new_running_sandbox_app();
    exact_lobby.return_to_menu();
    install_test_classic_host_lobby(&mut exact_lobby);
    exercise(&mut exact_lobby, "exact classic host lobby");

    for sheet in [
        clonk_frontend::startup_options_dlg::OptionsSheet::Program,
        clonk_frontend::startup_options_dlg::OptionsSheet::Graphics,
        clonk_frontend::startup_options_dlg::OptionsSheet::Sound,
        clonk_frontend::startup_options_dlg::OptionsSheet::Keyboard,
        clonk_frontend::startup_options_dlg::OptionsSheet::Gamepad,
        clonk_frontend::startup_options_dlg::OptionsSheet::Network,
    ] {
        let mut options = new_running_sandbox_app();
        options.return_to_menu();
        if sheet == clonk_frontend::startup_options_dlg::OptionsSheet::Program {
            options.open_options_menu();
        } else {
            enter_unported_startup_subscreen(&mut options, ClassicStartupSubscreen::Options(sheet));
        }
        main_assert_eq!(options.startup_options_dialog.as_ref().expect("retained Options model").active_sheet() => sheet);
        exercise(&mut options, &format!("retained Options {sheet:?} sheet"));
    }

    let mut nested = new_running_sandbox_app();
    nested.return_to_menu();
    enter_unported_startup_subscreen(
        &mut nested,
        ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
    );
    nested
        .push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Audio",
                "Nested startup modal",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            ),
            MessageDialogContinuation::None,
        )
        .test_value();
    exercise(&mut nested, "modal above retained Options Sound sheet");
    main_assert_eq!(nested.message_dialogs.len() => 1);

    let mut context = new_running_sandbox_app();
    context.return_to_menu();
    enter_unported_startup_subscreen(
        &mut context,
        ClassicStartupSubscreen::Options(clonk_frontend::startup_options_dlg::OptionsSheet::Sound),
    );
    context
        .open_context_menu_at(
            vec![ContextMenuEntry::<AppContextMenuCommand>::new(
                "Remain open",
            )],
            GuiPoint::new(120.0, 80.0),
        )
        .test_value();
    exercise(&mut context, "context above retained Options Sound sheet");
    main_assert!(context.context_menu.is_some());

    let mut loading = new_running_sandbox_app();
    loading.return_to_menu();
    loading.mode = AppMode::Loading;
    exercise(&mut loading, "GUI-owned loading phase");
}

// Escape in a submenu runs the close command back to the main menu
// (C4Menu::TryClose + SetCloseCommand("ActivateMenu:Main"),
// C4MainMenu.cpp:577).
#[test]
fn escape_in_submenu_returns_to_main_menu() {
    clonk_logging::init();
    let mut app = new_running_sandbox_app();
    app.open_ingame_menu().test_value();
    app.apply_ingame_menu_action(MenuAction::ActivateOptions)
        .test_value();
    main_assert_eq!(app.ingame_menu.as_ref().map(|menu| menu.page()) => Some(ingame_menu::MenuPage::Options));
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(app.ingame_menu.as_ref().map(|menu| menu.page()) => Some(ingame_menu::MenuPage::Main));
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.ingame_menu.is_none());
}

#[test]
fn load_frontend_scenarios_orders_folders_by_index() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let scenarios_dir = install_dir.path().join("Scenarios");
    fs::create_dir_all(&scenarios_dir).test_value();

    let missions_folder = scenarios_dir.join("Missions.c4f");
    fs::create_dir_all(&missions_folder).test_value();
    fs::write(
        missions_folder.join("Folder.txt"),
        "Title=Missions\nIndex=1\n",
    )
    .test_value();

    let arcade_folder = scenarios_dir.join("Arcade.c4f");
    fs::create_dir_all(&arcade_folder).test_value();
    fs::write(arcade_folder.join("Folder.txt"), "Title=Arcade\nIndex=2\n").test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    let identifiers: Vec<_> = scenarios
        .iter()
        .map(|entry| entry.identifier.as_str())
        .collect();
    main_assert_eq!(identifiers => vec!["Missions.c4f", "Arcade.c4f"], "folders should follow legacy indices");

    reset_cached_app_paths();
}

#[test]
fn alphabetical_sorting_gates_only_folder_index_and_difficulty() {
    let titled_entry = |title: &str| {
        let mut entry = FrontendScenario::fallback();
        entry.identifier = format!("{title}.c4s");
        entry.title = title.to_string();
        entry
    };
    let titles = |entries: &[FrontendScenario]| {
        entries
            .iter()
            .map(|entry| entry.title.clone())
            .collect::<Vec<_>>()
    };

    let mut alpha = titled_entry("Alpha");
    alpha.difficulty = Some(3);
    let mut beta = titled_entry("Beta");
    beta.difficulty = Some(1);
    let gamma = titled_entry("Gamma");
    let mut difficulty_entries = vec![alpha, beta, gamma];
    sort_frontend_entries(&mut difficulty_entries, false);
    main_assert_eq!(titles(&difficulty_entries) => vec!["Beta", "Alpha", "Gamma"]);
    sort_frontend_entries(&mut difficulty_entries, true);
    main_assert_eq!(titles(&difficulty_entries) => vec!["Alpha", "Beta", "Gamma"]);

    let mut alpha = titled_entry("Alpha");
    alpha.kind = ScenarioKind::Folder;
    alpha.folder_index = Some(2);
    let mut beta = titled_entry("Beta");
    beta.kind = ScenarioKind::Folder;
    beta.folder_index = Some(1);
    let mut gamma = titled_entry("Gamma");
    gamma.kind = ScenarioKind::Folder;
    let mut folder_entries = vec![alpha, beta, gamma];
    sort_frontend_entries(&mut folder_entries, false);
    main_assert_eq!(titles(&folder_entries) => vec!["Beta", "Alpha", "Gamma"]);
    sort_frontend_entries(&mut folder_entries, true);
    main_assert_eq!(titles(&folder_entries) => vec!["Alpha", "Beta", "Gamma"]);

    let mut alpha = titled_entry("Alpha");
    alpha.icon_index = Some(11);
    let mut beta = titled_entry("Beta");
    beta.icon_index = Some(2);
    let mut icon_entries = vec![alpha, beta];
    sort_frontend_entries(&mut icon_entries, true);
    main_assert_eq!(titles(&icon_entries) => vec!["Beta", "Alpha"]);

    let alpha = titled_entry("Alpha");
    let mut zulu = titled_entry("Zulu");
    zulu.kind = ScenarioKind::Folder;
    let mut kind_entries = vec![alpha, zulu];
    sort_frontend_entries(&mut kind_entries, true);
    main_assert_eq!(titles(&kind_entries) => vec!["Zulu", "Alpha"]);
}

#[test]
fn loader_reads_startup_alphabetical_sorting_recursively() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let worlds_folder = install_dir.path().join("Scenarios").join("Worlds.c4f");
    fs::create_dir_all(&worlds_folder).test_value();
    fs::write(worlds_folder.join("Folder.txt"), "Title=Worlds\n").test_value();
    let alpha_dir = worlds_folder.join("Alpha.c4s");
    fs::create_dir_all(&alpha_dir).test_value();
    fs::write(
        alpha_dir.join("Scenario.txt"),
        "[Head]\nTitle=Alpha\nDifficulty=3\n",
    )
    .test_value();
    let beta_dir = worlds_folder.join("Beta.c4s");
    fs::create_dir_all(&beta_dir).test_value();
    fs::write(
        beta_dir.join("Scenario.txt"),
        "[Head]\nTitle=Beta\nDifficulty=1\n",
    )
    .test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let (_guard, paths) = guarded_test_app_paths(Some(install_dir.path()), user_dir.as_path());
    paths.ensure_user_dirs().test_value();

    let child_titles = |entries: &[FrontendScenario]| {
        entries[0]
            .children
            .iter()
            .map(|entry| entry.title.clone())
            .collect::<Vec<_>>()
    };

    fs::write(paths.config_file(), "[Startup]\nAlphabeticalSorting=1\n").test_value();
    let alphabetical = load_frontend_scenarios_from_paths(&paths);
    main_assert_eq!(child_titles(&alphabetical) => vec!["Alpha", "Beta"]);

    fs::write(paths.config_file(), "[Startup]\nAlphabeticalSorting=0\n").test_value();
    let legacy = load_frontend_scenarios_from_paths(&paths);
    main_assert_eq!(child_titles(&legacy) => vec!["Beta", "Alpha"]);

    reset_cached_app_paths();
}

#[test]
fn load_frontend_scenarios_orders_by_icon_index() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let scenarios_dir = install_dir.path().join("Scenarios");
    let missions_folder = scenarios_dir.join("Missions.c4f");
    fs::create_dir_all(&missions_folder).test_value();
    fs::write(missions_folder.join("Folder.txt"), "Title=Missions\n").test_value();

    let bravo_dir = missions_folder.join("Bravo.c4s");
    fs::create_dir_all(&bravo_dir).test_value();
    fs::write(
        bravo_dir.join("Scenario.txt"),
        "[Head]\nTitle=Bravo\nIcon=3\n",
    )
    .test_value();

    let alpha_dir = missions_folder.join("Alpha.c4s");
    fs::create_dir_all(&alpha_dir).test_value();
    fs::write(
        alpha_dir.join("Scenario.txt"),
        "[Head]\nTitle=Alpha\nIcon=5\n",
    )
    .test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    main_assert_eq!(scenarios.len() => 1, "expected single folder entry");
    let folder = &scenarios[0];
    main_assert_eq!(folder.identifier => "Missions.c4f");
    let titles: Vec<_> = folder
        .children
        .iter()
        .map(|child| child.title.as_str())
        .collect();
    main_assert_eq!(titles => vec!["Bravo", "Alpha"], "icon indices should order scenarios before title fallback");

    reset_cached_app_paths();
}

#[test]
fn load_frontend_scenarios_preserves_legacy_ordering() {
    let _env_lock = crate::tests::env_lock().lock();
    reset_cached_app_paths();

    let install_dir = tempdir();
    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let install_folder = install_dir.path().join("Scenarios").join("Worlds.c4f");
    fs::create_dir_all(&install_folder).test_value();
    fs::write(install_folder.join("Folder.txt"), "Title=Worlds\n").test_value();
    let install_bravo = install_folder.join("Bravo.c4s");
    fs::create_dir_all(&install_bravo).test_value();
    fs::write(
        install_bravo.join("Scenario.txt"),
        "[Head]\nTitle=Bravo\nDifficulty=1\n",
    )
    .test_value();
    let install_charlie = install_folder.join("Charlie.c4s");
    fs::create_dir_all(&install_charlie).test_value();
    fs::write(
        install_charlie.join("Scenario.txt"),
        "[Head]\nTitle=Charlie\nDifficulty=2\n",
    )
    .test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let user_folder = user_dir.join("Scenarios").join("Worlds.c4f");
    fs::create_dir_all(&user_folder).test_value();
    fs::write(user_folder.join("Folder.txt"), "Title=Worlds\n").test_value();
    let user_alpha = user_folder.join("Alpha.c4s");
    fs::create_dir_all(&user_alpha).test_value();
    fs::write(
        user_alpha.join("Scenario.txt"),
        "[Head]\nTitle=Alpha Override\nDifficulty=3\n",
    )
    .test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    main_assert_eq!(scenarios.len() => 1, "expected merged folder");
    let folder = &scenarios[0];
    main_assert_eq!(folder.identifier => "Worlds.c4f");
    let identifiers: Vec<_> = folder
        .children
        .iter()
        .map(|child| child.identifier.as_str())
        .collect();
    main_assert_eq!(
        identifiers =>
        vec![
            "Worlds.c4f/Bravo.c4s",
            "Worlds.c4f/Charlie.c4s",
            "Worlds.c4f/Alpha.c4s"
        ],
        "merged children should follow legacy ordering rules"
    );
    main_assert_eq!(folder.children[2].title => "Alpha Override", "user override title should be retained");
    main_assert!(folder.children[2].path.as_ref().map(|path| path.starts_with(&user_dir)).unwrap_or(false), "user override should keep user path");

    reset_cached_app_paths();
}

#[test]
fn load_frontend_scenarios_sets_human_readable_location() {
    reset_cached_app_paths();

    let install_dir = tempdir();

    let planet_dir = install_dir.path().join("planet");
    fs::create_dir_all(&planet_dir).test_value();
    fs::write(planet_dir.join("System.c4g"), b"stub").test_value();

    let user_dir = install_dir.path().join("user-data");
    fs::create_dir_all(&user_dir).test_value();
    let scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
    fs::create_dir_all(&scenario_dir).test_value();
    fs::write(
        scenario_dir.join("Scenario.json"),
        br#"{"name":"Alpha Mission"}"#,
    )
    .test_value();

    let _guard = test_env_guard(install_dir.path(), user_dir.as_path());

    let scenarios = load_frontend_scenarios();
    main_assert_eq!(scenarios.len() => 1, "expected single scenario entry");
    let scenario = &scenarios[0];
    main_assert_eq!(scenario.location_label().as_deref() => Some("Scenarios / Alpha.c4s"), "location label should mirror catalog path");

    reset_cached_app_paths();
}

/// `C4Startup::SetStartScreen` (C4Startup.cpp:389-408) maps seven
/// case-insensitive names onto the dialog opened first; `C4Game` routes
/// `/startup:` straight into it (C4Game.cpp:3205). An unrecognized name changes
/// nothing.
#[test]
fn classic_startup_argument_selects_initial_cpp_view() {
    let view = |screen: &str| {
        let mut app = new_real_classic_menu_app(640, 480);
        app.apply_classic_startup_screen(screen);
        (app.startup_view, app.scenario_selector_mode)
    };

    main_assert_eq!(view("main").0 => StartupView::MainMenu);
    main_assert_eq!(view("scen").0 => StartupView::ScenarioBrowser);
    main_assert_eq!(view("net").0 => StartupView::NetworkGame);
    main_assert_eq!(view("options").0 => StartupView::Options);
    main_assert_eq!(view("plrsel").0 => StartupView::PlayerSelection);
    main_assert_eq!(view("about").0 => StartupView::About);

    // `netscen` is the scenario selector in network-host mode: same view as
    // `scen`, different selector mode, and neither is the `net` browser.
    let (netscen_view, netscen_mode) = view("netscen");
    main_assert_eq!(netscen_view => StartupView::ScenarioBrowser);
    main_assert_eq!(netscen_mode => ScenarioSelectorMode::NetworkHost);
    main_assert_ne!(netscen_mode => view("scen").1);

    // Every name is case-insensitive, exactly like SEqualNoCase.
    for (lower, upper) in [
        ("main", "MAIN"),
        ("scen", "Scen"),
        ("netscen", "NetScen"),
        ("net", "NET"),
        ("options", "Options"),
        ("plrsel", "PlrSel"),
        ("about", "ABOUT"),
    ] {
        main_assert_eq!(view(lower) => view(upper), "{lower} must be case-insensitive");
    }

    // An unknown name leaves the remembered/default view untouched and opens
    // no overlay of its own.
    let mut app = new_real_classic_menu_app(640, 480);
    app.open_about_dialog();
    let remembered = app.startup_view;
    app.apply_classic_startup_screen("nonsense");
    main_assert_eq!(app.startup_view => remembered);
    main_assert!(app.message_dialogs.is_empty());
    app.apply_classic_startup_screen("");
    main_assert_eq!(app.startup_view => remembered);
}

/// `C4StartupMainDlg` binds bare F6 to `SwitchToEditor`
/// (C4StartupMainDlg.cpp:95-100,313-325): on Windows it refuses when the
/// configured `Editor.exe` is absent — returning false so the key is not
/// consumed — and otherwise flags the launch and exits startup, with
/// `~C4Application` spawning it after teardown (C4Application.cpp:58-74).
/// Every other platform skips the `#ifdef _WIN32` body and consumes the key
/// without effect.
#[test]
fn startup_f6_launches_editor_when_available() {
    let install = tempdir();
    let user_data = tempdir();
    install_global_gui_and_loader_test_root(install.path());
    let (_guard, paths) = isolated_test_app_paths(install.path(), user_data.path());
    paths.ensure_user_dirs().test_value();

    // Without Editor.exe the shortcut is inert on every platform: startup
    // stays open and nothing is queued.
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.show_main_menu();
    main_assert_eq!(app.classic_editor_executable() => None);
    app.test_key(VirtualKeyCode::F6, ElementState::Pressed);
    main_assert_eq!(app.startup_view => StartupView::MainMenu);
    main_assert!(app.pending_editor_launch.is_none());
    main_assert!(!app.exit_requested);

    // With Editor.exe beside the engine, Windows queues the deferred launch
    // and exits; other platforms still consume the key with no effect.
    let editor = install.path().join("Editor.exe");
    fs::write(&editor, b"stub").test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.show_main_menu();
    main_assert_eq!(app.classic_editor_executable() => Some(editor.clone()));
    app.test_key(VirtualKeyCode::F6, ElementState::Pressed);
    if cfg!(windows) {
        main_assert_eq!(app.pending_editor_launch => Some(editor));
        main_assert!(app.exit_requested, "SwitchToEditor exits startup");
    } else {
        main_assert!(app.pending_editor_launch.is_none());
        main_assert!(!app.exit_requested);
    }
    main_assert_eq!(app.startup_view => StartupView::MainMenu);

    // A modified F6 is not the classic binding and must not reach it.
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.show_main_menu();
    app.keyboard_modifiers = ModifiersState::CONTROL;
    app.test_key(VirtualKeyCode::F6, ElementState::Pressed);
    main_assert!(app.pending_editor_launch.is_none());
    main_assert!(!app.exit_requested);

    // The binding belongs to the main dialog only.
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.open_about_dialog();
    app.test_key(VirtualKeyCode::F6, ElementState::Pressed);
    main_assert!(app.pending_editor_launch.is_none());
    main_assert_eq!(app.startup_view => StartupView::About);
}

#[test]
fn a_dedicated_server_keeps_the_fullscreen_startup_lineage_rule() {
    // `Application.UseStartupDialog` is `isFullScreen && !DirectJoinAddress &&
    // !ScenarioFilename && !RecordStream` (C4Game.cpp:3299), and only
    // `/console` clears `isFullScreen` (C4Game.cpp:3295-3296). A dedicated
    // server is a `USE_CONSOLE` build, not a `/console` run, so it keeps the
    // fullscreen lineage: `!console_mode` is already the right term, and the
    // gate takes no `headless` condition.
    for headless in [false, true] {
        let mut app = new_state_only_menu_app(320, 200);
        app.headless = headless;
        main_assert!(app.failed_open_game_returns_to_startup(), "an empty command line keeps a startup generation to return to");

        app.classic_command_line.scenario = Some(PathBuf::from("Broken.c4s"));
        main_assert!(!app.failed_open_game_returns_to_startup(), "an explicit command-line scenario suppresses the startup dialog");

        app.classic_command_line.scenario = None;
        app.classic_command_line.record_stream = Some(PathBuf::from("Broken.c4r"));
        main_assert!(!app.failed_open_game_returns_to_startup(), "a command-line record stream suppresses the startup dialog");
    }
}

#[test]
fn a_dedicated_server_quits_when_its_command_line_record_stream_fails() {
    // A nonempty `RecordStream` clears `UseStartupDialog` (C4Game.cpp:3299),
    // so the failed `OpenGame` unwinds `QuitGame` into `Quit` instead of
    // reconstructing a startup dialog (C4Application.cpp:376-404,438-450).
    // `run_headless_server` reaches this through `poll_loading`, which the
    // console event loop drives from `GameApp::update`, so the exit request
    // has to survive the headless path as well.
    let mut app = new_state_only_menu_app(320, 200);
    app.headless = true;
    app.classic_command_line.record_stream = Some(PathBuf::from("Broken.c4r"));
    app.classic_record_stream_activation_pending = true;
    app.mode = AppMode::Loading;

    main_assert!(!app.startup_dialog_in_use());
    app.finish_scenario_loading_failure("controlled headless load failure".to_string(), false)
        .test_value();

    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert!(app.take_exit_request(), "a dedicated server with no startup generation to return to quits");
}

#[test]
fn a_ticked_dont_show_again_box_suppresses_its_warning_before_any_save() {
    // `ShowMessageModal` returns early on the flag it was handed by pointer —
    // `if (pbConfigDontShowAgainSetting && *pbConfigDontShowAgainSetting)
    // return true;` (C4GuiDialogs.cpp:1060-1065) — so the tick takes effect
    // from memory, in the session that made it. The pointers are
    // `&Config.Startup.HideMsgPlrNoTakeOver` (C4GameLobby.cpp:462) and
    // `&Config.Startup.HideMsgStartDedicated` (C4StartupScenSelDlg.cpp:1697),
    // and neither file contains a `Config.Save()`.
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    fs::write(
        paths.config_file(),
        b"[Startup]\r\nHideMsgStartDedicated=0\r\n",
    )
    .test_value();
    let mut app = new_state_only_menu_app(320, 200);
    app.app_paths = Some(paths.clone());
    main_assert!(!app.startup_message_hidden("HideMsgStartDedicated"));

    app.deferred_config
        .set("Startup", "HideMsgStartDedicated", "1");

    main_assert!(app.startup_message_hidden("HideMsgStartDedicated"), "the tick suppresses the warning without waiting for a save");
    main_assert_eq!(
        Config::load(paths.config_file())
            .test_value()
            .get_in(Some("Startup"), "HideMsgStartDedicated") =>
        Some("0"),
        "and the file still holds what the session started from"
    );
    reset_cached_app_paths();
}

#[test]
fn a_dedicated_server_writes_its_runtime_config_on_a_clean_quit() {
    // `C4Application::Quit` ends in `if (Config.fConfigLoaded) Config.Save();`
    // (C4Application.cpp:367) with no `USE_CONSOLE` guard, so a dedicated
    // server persists its accumulated Config exactly like a fullscreen run.
    // `run_headless_server` returns before the winit loop exists, so its own
    // shutdown has to do that write.
    let _lock = env_lock().lock();
    let fixture = tempdir();
    let (_guard, paths) = exact_loader_test_paths(fixture.path(), None);
    fs::create_dir_all(paths.config_file().parent().test_value()).test_value();
    fs::write(
        paths.config_file(),
        b"[Network]\r\nControlRate=2\r\nComment=\"old comment\"\r\n",
    )
    .test_value();
    let mut app = new_state_only_menu_app(320, 200);
    app.app_paths = Some(paths.clone());
    app.headless = true;
    app.persist_game_option_value("Network", "ControlRate", "7".to_string());
    app.persist_game_option_text("Network", "Comment", "Weekend server");

    app.finish_console_shutdown();

    let saved = Config::load(paths.config_file()).test_value();
    main_assert_eq!(saved.get_in(Some("Network"), "ControlRate") => Some("7"));
    main_assert_eq!(saved.get_in(Some("Network"), "Comment") => Some("Weekend server"));
    reset_cached_app_paths();
}

#[test]
fn explicit_launch_failures_exit_instead_of_reconstructing_startup() {
    // `C4Application::QuitGame` reconstructs a startup generation only when one
    // is in use and otherwise reaches `Quit()` (C4Application.cpp:373-405), and
    // `ParseCommandLine` suppresses `UseStartupDialog` for a scenario, a direct
    // join or a record stream (C4Game.cpp:3321). So an explicit launch that
    // fails ends the process rather than settling into a menu it never came
    // from — on a headless server the alternative is a silent hang.
    //
    // `fQuitWithError` does NOT feed the exit status: it is read only by
    // `C4Startup::DoStartup` (C4Startup.cpp:276), which this path never
    // reaches, so a failed direct launch presents no log at all.
    for (label, apply) in [
        (
            "scenario",
            (|app: &mut GameApp| {
                app.classic_command_line.scenario = Some(PathBuf::from("Explicit.c4s"));
            }) as fn(&mut GameApp),
        ),
        ("join", |app: &mut GameApp| {
            app.classic_command_line.direct_join = Some("127.0.0.1:11112".to_string());
        }),
        ("replay", |app: &mut GameApp| {
            app.classic_command_line.record_stream = Some(PathBuf::from("Round.c4r"));
        }),
    ] {
        let mut app = new_state_only_menu_app(320, 200);
        app.console_mode = false;
        apply(&mut app);
        main_assert!(
            !app.failed_open_game_returns_to_startup(),
            "an explicit {label} launch has no startup generation behind it"
        );

        app.finish_scenario_loading_failure("boom".to_string(), false)
            .test_value();
        main_assert!(
            app.exit_requested,
            "a failed {label} launch must end the process, not expose a menu"
        );
    }

    // The control is the routing decision itself: an ordinary fullscreen start
    // DOES have a startup generation, so the same failure reconstructs it
    // instead of quitting. Only the decision is asserted here -- driving that
    // branch runs the whole log presentation, which needs more of the app than
    // a state-only fixture carries, and it is already covered by
    // `empty_restart_log_uses_regular_error_modal_over_restored_host_selector`.
    let ordinary = new_state_only_menu_app(320, 200);
    main_assert!(
        ordinary.failed_open_game_returns_to_startup(),
        "an ordinary fullscreen start keeps the startup lineage it came from"
    );
}

/// Startup text is rasterized at the scale it is drawn at, for every scale a
/// real display hands over (clonk-org/clonk-rs#566).
///
/// C++ rebuilds its GUI faces whenever the application scale changes
/// (`C4Startup::EnsureLoaded` -> `C4StartupGraphics::InitFonts`), so a glyph is
/// always rendered at its final size. The port must never present startup text
/// from an atlas built for a *different* scale, because upscaling a
/// lower-resolution text bitmap is exactly the blur this issue is about.
///
/// The 2x/3x cases already looked sharp, which is what made the fractional
/// scales easy to miss: nothing in the pipeline refuses a mismatched atlas, it
/// simply resamples. So the check here is the identity of the atlas — its own
/// reported scale — rather than anything about how it looks.
#[test]
fn startup_text_uses_a_native_atlas_at_every_supported_scale() {
    // 50% and 300% are the documented extremes (`Config.Graphics.Scale=50`
    // through 3x); 125/150/175 are the common fractional desktop scales.
    for scale in [0.5_f32, 1.25, 1.5, 1.75, 2.0, 3.0] {
        let mut app = new_menu_app(640, 480);
        install_native_test_fonts(&mut app, scale);

        let fonts = app
            .native_startup_fonts
            .as_ref()
            .unwrap_or_else(|| panic!("scale {scale} must build a native font set"));
        main_assert_eq!(
            fonts.scale() => scale,
            "the atlas must be rasterized for {scale}, not reused from another scale"
        );
        main_assert!(
            app.can_present_ordered_native_text(scale),
            "scale {scale} must present startup text natively"
        );
        // A neighbouring scale must be refused rather than resampled: this is
        // the assertion that fails if the port ever accepts a mismatched atlas.
        main_assert!(
            !app.can_present_ordered_native_text(scale + 0.25),
            "an atlas built for {scale} must not satisfy a different scale"
        );
    }
}
