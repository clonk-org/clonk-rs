// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

macro_rules! scensel_fixture {
    (frontend_scenario: $binding:ident, $identifier:expr, $title:expr $(,)?) => {
        let mut $binding = FrontendScenario::fallback();
        $binding.identifier = $identifier;
        $binding.title = $title;
    };
    (scenario: $identifier:expr, $title:expr, $kind:expr $(,)?) => {
        clonk_frontend::ScenarioSummary {
            identifier: $identifier,
            title: $title,
            kind: $kind,
        }
    };
    (goal_rule: $definition_id:expr, $name:expr $(,)?) => {
        GoalRuleEntry {
            definition_id: $definition_id,
            name: $name,
            description: None,
            fulfilled: false,
        }
    };
}

fn scensel_window_app(player_name: &str) -> (EnvGuard, tempfile::TempDir, GameApp) {
    let user_data = tempdir();
    let guard = EnvGuard::set(&[
        ("LC_INSTALL_ROOT", Some(test_repository_root())),
        ("LC_USER_DATA_DIR", Some(user_data.path())),
        ("LC_LANGUAGE", Some(Path::new("US"))),
    ]);
    let paths = test_app_paths();
    let mut app = GameApp::new(
        800,
        600,
        disabled_audio_options(),
        Some(&paths),
        test_runtime_config_with(player_name, false),
    )
    .test_value();
    wait_for_menu(&mut app);
    app.open_scenario_browser();
    (guard, user_data, app)
}

fn scensel_app(scenarios: &[FrontendScenario]) -> GameApp {
    let menu =
        StartupMenu::new(build_menu_entries(scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app(800, 600);
    app.menu_state = MenuState::new(menu, scenarios.to_vec());
    app.scensel.catalog = build_scenario_catalog(scenarios);
    app.open_scenario_browser();
    app
}

// The C++ book has no Back list row (C4StartupScenSelDlg has a Back
// button/K_LEFT instead) and selects the first entry
// (SelectFirstEntry, cpp:1536-1537).
#[test]
fn scensel_menu_state_without_back_row_selects_first_entry() {
    let scenarios = sample_scenarios();
    let entries = build_menu_entries(&scenarios, true);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.menu().resize(1280.0, 720.0);

    state.set_include_back(false);
    main_assert!(state.menu().entries().iter().all(|entry| entry.identifier != BACK_ENTRY_IDENTIFIER));

    let selection = state.select_default_entry();
    main_assert!(matches!(selection.as_slice(), [StartupMenuAction::SelectionChanged(summary)] if summary.identifier == "folder_missions"));
    main_assert_eq!(state.selected_scenario().map(|entry| entry.title.as_str()) => Some("Missions"));
}

#[test]
fn scensel_definition_checkbox_resets_only_on_selection_change() {
    let mut first = FrontendScenario::fallback();
    first.identifier = "first".to_string();
    first.local_only = Some(false);
    first.allow_user_change = Some(true);
    first.definition_modules = vec!["Objects.c4d".to_string(), "Knights.c4d".to_string()];
    let mut contradictory = FrontendScenario::fallback();
    contradictory.identifier = "contradictory".to_string();
    contradictory.local_only = Some(true);
    contradictory.allow_user_change = Some(true);
    contradictory.definition_modules = vec!["Ignored.c4d".to_string()];
    let scenarios = vec![first, contradictory];
    let entries = build_menu_entries(&scenarios, false);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    let _ = state.select_default_entry();
    state.sync_definition_checkbox_to_selection();
    main_assert!(state.definition_checkbox_enabled);
    main_assert!(state.definition_checkbox_checked);
    main_assert_eq!(scenario_fixed_definition_modules(state.selected_scenario().unwrap()) => ["Objects.c4d", "Knights.c4d"]);

    main_assert!(state.toggle_definition_checkbox());
    main_assert!(!state.definition_checkbox_checked);
    main_assert!(state.set_definition_checkbox_focused(true));
    // Opening/canceling the child selector does not resync this state.
    main_assert!(!state.definition_checkbox_checked);

    let _ = state.select_list_index(1);
    state.sync_definition_checkbox_to_selection();
    main_assert!(!state.definition_checkbox_enabled);
    main_assert!(state.definition_checkbox_checked);
    main_assert!(!state.definition_checkbox_focused);
    main_assert!(!state.toggle_definition_checkbox());
    main_assert_eq!(scenario_fixed_definition_modules(state.selected_scenario().unwrap()) => ["Objects.c4d"]);
}

// C4StartupScenSelDlg::UpdateList only evaluates the entries in the current
// folder (C4StartupScenSelDlg.cpp:1511-1537). Hidden descendants must not make
// opening the root selector perform loader-head work before the first frame.
#[test]
fn scenario_selector_openability_cache_only_covers_visible_entries() {
    scensel_fixture!(frontend_scenario: hidden, "pack/hidden".to_string(), "Hidden".to_string());
    scensel_fixture!(frontend_scenario: pack, "pack".to_string(), "Pack".to_string());
    pack.kind = ScenarioKind::Folder;
    pack.is_playable = false;
    pack.children = vec![hidden];

    let mut app = scensel_app(&[pack]);
    main_assert!(app.scensel.entry_enabled.contains_key("pack"));
    main_assert!(!app.scensel.entry_enabled.contains_key("pack/hidden"));

    app.enter_scenario_folder("pack");
    main_assert_eq!(app.scensel.entry_enabled.get("pack/hidden") => Some(&true));

    app.scensel_do_back().test_value();
    main_assert!(app.scensel.entry_enabled.contains_key("pack"));
    main_assert!(!app.scensel.entry_enabled.contains_key("pack/hidden"));
}

#[test]
fn checked_definition_checkbox_intercepts_start_even_when_local_only_disables_it() {
    let mut app = new_menu_app(640, 480);
    app.open_scenario_browser();
    scensel_fixture!(frontend_scenario: scenario, "definition_intercept".to_string(), "Definition intercept".to_string());
    scenario.path = Some(PathBuf::from("DefinitionIntercept.c4s"));
    scenario.local_only = Some(true);
    scenario.allow_user_change = Some(true);
    scenario.definition_modules = vec!["Ignored.c4d".to_string()];
    app.scensel.catalog
        .insert(scenario.identifier.clone(), scenario.clone());
    app.menu_state.definition_checkbox_enabled = false;
    app.menu_state.definition_checkbox_checked = true;

    app.handle_menu_input(|_| {
        vec![StartupMenuAction::StartScenario(scensel_fixture!(
            scenario:
                scenario.identifier.clone(),
                scenario.title.clone(),
                ScenarioKind::Scenario,
        ))]
    })
    .test_value();

    let selector = app.definition_selector.test_ref();
    main_assert_eq!(selector.accepted_selection() => ["Objects.c4d"]);
    main_assert!(app.loading_state.is_none());
    app.process_definition_selector_actions(vec![
        clonk_frontend::definition_sel::DefinitionSelAction::Cancelled,
    ])
    .test_value();
    main_assert!(app.menu_state.definition_checkbox_checked);
    main_assert!(matches!(app.mode, AppMode::Menu));
}

#[test]
fn scensel_mission_access_gates_rows_start_and_map_buttons_live() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    configure_test_startup_participant(&paths, user_data.path());
    persist_config_value(&paths, "General", "LanguageEx", "DE").test_value();
    persist_config_value(&paths, "General", "MissionAccess", "").test_value();
    let scenario_root = paths.scenario_dir().to_path_buf();
    fs::create_dir_all(&scenario_root).test_value();
    for (name, core) in [
            (
                "Allowed.c4s",
                "[Head]\nTitle=Allowed\nMinPlayer=1\nMaxPlayer=4\n",
            ),
            (
                "Locked.c4s",
                "[Head]\nTitle=Locked\nMinPlayer=1\nMaxPlayer=4\nMissionAccess=Secret\n\n[Definitions]\nAllowUserChange=1\n",
            ),
            (
                "TooFew.c4s",
                "[Head]\nTitle=Too few\nMinPlayer=2\nMaxPlayer=4\n",
            ),
        ] {
            let path = scenario_root.join(name);
            fs::create_dir(&path).test_value();
            fs::write(path.join("Scenario.txt"), core).test_value();
        }
    let native_path = scenario_root.join("Native.c4s");
    fs::create_dir(&native_path).test_value();
    fs::write(
        native_path.join("Scenario.txt"),
        b"[Head]\nTitle=Native access\nMinPlayer=1\nMaxPlayer=4\nMissionAccess=Secr\x80t\n",
    )
    .test_value();

    let map_path = scenario_root.join("Map.c4f");
    let map_scenario_path = map_path.join("MapLocked.c4s");
    fs::create_dir_all(&map_scenario_path).test_value();
    fs::write(map_path.join("Folder.txt"), "[Head]\nTitle=Access Map\n").test_value();
    write_map_png(&map_path.join("FolderMap.png"), 8, 8, [20, 30, 40, 255]);
    fs::write(
        map_path.join("FolderMap.txt"),
        "[FolderMap]\n    [Scenario]\n    File=MapLocked.c4s\n    Area=0,0,8,8\n",
    )
    .test_value();
    fs::write(
        map_scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Map locked\nMinPlayer=1\nMaxPlayer=4\nMissionAccess=Secret\n",
    )
    .test_value();

    let mut app = new_menu_app_with_paths(640, 480, &paths);
    let scenarios = resource_scenario::discover(&scenario_root)
        .test_value()
        .into_iter()
        .map(|entry| FrontendScenario::from_resource(entry, "Test scenarios"))
        .collect::<Vec<_>>();
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);
    app.open_scenario_browser();

    main_assert_eq!(app.scensel.entry_enabled.get("Allowed.c4s") => Some(&true));
    main_assert_eq!(app.scensel.entry_enabled.get("Locked.c4s") => Some(&false));
    main_assert_eq!(app.scensel.entry_enabled.get("Native.c4s") => Some(&false));
    main_assert_eq!(app.scensel.entry_enabled.get("TooFew.c4s") => Some(&false));

    // The dynamic renderer must pass CanOpen to ScenListItem: only the
    // label alpha changes; icons and row activation remain intact.
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
    for (index, entry) in app.menu_state.visible_entries().iter().enumerate() {
        let row_y = label_top + index as i32 * (item_h + 1);
        let max_alpha = (label_x..label_x + 120)
            .flat_map(|x| (row_y..row_y + item_h).map(move |y| (x, y)))
            .filter_map(|(x, y)| surface.get_pixel(x as u32, y as u32))
            .map(|color| color.a)
            .max()
            .unwrap_or(0);
        let enabled = app.scensel.entry_enabled[&entry.identifier];
        if enabled {
            main_assert!(max_alpha > 200, "{} row is enabled (max alpha {max_alpha})", entry.title);
        } else {
            main_assert!(max_alpha > 0 && max_alpha < 200, "{} row uses disabled 50%-black text (max alpha {max_alpha})", entry.title);
        }
    }

    let locked = app.scensel.catalog["Locked.c4s"].clone();
    main_assert!(locked.is_playable, "denied rows remain actionable");
    main_assert!(!locked.has_mission_access(&app.config.mission_access));
    app.enter_scenario_folder("Map.c4f");
    main_assert_eq!(app.menu_state.current_map().expect("mission-gated map view").scenarios.len() => 0, "a denied scenario produces no map button");
    app.menu_state.leave_folder();
    app.configure_current_folder_map();

    app.menu_state.definition_checkbox_checked = true;
    app.handle_menu_input(|_| {
        vec![StartupMenuAction::StartScenario(scensel_fixture!(
            scenario:
                locked.identifier.clone(),
                locked.title.clone(),
                ScenarioKind::Scenario,
        ))]
    })
    .test_value();
    main_assert!(app.loading_state.is_none());
    main_assert!(app.definition_selector.is_none());
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert_eq!(app.dialogs.messages[0].state.caption() => "Start nicht möglich.");
    main_assert_eq!(app.dialogs.messages[0].state.message() => "Noch kein Zugang zu dieser Mission.");
    main_assert_eq!(app.dialogs.messages[0].state.icon() => clonk_frontend::message_dialog::MessageDialogIcon::ERROR);
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();

    let native_password = clonk_script::c4_string_from_bytes(b"Secr\x80t");
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert_eq!(app.game_option_input_dialog.as_ref().expect("Mission Access dialog").purpose => PendingInputDialogPurpose::ScenarioMissionAccess);
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        " secret ".to_string(),
    )])
    .test_value();
    wait_for_scenario_selector_discovery(&mut app);
    main_assert_eq!(app.config.mission_access.snapshot() => "secret");
    // C++ keeps this in memory until a save (C4Script.cpp:2466-2471;
    // C4StartupScenSelDlg.cpp:1838-1856); the port writes earned access
    // straight out instead (`persist_mission_access_if_changed`).
    main_assert_eq!(load_configured_mission_access(&paths).expect("read saved mission access") => "secret");
    main_assert_eq!(app.scensel.entry_enabled.get("Locked.c4s") => Some(&true));
    main_assert_eq!(app.scensel.entry_enabled.get("Native.c4s") => Some(&false));
    main_assert!(locked.has_mission_access(&app.config.mission_access));
    app.enter_scenario_folder("Map.c4f");
    main_assert_eq!(
        app.menu_state
            .current_map()
            .expect("granted map view")
            .scenarios
            .len() =>
        1,
        "Alt+M grant immediately enables map-button creation after reload"
    );
    app.menu_state.leave_folder();
    app.configure_current_folder_map();

    // The classic text dialog cannot synthesize an arbitrary native byte,
    // but the same live apply path must preserve one loaded from config.
    app.apply_scenario_mission_access(&native_password)
        .test_value();
    wait_for_scenario_selector_discovery(&mut app);
    main_assert_eq!(app.config.mission_access.snapshot() => format!("secret;{native_password}"));
    main_assert_eq!(app.scensel.entry_enabled.get("Native.c4s") => Some(&true));
    main_assert_eq!(app.scensel.entry_enabled.get("TooFew.c4s") => Some(&false));
    main_assert_eq!(
        app.scenario_selector_open_error(
            &app.scensel.catalog["Native.c4s"],
            ScenarioSelectorMode::Local,
        )
        .expect("inspect native-byte access") =>
        None,
        "granted native bytes survive both catalog and loader-head parsers"
    );

    app.menu_state.definition_checkbox_checked = true;
    app.handle_menu_input(|_| {
        vec![StartupMenuAction::StartScenario(scensel_fixture!(
            scenario:
                locked.identifier.clone(),
                locked.title.clone(),
                ScenarioKind::Scenario,
        ))]
    })
    .test_value();
    main_assert!(app.dialogs.messages.is_empty());
    main_assert!(app.definition_selector.is_some(), "the same catalog entry proceeds to the start flow after grant");
    reset_cached_app_paths();
}

#[test]
fn scensel_recursive_focus_and_gamepad_pass_through_match_dialog_order() {
    scensel_fixture!(frontend_scenario: first, "first".to_string(), "First".to_string());
    first.allow_user_change = Some(false);
    scensel_fixture!(frontend_scenario: second, "second".to_string(), "Second".to_string());
    second.local_only = Some(true);
    second.allow_user_change = Some(false);
    let scenarios = vec![first, second];
    let mut app = scensel_app(&scenarios);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::List);
    main_assert!(app.menu_state.definition_checkbox_enabled);

    let tap_tab = |app: &mut GameApp| {
        app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
        app.test_key(VirtualKeyCode::Tab, ElementState::Released);
    };
    tap_tab(&mut app);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Back);
    tap_tab(&mut app);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Definitions);
    tap_tab(&mut app);
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew));
    tap_tab(&mut app);
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(clonk_frontend::game_option_buttons::GameOptionButton::Record));
    tap_tab(&mut app);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Open);
    tap_tab(&mut app);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search);
    tap_tab(&mut app);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::List);

    app.test_modifiers(ModifiersState::SHIFT);
    tap_tab(&mut app);
    app.test_modifiers(ModifiersState::empty());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search);

    app.set_scensel_dialog_focus(ScenselDialogFocus::List);
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Back);
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Definitions);
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew));

    let selected_before = app.menu_state.menu.selected_index();
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Down,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_ne!(app.menu_state.menu.selected_index() => selected_before);
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew));
    main_assert!(!app.menu_state.definition_checkbox_enabled);

    app.set_scensel_dialog_focus(ScenselDialogFocus::List);
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Right,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew));
    app.handle_gamepad_direction(
        GamepadSlot::new(0),
        ControlButton::Left,
        ElementState::Pressed,
    )
    .test_value();
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Back);

    app.handle_menu_input(|menu| menu.select_list_index(0))
        .test_value();
    app.scenario_game_options.set_focused_button(Some(
        clonk_frontend::game_option_buttons::GameOptionButton::FairCrew,
    ));
    app.menu_state.set_dialog_focus(ScenselDialogFocus::Options);
    app.scenario_game_options
        .set_selector_fair_crew_constraint(FairCrewConstraint::ForceNormal);
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
    main_assert_eq!(app.mode => AppMode::Running);
}

#[test]
fn empty_search_clears_forced_crew_constraint() {
    let scenario_root = tempdir();
    let scenario_path = scenario_root.path().join("Forced.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Forced\nForcedNoCrew=2\n",
    )
    .test_value();
    scensel_fixture!(frontend_scenario: scenario, "forced".to_string(), "Forced".to_string());
    scenario.path = Some(scenario_path);
    let scenarios = vec![scenario];
    let mut app = scensel_app(&scenarios);
    main_assert_eq!(app.scenario_game_options.values().selector_fair_crew_constraint => FairCrewConstraint::ForceNormal);
    main_assert!(!app.scenario_game_options.view(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew).expect("fair-crew option").enabled);

    app.menu_state.set_search_text("no matching scenario");
    app.submit_scenario_search().test_value();
    main_assert!(app.menu_state.selected_scenario().is_none());
    main_assert_eq!(app.scenario_game_options.values().selector_fair_crew_constraint => FairCrewConstraint::Free);
    main_assert!(app.scenario_game_options.view(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew).expect("reset fair-crew option").enabled);
}

// C++ edit pointer handling and the Back button use standard control
// routes (src/C4GuiEdit.cpp:458-527;
// src/C4StartupScenSelDlg.cpp:1297-1382,1705-1724;
// src/C4StartupScenSelDlg.h:427,434-437). The enhanced product query
// returns the descendant leaf and clears back to the folder row before
// exercising the classic Back bounds.
#[test]
fn scensel_touch_uses_live_search_and_classic_back_bounds() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    scensel_fixture!(frontend_scenario: target, "outer/inner/target".to_string(), "Touch Target".to_string());
    scensel_fixture!(frontend_scenario: inner, "outer/inner".to_string(), "Inner Touch Folder".to_string());
    inner.kind = ScenarioKind::Folder;
    inner.is_playable = false;
    inner.children = vec![target];
    scensel_fixture!(frontend_scenario: outer, "outer".to_string(), "Outer Touch Folder".to_string());
    outer.kind = ScenarioKind::Folder;
    outer.is_playable = false;
    outer.children = vec![inner];
    scensel_fixture!(frontend_scenario: sibling, "sibling".to_string(), "Sibling Folder".to_string());
    sibling.kind = ScenarioKind::Folder;
    sibling.is_playable = false;
    let scenarios = vec![outer, sibling];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);
    app.open_network_game_dialog();
    app.open_network_host_scenario_browser();
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let book = app.assets.book_fonts.clone().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);
    let tap = |app: &mut GameApp, point: GuiPoint| {
        app.test_touch(TouchPhase::Started, point);
        app.test_touch(TouchPhase::Ended, point);
    };

    let pitch = clonk_frontend::startup_scensel::scen_list_item_height(&book.text) + 1;
    tap(
        &mut app,
        GuiPoint::new(
            (layout.list.x + 12) as f32,
            (layout.list.y + 3 + pitch + 4) as f32,
        ),
    );
    main_assert_eq!(
        app.menu_state
            .selected_scenario()
            .map(|entry| entry.identifier.as_str()) =>
        Some("sibling"),
        "touch list hit-testing must use the rendered book rows"
    );

    tap(
        &mut app,
        GuiPoint::new((layout.list.x + 12) as f32, (layout.list.y + 3 + 4) as f32),
    );
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("outer"));
    let open = GuiPoint::new(
        (layout.open_button.x + layout.open_button.w / 2) as f32,
        (layout.open_button.y + layout.open_button.h / 2) as f32,
    );
    tap(&mut app, open);
    main_assert_eq!(app.menu_state.stack.len() => 2);
    main_assert_eq!(app.menu_state.book_caption() => "Outer Touch Folder");

    tap(
        &mut app,
        GuiPoint::new(
            (layout.search_edit.x + 8) as f32,
            (layout.search_edit.y + layout.search_edit.h / 2) as f32,
        ),
    );
    for character in "inner touch".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Released);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("outer/inner/target"));
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("outer/inner"));
    tap(&mut app, open);
    main_assert_eq!(app.menu_state.stack.len() => 3);
    main_assert_eq!(app.menu_state.book_caption() => "Inner Touch Folder");

    let back = GuiPoint::new(
        (layout.back_button.x + layout.back_button.w / 2) as f32,
        (layout.back_button.y + layout.back_button.h / 2) as f32,
    );
    tap(&mut app, back);
    main_assert_eq!(app.menu_state.stack.len() => 2);
    tap(&mut app, back);
    main_assert_eq!(app.menu_state.stack.len() => 1);
    tap(&mut app, back);
    main_assert_eq!(app.startup.view => StartupView::NetworkGame);

    app.open_scenario_browser();
    tap(&mut app, back);
    main_assert_eq!(app.startup.view => StartupView::MainMenu);
    reset_cached_app_paths();
}

#[test]
fn scensel_cached_chrome_leaves_game_option_bounds_empty_in_both_modes() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = guarded_test_app_paths(None, user_data.path());
    let app = new_menu_app_with_paths(800, 600, &paths);
    let assets = app.assets.scensel_assets().test_value();
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, fonts);
    let bounds = layout.game_option_bounds();
    let mut background = Surface::new(800, 600, PixelFormat::Rgba8888);
    clonk_frontend::draw_image_bilinear(
        &mut background,
        &GuiRect::new(-1.0, -1.0, 802.0, 602.0),
        &assets.background,
        Some(startup_gamma()),
    );
    for title in ["Start Game", "Start Network Game"] {
        let mut chrome = Surface::new(800, 600, PixelFormat::Rgba8888);
        clonk_frontend::startup_scensel::ScenSelScreen::render_chrome_without_game_options(
            &mut chrome,
            &assets,
            fonts,
            title,
            Some(startup_gamma()),
        );
        for y in bounds.y..bounds.y + bounds.h {
            for x in bounds.x..bounds.x + bounds.w {
                main_assert_eq!(chrome.get_pixel(x as u32, y as u32) => background.get_pixel(x as u32, y as u32), "{title} base chrome must not pre-render FairCrew/Record");
            }
        }
    }
    reset_cached_app_paths();
}

// C4StartupScenSelDlg::OnSearchBarEnter -> UpdateList filters the
// current folder by case-insensitive name substring, retaining a
// surviving selection or falling back to the first row
// (C4StartupScenSelDlg.cpp:1511-1537).
#[test]
fn scensel_search_does_not_recurse_into_unopened_folders() {
    scensel_fixture!(frontend_scenario: cavern, "pack/cavern".to_string(), "<c ff0000>Cavern</c>".to_string());

    scensel_fixture!(frontend_scenario: pack, "pack".to_string(), "Pack".to_string());
    pack.kind = ScenarioKind::Folder;
    pack.is_playable = false;
    pack.children = vec![cavern];

    let scenarios = vec![pack];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.menu().select_entry_by_index(0).test_value();

    state.set_search_text("cAvErN");
    let actions = state.submit_search();

    main_assert!(state.visible_entries().is_empty());
    main_assert!(state.selected_scenario().is_none());
    main_assert!(actions.is_empty());
}

// C++ UpdateList deliberately filters only the current folder
// (src/C4StartupScenSelDlg.cpp:1513-1537). The enhanced product path is
// separate: it searches loaded descendants and ranks an exact title ahead
// of a catalog-earlier prefix match.
#[test]
fn scensel_enhanced_search_recurses_and_ranks_exact_titles_first() {
    scensel_fixture!(frontend_scenario: prefix, "gold_rush_extended".to_string(), "Gold Rush Extended".to_string());

    scensel_fixture!(frontend_scenario: exact, "western/gold_rush".to_string(), "Gold Rush".to_string());

    scensel_fixture!(frontend_scenario: folder, "western".to_string(), "Western Pack".to_string());
    folder.kind = ScenarioKind::Folder;
    folder.is_playable = false;
    folder.children = vec![exact];

    let scenarios = vec![prefix, folder];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.set_search_text("gold rush");

    let actions = state.apply_enhanced_search();

    main_assert_eq!(state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["western/gold_rush", "gold_rush_extended"]);
    main_assert_eq!(state.search_result_context(0) => Some("Western Pack"));
    main_assert!(matches!(actions.as_slice(), [StartupMenuAction::SelectionChanged(summary)] if summary.identifier == "western/gold_rush"));
}

// C++ emits matches in current-folder traversal order
// (src/C4StartupScenSelDlg.cpp:1513-1521). The enhanced product rank keeps
// that catalog order as the deterministic tie-breaker.
#[test]
fn scensel_enhanced_search_preserves_catalog_order_for_equal_ranks() {
    scensel_fixture!(frontend_scenario: first, "zeta".to_string(), "Crystal Cavern".to_string());
    scensel_fixture!(frontend_scenario: second, "alpha".to_string(), "Crystal Crossing".to_string());
    let scenarios = vec![first, second];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.set_search_text("Crystal");

    state.apply_enhanced_search();

    main_assert_eq!(state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["zeta", "alpha"]);
}

// C++ validates the selected current-folder entry before activation
// (src/C4StartupScenSelDlg.cpp:1472-1537,1681-1702). Because the enhanced
// product search can surface a sibling-folder scenario, its validation
// path must instead resolve that result from the catalog root.
#[test]
fn scensel_enhanced_search_resolves_a_global_result_from_inside_a_folder() {
    scensel_fixture!(frontend_scenario: local, "local/mission".to_string(), "Local Mission".to_string());
    scensel_fixture!(frontend_scenario: local_folder, "local".to_string(), "Local Pack".to_string());
    local_folder.kind = ScenarioKind::Folder;
    local_folder.is_playable = false;
    local_folder.children = vec![local];

    scensel_fixture!(frontend_scenario: remote, "remote/crystal".to_string(), "Crystal Cavern".to_string());
    scensel_fixture!(frontend_scenario: remote_folder, "remote".to_string(), "Remote Pack".to_string());
    remote_folder.kind = ScenarioKind::Folder;
    remote_folder.is_playable = false;
    remote_folder.children = vec![remote];

    let scenarios = vec![local_folder, remote_folder];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.enter_folder("local");
    state.set_search_text("Crystal Cavern");

    state.apply_enhanced_search();

    main_assert_eq!(state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("remote/crystal"));
    main_assert_eq!(state.require_supported_activation("remote/crystal").expect("validate global result") => Some(ScenarioKind::Scenario));
}

// C++ F5 reloads the current folder and rebuilds UpdateList from the live
// edit text (src/C4StartupScenSelDlg.cpp:1472-1537,1727-1735). The
// enhanced product path must retain its catalog-wide semantics when
// discovery replaces the backing tree.
#[test]
fn scensel_enhanced_search_survives_catalog_rediscovery() {
    let build_scenarios = |extra: bool| {
        scensel_fixture!(frontend_scenario: first, "pack/crystal".to_string(), "Crystal Cavern".to_string());
        let mut children = vec![first];
        if extra {
            scensel_fixture!(frontend_scenario: second, "pack/crystal_lake".to_string(), "Crystal Lake".to_string());
            children.push(second);
        }
        scensel_fixture!(frontend_scenario: folder, "pack".to_string(), "Adventure Pack".to_string());
        folder.kind = ScenarioKind::Folder;
        folder.is_playable = false;
        folder.children = children;
        vec![folder]
    };
    let scenarios = build_scenarios(false);
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.set_search_text("Crystal");
    state.apply_enhanced_search();

    state.replace_discovered_entries(build_scenarios(true), None, true, true);

    main_assert_eq!(state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["pack/crystal", "pack/crystal_lake"]);
    main_assert_eq!(state.enhanced_search_caption().as_deref() => Some("2 of 2 scenarios"));
}

// C++ lowercases only the markup-stripped display title
// (src/C4StartupScenSelDlg.cpp:1513-1523). The enhanced product matcher
// normalizes user-visible metadata and lets terms span safe fields.
#[test]
fn scensel_enhanced_search_normalizes_terms_and_matches_safe_metadata() {
    scensel_fixture!(frontend_scenario: scenario, "western/crystal_run.c4s".to_string(), "Café-Cavern".to_string());
    scenario.description = Some("A crystal expedition underground.".to_string());
    scenario.author = Some("Zoë".to_string());

    scensel_fixture!(frontend_scenario: folder, "western".to_string(), "Western Adventures".to_string());
    folder.kind = ScenarioKind::Folder;
    folder.is_playable = false;
    folder.children = vec![scenario];

    let scenarios = vec![folder];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);

    for query in [
        "  CAFE   cavern ",
        "western crystal",
        "zoe expedition",
        "crystal_run western",
    ] {
        state.set_search_text(query);
        state.apply_enhanced_search();
        main_assert_eq!(state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["western/crystal_run.c4s"], "query {query:?}");
    }
}

// C++ treats the edit text as one literal title substring
// (src/C4StartupScenSelDlg.cpp:1513-1521). The enhanced product matcher
// requires every normalized term, even when terms span indexed fields.
#[test]
fn scensel_enhanced_search_requires_all_terms_across_fields() {
    scensel_fixture!(frontend_scenario: match_all, "crystal_cavern".to_string(), "Crystal Cavern".to_string());
    match_all.author = Some("Zoë".to_string());
    scensel_fixture!(frontend_scenario: title_only, "crystal_canyon".to_string(), "Crystal Canyon".to_string());
    title_only.author = Some("Anne".to_string());
    scensel_fixture!(frontend_scenario: author_only, "amber_mine".to_string(), "Amber Mine".to_string());
    author_only.author = Some("Zoë".to_string());
    let scenarios = vec![match_all, title_only, author_only];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.set_search_text("crystal zoe");

    state.apply_enhanced_search();

    main_assert_eq!(state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["crystal_cavern"]);
}

// C++ uses a literal title substring
// (src/C4StartupScenSelDlg.cpp:1513-1523). The enhanced product matcher
// adds conservative title-only typo recovery behind exact tiers.
#[test]
fn scensel_enhanced_search_tolerates_conservative_title_typos() {
    scensel_fixture!(frontend_scenario: scenario, "crystal_cavern".to_string(), "Crystal Cavern".to_string());
    let scenarios = vec![scenario];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.set_search_text("crytal caver");

    state.apply_enhanced_search();

    main_assert_eq!(state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["crystal_cavern"]);
}

// C++ uses an exact title substring with no typo fallback
// (src/C4StartupScenSelDlg.cpp:1513-1521). The enhanced fallback remains
// disabled for short or numeric terms and rejects excess edit distance.
#[test]
fn scensel_enhanced_search_rejects_unsafe_title_typos() {
    let scenarios = [
        ("short_case", "Cart"),
        ("numeric_case", "1234"),
        ("distance_case", "Coat"),
    ]
    .into_iter()
    .map(|(identifier, title)| {
        scensel_fixture!(frontend_scenario: scenario, identifier.to_string(), title.to_string());
        scenario
    })
    .collect::<Vec<_>>();
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);

    for query in ["crt", "1235", "crab"] {
        state.set_search_text(query);
        state.apply_enhanced_search();
        main_assert!(state.visible_entries().is_empty(), "unsafe typo query {query:?} must not match");
    }
}

#[test]
fn scensel_search_applies_on_submit_case_insensitively() {
    let mut scenarios = sample_scenarios();
    let mut beta = scenarios[0].children[0].clone();
    beta.identifier = "scenario_beta".to_string();
    beta.title = "<c ff0000>Crystal</c> Cavern".to_string();
    let mut gamma = scenarios[0].children[0].clone();
    gamma.identifier = "scenario_gamma".to_string();
    gamma.title = "Crystal Cavern Annex".to_string();
    scenarios[0].children.push(beta);
    scenarios[0].children.push(gamma);
    let entries = build_menu_entries(&scenarios, false);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.enter_folder("folder_missions");
    let _ = state.menu().select_entry_by_index(2).test_value();
    main_assert_eq!(state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("scenario_gamma"));

    state.set_search_text("cRyStAl cAvErN");
    main_assert_eq!(state.visible_entries().len() => 3, "typing alone does not submit");

    let actions = state.submit_search();
    main_assert_eq!(
        state
            .visible_entries()
            .iter()
            .map(|entry| entry.title.as_str())
            .collect::<Vec<_>>() =>
        vec!["<c ff0000>Crystal</c> Cavern", "Crystal Cavern Annex"]
    );
    main_assert!(matches!(actions.as_slice(), [StartupMenuAction::SelectionChanged(summary)] if summary.identifier == "scenario_gamma"));
    main_assert_eq!(state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("scenario_gamma"));
}

#[test]
fn scensel_search_edit_matches_selection_word_and_length_rules() {
    let mut edit = SearchEditState::default();
    edit.set_text("Alpha beta");
    edit.focus();
    main_assert_eq!(edit.selected_text() => Some("Alpha beta"));
    edit.insert_text("Z");
    main_assert_eq!(edit.text() => "Z", "typing replaces Ctrl+F select-all");

    edit.set_text("one  two_three!");
    edit.move_cursor(SearchCursorOperation::End, false, false);
    edit.move_cursor(SearchCursorOperation::Left, false, false);
    edit.move_cursor(SearchCursorOperation::Left, true, false);
    main_assert_eq!(edit.caret() => 5, "Ctrl+Left stops at the final word start");
    edit.backspace(true, false);
    main_assert_eq!(edit.text() => "two_three!", "Ctrl+Backspace removes one word");
    edit.move_cursor(SearchCursorOperation::Home, false, false);
    edit.move_cursor(SearchCursorOperation::Right, true, true);
    main_assert_eq!(edit.selected_text() => Some("two_three!"));
    edit.delete(false, false);
    main_assert_eq!(edit.text() => "");

    edit.set_text("");
    edit.insert_text(&"a".repeat(300));
    main_assert_eq!(edit.text().len() => SEARCH_EDIT_MAX_BYTES);
    edit.set_text("");
    edit.insert_text("left|right");
    main_assert_eq!(edit.text() => "left¦right");
    edit.set_text("éé");
    edit.move_cursor(SearchCursorOperation::Left, false, false);
    main_assert_eq!(edit.caret() => "é".len(), "caret stays on UTF-8 boundaries");
    edit.backspace(false, false);
    main_assert_eq!(edit.text() => "é");

    edit.set_text("alpha beta");
    edit.select_word_at(8);
    main_assert_eq!(edit.selected_text() => Some("beta"));
    edit.begin_pointer_selection(0);
    edit.drag_pointer_selection(edit.text().len());
    edit.end_pointer_selection(edit.text().len());
    main_assert_eq!(edit.selected_text() => Some("alpha beta"));

    edit.set_text("abcdef");
    edit.begin_pointer_selection(5);
    edit.drag_pointer_selection(2);
    main_assert_eq!(edit.selected_text() => Some("cde"));
    main_assert!(edit.backspace(false, false));
    main_assert_eq!(edit.text() => "abf");
    edit.drag_pointer_selection(edit.text().len());
    main_assert_eq!(edit.selected_text() => Some("f"), "selection deletion updates the still-active physical drag anchor");
    edit.end_pointer_selection(edit.text().len());

    edit.set_text("abcdef");
    edit.begin_pointer_selection(5);
    main_assert!(edit.backspace(false, false));
    main_assert_eq!(edit.text() => "abcdf");
    main_assert_eq!(edit.caret() => 4);
    edit.drag_pointer_selection(2);
    main_assert_eq!(edit.selected_text() => Some("cdf"), "collapsed cursor deletion preserves C++'s hidden drag anchor");
    edit.end_pointer_selection(2);

    edit.set_text("W".repeat(100));
    edit.scroll_cursor_in_view(500, 100, 3);
    main_assert!(edit.horizontal_scroll > 0);
    edit.move_cursor(SearchCursorOperation::Home, false, false);
    edit.scroll_cursor_in_view(0, 100, 3);
    main_assert_eq!(edit.horizontal_scroll => 1);
    main_assert!(edit.cursor_visible());
    for _ in 0..18 {
        edit.tick_blink();
    }
    main_assert!(!edit.cursor_visible());
}

// C++ notifies text change while deleting the selection before a
// replacement that cannot fit (src/C4GuiEdit.cpp:145-190). The Rust edit
// must likewise report that mutation so live results are refreshed.
#[test]
fn scensel_search_edit_reports_selection_deletion_when_replacement_does_not_fit() {
    let mut edit = SearchEditState::default();
    edit.set_text("a".repeat(SEARCH_EDIT_MAX_BYTES));
    edit.anchor = SEARCH_EDIT_MAX_BYTES - 1;
    edit.caret = SEARCH_EDIT_MAX_BYTES;

    let changed = edit.insert_text("é");

    main_assert!(changed);
    main_assert_eq!(edit.text().len() => SEARCH_EDIT_MAX_BYTES - 1);
}

#[test]
fn scensel_middle_down_inserts_raw_primary_without_focus_or_submit() {
    let mut app = new_menu_app(800, 600);
    app.open_scenario_browser();
    let fonts = app.assets.clonk_fonts.clone().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);

    app.menu_state.set_search_text("alpha beta");
    app.menu_state.search_edit.anchor = 0;
    app.menu_state.search_edit.caret = 5;
    main_assert!(!app.menu_state.search_focused());
    let insertion = "raw|primary\ntext";
    let clicked_position = "alpha ".len();
    let point = GuiPoint::new(
        (layout.search_edit.x + 4 + fonts.text.measure("alpha ", false).0) as f32,
        (layout.search_edit.y + layout.search_edit.h / 2) as f32,
    );
    main_assert_eq!(app.scensel_search_char_pos(point, true) => Some(clicked_position));
    main_assert!(app.handle_scensel_search_middle_down(point, Some(insertion)));
    main_assert_eq!(app.menu_state.search_text() => "alpha raw|primary\ntextbeta");
    main_assert_eq!(app.menu_state.search_edit.caret() => clicked_position + insertion.len());
    main_assert!(app.menu_state.search_edit.selection_range().is_none());
    main_assert!(!app.menu_state.search_focused(), "middle-down does not focus");
    main_assert_eq!(app.menu_state.applied_search_text => "", "raw PRIMARY insertion does not submit the search");

    let unchanged = app.menu_state.search_text().to_string();
    app.menu_state.search_edit.horizontal_scroll = 0;
    let start = GuiPoint::new(
        (layout.search_edit.x + 4) as f32,
        (layout.search_edit.y + layout.search_edit.h / 2) as f32,
    );
    app.menu_state.search_edit.blink_ticks = 7;
    app.menu_state.search_edit.dragging = true;
    main_assert!(app.handle_scensel_search_middle_down(start, None));
    main_assert_eq!(app.menu_state.search_text() => unchanged);
    main_assert_eq!(app.menu_state.search_edit.caret() => 0);
    main_assert_eq!(app.menu_state.search_edit.blink_ticks => 7);
    main_assert!(app.menu_state.search_edit.dragging, "middle-down does not cancel an active left-button drag");
    main_assert!(!app.handle_scensel_search_middle_down(GuiPoint::new(-10.0, -10.0), Some("ignored")));
    main_assert_eq!(app.menu_state.search_text() => unchanged);

    app.menu_state.set_search_text("tail");
    app.menu_state.search_edit.begin_pointer_selection(2);
    main_assert!(app.handle_scensel_search_middle_down(start, Some("raw")));
    main_assert!(app.menu_state.search_edit.dragging);
    let end = app.menu_state.search_text().len();
    app.menu_state.search_edit.drag_pointer_selection(end);
    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("rawtail"), "an active left drag retains the pre-insertion click as its anchor");
    app.menu_state.search_edit.end_pointer_selection(0);
    main_assert!(app.menu_state.search_edit.selection_range().is_none());
    main_assert_eq!(app.menu_state.search_edit.caret() => 0);

    app.menu_state.set_search_text("tail");
    app.menu_state.search_edit.begin_pointer_selection(0);
    let tail_end = GuiPoint::new(
        (layout.search_edit.x + 4 + fonts.text.measure("tail", false).0) as f32,
        (layout.search_edit.y + layout.search_edit.h / 2) as f32,
    );
    main_assert_eq!(app.scensel_search_char_pos(tail_end, true) => Some(4));
    main_assert!(app.handle_scensel_search_middle_down(tail_end, Some("raw")));
    app.menu_state
        .search_edit
        .move_cursor(SearchCursorOperation::End, false, true);
    app.menu_state.search_edit.drag_pointer_selection(0);
    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("tail"), "a no-op Shift+End preserves the hidden pre-insertion drag anchor");
    app.menu_state.search_edit.end_pointer_selection(0);

    app.menu_state.set_search_text("x".repeat(252));
    app.menu_state.search_edit.anchor = 10;
    app.menu_state.search_edit.caret = 20;
    app.menu_state.search_edit.blink_ticks = 9;
    app.menu_state.search_edit.dragging = false;
    main_assert!(app.handle_scensel_search_middle_down(start, Some("raw")));
    main_assert_eq!(app.menu_state.search_text().len() => SEARCH_EDIT_MAX_BYTES);
    main_assert!(app.menu_state.search_text().starts_with("ra"));
    main_assert_eq!(app.menu_state.search_edit.caret() => 2);
    main_assert_eq!(app.menu_state.search_edit.blink_ticks => 0);
    main_assert!(app.menu_state.search_edit.selection_range().is_none());

    let insertion_position = 100;
    let narrow_text = "i".repeat(insertion_position + 3);
    app.menu_state.set_search_text(narrow_text);
    let client_width = layout.search_edit.w - 8;
    let prefix_width = fonts.text.measure(&"i".repeat(insertion_position), false).0;
    main_assert!(prefix_width > client_width);
    let pointer_offset = client_width - 2;
    let old_scroll = prefix_width - pointer_offset;
    app.menu_state.search_edit.horizontal_scroll = old_scroll;
    let same_index_point = GuiPoint::new(
        (layout.search_edit.x + 4 + pointer_offset) as f32,
        (layout.search_edit.y + layout.search_edit.h / 2) as f32,
    );
    main_assert_eq!(app.scensel_search_char_pos(same_index_point, true) => Some(insertion_position));
    main_assert!(app.handle_scensel_search_middle_down(same_index_point, Some("WWW")));
    main_assert_eq!(app.menu_state.search_edit.caret() => insertion_position + 3, "the insertion can end at the old caret byte index");
    main_assert!(app.menu_state.search_edit.horizontal_scroll > old_scroll, "a successful same-index insertion still recomputes cursor scrolling");

    app.menu_state.set_search_text("");
    main_assert!(app.handle_scensel_search_middle_down(start, Some(&"W".repeat(SEARCH_EDIT_MAX_BYTES))));
    main_assert!(app.menu_state.search_edit.horizontal_scroll > 0, "raw insertion scrolls the advanced caret into view");

    app.menu_state
        .set_search_text("W".repeat(SEARCH_EDIT_MAX_BYTES));
    app.menu_state.set_search_focused(true);
    app.menu_state.search_edit.blink_ticks = 7;
    app.menu_state.search_edit.dragging = true;
    app.menu_state.set_pointer_position(Some(start));
    app.startup.dialog_fade = None;
    app.handle_other_mouse_button(ElementState::Pressed)
        .test_value();
    main_assert_eq!(app.menu_state.search_text().len() => SEARCH_EDIT_MAX_BYTES);
    main_assert_eq!(app.menu_state.search_edit.caret() => 0);
    main_assert!(app.menu_state.search_edit.selection_range().is_none());
    main_assert_eq!(app.menu_state.search_edit.blink_ticks => 7, "a full buffer cannot insert PRIMARY and does not restart blink");
    main_assert!(app.menu_state.search_edit.dragging);
}

#[test]
fn scensel_search_context_entries_match_cpp_conditions_and_order() {
    let mut edit = SearchEditState::default();
    main_assert!(scensel_search_context_entries(&edit, false).is_empty());

    let paste_only = scensel_search_context_entries(&edit, true);
    main_assert_eq!(paste_only.len() => 1);
    main_assert_eq!(paste_only[0].text => "Paste");

    edit.set_text("alpha beta");
    let select_only = scensel_search_context_entries(&edit, false);
    main_assert_eq!(select_only.len() => 1);
    main_assert_eq!(select_only[0].text => "Select all");

    edit.anchor = 0;
    edit.caret = 5;
    let entries = scensel_search_context_entries(&edit, true);
    main_assert_eq!(entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>() => vec!["Cut", "Copy", "Paste", "Clear", "Select all"]);
    main_assert_eq!(
        entries
            .iter()
            .map(|entry| entry.tooltip.as_deref())
            .collect::<Vec<_>>() =>
        vec![
            Some("Moves the selection to the clipboard."),
            Some("Copies the selection to the clipboard."),
            Some("Inserts the contents of the clipboard."),
            Some("Clears the selection."),
            Some("Selects the complete text"),
        ]
    );
    main_assert!(entries.iter().all(|entry| { entry.icon == ContextMenuIcon::None && entry.hotkey.is_none() }));

    edit.anchor = edit.text().len();
    edit.caret = 0;
    let whole_reverse = scensel_search_context_entries(&edit, false);
    main_assert_eq!(
        whole_reverse
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>() =>
        vec!["Cut", "Copy", "Clear"],
        "whole selection omits Select all in either direction"
    );
}

#[test]
fn scensel_search_space_stays_in_the_focused_edit() {
    let scenarios = sample_scenarios();
    let entries = build_menu_entries(&scenarios, false);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut app = new_menu_app(800, 600);
    app.menu_state = MenuState::new(menu, scenarios);
    app.menu_state.set_include_back(false);
    let _ = app.menu_state.select_default_entry();
    app.open_scenario_browser();
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.menu_state.set_search_text("two");

    // Dialog::CharIn routes to the focused control, and Edit::CharIn
    // accepts ASCII space; the unfocused list cannot consume it
    // (src/C4GuiDialogs.cpp:552-560; src/C4GuiEdit.cpp:448-455;
    // src/C4Gui.h:1622-1635).
    app.test_key(VirtualKeyCode::Space, ElementState::Pressed);
    main_assert_eq!(app.menu_state.stack.len() => 1);
    main_assert!(app.menu_state.search_focused());
    app.test_text_input(' ');
    app.test_key(VirtualKeyCode::Space, ElementState::Released);

    main_assert_eq!(app.menu_state.search_text() => "two ");
    main_assert_eq!(app.menu_state.stack.len() => 1);
    main_assert!(app.menu_state.search_focused());
}

// C4GUI::Edit consumes plain cursor operations and moves the caret by one
// character (src/C4GuiEdit.cpp:371-445). The selector's dialog-wide
// Left/Right bindings must not see those keys while the edit has focus.
#[test]
fn scensel_search_plain_arrows_move_the_caret_without_navigating() {
    let mut app = new_menu_app(800, 600);
    app.open_scenario_browser();
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.menu_state.set_search_text("cave");
    let selected = app
        .menu_state
        .selected_scenario()
        .map(|entry| entry.identifier.clone());

    app.test_key(VirtualKeyCode::ArrowLeft, ElementState::Pressed);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.menu_state.stack.len() => 1);
    main_assert_eq!(app.menu_state.search_edit.caret() => 3);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.clone()) => selected);

    app.test_key(VirtualKeyCode::ArrowRight, ElementState::Pressed);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert_eq!(app.menu_state.stack.len() => 1);
    main_assert_eq!(app.menu_state.search_edit.caret() => 4);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.clone()) => selected);
}

// C++ binds Ctrl+F through the startup dialog accelerator
// (src/C4StartupScenSelDlg.cpp:1400-1401). The macOS product path also
// accepts the platform-standard Command+F and reselects an active query.
#[cfg(target_os = "macos")]
#[test]
fn scensel_search_command_f_focuses_and_reselects_the_query() {
    let mut app = new_menu_app(800, 600);
    app.open_scenario_browser();
    app.menu_state.set_search_text("crystal");
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.menu_state
        .search_edit
        .move_cursor(SearchCursorOperation::Left, false, false);
    main_assert!(app.menu_state.search_edit.selection_range().is_none());
    app.test_modifiers(ModifiersState::SUPER);

    app.test_key(VirtualKeyCode::KeyF, ElementState::Pressed);

    main_assert!(app.menu_state.search_focused());
    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("crystal"));
}

// C++ registers Ctrl for edit commands (src/C4GuiEdit.cpp:61-78).
// The macOS product path also accepts Command for the standard edit
// shortcut family.
#[cfg(target_os = "macos")]
#[test]
fn scensel_search_command_a_selects_the_query() {
    let mut app = new_menu_app(800, 600);
    app.open_scenario_browser();
    app.menu_state.set_search_text("crystal");
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.menu_state
        .search_edit
        .move_cursor(SearchCursorOperation::Left, false, false);
    app.test_modifiers(ModifiersState::SUPER);

    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);

    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("crystal"));
}

// C++ waits for Enter and only blurs on Escape
// (src/C4StartupScenSelDlg.cpp:1513-1537,1810-1817). The enhanced
// product path filters in-memory scenarios immediately; the first Escape
// clears and restores browsing state, and the second leaves the edit.
#[test]
fn scensel_enhanced_search_filters_live_and_escape_restores_browsing_state() {
    scensel_fixture!(frontend_scenario: alpha, "alpha".to_string(), "Alpha Mission".to_string());
    scensel_fixture!(frontend_scenario: beta, "beta".to_string(), "Beta Mission".to_string());
    let scenarios = vec![alpha, beta];
    let mut app = scensel_app(&scenarios);
    app.handle_menu_input(|menu| menu.select_list_index(1))
        .test_value();
    app.menu_state.scenario_list_scroll = 47;
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);

    for character in "Alpha".chars() {
        app.test_text_input(character);
    }

    main_assert_eq!(app.menu_state.applied_search_text => "Alpha");
    main_assert_eq!(app.menu_state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["alpha"]);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("alpha"));
    main_assert!(app.menu_state.search_focused());

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(app.menu_state.search_text() => "");
    main_assert_eq!(app.menu_state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["alpha", "beta"]);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("beta"));
    main_assert_eq!(app.menu_state.scenario_list_scroll => 47);
    main_assert!(app.menu_state.search_focused());

    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(!app.menu_state.search_focused());
}

// C++ exposes the search edit without a trailing action
// (src/C4GuiEdit.cpp:556-634). The enhanced product control clears
// immediately from its field-contained target and retains edit focus.
#[test]
fn scensel_enhanced_search_clear_button_restores_browsing_state() {
    scensel_fixture!(frontend_scenario: alpha, "alpha".to_string(), "Alpha Mission".to_string());
    scensel_fixture!(frontend_scenario: beta, "beta".to_string(), "Beta Mission".to_string());
    let scenarios = vec![alpha, beta];
    let mut app = scensel_app(&scenarios);
    app.handle_menu_input(|menu| menu.select_list_index(1))
        .test_value();
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    for character in "Alpha".chars() {
        app.test_text_input(character);
    }

    let fonts = app.assets.clonk_fonts.clone().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);
    let clear = clonk_frontend::startup_scensel::search_clear_button_bounds(&layout);
    app.test_cursor(PhysicalPosition::new(
        f64::from(clear.x + clear.w / 2),
        f64::from(clear.y + clear.h / 2),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_left_button(ElementState::Released);

    main_assert_eq!(app.menu_state.search_text() => "");
    main_assert_eq!(app.menu_state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["alpha", "beta"]);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("beta"));
    main_assert!(app.menu_state.search_focused());
}

// C4GUI::Edit mutates the buffer immediately while C++ waits for Enter to
// rebuild rows (src/C4GuiEdit.cpp:371-445;
// src/C4StartupScenSelDlg.cpp:1513-1537). The product path reapplies after
// keyboard deletion.
#[test]
fn scensel_enhanced_search_updates_after_keyboard_deletion() {
    scensel_fixture!(frontend_scenario: alpha, "alpha".to_string(), "Alpha Mission".to_string());
    let scenarios = vec![alpha];
    let mut app = scensel_app(&scenarios);
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    for character in "Alphzz".chars() {
        app.test_text_input(character);
    }
    main_assert!(app.menu_state.visible_entries().is_empty());

    app.test_key(VirtualKeyCode::Backspace, ElementState::Pressed);

    main_assert_eq!(app.menu_state.search_text() => "Alphz");
    main_assert_eq!(app.menu_state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["alpha"]);
}

// C++ routes Clear through the edit context command but does not rebuild
// until submission (src/C4GuiEdit.h:47-53;
// src/C4GuiEdit.cpp:145-158,645-672;
// src/C4StartupScenSelDlg.h:434-437;
// src/C4StartupScenSelDlg.cpp:1472-1537). The product path reapplies it
// immediately.
#[test]
fn scensel_enhanced_search_context_clear_updates_results_immediately() {
    scensel_fixture!(frontend_scenario: alpha, "alpha".to_string(), "Alpha Mission".to_string());
    scensel_fixture!(frontend_scenario: beta, "beta".to_string(), "Beta Mission".to_string());
    let scenarios = vec![alpha, beta];
    let mut app = scensel_app(&scenarios);
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    for character in "Alpha".chars() {
        app.test_text_input(character);
    }
    main_assert_eq!(app.menu_state.visible_entries().len() => 1);
    app.menu_state.search_edit.select_all();

    app.execute_scenario_search_context_command(ScenselSearchContextCommand::Clear)
        .test_value();

    main_assert_eq!(app.menu_state.search_text() => "");
    main_assert_eq!(app.menu_state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["alpha", "beta"]);
}

// C++ routes Paste through the edit context command but does not rebuild
// until submission (src/C4GuiEdit.h:47-53;
// src/C4GuiEdit.cpp:316-350,645-672;
// src/C4StartupScenSelDlg.h:434-437;
// src/C4StartupScenSelDlg.cpp:1472-1537). The product path reapplies it
// immediately.
#[test]
fn scensel_enhanced_search_paste_updates_results_immediately() {
    scensel_fixture!(frontend_scenario: alpha, "alpha".to_string(), "Alpha Mission".to_string());
    scensel_fixture!(frontend_scenario: beta, "beta".to_string(), "Beta Mission".to_string());
    let scenarios = vec![alpha, beta];
    let mut app = scensel_app(&scenarios);
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    for character in "Alpha".chars() {
        app.test_text_input(character);
    }
    app.menu_state.search_edit.select_all();

    app.paste_scenario_search_text("Beta").test_value();

    main_assert_eq!(app.menu_state.search_text() => "Beta");
    main_assert_eq!(app.menu_state.visible_entries().iter().map(|entry| entry.identifier.as_str()).collect::<Vec<_>>() => vec!["beta"]);
}

// C++ retains the folder caption and leaves an unmatched list blank
// (src/C4StartupScenSelDlg.cpp:1527-1537). The enhanced product renderer
// exposes a count and query-aware recovery state.
#[test]
fn scensel_enhanced_search_reports_counts_and_a_query_aware_empty_state() {
    let scenarios = ["Alpha Mission", "Beta Mission"]
        .into_iter()
        .enumerate()
        .map(|(index, title)| {
            scensel_fixture!(frontend_scenario: scenario, format!("scenario_{index}"), title.to_string());
            scenario
        })
        .collect::<Vec<_>>();
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.set_search_text("mission");
    state.apply_enhanced_search();
    main_assert_eq!(state.enhanced_search_caption().as_deref() => Some("2 of 2 scenarios"));
    main_assert_eq!(state.enhanced_search_empty_message() => None);

    state.set_search_text("missing castle");
    state.apply_enhanced_search();
    main_assert_eq!(state.enhanced_search_caption().as_deref() => Some("No matches among 2 scenarios"));
    main_assert_eq!(state.enhanced_search_empty_message().as_deref() => Some("No scenarios match \"missing castle\"."));
}

// C++ keeps the folder title as the caption
// (src/C4StartupScenSelDlg.cpp:1527-1535). The enhanced product caption
// uses grammatically correct result status for a one-item catalog.
#[test]
fn scensel_enhanced_search_uses_singular_result_status() {
    scensel_fixture!(frontend_scenario: scenario, "alpha".to_string(), "Alpha Mission".to_string());
    let scenarios = vec![scenario];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    state.set_search_text("Alpha");
    state.apply_enhanced_search();
    main_assert_eq!(state.enhanced_search_caption().as_deref() => Some("1 of 1 scenario"));

    state.set_search_text("missing");
    state.apply_enhanced_search();
    main_assert_eq!(state.enhanced_search_caption().as_deref() => Some("No matches among 1 scenario"));
}

// The real window route must consume Ctrl+F/text/Enter in the search
// edit. The product path filters live, while Enter still confirms the
// edit instead of starting the selected scenario
// (src/C4StartupScenSelDlg.cpp:1400-1401,1804-1808;
// src/C4GuiEdit.cpp:364-368).
#[test]
fn scensel_search_routes_window_text_and_enter() {
    let _lock = env_lock().lock();
    let (_guard, _user_data, mut app) = scensel_window_app("Search Tester");

    // The first *scenario*, not simply the first row. Enhanced search never
    // matches a folder: `collect_enhanced_scenario_search_matches` pushes a
    // folder's title onto the ancestor context and recurses into its children,
    // so searching a folder title returns the scenarios inside it rather than
    // the folder. Seeding the query from a folder row would therefore assert
    // that a row reselects itself when it never can. This only became reachable
    // once a folder sorted to the top of the list.
    // Seeded from a scenario, reached by descending, rather than from the first
    // row. Every top-level row is a folder, and enhanced search never matches
    // one: `collect_enhanced_scenario_search_matches` pushes a folder's title
    // onto the ancestor context and recurses into its children. Searching a
    // folder title therefore returns the scenarios *inside* it, so asserting the
    // row reselects itself only ever held because the top folder was "Tutorial"
    // and a scenario happened to share that title. The first folder is no longer
    // "Tutorial", so the coincidence is gone.
    fn first_scenario_title(entries: &[FrontendScenario]) -> Option<String> {
        entries.iter().find_map(|entry| match entry.kind {
            ScenarioKind::Folder => first_scenario_title(&entry.children),
            _ => Some(entry.title.clone()),
        })
    }
    let mut query = first_scenario_title(app.menu_state.visible_entries()).test_value();
    Markup::strip_markup(&mut query);
    query.make_ascii_lowercase();

    app.menu_state.set_search_text("replace this");
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::KeyF, ElementState::Pressed);
    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("replace this"));
    app.test_modifiers(ModifiersState::empty());
    for character in query.chars() {
        app.test_text_input(character);
    }
    main_assert!(app.menu_state.search_focused());
    main_assert_eq!(app.menu_state.applied_search_text => query);
    main_assert!(!app.menu_state.visible_entries().is_empty());
    let mut selected_title = app
        .menu_state
        .selected_scenario()
        .test_value()
        .title
        .clone();
    Markup::strip_markup(&mut selected_title);
    selected_title.make_ascii_lowercase();
    main_assert_eq!(selected_title => query);

    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert_eq!(app.mode => AppMode::Menu, "Enter must not start a scenario");
    main_assert!(!app.menu_state.visible_entries().is_empty());
    main_assert_eq!(app.menu_state.applied_search_text => query);

    let fonts = app.assets.clonk_fonts.clone().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);
    app.menu_state.set_search_text("alpha beta");
    let beta_x = layout.search_edit.x + 4 + fonts.text.measure("alpha be", false).0;
    let edit_y = layout.search_edit.y + layout.search_edit.h / 2;
    for _ in 0..2 {
        app.test_cursor(PhysicalPosition::new(f64::from(beta_x), f64::from(edit_y)));
        app.test_left_button(ElementState::Pressed);
        app.test_left_button(ElementState::Released);
    }
    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("beta"));

    app.test_cursor(PhysicalPosition::new(
        f64::from(layout.search_edit.x + 4),
        f64::from(edit_y),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_cursor(PhysicalPosition::new(
        f64::from(layout.search_edit.x + layout.search_edit.w + 200),
        f64::from(edit_y),
    ));
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("alpha beta"));

    app.menu_state.set_search_text("W".repeat(100));
    let mut frame = vec![0_u8; 800 * 600 * 4];
    app.test_render(&mut frame);
    main_assert!(app.menu_state.search_edit.horizontal_scroll > 0);
    app.test_key(VirtualKeyCode::Home, ElementState::Pressed);
    app.test_render(&mut frame);
    main_assert!(app.menu_state.search_edit.horizontal_scroll <= 2);
    reset_cached_app_paths();
}

#[test]
fn scensel_selector_shortcuts_execute_before_conflicting_controls() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let scenario_path = paths.scenario_dir().join("Shortcut.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Shortcut Target\n",
    )
    .test_value();
    scensel_fixture!(frontend_scenario: scenario, "Shortcut.c4s".to_string(), "Shortcut Target".to_string());
    scenario.path = Some(scenario_path);
    scenario.source_paths = vec![scenario.path.clone().expect("scenario path")];
    let scenarios = vec![scenario];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.menu_state = MenuState::new(menu, scenarios);
    app.open_network_host_scenario_browser();
    main_assert!(app.menu_state.selected_scenario().is_some());

    let values = GameOptionValues {
        comment: "unchanged comment".to_string(),
        ..GameOptionValues::default()
    };
    app.scenario_game_options =
        GameOptionButtons::new(GameOptionContext::NetworkHostSelector, values);
    app.sync_scenario_game_option_bounds();
    app.scenario_game_options.set_focused_button(Some(
        clonk_frontend::game_option_buttons::GameOptionButton::Comment,
    ));
    app.menu_state.set_dialog_focus(ScenselDialogFocus::Options);
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert_eq!(app.game_option_input_dialog.as_ref().expect("Mission Access input dialog").purpose => PendingInputDialogPurpose::ScenarioMissionAccess);
    main_assert_eq!(app.scenario_game_options.values().comment => "unchanged comment");
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
        .test_value();
    main_assert!(app.game_option_input_dialog.is_none());

    app.test_modifiers(ModifiersState::ALT | ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert!(app.game_option_input_dialog.is_none());

    app.test_modifiers(ModifiersState::ALT | ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert_eq!(
        app.game_option_input_dialog
            .as_ref()
            .expect("Comment input dialog")
            .purpose =>
        PendingInputDialogPurpose::GameOption(GameOptionInputKind::Comment)
    );
    app.game_option_input_dialog = None;
    app.game_option_input_consumed_keys.clear();
    app.game_option_consumed_keys.clear();

    app.test_modifiers(ModifiersState::ALT | ModifiersState::SUPER);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert_eq!(app.game_option_input_dialog.as_ref().expect("Mission Access input dialog").purpose => PendingInputDialogPurpose::ScenarioMissionAccess);
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
        .test_value();

    app.test_modifiers(ModifiersState::empty());
    app.menu_state.set_search_text("context");
    app.menu_state.set_search_focused(true);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert!(app.context_menu.is_some());
    main_assert!(app.game_option_input_dialog.is_none());
    main_assert_eq!(app.scenario_game_options.values().comment => "unchanged comment");
    app.close_context_menu_silently();

    app.test_modifiers(ModifiersState::empty());
    app.menu_state.set_search_text("");
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    main_assert_eq!(app.menu_state.rename_edit.as_ref().map(|rename| rename.edit.selected_text()) => Some(Some("Shortcut Target")));
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search, "RenameEdit restores the control focused before F2");
    main_assert!(app.menu_state.search_focused());

    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    app.test_modifiers(ModifiersState::SUPER);
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    wait_for_scenario_selector_discovery(&mut app);

    app.test_modifiers(ModifiersState::empty());
    app.menu_state.set_search_text("alpha beta");
    app.menu_state.set_search_focused(true);
    app.menu_state.search_edit.anchor = 0;
    app.menu_state.search_edit.caret = 0;
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert_eq!(app.menu_state.search_text() => "alpha beta");
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();

    // The selector binds only unmodified Delete. Ctrl+Delete remains an
    // edit operation, matching Edit::RegisterCursorOp's modifier list.
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.menu_state.search_text() => "beta");
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.menu_state.search_text() => "beta");
    reset_cached_app_paths();
}

#[test]
fn scensel_rename_restores_search_and_specific_option_focus() {
    scensel_fixture!(frontend_scenario: scenario, "Focus.c4s".to_string(), "Focus Target".to_string());
    let scenarios = vec![scenario];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app(800, 600);
    app.menu_state = MenuState::new(menu, scenarios);
    app.open_scenario_browser();

    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    main_assert!(!app.menu_state.search_focused(), "inline edit steals focus");
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search);
    main_assert!(app.menu_state.search_focused());

    app.set_scensel_dialog_focus(ScenselDialogFocus::Options);
    app.scenario_game_options
        .set_focused_button(Some(GameOptionButton::Record));
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    main_assert_eq!(app.scenario_game_options.focused_button() => None);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Options);
    main_assert_eq!(app.scenario_game_options.focused_button() => Some(GameOptionButton::Record));

    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search);
    main_assert!(app.menu_state.search_focused());

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    let original_title = app
        .menu_state
        .rename_edit
        .test_ref()
        .edit
        .text()
        .to_string();
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.menu_state.rename_edit.as_ref().expect("Alt+Delete keeps rename active").edit.text() => original_title);
    app.test_key(VirtualKeyCode::KeyZ, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_some());
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_some());
    main_assert_eq!(app.game_option_input_dialog.as_ref().expect("Mission Access input").purpose => PendingInputDialogPurpose::ScenarioMissionAccess);
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
        .test_value();
    main_assert!(app.menu_state.rename_edit.is_some());
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_some());
    app.test_modifiers(ModifiersState::SHIFT);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_some());
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());

    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::KeyF, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::List, "RR_Deleted focus cancels the original Control+F transfer");

    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search, "empty abort restores focus and cancels the original Tab transfer");

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "MissionPass".to_string(),
    )])
    .test_value();
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::Search, "UpdateList aborts rename and restores its saved focus before rebuilding");
}

#[test]
fn scensel_rename_gamepad_low_and_directions_match_dialog_bindings() {
    scensel_fixture!(frontend_scenario: child, "Folder.c4f/Child.c4s".to_string(), "Child".to_string());
    child.path = None;

    scensel_fixture!(frontend_scenario: folder, "Folder.c4f".to_string(), "Folder".to_string());
    folder.kind = ScenarioKind::Folder;
    folder.is_playable = false;
    folder.path = None;
    folder.children = vec![child];

    let scenarios = vec![folder];
    let mut app = scensel_app(&scenarios);
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);

    let slot = GamepadSlot::new(0);
    let source = |gamepad, event| SourcedGamepadEvent {
        gamepad,
        cluster: gamepad as u64,
        event,
    };
    app.process_sourced_gamepad_event_batch(
        [source(
            0,
            gamepad_direction_event(slot, ControlButton::Right, ElementState::Pressed),
        )],
        false,
    )
    .test_value();
    main_assert!(app.menu_state.rename_edit.is_some());
    let wrong_slot = GamepadSlot::new(1);
    app.process_sourced_gamepad_event_batch(
        [source(
            1,
            gamepad_direction_event(wrong_slot, ControlButton::Left, ElementState::Pressed),
        )],
        true,
    )
    .test_value();
    main_assert!(app.menu_state.rename_edit.is_some());
    app.test_gamepad_events([gamepad_direction_event(
        slot,
        ControlButton::Up,
        ElementState::Pressed,
    )]);
    main_assert!(app.menu_state.rename_edit.is_some());
    app.test_gamepad_events([gamepad_direction_event(
        slot,
        ControlButton::Right,
        ElementState::Pressed,
    )]);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::List, "successful RR_Deleted rename owns focus; the original advance is cancelled");
    main_assert!(app.menu_state.current_folder().is_none());

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_gamepad_events([
        gamepad_gui_button_event(slot, GuiButtonClass::Low, ElementState::Pressed),
        gamepad_action_event(slot, GamepadActionType::Cancel, ElementState::Pressed),
    ]);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.current_folder().map(|folder| folder.identifier.as_str()) => Some("Folder.c4f"));
}

#[test]
fn scensel_rename_abort_paths_do_not_mutate_and_focus_loss_commits() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let old_path = paths.scenario_dir().join("Old.c4s");
    let new_path = paths.scenario_dir().join("New.c4s");
    fs::create_dir_all(&old_path).test_value();
    fs::write(old_path.join("Scenario.txt"), "[Head]\nTitle=Old\n").test_value();

    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    let index = app
        .menu_state
        .visible_entries()
        .iter()
        .position(|entry| entry.identifier == "Old.c4s")
        .test_value();
    app.handle_menu_input(|menu| menu.select_list_index(index))
        .test_value();

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    for character in "New".chars() {
        app.test_text_input(character);
    }
    let slot = GamepadSlot::new(0);
    app.test_gamepad_events([gamepad_action_event(
        slot,
        GamepadActionType::Cancel,
        ElementState::Pressed,
    )]);
    main_assert!(app.menu_state.rename_edit.is_some());

    let source = |gamepad, cluster, event| SourcedGamepadEvent {
        gamepad,
        cluster,
        event,
    };
    app.process_sourced_gamepad_event_batch(
        [
            source(
                0,
                10,
                gamepad_gui_button_event(slot, GuiButtonClass::High, ElementState::Pressed),
            ),
            source(
                0,
                10,
                gamepad_action_event(slot, GamepadActionType::MenuToggle, ElementState::Pressed),
            ),
        ],
        false,
    )
    .test_value();
    main_assert!(app.menu_state.rename_edit.is_some());

    let wrong_slot = GamepadSlot::new(1);
    app.process_sourced_gamepad_event_batch(
        [
            source(
                1,
                11,
                gamepad_gui_button_event(wrong_slot, GuiButtonClass::High, ElementState::Pressed),
            ),
            source(
                1,
                11,
                gamepad_action_event(
                    wrong_slot,
                    GamepadActionType::MenuToggle,
                    ElementState::Pressed,
                ),
            ),
        ],
        true,
    )
    .test_value();
    main_assert!(app.menu_state.rename_edit.is_some());

    app.test_gamepad_events([
        gamepad_gui_button_event(slot, GuiButtonClass::High, ElementState::Pressed),
        gamepad_action_event(slot, GamepadActionType::MenuToggle, ElementState::Pressed),
    ]);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);
    main_assert!(old_path.exists());
    main_assert!(!new_path.exists());

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert!(old_path.exists());
    main_assert!(!new_path.exists());

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    for character in "New".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Tab, ElementState::Pressed);
    wait_for_scenario_selector_discovery(&mut app);
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert!(!old_path.exists());
    main_assert!(new_path.exists());
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| (entry.identifier.as_str(), entry.title.as_str())) => Some(("New.c4s", "New")));
    main_assert!(app.scensel.catalog.contains_key("New.c4s"));
    reset_cached_app_paths();
}

#[test]
fn scensel_delete_falls_through_to_search_edit_without_a_selection() {
    let menu = StartupMenu::new(Vec::new(), test_font(), None).test_value();
    let mut app = new_menu_app(800, 600);
    app.menu_state = MenuState::new(menu, Vec::new());
    app.open_scenario_browser();
    main_assert!(app.menu_state.selected_scenario().is_none());

    app.menu_state.set_search_text("abc");
    app.menu_state.set_search_focused(true);
    app.menu_state.search_edit.anchor = 0;
    app.menu_state.search_edit.caret = 0;
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.menu_state.search_text() => "bc");

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
}

// C++ F5 reloads the current folder and reapplies the live edit
// (src/C4StartupScenSelDlg.cpp:1472-1537,1727-1735). The enhanced product
// path reapplies its catalog-wide query atomically after rediscovery.
#[test]
fn scensel_f5_rediscovers_current_folder_and_applies_live_search() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let folder = paths.scenario_dir().join("RefreshPack.c4f");
    let alpha = folder.join("Alpha.c4s");
    let beta = folder.join("Beta.c4s");
    let delta = folder.join("Delta.c4s");
    fs::create_dir_all(&alpha).test_value();
    fs::create_dir_all(&beta).test_value();
    fs::create_dir_all(&delta).test_value();
    fs::write(folder.join("Folder.txt"), "[Head]\nIndex=1\n").test_value();
    fs::write(
        alpha.join("Scenario.txt"),
        "[Head]\nTitle=Refresh Alpha Mission\n",
    )
    .test_value();
    fs::write(
        beta.join("Scenario.txt"),
        "[Head]\nTitle=Refresh Beta Mission\n",
    )
    .test_value();
    fs::write(delta.join("Scenario.txt"), "[Head]\nTitle=Unrelated\n").test_value();

    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    app.menu_state.enter_folder("RefreshPack.c4f");
    main_assert_eq!(app.menu_state.current_folder().map(|folder| folder.identifier.as_str()) => Some("RefreshPack.c4f"));
    let beta_index = app
        .menu_state
        .visible_entries()
        .iter()
        .position(|entry| entry.identifier == "RefreshPack.c4f/Beta.c4s")
        .test_value();
    app.handle_menu_input(|menu| menu.menu().select_entry_by_index(beta_index).test_value())
        .test_value();
    app.menu_state.set_search_text("refresh mission");
    app.menu_state.set_search_focused(true);

    let gamma = folder.join("Gamma.c4s");
    fs::create_dir_all(&gamma).test_value();
    fs::write(
        gamma.join("Scenario.txt"),
        "[Head]\nTitle=Refresh Gamma Mission\n",
    )
    .test_value();
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);

    main_assert_eq!(app.scenario_selector_loading_label().as_deref() => Some("Loading... (0%)"), "the loading book is observable before the worker can be polled");
    let mut zero_percent_frame = vec![0_u8; 800 * 600 * 4];
    app.test_render(&mut zero_percent_frame);
    app.scensel.discovery.test_mut().progress_percent = 37;
    let mut progressed_frame = vec![0_u8; 800 * 600 * 4];
    app.test_render(&mut progressed_frame);
    main_assert_ne!(zero_percent_frame => progressed_frame, "the visible loading label must track the percentage state");
    main_assert_eq!(app.menu_state.search_text() => "refresh mission");
    app.test_key(VirtualKeyCode::Home, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.menu_state.search_text() => "efresh mission");
    main_assert_eq!(
        app.menu_state
            .selected_scenario()
            .map(|entry| entry.identifier.as_str()) =>
        Some("RefreshPack.c4f/Beta.c4s"),
        "the old tree remains intact behind the loading book"
    );
    main_assert!(!app.scensel.catalog.contains_key("RefreshPack.c4f/Gamma.c4s"), "the discovered tree must not leak in before the atomic completion");

    wait_for_scenario_selector_discovery(&mut app);

    main_assert_eq!(app.menu_state.current_folder().map(|folder| folder.identifier.as_str()) => Some("RefreshPack.c4f"));
    main_assert_eq!(
        app.menu_state
            .visible_entries()
            .iter()
            .map(|entry| entry.identifier.as_str())
            .collect::<Vec<_>>() =>
        vec![
            "RefreshPack.c4f/Alpha.c4s",
            "RefreshPack.c4f/Beta.c4s",
            "RefreshPack.c4f/Gamma.c4s",
        ]
    );
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("RefreshPack.c4f/Beta.c4s"));
    main_assert_eq!(app.menu_state.search_text() => "efresh mission");
    main_assert_eq!(app.menu_state.applied_search_text => "efresh mission");
    main_assert!(app.scensel.catalog.contains_key("RefreshPack.c4f/Gamma.c4s"));
    reset_cached_app_paths();
}

#[test]
fn scensel_f2_renames_unpacked_scenario_rewrites_title_and_refocuses() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "Language", "DE").test_value();
    let old_path = paths.scenario_dir().join("Old.c4s");
    fs::create_dir_all(&old_path).test_value();
    fs::write(
        old_path.join("Scenario.txt"),
        "[Head]\nTitle=Old display title\n",
    )
    .test_value();
    fs::write(old_path.join("Title.txt"), "US:Old display title").test_value();

    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    let index = app
        .menu_state
        .visible_entries()
        .iter()
        .position(|entry| entry.identifier == "Old.c4s")
        .test_value();
    app.handle_menu_input(|menu| menu.select_list_index(index))
        .test_value();
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    main_assert!(old_path.exists());
    main_assert!(app.menu_state.rename_edit.is_none());
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.menu_state.rename_edit.as_ref().map(|rename| rename.edit.text()) => Some(""));
    main_assert!(app.dialogs.messages.is_empty());
    for character in "New Name".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);
    wait_for_scenario_selector_discovery(&mut app);

    let new_path = paths.scenario_dir().join("New Name.c4s");
    main_assert!(!old_path.exists());
    main_assert!(new_path.is_dir());
    main_assert_eq!(fs::read_to_string(new_path.join("Title.txt")).expect("read rewritten title") => "DE:New Name");
    main_assert!(app.menu_state.rename_edit.is_none());
    main_assert_eq!(app.menu_state.dialog_focus() => ScenselDialogFocus::List);
    main_assert!(!app.menu_state.search_focused());
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| (entry.identifier.as_str(), entry.title.as_str())) => Some(("New Name.c4s", "New Name")));
    main_assert!(app.scensel.catalog.contains_key("New Name.c4s"));
    reset_cached_app_paths();
}

#[test]
fn scenario_storage_renames_and_deletes_nested_packed_child() {
    let directory = tempdir();
    let outer_path = directory.path().join("Campaign.c4f");
    let mut scenario = MutableGroup::new("Old.c4s");
    scenario
        .add_file("Scenario.txt", b"[Head]\nTitle=Old\n".to_vec())
        .test_value();
    scenario
        .add_file("Title.txt", b"US:Old".to_vec())
        .test_value();
    let mut chapter = MutableGroup::new("Chapter.c4f");
    chapter.add_child("Old.c4s", scenario).test_value();
    let mut campaign = MutableGroup::new("Campaign.c4f");
    campaign.add_child("Chapter.c4f", chapter).test_value();
    fs::write(&outer_path, campaign.pack().test_value()).test_value();

    main_assert_eq!(scenario_filename_from_title("Foo.c4s", ScenarioKind::Scenario, Path::new("Old.c4s")) => "Fooc4s.c4s");
    main_assert_eq!(scenario_filename_from_title(".!", ScenarioKind::Scenario, Path::new("Old.c4s")) => "unnamed.c4s");
    main_assert_eq!(scenario_filename_from_title("New Pack", ScenarioKind::Folder, Path::new("Old.c4f")) => "New Pack.c4f");
    main_assert_eq!(scenario_filename_from_title("New Dir", ScenarioKind::Folder, Path::new("OldDir")) => "New Dir");
    let renamed = rename_scenario_storage(
        &outer_path.join("Chapter.c4f/Old.c4s"),
        ScenarioKind::Scenario,
        "Packed New",
        "US",
    )
    .test_value();
    main_assert_eq!(renamed => outer_path.join("Chapter.c4f/Packed New.c4s"));
    let campaign = Group::open(&outer_path).test_value();
    let chapter = campaign.open_child("Chapter.c4f").test_value();
    main_assert!(!chapter.exists("Old.c4s"));
    let renamed_group = chapter.open_child("Packed New.c4s").test_value();
    main_assert_eq!(renamed_group.read_file("Title.txt").expect("read packed title") => b"US:Packed New");

    delete_scenario_storage(&renamed).test_value();
    let campaign = Group::open(&outer_path).test_value();
    let chapter = campaign.open_child("Chapter.c4f").test_value();
    main_assert!(!chapter.exists("Packed New.c4s"));
}

#[test]
fn scensel_rename_collision_is_modal_and_keeps_editor_and_storage() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    for (filename, title) in [("Source.c4s", "Source"), ("Taken.c4s", "Taken")] {
        let path = paths.scenario_dir().join(filename);
        fs::create_dir_all(&path).test_value();
        fs::write(
            path.join("Scenario.txt"),
            format!("[Head]\nTitle={title}\n"),
        )
        .test_value();
    }
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    let index = app
        .menu_state
        .visible_entries()
        .iter()
        .position(|entry| entry.identifier == "Source.c4s")
        .test_value();
    app.handle_menu_input(|menu| menu.select_list_index(index))
        .test_value();
    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    for character in "Taken".chars() {
        app.test_text_input(character);
    }
    app.test_key(VirtualKeyCode::Enter, ElementState::Pressed);

    main_assert!(paths.scenario_dir().join("Source.c4s").exists());
    main_assert!(paths.scenario_dir().join("Taken.c4s").exists());
    let rename = app.menu_state.rename_edit.test_ref();
    main_assert!(rename.edit.is_focused());
    main_assert_eq!(rename.edit.selected_text() => Some("Taken"));
    main_assert_eq!(app.dialogs.messages.len() => 1);
    main_assert_eq!(app.dialogs.messages[0].state.caption() => "Rename failure");
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
        .test_value();
    let rename = app.menu_state.rename_edit.test_ref();
    main_assert!(rename.edit.is_focused());
    main_assert_eq!(rename.edit.selected_text() => Some("Taken"));
    reset_cached_app_paths();
}

#[test]
fn scensel_delete_confirms_exact_subject_deletes_and_selects_next() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    for (filename, title) in [("A.c4s", "Alpha"), ("B.c4s", "Beta"), ("C.c4s", "Gamma")] {
        let path = paths.scenario_dir().join(filename);
        fs::create_dir_all(&path).test_value();
        fs::write(
            path.join("Scenario.txt"),
            format!("[Head]\nTitle={title}\n"),
        )
        .test_value();
    }
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    let index = app
        .menu_state
        .visible_entries()
        .iter()
        .position(|entry| entry.identifier == "B.c4s")
        .test_value();
    let expected_next = app
        .menu_state
        .visible_entries()
        .get(index + 1)
        .test_value()
        .identifier
        .clone();
    app.handle_menu_input(|menu| menu.select_list_index(index))
        .test_value();
    app.menu_state
        .set_search_text("pending query that excludes Gamma");
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(app.dialogs.messages.last().expect("delete confirmation").state.message() => "Delete Scenario Beta?");
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();
    wait_for_scenario_selector_discovery(&mut app);
    main_assert!(!paths.scenario_dir().join("B.c4s").exists());
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some(expected_next.as_str()));
    main_assert_eq!(app.menu_state.search_text() => "pending query that excludes Gamma");
    main_assert!(!app.scensel.catalog.contains_key("B.c4s"));
    reset_cached_app_paths();
}

#[test]
fn scensel_delete_uses_original_group_warning() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let original_path = paths.scenario_dir().join("Original.c4s");
    fs::create_dir_all(paths.scenario_dir()).test_value();
    let mut original = MutableGroup::new("Original.c4s");
    original.make_original(true);
    original
        .add_file("Scenario.txt", b"[Head]\nTitle=Original\n".to_vec())
        .test_value();
    fs::write(&original_path, original.pack().test_value()).test_value();

    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    let index = app
        .menu_state
        .visible_entries()
        .iter()
        .position(|entry| entry.identifier == "Original.c4s")
        .test_value();
    app.handle_menu_input(|menu| menu.select_list_index(index))
        .test_value();
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert_eq!(
        app.dialogs.messages
            .last()
            .expect("original warning")
            .state
            .message() =>
        "Scenario Original is an original file. Are your sure you want to delete it?"
    );
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
        .test_value();
    main_assert!(original_path.exists());
    reset_cached_app_paths();
}

#[test]
fn scensel_delete_failure_is_nonfatal_and_keeps_row_selected() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let scenario_path = paths.scenario_dir().join("Failure.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Failure\n",
    )
    .test_value();
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    let index = app
        .menu_state
        .visible_entries()
        .iter()
        .position(|entry| entry.identifier == "Failure.c4s")
        .test_value();
    app.handle_menu_input(|menu| menu.select_list_index(index))
        .test_value();
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    fs::remove_dir_all(&scenario_path).test_value();
    app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
        .test_value();

    let failure = app.dialogs.messages.last().test_value();
    main_assert_eq!(failure.state.caption() => "Delete");
    main_assert_eq!(failure.state.message() => "Delete failure.");
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("Failure.c4s"));
    main_assert!(app.scensel.catalog.contains_key("Failure.c4s"));
    reset_cached_app_paths();
}

#[test]
fn scensel_alt_m_updates_shared_and_persisted_mission_access() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_network_host_scenario_browser();
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    let dialog = app.game_option_input_dialog.test_ref();
    main_assert_eq!(dialog.purpose => PendingInputDialogPurpose::ScenarioMissionAccess);
    main_assert_eq!(dialog.controller.caption() => "Mission Access");
    main_assert_eq!(dialog.controller.message() => "Enter mission password:");
    main_assert_eq!(dialog.controller.icon() => InputDialogIcon::OPTIONS);
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "Secret;Second".to_string(),
    )])
    .test_value();
    wait_for_scenario_selector_discovery(&mut app);
    main_assert_eq!(app.config.mission_access.snapshot() => "Secret;Second");
    // Both native mutation sites change `Config.General.MissionAccess` in
    // memory alone and neither calls `Config.Save()`
    // (C4Script.cpp:2466-2471; C4StartupScenSelDlg.cpp:1838-1856). The port
    // writes the changed list out at once so no aborted run can lose it.
    main_assert_eq!(load_configured_mission_access(&paths).expect("read granted mission access") => "Secret;Second");

    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
        "-secret".to_string(),
    )])
    .test_value();
    wait_for_scenario_selector_discovery(&mut app);
    main_assert_eq!(app.config.mission_access.snapshot() => "Second");
    main_assert_eq!(
        load_configured_mission_access(&paths).expect("read reduced mission access") =>
        "Second",
        "a removal replaces the saved value rather than appending a second one"
    );
    reset_cached_app_paths();
}

#[test]
fn script_earned_mission_access_reaches_the_saved_config() {
    // `FnGainMissionAccess` grows the live `Config.General.MissionAccess`
    // and queues nothing (C4Script.cpp:2466-2471); the host function
    // mutates the very string this store shares with every engine
    // (`configured_mission_access_reaches_fresh_engines_and_survives_replacement`).
    // C++ leaves the write to `Config.Save()` on a clean quit
    // (C4Application.cpp:367), but a mission the player already unlocked is
    // earned progress rather than a runtime toggle, so this port writes it
    // as soon as the list changes: an aborted round must not relock it.
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.config.mission_access.update_modules("Earned", false);

    app.persist_mission_access_if_changed();

    main_assert_eq!(load_configured_mission_access(&paths).expect("read saved mission access") => "Earned");
    // `C4Config` registers MissionAccess as a `CFG_MaxString` escaped
    // string (C4Config.cpp:379), so the quoted C++ form is the only one a
    // shared LegacyClonk install reads back.
    let saved = fs::read_to_string(paths.config_file()).test_value();
    main_assert!(saved.contains("MissionAccess=\"Earned\""), "escaped C4Config string expected, got: {saved}");
    reset_cached_app_paths();
}

#[test]
fn earned_mission_access_survives_an_aborted_session() {
    // The reported failure (clonk-org/clonk-rs#50): a mission unlocked
    // during a round was locked again on the next start, because nothing
    // had written `Config.General.MissionAccess` before the process ended.
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.config.mission_access.update_modules("WestGR", false);
    app.persist_mission_access_if_changed();
    // No shutdown flush: this session never reaches one.
    drop(app);

    let restarted = new_menu_app_with_paths(800, 600, &paths);

    main_assert!(restarted.config.mission_access.contains("westgr"));
    reset_cached_app_paths();
}

#[test]
fn scensel_search_context_routes_pointer_apps_focus_and_release_capture() {
    let _lock = env_lock().lock();
    let (_guard, _user_data, mut app) = scensel_window_app("Search Context Tester");

    let fonts = app.assets.clonk_fonts.clone().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);
    let label_point = PhysicalPosition::new(
        f64::from(layout.search_label.x + layout.search_label.w / 2),
        f64::from(layout.search_label.y + layout.search_label.h / 2),
    );
    app.test_cursor(label_point);
    app.test_right_button(ElementState::Pressed);
    main_assert!(app.context_menu.is_none(), "wooden label has no edit context");

    app.menu_state.set_search_text("alpha beta");
    app.menu_state.search_edit.anchor = 0;
    app.menu_state.search_edit.caret = 5;
    main_assert!(!app.menu_state.search_focused());
    let edit_point = PhysicalPosition::new(
        f64::from(layout.search_edit.x + 4),
        f64::from(layout.search_edit.y + layout.search_edit.h / 2),
    );
    app.test_cursor(edit_point);
    let anchor = gui_point_from_position(edit_point);
    let expected_entries =
        scensel_search_context_entries(&app.menu_state.search_edit, clipboard_text_available());
    let clear_index = expected_entries
        .iter()
        .position(|entry| {
            entry.action
                == Some(AppContextMenuCommand::ScenarioSearch(
                    ScenselSearchContextCommand::Clear,
                ))
        })
        .test_value();
    app.test_right_button(ElementState::Pressed);
    let popup = app.context_menu.test_ref();
    main_assert_eq!(popup.pointer_position() => anchor);
    main_assert_eq!(popup.layout().panels[0].rows.len() => expected_entries.len());
    main_assert_eq!(app.menu_state.search_edit.selected_text() => Some("alpha"));
    main_assert!(!app.menu_state.search_focused(), "right-down does not focus edit");
    app.test_right_button(ElementState::Released);

    let popup_margin = app.context_menu.test_ref().layout().panels[0].bounds;
    app.test_cursor(PhysicalPosition::new(
        f64::from(popup_margin.x + 1),
        f64::from(popup_margin.y + 1),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert_eq!(app.context_menu_pointer_capture => Some(ContextMenuPointerButton::Left));
    app.pointer_left().test_value();
    main_assert_eq!(app.context_menu_pointer_capture => Some(ContextMenuPointerButton::Left), "an open popup retains capture across CursorLeft");
    app.test_cursor(PhysicalPosition::new(0.0, 0.0));
    app.test_left_button(ElementState::Released);
    main_assert!(app.context_menu.is_some());
    main_assert_eq!(app.context_menu_pointer_capture => None);

    let clear = app.context_menu.test_ref().layout().panels[0].rows[clear_index].rect;
    app.test_cursor(PhysicalPosition::new(
        f64::from(clear.x + 1),
        f64::from(clear.y + 1),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.menu_state.search_text() => " beta");
    main_assert_eq!(app.menu_state.applied_search_text => " beta");
    app.test_left_button(ElementState::Released);
    main_assert!(!app.menu_state.search_focused(), "activation release must not click the underlying edit");
    main_assert_eq!(app.context_menu_pointer_capture => None);

    app.menu_state.set_search_focused(true);
    app.menu_state.search_edit.anchor = app.menu_state.search_edit.caret;
    let before = app.menu_state.search_text().to_string();
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    let expected_center = GuiPoint::new(
        (layout.search_edit.x + layout.search_edit.w / 2) as f32,
        (layout.search_edit.y + layout.search_edit.h / 2) as f32,
    );
    main_assert_eq!(app.context_menu.as_ref().expect("Apps context").pointer_position() => expected_center);
    app.test_text_input('Z');
    app.test_modifiers(ModifiersState::CONTROL);
    app.test_key(VirtualKeyCode::KeyA, ElementState::Pressed);
    main_assert_eq!(app.menu_state.search_text() => before);
    main_assert!(app.menu_state.search_edit.selection_range().is_none());
    app.test_modifiers(ModifiersState::empty());
    app.test_key(VirtualKeyCode::Escape, ElementState::Pressed);
    app.test_key(VirtualKeyCode::Escape, ElementState::Released);
    main_assert!(app.context_menu.is_none());
    main_assert!(app.menu_state.search_focused(), "logical focus is retained");

    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    let select_all = app.context_menu.test_ref().layout().panels[0]
        .rows
        .last()
        .test_value()
        .rect;
    app.test_cursor(PhysicalPosition::new(
        f64::from(select_all.x + 1),
        f64::from(select_all.y + 1),
    ));
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.context_menu.is_none());
    main_assert_eq!(app.context_menu_pointer_capture => Some(ContextMenuPointerButton::Left));
    app.pointer_left().test_value();
    main_assert_eq!(app.context_menu_pointer_capture => None);

    app.test_cursor(edit_point);
    app.test_left_button(ElementState::Pressed);
    main_assert!(app.menu_state.search_edit.dragging);
    app.test_left_button(ElementState::Released);
    main_assert!(!app.menu_state.search_edit.dragging, "stale context capture must not swallow a later release");

    app.menu_state.set_search_focused(false);
    app.test_key(VirtualKeyCode::ContextMenu, ElementState::Pressed);
    main_assert!(app.context_menu.is_none());

    let empty_entries = scensel_search_context_entries(&SearchEditState::default(), false);
    app.open_context_menu_at(empty_entries, expected_center)
        .test_value();
    let empty = app.context_menu.test_ref().layout();
    main_assert!(empty.panels[0].rows.is_empty());
    main_assert_eq!((empty.panels[0].bounds.w, empty.panels[0].bounds.h) => (40, 7));
    app.close_context_menu_silently();

    let assets = app.assets.scensel_assets().test_value();
    let button_down = app.assets.dialog_image("GUIButtonDown.png").test_value();
    let book = app.assets.book_fonts.clone().test_value();
    app.menu_state.set_search_text("caret");
    app.menu_state.set_search_focused(true);
    app.menu_state.search_edit.anchor = app.menu_state.search_edit.caret;
    let mut focused = Surface::new(800, 600, PixelFormat::Rgba8888);
    let mut suppressed = Surface::new(800, 600, PixelFormat::Rgba8888);
    draw_scensel_dynamic(
        &mut focused,
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
    draw_scensel_dynamic(
        &mut suppressed,
        &mut app.menu_state,
        &app.scensel.entry_enabled,
        &assets,
        &button_down,
        &fonts,
        &book,
        None,
        startup_gamma(),
        false,
    )
    .test_value();
    main_assert!(focused.pixels() != suppressed.pixels());
    reset_cached_app_paths();
}

// Wheel input is hit-tested to the right-page ScrollWindow and one SDL
// notch advances 60 logical pixels regardless of output scale
// (C4FullScreen.cpp:408; C4GuiContainers.cpp:612-620).
#[test]
fn scensel_description_wheel_scrolls_and_clamps() {
    let _lock = env_lock().lock();
    let (_guard, _user_data, mut app) = scensel_window_app("Scroll Tester");
    app.menu_state.stack[0].entries[0].description = Some(
        (0..100)
            .map(|index| format!("long description line {index}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.menu_state.refresh_menu_entries();
    let _ = app.menu_state.select_default_entry();
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, fonts);
    app.menu_state.set_pointer_position(Some(GuiPoint::new(
        (layout.selection_info.x + 10) as f32,
        (layout.selection_info.y + 10) as f32,
    )));

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert_eq!(app.menu_state.selection_info_scroll => 60);

    app.menu_state.selection_info_scroll = 0;
    app.test_mouse_wheel(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -180.0)),
        3.0,
    );
    main_assert_eq!(app.menu_state.selection_info_scroll => 60);

    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 100.0), 1.0);
    main_assert_eq!(app.menu_state.selection_info_scroll => 0);

    app.menu_state
        .set_pointer_position(Some(GuiPoint::new(0.0, 0.0)));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert_eq!(app.menu_state.selection_info_scroll => 0);

    let template = app.menu_state.stack[0].entries[0].clone();
    app.menu_state.stack[0].entries = (0..20)
        .map(|index| {
            let mut entry = template.clone();
            entry.identifier = format!("scroll_{index:02}");
            entry.title = format!("Scroll {index:02}");
            entry
        })
        .collect();
    app.menu_state.refresh_menu_entries();
    let _ = app.menu_state.select_default_entry();
    app.menu_state.set_pointer_position(Some(GuiPoint::new(
        (layout.list.x + 8) as f32,
        (layout.list.y + 8) as f32,
    )));
    app.test_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0);
    main_assert_eq!(app.menu_state.scenario_list_scroll() => 60);
    let mut scrolled_frame = vec![0_u8; 800 * 600 * 4];
    app.test_render(&mut scrolled_frame);
    main_assert_eq!(app.menu_state.scenario_list_scroll() => 60, "rendering must not snap a manually scrolled list back to its unchanged selection");

    let book_fonts = app.assets.book_fonts.clone().test_value();
    let item_height = clonk_frontend::startup_scensel::scen_list_item_height(&book_fonts.text);
    let click = PhysicalPosition::new(
        f64::from(layout.list.x + 8),
        f64::from(layout.list.y + 3 + item_height / 2),
    );
    app.test_cursor(click);
    app.test_left_button(ElementState::Released);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("scroll_02"));

    // Pressing the list track jumps the fixed pin under the pointer and
    // captures subsequent motion even outside the scrollbar.
    app.menu_state.scenario_list_scroll = 0;
    let list_bar = layout.list_scrollbar;
    app.test_cursor(PhysicalPosition::new(
        f64::from(list_bar.x + 8),
        f64::from(list_bar.y + list_bar.h - 24),
    ));
    app.test_left_button(ElementState::Pressed);
    let list_max_scroll = app
        .menu_state
        .scenario_list_max_scroll(layout.list.h - 6, item_height + 1);
    main_assert_eq!(app.menu_state.scenario_list_scroll() => list_max_scroll);
    app.test_cursor(PhysicalPosition::new(
        f64::from(list_bar.x - 200),
        f64::from(list_bar.y - 200),
    ));
    main_assert_eq!(app.menu_state.scenario_list_scroll() => 0);
    app.test_left_button(ElementState::Released);
    main_assert!(app.menu_state.scrollbar_interaction.is_none());

    // Held arrows advance their persistent bar position by one on every
    // startup draw/update instead of applying a row-sized scroll.
    app.test_cursor(PhysicalPosition::new(
        f64::from(list_bar.x + 8),
        f64::from(list_bar.y + list_bar.h - 8),
    ));
    app.test_left_button(ElementState::Pressed);
    app.test_update();
    main_assert!(matches!(app.menu_state.scrollbar_interaction, Some(ScenselScrollbarInteraction {kind: ScenselScrollbarInteractionKind::Arrow(1), pin: 1,..})));
    main_assert!(app.menu_state.scenario_list_scroll() > 0);
    app.test_update();
    main_assert!(matches!(app.menu_state.scrollbar_interaction, Some(ScenselScrollbarInteraction { pin: 2, .. })));
    app.test_left_button(ElementState::Released);

    // The right description page uses the same captured fixed-thumb
    // interaction, not just wheel scrolling.
    let description_bar = clonk_frontend::startup_scensel::selection_info_scrollbar_rect(&layout);
    app.test_cursor(PhysicalPosition::new(
        f64::from(description_bar.x + 8),
        f64::from(description_bar.y + description_bar.h - 24),
    ));
    app.test_left_button(ElementState::Pressed);
    let description_metrics = {
        let info = scensel_selection_info(&app.menu_state);
        clonk_frontend::startup_scensel::selection_info_scroll_metrics(
            &layout,
            book_fonts.as_ref(),
            &info,
        )
    };
    main_assert_eq!(app.menu_state.selection_info_scroll => description_metrics.max_scroll);
    app.test_cursor(PhysicalPosition::new(
        f64::from(description_bar.x + 200),
        f64::from(description_bar.y - 200),
    ));
    main_assert_eq!(app.menu_state.selection_info_scroll => 0);
    app.test_left_button(ElementState::Released);
    reset_cached_app_paths();
}

// C4GUI::ScrollBar keeps a fixed 16px pin between two 16px arrows.
// Offset<->pin conversion uses integer truncation, while a track press
// centers the pin under the pointer and begins a captured drag
// (C4GuiContainers.cpp:343-473).
#[test]
fn scensel_fixed_scrollbar_geometry_matches_cpp() {
    main_assert_eq!(scensel_scrollbar_pin_travel(48) => None);
    main_assert_eq!(scensel_scrollbar_pin_travel(49) => Some(1));
    main_assert_eq!(scensel_scrollbar_pin_travel(100) => Some(52));
    main_assert_eq!(scensel_scrollbar_pin_from_offset(0, 101, 100) => Some(0));
    main_assert_eq!(scensel_scrollbar_pin_from_offset(50, 101, 100) => Some(25));
    main_assert_eq!(scensel_scrollbar_pin_from_offset(101, 101, 100) => Some(52));
    main_assert_eq!(scensel_scrollbar_offset_from_pin(0, 101, 100) => Some(0));
    main_assert_eq!(scensel_scrollbar_offset_from_pin(26, 101, 100) => Some(50));
    main_assert_eq!(scensel_scrollbar_offset_from_pin(52, 101, 100) => Some(101));
    main_assert_eq!(scensel_scrollbar_jump_pin(-50, 100) => Some(0));
    main_assert_eq!(scensel_scrollbar_jump_pin(50, 100) => Some(26));
    main_assert_eq!(scensel_scrollbar_jump_pin(500, 100) => Some(52));
    main_assert_eq!(scensel_scrollbar_offset_from_pin(0, 0, 100) => None);
}

// C4GUI::ListBox::SelectEntry calls ScrollRangeInView so keyboard
// selection remains visible, clamped against the complete item height
// (C4GuiListBox.cpp:179-193; C4GuiContainers.cpp:549-582).
#[test]
fn scensel_list_scroll_keeps_selection_in_view() {
    let scenarios = (0..20)
        .map(|index| {
            scensel_fixture!(frontend_scenario: entry, format!("scenario_{index:02}"), format!("Scenario {index:02}"));
            entry
        })
        .collect::<Vec<_>>();
    let entries = build_menu_entries(&scenarios, false);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);

    let _ = state.menu().select_entry_by_index(19).test_value();
    state.ensure_list_selection_visible(100, 27, 26);
    main_assert_eq!(state.scenario_list_scroll() => 439);

    main_assert!(state.scroll_scenario_list_by(-60, 100, 27));
    main_assert_eq!(state.scenario_list_scroll() => 379);

    let _ = state.menu().select_entry_by_index(0).test_value();
    state.ensure_list_selection_visible(100, 27, 26);
    main_assert_eq!(state.scenario_list_scroll() => 0);
}

#[test]
fn scensel_list_keys_stop_at_ends_and_page_by_visible_rows() {
    let scenarios = (0..10)
        .map(|index| {
            scensel_fixture!(frontend_scenario: entry, format!("scenario_{index:02}"), format!("Scenario {index:02}"));
            entry
        })
        .collect::<Vec<_>>();
    let entries = build_menu_entries(&scenarios, false);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.set_include_back(false);
    let _ = state.select_default_entry();

    main_assert!(state.move_list_selection_clamped(-1).is_empty());
    main_assert_eq!(state.menu.selected_index() => Some(0));
    let _ = state.select_list_end();
    main_assert_eq!(state.menu.selected_index() => Some(9));
    main_assert!(state.move_list_selection_clamped(1).is_empty());
    main_assert_eq!(state.menu.selected_index() => Some(9));
    let _ = state.select_list_home();

    // With 26px rows, 1px spacing and a 100px viewport, rows 0..=2
    // are fully visible. PageDown chooses row 2; another PageDown first
    // scrolls one viewport and chooses the last fully visible row 6.
    main_assert!(!state.page_list_selection(1, 100, 27, 26).is_empty());
    main_assert_eq!(state.menu.selected_index() => Some(2));
    main_assert_eq!(state.scenario_list_scroll() => 0);
    main_assert!(!state.page_list_selection(1, 100, 27, 26).is_empty());
    main_assert_eq!(state.menu.selected_index() => Some(6));
    main_assert_eq!(state.scenario_list_scroll() => 100);

    main_assert!(!state.page_list_selection(-1, 100, 27, 26).is_empty());
    main_assert_eq!(state.menu.selected_index() => Some(4));
    main_assert!(!state.page_list_selection(-1, 100, 27, 26).is_empty());
    main_assert_eq!(state.menu.selected_index() => Some(0));
    main_assert_eq!(state.scenario_list_scroll() => 0);
}

#[test]
fn scensel_typeahead_cycles_only_with_list_focus() {
    let scenarios = ["Thomas", "Ada", "tina", "Tori"]
        .into_iter()
        .enumerate()
        .map(|(index, title)| {
            scensel_fixture!(frontend_scenario: entry, format!("scenario_{index}"), title.to_string());
            entry
        })
        .collect::<Vec<_>>();
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    let mut app = new_menu_app(800, 600);
    app.menu_state = MenuState::new(menu, scenarios);
    app.open_scenario_browser();
    app.sound.ui_log.clear();

    for (character, expected) in [('T', 2), ('T', 3), ('t', 0), ('T', 2)] {
        let sound_count = app.sound.ui_log.len();
        app.test_text_input(character);
        main_assert_eq!(app.menu_state.menu.selected_index() => Some(expected));
        main_assert_eq!(app.sound.ui_log.len() => sound_count + 1);
        main_assert_eq!(app.sound.ui_log.last().map(String::as_str) => Some("Command"));
    }

    let sound_count = app.sound.ui_log.len();
    app.test_text_input('x');
    main_assert_eq!(app.menu_state.menu.selected_index() => Some(2));
    main_assert_eq!(app.sound.ui_log.len() => sound_count);

    app.menu_state.set_search_text("");
    app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
    app.test_text_input('T');
    main_assert_eq!(app.menu_state.search_text() => "T");
    main_assert_eq!(app.menu_state.menu.selected_index() => Some(1));
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("scenario_2"));
    main_assert_eq!(app.sound.ui_log.len() => sound_count);

    app.set_scensel_dialog_focus(ScenselDialogFocus::Back);
    app.test_text_input('T');
    main_assert_eq!(app.menu_state.search_text() => "T");
    main_assert_eq!(app.menu_state.menu.selected_index() => Some(1));
    main_assert_eq!(app.sound.ui_log.len() => sound_count);
}

#[test]
fn window_keys_map_to_shared_list_navigation_codes() {
    for (window_key, gui_key) in [
        (VirtualKeyCode::Home, KeyCode::Home),
        (VirtualKeyCode::End, KeyCode::End),
        (VirtualKeyCode::PageUp, KeyCode::PageUp),
        (VirtualKeyCode::PageDown, KeyCode::PageDown),
    ] {
        main_assert_eq!(map_key_code(window_key) => Some(gui_key));
    }
}

// Selected-row -> scenario mapping honours the Back-row offset used by
// the network lobby list.
#[test]
fn selected_scenario_maps_through_back_row_offset() {
    let scenarios = sample_scenarios();
    let entries = build_menu_entries(&scenarios, true);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.menu().resize(1280.0, 720.0);

    let _ = state.menu().select_entry_by_index(0); // Back row
    main_assert!(state.selected_scenario().is_none());
    let _ = state.menu().select_entry_by_index(1);
    main_assert_eq!(state.selected_scenario().map(|entry| entry.title.as_str()) => Some("Missions"));
}

// Caption above the list: current folder name, "Scenarios" at root
// (C4StartupScenSelDlg::UpdateList, cpp:1527-1535); with no selection
// the right page falls back to the listed folder (cpp:1566-1572).
#[test]
fn book_caption_and_folder_fallback_track_the_stack() {
    let scenarios = sample_scenarios();
    let entries = build_menu_entries(&scenarios, false);
    let menu = StartupMenu::new(entries, test_font(), None).test_value();
    let mut state = MenuState::new(menu, scenarios);
    state.menu().resize(1280.0, 720.0);
    state.set_include_back(false);

    main_assert_eq!(state.book_caption() => "Scenarios");
    main_assert!(state.current_folder().is_none(), "root has no folder info");

    state.enter_folder("folder_missions");
    main_assert_eq!(state.book_caption() => "Missions");
    main_assert_eq!(state.current_folder().map(|folder| folder.title.as_str()) => Some("Missions"));

    state.leave_folder();
    main_assert_eq!(state.book_caption() => "Scenarios");
}

// List icon defaults (C4StartupScenSelDlg.cpp:705-710,951-952,1036-1037):
// scenario Icon= clamped to the 52-icon strip else 14; .c4f folder 0;
// plain directory 44.
#[test]
fn scensel_entry_icons_follow_cpp_defaults() {
    let mut scenario = FrontendScenario::fallback();
    scenario.kind = ScenarioKind::Scenario;
    scenario.icon_index = Some(15);
    main_assert_eq!(scensel_entry_icon(&scenario) => 15);
    scenario.icon_index = Some(99);
    main_assert_eq!(scensel_entry_icon(&scenario) => 14);
    scenario.icon_index = None;
    main_assert_eq!(scensel_entry_icon(&scenario) => 14);

    let mut folder = FrontendScenario::fallback();
    folder.kind = ScenarioKind::Folder;
    folder.path = Some(PathBuf::from("/tmp/Fantasy.c4f"));
    main_assert_eq!(scensel_entry_icon(&folder) => 0);
    folder.path = Some(PathBuf::from("/tmp/Downloads"));
    main_assert_eq!(scensel_entry_icon(&folder) => 44);
}

fn map_test_scenario(folder: &Path, filename: &str, title: &str) -> FrontendScenario {
    scensel_fixture!(frontend_scenario: scenario, format!("Map.c4f/{filename}"), title.to_string());
    scenario.description = Some(format!("Description for {title}"));
    scenario.path = Some(folder.join(filename));
    scenario
}

fn map_test_folder(path: &Path, children: Vec<FrontendScenario>) -> FrontendScenario {
    scensel_fixture!(frontend_scenario: folder, "Map.c4f".to_string(), "Map Folder".to_string());
    folder.kind = ScenarioKind::Folder;
    folder.is_playable = false;
    folder.path = Some(path.to_path_buf());
    folder.children = children;
    folder
}

fn open_map_test_folder(app: &mut GameApp, folder: FrontendScenario) {
    let scenarios = vec![folder.clone()];
    let menu =
        StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
    app.menu_state = MenuState::new(menu, scenarios.clone());
    app.scensel.catalog = build_scenario_catalog(&scenarios);
    app.open_scenario_browser();
    app.handle_menu_input(|_| {
        vec![StartupMenuAction::OpenEntry(scensel_fixture!(
            scenario:
                folder.identifier.clone(),
                folder.title.clone(),
                ScenarioKind::Folder,
        ))]
    })
    .test_value();
}

/// `C4MapFolderData::Load` answers `ImageDump=1` by blitting the scenario's
/// `Area` out of the FolderMap background and writing it to `BaseImage` as
/// a PNG, then skipping the ordinary base load
/// (src/C4StartupScenSelDlg.cpp:145-161). `CreateGUIElements` picks the
/// title font with `GetFontByHeight`'s ascending line-height ladder and its
/// `iHgt / lineHeight` zoom, snapped to 1.0 inside [0.8, 1.25)
/// (src/C4StartupScenSelDlg.cpp:374-381; src/C4Gui.cpp:1235-1253;
/// src/C4Startup.cpp:125-143).
#[test]
fn folder_map_image_dump_and_dynamic_title_font_match_cpp() {
    let root = tempdir();
    let map_path = root.path().join("Dump.c4f");
    let scenario_path = map_path.join("Dumped.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(map_path.join("Folder.txt"), "[Head]\nTitle=Dump Map\n").test_value();
    fs::write(
        scenario_path.join("Scenario.txt"),
        "[Head]\nTitle=Dumped\nMinPlayer=1\nMaxPlayer=4\n",
    )
    .test_value();
    // A background whose crop window is distinguishable from the whole.
    let mut background = image::RgbaImage::from_pixel(16, 16, image::Rgba([10, 20, 30, 255]));
    for y in 4..8 {
        for x in 2..6 {
            background.put_pixel(x, y, image::Rgba([200, 100, 50, 255]));
        }
    }
    background.save(map_path.join("FolderMap.png")).test_value();
    fs::write(map_path.join("FolderMap.txt"), "[FolderMap]\n    [Scenario]\n    File=Dumped.c4s\n    Area=2,4,4,4\n    BaseImage=Dumped.png\n    ImageDump=1\n").test_value();

    let folder = FrontendScenario::from_resource(
        resource_scenario::discover(root.path())
            .expect("discover map folder")
            .into_iter()
            .find(|entry| entry.path == map_path)
            .test_value(),
        "Test scenarios",
    );
    let map = load_map_folder_data(
        &folder,
        640,
        480,
        &MissionAccessStore::default(),
        &["US".to_string()],
    )
    .test_value();

    // The dump is written next to the FolderMap with the exact crop.
    let dumped = image::open(map_path.join("Dumped.png"))
        .test_value()
        .into_rgba8();
    main_assert_eq!(dumped.dimensions() => (4, 4));
    main_assert!(dumped.pixels().all(|pixel| pixel.0 == [200, 100, 50, 255]));
    // `continue` skips the ordinary base load, so no base image is retained.
    main_assert!(map.scenarios[0].base_image.is_none());

    // GetFontByHeight ladders on line height, not on 15/20 thresholds.
    let app = new_real_classic_menu_app(320, 200);
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let pick = |height: i32| {
        let (font, zoom) = gui_font_by_height(fonts, height);
        (font.line_height, zoom)
    };
    main_assert_eq!(pick(fonts.mini.line_height) => (fonts.mini.line_height, 1.0));
    main_assert_eq!(pick(fonts.text.line_height) => (fonts.text.line_height, 1.0));
    main_assert_eq!(pick(fonts.caption.line_height) => (fonts.caption.line_height, 1.0));
    // Above every tier the title font takes over and keeps a real zoom.
    let huge = fonts.title.line_height * 3;
    let (line_height, zoom) = pick(huge);
    main_assert_eq!(line_height => fonts.title.line_height);
    main_assert!((zoom - huge as f32 / fonts.title.line_height as f32).abs() < 1e-4);
    // Inside the tolerance band the zoom snaps back to 1.0.
    let tolerated = (fonts.caption.line_height as f32 * 0.9).round() as i32;
    main_assert_eq!(pick(tolerated).1 => 1.0);
}

#[test]
fn folder_map_f5_refresh_preserves_map_and_book_only_shortcuts() {
    let _lock = env_lock().lock();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    let map_path = paths.scenario_dir().join("Map.c4f");
    let alpha_path = map_path.join("Alpha.c4s");
    fs::create_dir_all(&alpha_path).test_value();
    fs::write(map_path.join("Folder.txt"), "[Head]\nIndex=1\n").test_value();
    fs::write(
        alpha_path.join("Scenario.txt"),
        "[Head]\nTitle=Alpha Mission\n",
    )
    .test_value();
    write_map_png(&map_path.join("FolderMap.png"), 16, 8, [20, 30, 40, 255]);
    fs::write(map_path.join("FolderMap.txt"), "[FolderMap]\n    [Scenario]\n    File=Alpha.c4s\n    Area=0,0,8,8\n    [Scenario]\n    File=Beta.c4s\n    Area=8,0,8,8\n").test_value();

    let mut app = new_menu_app_with_paths(800, 600, &paths);
    app.open_scenario_browser();
    app.enter_scenario_folder("Map.c4f");
    main_assert!(app.menu_state.current_map().is_some());

    let select_alpha = app.menu_state.activate_map_button(0).test_value();
    app.handle_menu_input(move |_| vec![select_alpha])
        .test_value();
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some("Map.c4f/Alpha.c4s"));

    app.test_key(VirtualKeyCode::F2, ElementState::Pressed);
    main_assert!(app.menu_state.rename_edit.is_none());
    app.test_key(VirtualKeyCode::Delete, ElementState::Pressed);
    main_assert!(app.dialogs.messages.is_empty());
    app.test_modifiers(ModifiersState::ALT);
    app.test_key(VirtualKeyCode::KeyM, ElementState::Pressed);
    main_assert!(app.game_option_input_dialog.is_none());
    app.test_modifiers(ModifiersState::empty());

    let beta_path = map_path.join("Beta.c4s");
    fs::create_dir_all(&beta_path).test_value();
    fs::write(
        beta_path.join("Scenario.txt"),
        "[Head]\nTitle=Beta Mission\n",
    )
    .test_value();
    app.test_key(VirtualKeyCode::F5, ElementState::Pressed);
    wait_for_scenario_selector_discovery(&mut app);

    main_assert_eq!(app.menu_state.current_folder().map(|folder| folder.identifier.as_str()) => Some("Map.c4f"));
    let map = app.menu_state.current_map().test_value();
    main_assert!(map.selected_entry().is_none(), "F5 rebuild clears the visible map selection");
    main_assert!(map.scenarios.iter().any(|button| {button.entry.as_ref().is_some_and(|entry| entry.identifier == "Map.c4f/Beta.c4s")}));
    main_assert!(app.scensel.catalog.contains_key("Map.c4f/Beta.c4s"));
    main_assert!(
        app.scensel.entry_enabled.is_empty(),
        "rediscovering a map sheet must not rebuild the book CanOpen cache"
    );
    reset_cached_app_paths();
}

#[test]
fn folder_map_disabled_opens_a_normal_book_without_inspecting_the_marker() {
    let root = tempdir();
    let map_path = root.path().join("Map.c4f");
    fs::create_dir(&map_path).test_value();
    fs::write(
        map_path.join("FolderMap.txt"),
        "this is deliberately malformed and has no background",
    )
    .test_value();
    let alpha = map_test_scenario(&map_path, "Alpha.c4s", "Alpha");
    let folder = map_test_folder(&map_path, vec![alpha.clone()]);
    let mut app = new_menu_app(640, 480);
    app.config.show_folder_maps = false;

    open_map_test_folder(&mut app, folder.clone());

    main_assert_eq!(app.menu_state.stack.len() => 2);
    main_assert!(app.menu_state.current_map().is_none());
    main_assert_eq!(app.menu_state.visible_entries().len() => 1);
    main_assert_eq!(app.menu_state.selected_scenario().map(|entry| entry.identifier.as_str()) => Some(alpha.identifier.as_str()));
    main_assert_eq!(app.mode => AppMode::Menu);
    main_assert_eq!(app.startup.view => StartupView::ScenarioBrowser);

    let mut invalid_map_app = new_menu_app(640, 480);
    open_map_test_folder(&mut invalid_map_app, folder);
    main_assert!(invalid_map_app.menu_state.current_map().is_none());
    main_assert_eq!(invalid_map_app.menu_state.visible_entries().len() => 1);
    main_assert_eq!(invalid_map_app.mode => AppMode::Menu);
}

#[test]
fn folder_map_parser_honors_indentation_dedents_and_first_scalar() {
    let parsed = parse_map_folder(
            "[FolderMap]\nMinResX=640\nMinResX=999\n    [Other]\n        [Scenario]\n        File=Nested.c4s\n    [Scenario]\n    File=Direct.c4s\nMinResY=480\n",
        ).test_value();
    main_assert_eq!(parsed.min_res_x => 640);
    main_assert_eq!(parsed.min_res_y => 480);
    main_assert_eq!(parsed.scenarios.len() => 1);
    main_assert_eq!(parsed.scenarios[0].filename => "Direct.c4s");
}

#[test]
fn folder_map_loads_renders_titles_access_overlays_and_cpp_click_semantics() {
    let root = tempdir();
    let map_path = root.path().join("Map.c4f");
    fs::create_dir(&map_path).test_value();
    write_map_png(&map_path.join("FolderMap.png"), 100, 100, [20, 30, 40, 255]);
    for (name, pixel) in [
        ("AlphaBase.png", [80, 0, 0, 255]),
        ("AlphaOver.png", [120, 0, 0, 255]),
        ("BetaBase.png", [0, 80, 0, 255]),
        ("BetaOver.png", [0, 120, 0, 255]),
        ("Always.png", [0, 0, 80, 255]),
        ("Granted.png", [0, 0, 120, 255]),
        ("Denied.png", [120, 120, 0, 255]),
    ] {
        write_map_png(&map_path.join(name), 1, 1, pixel);
    }
    fs::write(map_path.join("StringTblUS.txt"), "PLAY=Play\n").test_value();
    fs::write(
        map_path.join("fOlDeRmAp.TxT"),
        r#"[FolderMap]
    ScenInfoArea=70,5,25,90
        [AccessGfx]
        Access=
        OverlayImage=Always.png
        Area=20,20,5,5
        [AccessGfx]
        Access=MapPass
        OverlayImage=Granted.png
        Area=30,20,5,5
        [AccessGfx]
        Access=MissingPass
        OverlayImage=Denied.png
        Area=40,20,5,5
        [Scenario]
        File=Alpha.c4s
        BaseImage=AlphaBase
        OverlayImage=AlphaOver.png
        Area=5,5,10,10
        Title=$PLAY$ TITLE
        [Scenario]
        File=Beta.c4s
        BaseImage=BetaBase.png
        OverlayImage=BetaOver.png
        SingleClick=1
        Area=20,5,10,10
        Title=Visit TITLE now
        [Scenario]
        File=Gamma.c4s
        BaseImage=AlphaBase
        OverlayImage=AlphaOver.png
        Area=35,5,10,10
        Title=TITLE
    "#,
    )
    .test_value();
    let alpha = map_test_scenario(&map_path, "Alpha.c4s", "Alpha Mission");
    let beta = map_test_scenario(&map_path, "Beta.c4s", "Beta Mission");
    let folder = map_test_folder(&map_path, vec![alpha.clone(), beta.clone()]);
    let mut app = new_real_menu_app(640, 480);
    app.config.mission_access = MissionAccessStore::new("Other; mappass ");

    open_map_test_folder(&mut app, folder);

    let map = app.menu_state.current_map().test_value();
    main_assert_eq!(map.source_path => map_path);
    main_assert_eq!(map.scenarios.len() => 3);
    main_assert_eq!(map.scenarios[0].title => "Play Alpha Mission");
    main_assert_eq!(map.scenarios[1].title => "Visit Beta Mission now");
    main_assert!(map.scenarios[2].entry.is_none());
    main_assert_eq!(map.scenarios[2].title => "<c ff0000>ERROR</c>");
    main_assert_eq!(map.access_overlays.len() => 2);
    main_assert_eq!(map.access_overlays[0].image.as_ref().expect("unconditional access image").pixels()[..4] => [0, 0, 80, 255]);
    main_assert_eq!(map.access_overlays[1].image.as_ref().expect("granted access image").pixels()[..4] => [0, 0, 120, 255]);

    let first = app.menu_state.activate_map_button(0).test_value();
    main_assert!(matches!(&first, StartupMenuAction::SelectionChanged(summary) if summary.identifier == alpha.identifier));
    app.process_menu_actions(vec![first]).test_value();
    main_assert_eq!(scensel_selection_info(&app.menu_state).title => Some("Alpha Mission"));
    let second = app.menu_state.activate_map_button(0).test_value();
    let (start, _) = app.process_menu_actions(vec![second]).test_value();
    main_assert_eq!(start.as_deref() => Some(alpha.identifier.as_str()));

    let layout = clonk_frontend::startup_scensel::scen_sel_layout(
        640,
        480,
        app.assets.clonk_fonts.as_deref().test_value(),
    );
    let transform =
        MapFolderTransform::for_map(app.menu_state.current_map().test_value(), &layout, 640, 480);
    let (background_x, background_y) = transform.point(50, 50);
    main_assert!(app.handle_scensel_map_pointer_down(GuiPoint::new(background_x as f32, background_y as f32,)));
    main_assert!(app.menu_state.current_map().expect("map remains active").selected_entry().is_none());
    let single = app.menu_state.activate_map_button(1).test_value();
    main_assert!(matches!(single, StartupMenuAction::StartScenario(summary) if summary.identifier == beta.identifier));

    let (sample_x, sample_y) = transform.point(50, 50);
    let mut frame = vec![0_u8; 640 * 480 * 4];
    app.test_render(&mut frame);
    let pixel = app
        .graphics
        .surface()
        .get_pixel(sample_x as u32, sample_y as u32)
        .test_value();
    main_assert_eq!([pixel.r, pixel.g, pixel.b] => [20, 30, 40]);
}

// C4MapFolderData::CreateGUIElements only filters mission access when it
// creates map buttons (C4StartupScenSelDlg.cpp:344-357). The map sheet does
// not render ScenListItem rows, so its children must not populate the book
// CanOpen cache; activation still performs the exact final check.
#[test]
fn folder_map_does_not_build_book_openability_cache() {
    let root = tempdir();
    let map_path = root.path().join("Map.c4f");
    fs::create_dir(&map_path).test_value();
    write_map_png(&map_path.join("FolderMap.png"), 8, 8, [20, 30, 40, 255]);
    fs::write(
        map_path.join("FolderMap.txt"),
        "[FolderMap]\n    [Scenario]\n    File=Visible.c4s\n    Area=0,0,8,8\n",
    )
    .test_value();

    let visible = map_test_scenario(&map_path, "Visible.c4s", "Visible");
    let hidden = map_test_scenario(&map_path, "Hidden.c4s", "Hidden");
    let folder = map_test_folder(&map_path, vec![visible, hidden]);
    let mut app = new_menu_app(640, 480);

    open_map_test_folder(&mut app, folder);

    main_assert!(app.menu_state.current_map().is_some());
    main_assert!(
        app.scensel.entry_enabled.is_empty(),
        "map sheets do not render book rows or use their CanOpen cache"
    );
}

#[test]
fn folder_map_hides_locked_button_until_access_is_granted() {
    let _lock = env_lock().lock();
    reset_cached_app_paths();
    let user_data = tempdir();
    let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
    persist_config_value(&paths, "General", "MissionAccess", "OtherPass").test_value();
    let map_path = paths.scenario_dir().join("Map.c4f");
    fs::create_dir_all(&map_path).test_value();
    write_map_png(&map_path.join("FolderMap.png"), 8, 8, [20, 30, 40, 255]);
    fs::write(
        map_path.join("FolderMap.txt"),
        "[FolderMap]\n    [Scenario]\n    File=Locked.c4s\n    SingleClick=1\n    Area=0,0,8,8\n",
    )
    .test_value();
    let locked_path = map_path.join("Locked.c4s");
    fs::create_dir(&locked_path).test_value();
    fs::write(
        locked_path.join("Scenario.txt"),
        "[Head]\nTitle=Locked Mission\nMissionAccess=MissingPass\n",
    )
    .test_value();
    let mut app = new_menu_app_with_paths(640, 480, &paths);
    app.open_scenario_browser();
    app.enter_scenario_folder("Map.c4f");

    main_assert_eq!(app.menu_state.current_map().expect("map view active").scenarios.len() => 0, "a denied real scenario has no map button or base image");

    app.apply_scenario_mission_access("MissingPass")
        .test_value();
    wait_for_scenario_selector_discovery(&mut app);
    main_assert_eq!(app.config.mission_access.snapshot() => "OtherPass;MissingPass");
    app.enter_scenario_folder("Map.c4f");
    main_assert_eq!(app.menu_state.current_map().expect("map view restored").scenarios.len() => 1, "granting the module creates the map button on rebuild");
    reset_cached_app_paths();
}

#[test]
fn folder_map_minimum_resolution_and_image_failures_fall_back_to_book() {
    let root = tempdir();
    let map_path = root.path().join("Map.c4f");
    fs::create_dir(&map_path).test_value();
    write_map_png(&map_path.join("FolderMap.png"), 4, 4, [10, 20, 30, 255]);
    let mut child = map_test_scenario(&map_path, "Alpha.c4s", "Alpha");
    child.mission_access = Some("Never".to_string());
    let folder = map_test_folder(&map_path, vec![child]);
    let access = MissionAccessStore::default();

    for (min_x, min_y, loads) in [(641, 0, false), (0, 481, false), (640, 480, true)] {
        fs::write(
            map_path.join("FolderMap.txt"),
            format!("[FolderMap]\nMinResX={min_x}\nMinResY={min_y}\n"),
        )
        .test_value();
        main_assert_eq!(load_map_folder_data(&folder, 640, 480, &access, &["US".to_string()],).is_some() => loads, "minimum {min_x}x{min_y}");
    }

    fs::write(
        map_path.join("FolderMap.txt"),
        "[FolderMap]\n    [AccessGfx]\n    Access=Never\n    OverlayImage=Missing.png\n",
    )
    .test_value();
    main_assert!(
        load_map_folder_data(&folder, 640, 480, &access, &["US".to_string()],).is_none(),
        "even an inaccessible named image is loaded before access filtering"
    );

    fs::write(
        map_path.join("FolderMap.txt"),
        "[FolderMap]\n    [Scenario]\n    File=Alpha.c4s\n    BaseImage=Missing.png\n",
    )
    .test_value();
    main_assert!(
        load_map_folder_data(&folder, 640, 480, &access, &["US".to_string()],).is_none(),
        "a denied scenario image is still loaded before the button is filtered"
    );
}

#[test]
fn extensionless_regular_folder_ignores_folder_map_marker() {
    let root = tempdir();
    let distant_subfolder = root.path().join("Distant.c4f");
    let regular_path = distant_subfolder.join("Regular");
    let scenario_path = regular_path.join("Child.c4s");
    fs::create_dir_all(&scenario_path).test_value();
    fs::write(distant_subfolder.join("FolderMap.txt"), "[FolderMap]\n").test_value();
    fs::write(regular_path.join("FolderMap.txt"), "[FolderMap]\n").test_value();

    let mut child = FrontendScenario::fallback();
    child.identifier = "Regular/Child.c4s".to_string();
    child.path = Some(scenario_path);
    let mut regular = FrontendScenario::fallback();
    regular.identifier = "Regular".to_string();
    regular.kind = ScenarioKind::Folder;
    regular.is_playable = false;
    regular.path = Some(regular_path);
    regular.children = vec![child.clone()];
    let entries = vec![regular];
    let menu =
        StartupMenu::new(build_menu_entries(&entries, false), test_font(), None).test_value();
    let mut state = MenuState::new(menu, entries);
    state.enter_folder("Regular");

    main_assert!(!state.configure_current_folder_map(true, 640, 480, &MissionAccessStore::default(), &["US".to_string()],));
    main_assert!(state.current_map().is_none());
    main_assert_eq!(state.current_entries()[0].identifier => child.identifier);
}

#[test]
fn merged_folder_map_uses_a_later_contributing_group() {
    let root = tempdir();
    let first = root.path().join("first/Worlds.c4f");
    let later = root.path().join("later/Worlds.c4f");
    fs::create_dir_all(&first).test_value();
    fs::create_dir_all(&later).test_value();
    fs::write(later.join("FolderMap.txt"), "[FolderMap]\n").test_value();
    write_map_png(&later.join("FolderMap.png"), 2, 2, [1, 2, 3, 255]);
    let mut folder = map_test_folder(&first, Vec::new());
    folder.identifier = "Worlds.c4f".to_string();
    folder.source_paths = vec![first, later.clone()];

    let map = load_map_folder_data(
        &folder,
        640,
        480,
        &MissionAccessStore::default(),
        &["US".to_string()],
    )
    .test_value();
    main_assert_eq!(map.source_path => later);
}

#[test]
fn merged_scenario_keeps_first_root_mission_access_requirement() {
    let mut unlocked = FrontendScenario::fallback();
    unlocked.identifier = "Duplicate.c4s".to_string();
    let mut locked = unlocked.clone();
    locked.mission_access = Some("Secret".to_string());

    let user_first = merge_frontend_scenarios(vec![unlocked.clone(), locked.clone()], false);
    main_assert_eq!(user_first[0].mission_access => None);

    let install_first = merge_frontend_scenarios(vec![locked, unlocked], false);
    main_assert_eq!(install_first[0].mission_access.as_deref() => Some("Secret"));
}

#[test]
fn folder_group_rewrite_restores_original_when_commit_fails() {
    let directory = tempdir();
    let destination = directory.path().join("Local.c4p");
    fs::create_dir(&destination).test_value();
    fs::write(destination.join("Player.txt"), b"old player").test_value();
    fs::write(destination.join("Keep.dat"), b"old sentinel").test_value();
    fs::write(destination.join(".metadata"), b"hidden sentinel").test_value();

    let mut replacement = MutableGroup::new("Local.c4p");
    replacement
        .add_file("Player.txt", b"new player".to_vec())
        .test_value();
    let source =
        Group::from_memory(destination.clone(), replacement.pack().test_value()).test_value();
    let error = replace_directory_from_same_parent_with_hook(&source, &destination, || {
        Err(io::Error::other("synthetic commit failure"))
    })
    .expect_err("commit failure must roll the original folder group back");
    main_assert!(format!("{error:#}").contains("synthetic commit failure"));
    main_assert_eq!(fs::read(destination.join("Player.txt")).unwrap() => b"old player");
    main_assert_eq!(fs::read(destination.join("Keep.dat")).unwrap() => b"old sentinel");
    main_assert_eq!(fs::read(destination.join(".metadata")).unwrap() => b"hidden sentinel");
    main_assert!(fs::read_dir(directory.path())
        .expect("enumerate folder-group root")
        .all(|entry| !entry
            .expect("folder-group root entry")
            .file_name()
            .to_string_lossy()
            .starts_with(".Local.c4p.lc-rewrite-")));
}

#[test]
fn packed_group_rewrite_safely_replaces_an_existing_directory() {
    let directory = tempdir();
    let destination = directory.path().join("Copy.c4s");
    fs::create_dir(&destination).test_value();
    fs::write(destination.join("Old.txt"), b"old").test_value();
    let mut replacement = MutableGroup::new("Copy.c4s");
    replacement
        .add_file("Scenario.txt", b"new".to_vec())
        .test_value();

    persist_console_save_group(&replacement, &destination, false).test_value();
    main_assert!(destination.is_file());
    main_assert_eq!(Group::open(&destination).expect("open packed replacement").read_file("Scenario.txt").unwrap() => b"new");
}

#[cfg(unix)]
#[test]
fn folder_group_rewrite_never_follows_nested_directory_symlinks() {
    let directory = tempdir();
    let destination = directory.path().join("Local.c4p");
    let external = directory.path().join("External.c4i");
    fs::create_dir(&destination).test_value();
    fs::create_dir(&external).test_value();
    fs::write(external.join("Outside.dat"), b"outside").test_value();
    std::os::unix::fs::symlink(&external, destination.join("Hero.c4i")).test_value();

    let mut replacement = MutableGroup::new("Local.c4p");
    let mut hero = MutableGroup::new("Hero.c4i");
    hero.add_file("ObjectInfo.txt", b"new crew".to_vec())
        .test_value();
    replacement.add_child("Hero.c4i", hero).test_value();
    persist_console_save_group(&replacement, &destination, true).test_value();

    main_assert_eq!(fs::read(external.join("Outside.dat")).unwrap() => b"outside");
    main_assert!(!external.join("ObjectInfo.txt").exists());
    let saved_child = destination.join("Hero.c4i");
    main_assert!(fs::symlink_metadata(&saved_child).expect("saved child metadata").file_type().is_dir());
    main_assert_eq!(fs::read(saved_child.join("ObjectInfo.txt")).unwrap() => b"new crew");
}

#[cfg(unix)]
#[test]
fn folder_group_rewrite_preserves_a_root_directory_symlink() {
    let directory = tempdir();
    let physical = directory.path().join("Physical.c4p");
    let linked = directory.path().join("Linked.c4p");
    fs::create_dir(&physical).test_value();
    fs::write(physical.join("Player.txt"), b"old").test_value();
    std::os::unix::fs::symlink(&physical, &linked).test_value();
    let mut replacement = MutableGroup::new("Linked.c4p");
    replacement
        .add_file("Player.txt", b"new".to_vec())
        .test_value();

    persist_console_save_group(&replacement, &linked, true).test_value();
    main_assert!(fs::symlink_metadata(&linked).expect("linked profile metadata").file_type().is_symlink());
    main_assert_eq!(fs::read(physical.join("Player.txt")).unwrap() => b"new");
}

#[test]
fn cache_definition_icons_distinguishes_blank_from_malformed_picture() {
    let mut app = new_running_sandbox_app();
    let mut blank = test_definition("BLNK", "Blank rule", "#strict 3\n");
    blank.set_category(C4D_RULE);
    app.engine.register_test_definition(blank);
    let blank_entry = scensel_fixture!(goal_rule: "BLNK".to_string(), "Blank rule".to_string());
    app.cache_definition_icons(std::slice::from_ref(&blank_entry))
        .test_value();
    main_assert!(!app.ingame_menu_gfx.as_ref().expect("blank cache initializes menu graphics").definition_icons.contains_key("BLNK"));

    let temp = tempdir();
    let valid_dir = temp.path().join("Valid.c4d");
    fs::create_dir(&valid_dir).test_value();
    fs::write(
        valid_dir.join("DefCore.txt"),
        b"[DefCore]\nid=PCTR\nPicture=0,0,1,1\n",
    )
    .test_value();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
        .save(valid_dir.join("Graphics.png"))
        .test_value();
    let valid_resource =
        ResourceDefinitionData::load(&Group::open(&valid_dir).test_value()).test_value();
    let mut valid = Definition::from_resource(&valid_resource).test_value();
    valid.set_category(C4D_RULE);
    app.engine.register_test_definition(valid);
    main_assert!(app.engine.try_definition_picture_image("PCTR").expect("valid definition resolves").is_some());
    let valid_entry = scensel_fixture!(goal_rule: "PCTR".to_string(), "Pictured rule".to_string());

    let malformed_dir = temp.path().join("Malformed.c4d");
    fs::create_dir(&malformed_dir).test_value();
    fs::write(
        malformed_dir.join("DefCore.txt"),
        b"[DefCore]\nid=BADG\nWidth=1\nHeight=1\n",
    )
    .test_value();
    fs::write(malformed_dir.join("Graphics.png"), b"not a png").test_value();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
        .save(malformed_dir.join("Graphics.bmp"))
        .test_value();
    let malformed_group = Group::open(&malformed_dir).test_value();
    main_assert!(matches!(
        ResourceDefinitionData::load(&malformed_group),
        Err(ResourceDefinitionError::Graphics { path, reason })
            if path == Path::new("Graphics.png") && !reason.is_empty()
    ));

    let malformed_entry =
        scensel_fixture!(goal_rule: "BADG".to_string(), "Malformed rule".to_string());
    let error = app
        .cache_definition_icons(&[valid_entry, blank_entry, malformed_entry])
        .expect_err("a rejected graphics definition must not become a blank menu symbol");
    let EngineError::ClassicMenuParityBoundary { detail } = error else {
        panic!("unexpected malformed definition error: {error:?}");
    };
    main_assert_eq!(detail => "classic in-game goal/rule symbol definition `BADG` is unavailable: unknown definition `BADG`; refusing a blank symbol substitute");
    main_assert!(
        app.ingame_menu_gfx
            .as_ref()
            .expect("menu graphics remain allocated")
            .definition_icons
            .is_empty(),
        "validation must complete before mutating the icon cache"
    );
}

#[test]
fn definition_pack_graphics_sits_below_scenario_folders_and_extra_but_above_base() {
    let _env_lock = crate::tests::env_lock().lock();
    let root = tempdir();
    let (_guard, paths, content) = loader_origin_fixture_paths(root.path());
    let family = content.join("Family.c4f");
    let scenario = family.join("Scenario.c4s");
    let scenario_graphics = scenario.join("Graphics.c4g");
    let folder_graphics = family.join("Graphics.c4g");
    let extra_graphics = content.join("Extra.c4g/Graphics.c4g");
    let first_pack = root.path().join("First.c4d");
    let second_pack = root.path().join("Second.c4d");
    let first_graphics = first_pack.join("Graphics.c4g");
    let second_graphics = second_pack.join("Graphics.c4g");
    let base_graphics = root.path().join("planet/Graphics.c4g");
    for graphics in [
        &scenario_graphics,
        &folder_graphics,
        &extra_graphics,
        &first_graphics,
        &second_graphics,
        &base_graphics,
    ] {
        fs::create_dir_all(graphics).test_value();
    }
    for (graphics, entries) in [
        (&scenario_graphics, &["ScenarioWins.png"][..]),
        (
            &folder_graphics,
            &["ScenarioWins.png", "FolderWins.png"][..],
        ),
        (
            &extra_graphics,
            &["ScenarioWins.png", "FolderWins.png", "ExtraWins.png"][..],
        ),
        (
            &first_graphics,
            &[
                "ScenarioWins.png",
                "FolderWins.png",
                "ExtraWins.png",
                "PackWins.png",
                "PackTie.png",
            ][..],
        ),
        (&second_graphics, &["PackTie.png"][..]),
        (
            &base_graphics,
            &[
                "ScenarioWins.png",
                "FolderWins.png",
                "ExtraWins.png",
                "PackWins.png",
                "PackTie.png",
            ][..],
        ),
    ] {
        for entry in entries {
            fs::write(graphics.join(entry), graphics.to_string_lossy().as_bytes()).test_value();
        }
    }

    let scenario_group = Group::open(&scenario).test_value();
    let definition_roots = [
        Group::open(&first_pack).test_value(),
        Group::open(&second_pack).test_value(),
    ];
    let graphics = InstallDefinitionResolver::new(Some(Arc::new(paths)))
        .resolve_graphics_groups_with_definition_roots(&scenario_group, &definition_roots)
        .test_value();

    main_assert_eq!(
        graphics
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect::<Vec<_>>() =>
        [
            scenario_graphics.clone(),
            folder_graphics.clone(),
            extra_graphics.clone(),
            first_graphics.clone(),
            second_graphics,
            base_graphics,
        ]
    );
    let winner = |name: &str| {
        graphics
            .iter()
            .find(|group| group.read_file(name).is_ok())
            .map(|group| group.root().to_path_buf())
            .test_value()
    };
    main_assert_eq!(winner("ScenarioWins.png") => scenario_graphics);
    main_assert_eq!(winner("FolderWins.png") => folder_graphics);
    main_assert_eq!(winner("ExtraWins.png") => extra_graphics);
    main_assert_eq!(winner("PackWins.png") => first_graphics);
    main_assert_eq!(winner("PackTie.png") => first_pack.join("Graphics.c4g"), "RegisterMainGroups' second reversal makes the first selected definition pack win");
}

// clonk-org/clonk-rs#392: the platform bridge is handed a description only
// while the scenario selector is the screen on show. Elsewhere the window has
// nothing to announce, and a reader must not keep finding a search field that
// is no longer drawn.
#[test]
fn scensel_accessibility_describes_the_search_field_only_while_the_selector_shows() {
    let mut app = new_menu_app(800, 600);
    main_assert!(app.scen_sel_accessibility().nodes.is_empty());

    app.open_scenario_browser();
    app.menu_state.set_search_text("Crystal");
    app.menu_state.set_search_focused(true);

    let semantics = app.scen_sel_accessibility();
    let field = semantics
        .node(clonk_frontend::accessibility::Role::TextInput)
        .test_value();
    main_assert_eq!(field.name.as_str() => "Scenario search");
    main_assert_eq!(field.value.as_deref() => Some("Crystal"));
    main_assert!(field.focused);

    app.close_scenario_browser();
    main_assert!(app.scen_sel_accessibility().nodes.is_empty());
}

// The count and the no-result guidance are drawn together and are announced
// together (clonk-org/clonk-rs#392); the bridge reads them from the same
// enhanced-search presentation the screen itself draws.
#[test]
fn scensel_accessibility_announces_the_enhanced_search_result_status() {
    let mut app = new_menu_app(800, 600);
    app.open_scenario_browser();
    app.menu_state.set_search_text("zzzznomatch");
    app.menu_state.apply_enhanced_search();

    let caption = app.menu_state.enhanced_search_caption().test_value();
    let guidance = app.menu_state.enhanced_search_empty_message().test_value();
    let semantics = app.scen_sel_accessibility();
    let status = semantics
        .node(clonk_frontend::accessibility::Role::Status)
        .test_value();
    main_assert_eq!(status.value.clone() => Some(format!("{caption} {guidance}")));
}


// `C4StartupScenSelDlg` hands `fFontZoom` to `SetTextFont`
// (src/C4StartupScenSelDlg.cpp:371-377), so a folder-map caption whose scaled
// height falls outside `GetFontByHeight`'s tolerance band is rasterized at the
// resolved zoom rather than at the font's native size
// (clonk-org/clonk-rs#1174).
#[test]
fn folder_map_titles_rasterize_at_the_resolved_font_zoom() {
    let app = new_real_classic_menu_app(320, 200);
    let fonts = app.assets.clonk_fonts.as_deref().test_value();
    let book_fonts = app.assets.book_fonts.as_deref().test_value();
    let gamma = clonk_graphics::GammaRamp::identity();

    // `title` painted height for one requested size, measured on its own
    // surface so nothing else can contribute a pixel.
    let painted_height = |size: i32, use_book_font: bool| {
        let button = MapFolderScenarioButton::title_probe_for_test("III", size, use_book_font);
        let mut surface = Surface::new(256, 256, PixelFormat::Rgba8888);
        draw_map_scenario_title(
            &mut surface,
            fonts,
            book_fonts,
            &button,
            &MapFolderTransform::identity_for_test(),
            false,
            &gamma,
        );
        let rows = (0..256)
            .filter(|y| {
                (0..256).any(|x| {
                    surface
                        .get_pixel(x, *y as u32)
                        .is_some_and(|pixel| pixel.a > 0)
                })
            })
            .collect::<Vec<_>>();
        rows.last().copied().unwrap_or(0) - rows.first().copied().unwrap_or(0) + 1
    };

    for (label, native) in [
        ("gui", fonts.title.line_height),
        ("book", book_fonts.title.line_height),
    ] {
        let use_book_font = label == "book";
        let inside = painted_height(native, use_book_font);
        let outside = painted_height(native * 3, use_book_font);
        main_assert!(
            outside > inside * 2,
            "{label} titles outside the tolerance band draw at the zoom: {inside} -> {outside}"
        );
    }

    // Inside `[0.8, 1.25)` the zoom snaps to 1.0, so nothing changes there.
    let native = fonts.caption.line_height;
    let tolerated = (native as f32 * 0.9).round() as i32;
    main_assert_eq!(painted_height(tolerated, false) => painted_height(native, false));
}

// The enhanced scenario search is a port divergence, so its wording is
// port-owned `IDS_` text rather than an oracle string — but it is still read
// from the frozen active language table, and English-only was never part of
// the accepted divergence (clonk-org/clonk-rs#1175).
#[test]
fn scensel_enhanced_search_presentation_follows_the_active_language() {
    let german = HashMap::from([
        (
            "IDS_MSG_SEARCHRESULTS".to_string(),
            "%d von %d %s".to_string(),
        ),
        (
            "IDS_MSG_SEARCHNOMATCHES".to_string(),
            "Keine Treffer unter %d %s".to_string(),
        ),
        (
            "IDS_MSG_SEARCHNORESULT".to_string(),
            "Keine Szenarien passen zu \"%s\".".to_string(),
        ),
        (
            "IDS_MSG_SEARCHCLEARHINT".to_string(),
            "Esc drücken, um die Suche zu löschen.".to_string(),
        ),
        (
            "IDS_MSG_SEARCHSCENARIO".to_string(),
            "Szenario".to_string(),
        ),
        (
            "IDS_MSG_SEARCHSCENARIOS".to_string(),
            "Szenarien".to_string(),
        ),
    ]);

    let build = |titles: &[&str], term: &str, resources: Option<&HashMap<String, String>>| {
        let scenarios = titles
            .iter()
            .enumerate()
            .map(|(index, title)| {
                scensel_fixture!(frontend_scenario: entry, format!("s{index}"), (*title).to_string());
                entry
            })
            .collect::<Vec<_>>();
        let menu =
            StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None).test_value();
        let mut state = MenuState::new(menu, scenarios);
        state.set_include_back(false);
        if let Some(resources) = resources {
            state.set_enhanced_search_resources(enhanced_search_resources(resources));
        }
        state.set_search_text(term);
        state.apply_enhanced_search();
        state
    };

    // Zero, one and several matches, each with its own noun.
    let none = build(&["Crystal Cavern", "Gold Rush"], "zzzz", Some(&german));
    main_assert_eq!(none.enhanced_search_caption().as_deref() => Some("Keine Treffer unter 2 Szenarien"));
    main_assert_eq!(none.enhanced_search_empty_message().as_deref() => Some("Keine Szenarien passen zu \"zzzz\"."));

    let one = build(&["Crystal Cavern"], "Crystal", Some(&german));
    main_assert_eq!(one.enhanced_search_caption().as_deref() => Some("1 von 1 Szenario"));
    main_assert!(one.enhanced_search_empty_message().is_none());

    let several = build(
        &["Crystal Cavern", "Crystal Lake", "Gold Rush"],
        "Crystal",
        Some(&german),
    );
    main_assert_eq!(several.enhanced_search_caption().as_deref() => Some("2 von 3 Szenarien"));
    main_assert_eq!(several.enhanced_search_clear_hint() => "Esc drücken, um die Suche zu löschen.");

    // A table without the keys keeps the shipped English.
    let english = build(&["Crystal Cavern", "Gold Rush"], "Crystal", None);
    main_assert_eq!(english.enhanced_search_caption().as_deref() => Some("1 of 2 scenarios"));
    main_assert_eq!(english.enhanced_search_clear_hint() => "Press Esc to clear search.");
}

// Translated captions have to reach the pixels, not just the model: the
// caption is drawn through the book caption and the guidance through the
// list area, both of which read the same strings the model resolved.
#[test]
fn scensel_enhanced_search_translation_reaches_the_rendered_frame() {
    let mut app = new_real_classic_menu_app(800, 600);
    app.open_scenario_browser();
    app.menu_state
        .set_enhanced_search_resources(enhanced_search_resources(&HashMap::from([(
            "IDS_MSG_SEARCHCLEARHINT".to_string(),
            "Esc drücken, um die Suche zu löschen.".to_string(),
        )])));
    app.menu_state.set_search_text("zzzznomatch");
    app.menu_state.apply_enhanced_search();

    let mut frame = vec![0_u8; 800 * 600 * 4];
    main_assert!(app.render(&mut frame).expect("render the search result"));
    let hinted = app.graphics.surface().pixels().to_vec();

    app.menu_state
        .set_enhanced_search_resources(EnhancedSearchResources::default());
    let mut plain = vec![0_u8; 800 * 600 * 4];
    main_assert!(app.render(&mut plain).expect("render the English hint"));

    main_assert_ne!(
        hinted => app.graphics.surface().pixels().to_vec(),
        "the translated clear hint is drawn, not the English one"
    );
}
