// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

    // The C++ book has no Back list row (C4StartupScenSelDlg has a Back
    // button/K_LEFT instead) and selects the first entry
    // (SelectFirstEntry, cpp:1536-1537).
    #[test]
    fn scensel_menu_state_without_back_row_selects_first_entry() {
        let scenarios = sample_scenarios();
        let entries = build_menu_entries(&scenarios, true);
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.menu().resize(1280.0, 720.0);

        state.set_include_back(false);
        assert!(state
            .menu()
            .entries()
            .iter()
            .all(|entry| entry.identifier != BACK_ENTRY_IDENTIFIER));

        let selection = state.select_default_entry();
        assert!(matches!(
            selection.as_slice(),
            [StartupMenuAction::SelectionChanged(summary)]
            if summary.identifier == "folder_missions"
        ));
        assert_eq!(
            state.selected_scenario().map(|entry| entry.title.as_str()),
            Some("Missions")
        );
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
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.set_include_back(false);
        let _ = state.select_default_entry();
        state.sync_definition_checkbox_to_selection();
        assert!(state.definition_checkbox_enabled);
        assert!(state.definition_checkbox_checked);
        assert_eq!(
            scenario_fixed_definition_modules(state.selected_scenario().unwrap()),
            ["Objects.c4d", "Knights.c4d"]
        );

        assert!(state.toggle_definition_checkbox());
        assert!(!state.definition_checkbox_checked);
        assert!(state.set_definition_checkbox_focused(true));
        // Opening/canceling the child selector does not resync this state.
        assert!(!state.definition_checkbox_checked);

        let _ = state.select_list_index(1);
        state.sync_definition_checkbox_to_selection();
        assert!(!state.definition_checkbox_enabled);
        assert!(state.definition_checkbox_checked);
        assert!(!state.definition_checkbox_focused);
        assert!(!state.toggle_definition_checkbox());
        assert_eq!(
            scenario_fixed_definition_modules(state.selected_scenario().unwrap()),
            ["Objects.c4d"]
        );
    }

    #[test]
    fn checked_definition_checkbox_intercepts_start_even_when_local_only_disables_it() {
        let mut app = new_menu_app(640, 480);
        app.open_scenario_browser();
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = "definition_intercept".to_string();
        scenario.title = "Definition intercept".to_string();
        scenario.path = Some(PathBuf::from("DefinitionIntercept.c4s"));
        scenario.local_only = Some(true);
        scenario.allow_user_change = Some(true);
        scenario.definition_modules = vec!["Ignored.c4d".to_string()];
        app.scenario_catalog
            .insert(scenario.identifier.clone(), scenario.clone());
        app.menu_state.definition_checkbox_enabled = false;
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
        .expect("checked start opens selector");

        let selector = app
            .definition_selector
            .as_ref()
            .expect("disabled-but-checked state still opens C4DefinitionSelDlg");
        assert_eq!(selector.accepted_selection(), ["Objects.c4d"]);
        assert!(app.loading_state.is_none());
        app.process_definition_selector_actions(vec![
            clonk_frontend::definition_sel::DefinitionSelAction::Cancelled,
        ])
        .expect("cancel selector");
        assert!(app.menu_state.definition_checkbox_checked);
        assert!(matches!(app.mode, AppMode::Menu));
    }

    #[test]
    fn scensel_mission_access_gates_rows_start_and_map_buttons_live() {
        let _lock = env_lock().lock();
        reset_cached_app_paths();
        let user_data = tempdir().expect("isolated mission-gate user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        configure_test_startup_participant(&paths, user_data.path());
        persist_config_value(&paths, "General", "LanguageEx", "DE")
            .expect("select German mission-gate resources");
        persist_config_value(&paths, "General", "MissionAccess", "")
            .expect("start without mission access");
        let scenario_root = paths.scenario_dir().to_path_buf();
        fs::create_dir_all(&scenario_root).expect("create mission-gate scenario root");
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
            fs::create_dir(&path).expect("create scenario group");
            fs::write(path.join("Scenario.txt"), core).expect("write scenario core");
        }
        let native_path = scenario_root.join("Native.c4s");
        fs::create_dir(&native_path).expect("create native-byte scenario group");
        fs::write(
            native_path.join("Scenario.txt"),
            b"[Head]\nTitle=Native access\nMinPlayer=1\nMaxPlayer=4\nMissionAccess=Secr\x80t\n",
        )
        .expect("write native-byte scenario core");

        let map_path = scenario_root.join("Map.c4f");
        let map_scenario_path = map_path.join("MapLocked.c4s");
        fs::create_dir_all(&map_scenario_path).expect("create mission-gated map scenario");
        fs::write(map_path.join("Folder.txt"), "[Head]\nTitle=Access Map\n")
            .expect("write map folder core");
        write_map_png(&map_path.join("FolderMap.png"), 8, 8, [20, 30, 40, 255]);
        fs::write(
            map_path.join("FolderMap.txt"),
            "[FolderMap]\n    [Scenario]\n    File=MapLocked.c4s\n    Area=0,0,8,8\n",
        )
        .expect("write mission-gated map layout");
        fs::write(
            map_scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Map locked\nMinPlayer=1\nMaxPlayer=4\nMissionAccess=Secret\n",
        )
        .expect("write mission-gated map scenario core");

        let mut app = new_menu_app_with_paths(640, 480, &paths);
        let scenarios = resource_scenario::discover(&scenario_root)
            .expect("discover mission-gate scenarios")
            .into_iter()
            .map(|entry| FrontendScenario::from_resource(entry, "Test scenarios"))
            .collect::<Vec<_>>();
        let menu = StartupMenu::new(
            build_menu_entries(&scenarios, false),
            test_font(),
            None,
        )
        .expect("mission-gate menu");
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_scenario_browser();

        assert_eq!(app.scenario_entry_enabled.get("Allowed.c4s"), Some(&true));
        assert_eq!(app.scenario_entry_enabled.get("Locked.c4s"), Some(&false));
        assert_eq!(app.scenario_entry_enabled.get("Native.c4s"), Some(&false));
        assert_eq!(app.scenario_entry_enabled.get("TooFew.c4s"), Some(&false));

        // The dynamic renderer must pass CanOpen to ScenListItem: only the
        // label alpha changes; icons and row activation remain intact.
        let assets = app.assets.scensel_assets().expect("scenario assets");
        let button_down = app
            .assets
            .dialog_image("GUIButtonDown.png")
            .expect("scenario button-down plank");
        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        let book = app.assets.book_fonts.clone().expect("book fonts");
        let mut surface = Surface::new(640, 480, PixelFormat::Rgba8888);
        draw_scensel_dynamic(
            &mut surface,
            &mut app.menu_state,
            &app.scenario_entry_enabled,
            &assets,
            &button_down,
            &fonts,
            &book,
            None,
            startup_gamma(),
            true,
        )
        .expect("render mission-gated rows");
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
            let enabled = app.scenario_entry_enabled[&entry.identifier];
            if enabled {
                assert!(
                    max_alpha > 200,
                    "{} row is enabled (max alpha {max_alpha})",
                    entry.title
                );
            } else {
                assert!(
                    max_alpha > 0 && max_alpha < 200,
                    "{} row uses disabled 50%-black text (max alpha {max_alpha})",
                    entry.title
                );
            }
        }

        let locked = app.scenario_catalog["Locked.c4s"].clone();
        assert!(locked.is_playable, "denied rows remain actionable");
        assert!(!locked.has_mission_access(&app.mission_access));
        app.enter_scenario_folder("Map.c4f");
        assert_eq!(
            app.menu_state
                .current_map()
                .expect("mission-gated map view")
                .scenarios
                .len(),
            0,
            "a denied scenario produces no map button"
        );
        app.menu_state.leave_folder();
        app.configure_current_folder_map();

        app.menu_state.definition_checkbox_checked = true;
        app.handle_menu_input(|_| {
            vec![StartupMenuAction::StartScenario(
                clonk_frontend::ScenarioSummary {
                    identifier: locked.identifier.clone(),
                    title: locked.title.clone(),
                    kind: ScenarioKind::Scenario,
                },
            )]
        })
        .expect("mission denial is a handled modal");
        assert!(app.loading_state.is_none());
        assert!(app.definition_selector.is_none());
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.message_dialogs[0].state.caption(), "Start nicht möglich.");
        assert_eq!(
            app.message_dialogs[0].state.message(),
            "Noch kein Zugang zu dieser Mission."
        );
        assert_eq!(
            app.message_dialogs[0].state.icon(),
            clonk_frontend::message_dialog::MessageDialogIcon::ERROR
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss mission-access error");

        let native_password = clonk_script::c4_string_from_bytes(b"Secr\x80t");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set Alt for Mission Access");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("open Mission Access dialog");
        assert_eq!(
            app.game_option_input_dialog
                .as_ref()
                .expect("Mission Access dialog")
                .purpose,
            PendingInputDialogPurpose::ScenarioMissionAccess
        );
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
            " secret ".to_string(),
        )])
        .expect("grant and reload Mission Access");
        wait_for_scenario_selector_discovery(&mut app);
        assert_eq!(app.mission_access.snapshot(), "secret");
        assert_eq!(
            load_configured_mission_access(&paths).expect("load persisted mission access"),
            "secret"
        );
        assert_eq!(app.scenario_entry_enabled.get("Locked.c4s"), Some(&true));
        assert_eq!(app.scenario_entry_enabled.get("Native.c4s"), Some(&false));
        assert!(locked.has_mission_access(&app.mission_access));
        app.enter_scenario_folder("Map.c4f");
        assert_eq!(
            app.menu_state
                .current_map()
                .expect("granted map view")
                .scenarios
                .len(),
            1,
            "Alt+M grant immediately enables map-button creation after reload"
        );
        app.menu_state.leave_folder();
        app.configure_current_folder_map();

        // The classic text dialog cannot synthesize an arbitrary native byte,
        // but the same live apply path must preserve one loaded from config.
        app.apply_scenario_mission_access(&native_password)
            .expect("apply native-byte mission access");
        wait_for_scenario_selector_discovery(&mut app);
        assert_eq!(
            app.mission_access.snapshot(),
            format!("secret;{native_password}")
        );
        assert_eq!(app.scenario_entry_enabled.get("Native.c4s"), Some(&true));
        assert_eq!(app.scenario_entry_enabled.get("TooFew.c4s"), Some(&false));
        assert_eq!(
            app.scenario_selector_open_error(
                &app.scenario_catalog["Native.c4s"],
                ScenarioSelectorMode::Local,
            )
            .expect("inspect native-byte access"),
            None,
            "granted native bytes survive both catalog and loader-head parsers"
        );

        app.menu_state.definition_checkbox_checked = true;
        app.handle_menu_input(|_| {
            vec![StartupMenuAction::StartScenario(
                clonk_frontend::ScenarioSummary {
                    identifier: locked.identifier.clone(),
                    title: locked.title.clone(),
                    kind: ScenarioKind::Scenario,
                },
            )]
        })
        .expect("granted mission continues through DoOK");
        assert!(app.message_dialogs.is_empty());
        assert!(
            app.definition_selector.is_some(),
            "the same catalog entry proceeds to the start flow after grant"
        );
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_recursive_focus_and_gamepad_pass_through_match_dialog_order() {
        let mut first = FrontendScenario::fallback();
        first.identifier = "first".to_string();
        first.title = "First".to_string();
        first.allow_user_change = Some(false);
        let mut second = FrontendScenario::fallback();
        second.identifier = "second".to_string();
        second.title = "Second".to_string();
        second.local_only = Some(true);
        second.allow_user_change = Some(false);
        let scenarios = vec![first, second];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("focus-order menu");
        let mut app = new_menu_app(800, 600);
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_scenario_browser();
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::List);
        assert!(app.menu_state.definition_checkbox_enabled);

        let tap_tab = |app: &mut GameApp| {
            app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
                .expect("Tab down");
            app.handle_key(VirtualKeyCode::Tab, ElementState::Released)
                .expect("Tab up");
        };
        tap_tab(&mut app);
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Back);
        tap_tab(&mut app);
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::Definitions
        );
        tap_tab(&mut app);
        assert_eq!(
            app.scenario_game_options.focused_button(),
            Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
        );
        tap_tab(&mut app);
        assert_eq!(
            app.scenario_game_options.focused_button(),
            Some(clonk_frontend::game_option_buttons::GameOptionButton::Record)
        );
        tap_tab(&mut app);
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Open);
        tap_tab(&mut app);
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Search);
        tap_tab(&mut app);
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::List);

        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("set keyboard modifiers");
        tap_tab(&mut app);
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Search);

        app.set_scensel_dialog_focus(ScenselDialogFocus::List);
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )
        .expect("List -> Back");
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Back);
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )
        .expect("Back -> Definitions");
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::Definitions
        );
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )
        .expect("Definitions -> first option");
        assert_eq!(
            app.scenario_game_options.focused_button(),
            Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
        );

        let selected_before = app.menu_state.menu.selected_index();
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Down,
            ElementState::Pressed,
        )
        .expect("unhandled option Down reaches list");
        assert_ne!(app.menu_state.menu.selected_index(), selected_before);
        assert_eq!(
            app.scenario_game_options.focused_button(),
            Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
        );
        assert!(!app.menu_state.definition_checkbox_enabled);

        app.set_scensel_dialog_focus(ScenselDialogFocus::List);
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )
        .expect("List -> Back with disabled definitions");
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Right,
            ElementState::Pressed,
        )
        .expect("disabled Definitions are skipped");
        assert_eq!(
            app.scenario_game_options.focused_button(),
            Some(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
        );
        app.handle_gamepad_direction(
            GamepadSlot::new(0),
            ControlButton::Left,
            ElementState::Pressed,
        )
        .expect("option boundary skips disabled Definitions backwards");
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Back);

        app.handle_menu_input(|menu| menu.select_list_index(0))
            .expect("reselect first scenario");
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
        .expect("disabled option passes low down");
        app.handle_gamepad_action(
            GamepadSlot::new(0),
            GamepadActionType::Select,
            ElementState::Released,
        )
        .expect("disabled option passes low up to scenario Enter");
        assert_eq!(app.mode, AppMode::Running);
    }

    #[test]
    fn empty_search_clears_forced_crew_constraint() {
        let scenario_root = tempdir().expect("forced scenario root");
        let scenario_path = scenario_root.path().join("Forced.c4s");
        fs::create_dir_all(&scenario_path).expect("forced scenario group");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Forced\nForcedNoCrew=2\n",
        )
        .expect("forced scenario core");
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = "forced".to_string();
        scenario.title = "Forced".to_string();
        scenario.path = Some(scenario_path);
        let scenarios = vec![scenario];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("forced scenario menu");
        let mut app = new_menu_app(800, 600);
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_scenario_browser();
        assert_eq!(
            app.scenario_game_options
                .values()
                .selector_fair_crew_constraint,
            FairCrewConstraint::ForceNormal
        );
        assert!(
            !app.scenario_game_options
                .view(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
                .expect("fair-crew option")
                .enabled
        );

        app.menu_state.set_search_text("no matching scenario");
        app.submit_scenario_search().expect("submit empty search");
        assert!(app.menu_state.selected_scenario().is_none());
        assert_eq!(
            app.scenario_game_options
                .values()
                .selector_fair_crew_constraint,
            FairCrewConstraint::Free
        );
        assert!(
            app.scenario_game_options
                .view(clonk_frontend::game_option_buttons::GameOptionButton::FairCrew)
                .expect("reset fair-crew option")
                .enabled
        );
    }

    #[test]
    fn scensel_touch_uses_classic_list_search_and_back_bounds() {
        let _lock = env_lock().lock();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated touch config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        let mut target = FrontendScenario::fallback();
        target.identifier = "outer/inner/target".to_string();
        target.title = "Touch Target".to_string();
        let mut inner = FrontendScenario::fallback();
        inner.identifier = "outer/inner".to_string();
        inner.title = "Inner Touch Folder".to_string();
        inner.kind = ScenarioKind::Folder;
        inner.is_playable = false;
        inner.children = vec![target];
        let mut outer = FrontendScenario::fallback();
        outer.identifier = "outer".to_string();
        outer.title = "Outer Touch Folder".to_string();
        outer.kind = ScenarioKind::Folder;
        outer.is_playable = false;
        outer.children = vec![inner];
        let mut sibling = FrontendScenario::fallback();
        sibling.identifier = "sibling".to_string();
        sibling.title = "Sibling Folder".to_string();
        sibling.kind = ScenarioKind::Folder;
        sibling.is_playable = false;
        let scenarios = vec![outer, sibling];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("touch selector menu");
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_network_game_dialog();
        app.open_network_host_scenario_browser();
        let fonts = app.assets.clonk_fonts.clone().expect("classic GUI fonts");
        let book = app.assets.book_fonts.clone().expect("classic book fonts");
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);
        let tap = |app: &mut GameApp, point: GuiPoint| {
            app.handle_touch(TouchPhase::Started, point)
                .expect("touch start");
            app.handle_touch(TouchPhase::Ended, point)
                .expect("touch end");
        };

        let pitch = clonk_frontend::startup_scensel::scen_list_item_height(&book.text) + 1;
        tap(
            &mut app,
            GuiPoint::new(
                (layout.list.x + 12) as f32,
                (layout.list.y + 3 + pitch + 4) as f32,
            ),
        );
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("sibling"),
            "touch list hit-testing must use the rendered book rows"
        );

        tap(
            &mut app,
            GuiPoint::new(
                (layout.list.x + 12) as f32,
                (layout.list.y + 3 + 4) as f32,
            ),
        );
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("outer")
        );
        let open = GuiPoint::new(
            (layout.open_button.x + layout.open_button.w / 2) as f32,
            (layout.open_button.y + layout.open_button.h / 2) as f32,
        );
        tap(&mut app, open);
        assert_eq!(app.menu_state.stack.len(), 2);
        assert_eq!(app.menu_state.book_caption(), "Outer Touch Folder");

        tap(
            &mut app,
            GuiPoint::new(
                (layout.search_edit.x + 8) as f32,
                (layout.search_edit.y + layout.search_edit.h / 2) as f32,
            ),
        );
        for character in "inner touch".chars() {
            app.handle_text_input(character).expect("type touch search");
        }
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("submit touch search");
        app.handle_key(VirtualKeyCode::Return, ElementState::Released)
            .expect("release touch search");
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("outer/inner")
        );
        tap(&mut app, open);
        assert_eq!(app.menu_state.stack.len(), 3);
        assert_eq!(app.menu_state.book_caption(), "Inner Touch Folder");

        let back = GuiPoint::new(
            (layout.back_button.x + layout.back_button.w / 2) as f32,
            (layout.back_button.y + layout.back_button.h / 2) as f32,
        );
        tap(&mut app, back);
        assert_eq!(app.menu_state.stack.len(), 2);
        tap(&mut app, back);
        assert_eq!(app.menu_state.stack.len(), 1);
        tap(&mut app, back);
        assert_eq!(app.startup_view, StartupView::NetworkGame);

        app.open_scenario_browser();
        tap(&mut app, back);
        assert_eq!(app.startup_view, StartupView::MainMenu);
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_cached_chrome_leaves_game_option_bounds_empty_in_both_modes() {
        let _lock = env_lock().lock();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated chrome config");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
        ]);
        let paths = AppPaths::discover().expect("discover app paths");
        let app = new_menu_app_with_paths(800, 600, &paths);
        let assets = app.assets.scensel_assets().expect("scenario assets");
        let fonts = app.assets.clonk_fonts.as_deref().expect("classic fonts");
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
                    assert_eq!(
                        chrome.get_pixel(x as u32, y as u32),
                        background.get_pixel(x as u32, y as u32),
                        "{title} base chrome must not pre-render FairCrew/Record"
                    );
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
        let mut cavern = FrontendScenario::fallback();
        cavern.identifier = "pack/cavern".to_string();
        cavern.title = "<c ff0000>Cavern</c>".to_string();

        let mut pack = FrontendScenario::fallback();
        pack.identifier = "pack".to_string();
        pack.title = "Pack".to_string();
        pack.kind = ScenarioKind::Folder;
        pack.is_playable = false;
        pack.children = vec![cavern];

        let scenarios = vec![pack];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.set_include_back(false);
        state
            .menu()
            .select_entry_by_index(0)
            .expect("select Pack");

        state.set_search_text("cAvErN");
        let actions = state.submit_search();

        assert!(state.visible_entries().is_empty());
        assert!(state.selected_scenario().is_none());
        assert!(actions.is_empty());
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
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.set_include_back(false);
        state.enter_folder("folder_missions");
        let _ = state
            .menu()
            .select_entry_by_index(2)
            .expect("select matching non-first entry");
        assert_eq!(
            state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("scenario_gamma")
        );

        state.set_search_text("cRyStAl cAvErN");
        assert_eq!(
            state.visible_entries().len(),
            3,
            "typing alone does not submit"
        );

        let actions = state.submit_search();
        assert_eq!(
            state
                .visible_entries()
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            vec!["<c ff0000>Crystal</c> Cavern", "Crystal Cavern Annex"]
        );
        assert!(matches!(
            actions.as_slice(),
            [StartupMenuAction::SelectionChanged(summary)]
            if summary.identifier == "scenario_gamma"
        ));
        assert_eq!(
            state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("scenario_gamma")
        );
    }

    #[test]
    fn scensel_search_edit_matches_selection_word_and_length_rules() {
        let mut edit = SearchEditState::default();
        edit.set_text("Alpha beta");
        edit.focus();
        assert_eq!(edit.selected_text(), Some("Alpha beta"));
        edit.insert_text("Z");
        assert_eq!(edit.text(), "Z", "typing replaces Ctrl+F select-all");

        edit.set_text("one  two_three!");
        edit.move_cursor(SearchCursorOperation::End, false, false);
        edit.move_cursor(SearchCursorOperation::Left, false, false);
        edit.move_cursor(SearchCursorOperation::Left, true, false);
        assert_eq!(edit.caret(), 5, "Ctrl+Left stops at the final word start");
        edit.backspace(true, false);
        assert_eq!(edit.text(), "two_three!", "Ctrl+Backspace removes one word");
        edit.move_cursor(SearchCursorOperation::Home, false, false);
        edit.move_cursor(SearchCursorOperation::Right, true, true);
        assert_eq!(edit.selected_text(), Some("two_three!"));
        edit.delete(false, false);
        assert_eq!(edit.text(), "");

        edit.set_text("");
        edit.insert_text(&"a".repeat(300));
        assert_eq!(edit.text().len(), SEARCH_EDIT_MAX_BYTES);
        edit.set_text("");
        edit.insert_text("left|right");
        assert_eq!(edit.text(), "left¦right");
        edit.set_text("éé");
        edit.move_cursor(SearchCursorOperation::Left, false, false);
        assert_eq!(edit.caret(), "é".len(), "caret stays on UTF-8 boundaries");
        edit.backspace(false, false);
        assert_eq!(edit.text(), "é");

        edit.set_text("alpha beta");
        edit.select_word_at(8);
        assert_eq!(edit.selected_text(), Some("beta"));
        edit.begin_pointer_selection(0);
        edit.drag_pointer_selection(edit.text().len());
        edit.end_pointer_selection(edit.text().len());
        assert_eq!(edit.selected_text(), Some("alpha beta"));

        edit.set_text("abcdef");
        edit.begin_pointer_selection(5);
        edit.drag_pointer_selection(2);
        assert_eq!(edit.selected_text(), Some("cde"));
        assert!(edit.backspace(false, false));
        assert_eq!(edit.text(), "abf");
        edit.drag_pointer_selection(edit.text().len());
        assert_eq!(
            edit.selected_text(),
            Some("f"),
            "selection deletion updates the still-active physical drag anchor"
        );
        edit.end_pointer_selection(edit.text().len());

        edit.set_text("abcdef");
        edit.begin_pointer_selection(5);
        assert!(edit.backspace(false, false));
        assert_eq!(edit.text(), "abcdf");
        assert_eq!(edit.caret(), 4);
        edit.drag_pointer_selection(2);
        assert_eq!(
            edit.selected_text(),
            Some("cdf"),
            "collapsed cursor deletion preserves C++'s hidden drag anchor"
        );
        edit.end_pointer_selection(2);

        edit.set_text("W".repeat(100));
        edit.scroll_cursor_in_view(500, 100, 3);
        assert!(edit.horizontal_scroll > 0);
        edit.move_cursor(SearchCursorOperation::Home, false, false);
        edit.scroll_cursor_in_view(0, 100, 3);
        assert_eq!(edit.horizontal_scroll, 1);
        assert!(edit.cursor_visible());
        for _ in 0..18 {
            edit.tick_blink();
        }
        assert!(!edit.cursor_visible());
    }

    #[test]
    fn scensel_middle_down_inserts_raw_primary_without_focus_or_submit() {
        let mut app = new_menu_app(800, 600);
        app.open_scenario_browser();
        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);

        app.menu_state.set_search_text("alpha beta");
        app.menu_state.search_edit.anchor = 0;
        app.menu_state.search_edit.caret = 5;
        assert!(!app.menu_state.search_focused());
        let insertion = "raw|primary\ntext";
        let clicked_position = "alpha ".len();
        let point = GuiPoint::new(
            (layout.search_edit.x
                + 4
                + fonts.text.measure("alpha ", false).0) as f32,
            (layout.search_edit.y + layout.search_edit.h / 2) as f32,
        );
        assert_eq!(
            app.scensel_search_char_pos(point, true),
            Some(clicked_position)
        );
        assert!(app.handle_scensel_search_middle_down(point, Some(insertion)));
        assert_eq!(app.menu_state.search_text(), "alpha raw|primary\ntextbeta");
        assert_eq!(
            app.menu_state.search_edit.caret(),
            clicked_position + insertion.len()
        );
        assert!(app.menu_state.search_edit.selection_range().is_none());
        assert!(!app.menu_state.search_focused(), "middle-down does not focus");
        assert_eq!(
            app.menu_state.applied_search_text, "",
            "raw PRIMARY insertion does not submit the search"
        );

        let unchanged = app.menu_state.search_text().to_string();
        app.menu_state.search_edit.horizontal_scroll = 0;
        let start = GuiPoint::new(
            (layout.search_edit.x + 4) as f32,
            (layout.search_edit.y + layout.search_edit.h / 2) as f32,
        );
        app.menu_state.search_edit.blink_ticks = 7;
        app.menu_state.search_edit.dragging = true;
        assert!(app.handle_scensel_search_middle_down(start, None));
        assert_eq!(app.menu_state.search_text(), unchanged);
        assert_eq!(app.menu_state.search_edit.caret(), 0);
        assert_eq!(app.menu_state.search_edit.blink_ticks, 7);
        assert!(
            app.menu_state.search_edit.dragging,
            "middle-down does not cancel an active left-button drag"
        );
        assert!(!app.handle_scensel_search_middle_down(
            GuiPoint::new(-10.0, -10.0),
            Some("ignored")
        ));
        assert_eq!(app.menu_state.search_text(), unchanged);

        app.menu_state.set_search_text("tail");
        app.menu_state.search_edit.begin_pointer_selection(2);
        assert!(app.handle_scensel_search_middle_down(start, Some("raw")));
        assert!(app.menu_state.search_edit.dragging);
        let end = app.menu_state.search_text().len();
        app.menu_state.search_edit.drag_pointer_selection(end);
        assert_eq!(
            app.menu_state.search_edit.selected_text(),
            Some("rawtail"),
            "an active left drag retains the pre-insertion click as its anchor"
        );
        app.menu_state.search_edit.end_pointer_selection(0);
        assert!(app.menu_state.search_edit.selection_range().is_none());
        assert_eq!(app.menu_state.search_edit.caret(), 0);

        app.menu_state.set_search_text("tail");
        app.menu_state.search_edit.begin_pointer_selection(0);
        let tail_end = GuiPoint::new(
            (layout.search_edit.x + 4 + fonts.text.measure("tail", false).0) as f32,
            (layout.search_edit.y + layout.search_edit.h / 2) as f32,
        );
        assert_eq!(app.scensel_search_char_pos(tail_end, true), Some(4));
        assert!(app.handle_scensel_search_middle_down(tail_end, Some("raw")));
        app.menu_state
            .search_edit
            .move_cursor(SearchCursorOperation::End, false, true);
        app.menu_state.search_edit.drag_pointer_selection(0);
        assert_eq!(
            app.menu_state.search_edit.selected_text(),
            Some("tail"),
            "a no-op Shift+End preserves the hidden pre-insertion drag anchor"
        );
        app.menu_state.search_edit.end_pointer_selection(0);

        app.menu_state.set_search_text("x".repeat(252));
        app.menu_state.search_edit.anchor = 10;
        app.menu_state.search_edit.caret = 20;
        app.menu_state.search_edit.blink_ticks = 9;
        app.menu_state.search_edit.dragging = false;
        assert!(app.handle_scensel_search_middle_down(start, Some("raw")));
        assert_eq!(app.menu_state.search_text().len(), SEARCH_EDIT_MAX_BYTES);
        assert!(app.menu_state.search_text().starts_with("ra"));
        assert_eq!(app.menu_state.search_edit.caret(), 2);
        assert_eq!(app.menu_state.search_edit.blink_ticks, 0);
        assert!(app.menu_state.search_edit.selection_range().is_none());

        let insertion_position = 100;
        let narrow_text = "i".repeat(insertion_position + 3);
        app.menu_state.set_search_text(narrow_text);
        let client_width = layout.search_edit.w - 8;
        let prefix_width = fonts
            .text
            .measure(&"i".repeat(insertion_position), false)
            .0;
        assert!(prefix_width > client_width);
        let pointer_offset = client_width - 2;
        let old_scroll = prefix_width - pointer_offset;
        app.menu_state.search_edit.horizontal_scroll = old_scroll;
        let same_index_point = GuiPoint::new(
            (layout.search_edit.x + 4 + pointer_offset) as f32,
            (layout.search_edit.y + layout.search_edit.h / 2) as f32,
        );
        assert_eq!(
            app.scensel_search_char_pos(same_index_point, true),
            Some(insertion_position)
        );
        assert!(app.handle_scensel_search_middle_down(same_index_point, Some("WWW")));
        assert_eq!(
            app.menu_state.search_edit.caret(),
            insertion_position + 3,
            "the insertion can end at the old caret byte index"
        );
        assert!(
            app.menu_state.search_edit.horizontal_scroll > old_scroll,
            "a successful same-index insertion still recomputes cursor scrolling"
        );

        app.menu_state.set_search_text("");
        assert!(app.handle_scensel_search_middle_down(
            start,
            Some(&"W".repeat(SEARCH_EDIT_MAX_BYTES))
        ));
        assert!(
            app.menu_state.search_edit.horizontal_scroll > 0,
            "raw insertion scrolls the advanced caret into view"
        );

        app.menu_state
            .set_search_text("W".repeat(SEARCH_EDIT_MAX_BYTES));
        app.menu_state.set_search_focused(true);
        app.menu_state.search_edit.blink_ticks = 7;
        app.menu_state.search_edit.dragging = true;
        app.menu_state.set_pointer_position(Some(start));
        app.startup_dialog_fade = None;
        app.handle_other_mouse_button(ElementState::Pressed)
            .expect("route scenario middle-down");
        assert_eq!(app.menu_state.search_text().len(), SEARCH_EDIT_MAX_BYTES);
        assert_eq!(app.menu_state.search_edit.caret(), 0);
        assert!(app.menu_state.search_edit.selection_range().is_none());
        assert_eq!(
            app.menu_state.search_edit.blink_ticks, 7,
            "a full buffer cannot insert PRIMARY and does not restart blink"
        );
        assert!(app.menu_state.search_edit.dragging);
    }

    #[test]
    fn scensel_search_context_entries_match_cpp_conditions_and_order() {
        let mut edit = SearchEditState::default();
        assert!(scensel_search_context_entries(&edit, false).is_empty());

        let paste_only = scensel_search_context_entries(&edit, true);
        assert_eq!(paste_only.len(), 1);
        assert_eq!(paste_only[0].text, "Paste");

        edit.set_text("alpha beta");
        let select_only = scensel_search_context_entries(&edit, false);
        assert_eq!(select_only.len(), 1);
        assert_eq!(select_only[0].text, "Select all");

        edit.anchor = 0;
        edit.caret = 5;
        let entries = scensel_search_context_entries(&edit, true);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Cut", "Copy", "Paste", "Clear", "Select all"]
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.tooltip.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("Moves the selection to the clipboard."),
                Some("Copies the selection to the clipboard."),
                Some("Inserts the contents of the clipboard."),
                Some("Clears the selection."),
                Some("Selects the complete text"),
            ]
        );
        assert!(entries
            .iter()
            .all(|entry| { entry.icon == ContextMenuIcon::None && entry.hotkey.is_none() }));

        edit.anchor = edit.text().len();
        edit.caret = 0;
        let whole_reverse = scensel_search_context_entries(&edit, false);
        assert_eq!(
            whole_reverse
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Cut", "Copy", "Clear"],
            "whole selection omits Select all in either direction"
        );
    }

    // The real window route must consume Ctrl+F/text/Enter in the search
    // edit. Enter confirms the edit instead of starting the selected scenario
    // (C4StartupScenSelDlg.cpp:1400-1401,1804-1808; C4GuiEdit.cpp:364-368).
    #[test]
    fn scensel_search_routes_window_text_and_enter() {
        let _lock = env_lock().lock();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_LANGUAGE", Some(Path::new("US"))),
        ]);
        let paths = AppPaths::discover().expect("discover repository install");
        let mut app = GameApp::new(
            800,
            600,
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
                player_name: "Search Tester".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_scenario_browser();

        let original_count = app.menu_state.visible_entries().len();
        let mut query = app.menu_state.visible_entries()[0].title.clone();
        Markup::strip_markup(&mut query);
        query.make_ascii_lowercase();

        app.menu_state.set_search_text("replace this");
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::F, ElementState::Pressed)
            .expect("focus search");
        assert_eq!(
            app.menu_state.search_edit.selected_text(),
            Some("replace this")
        );
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        for character in query.chars() {
            app.handle_text_input(character).expect("type query");
        }
        assert!(app.menu_state.search_focused());
        assert_eq!(
            app.menu_state.visible_entries().len(),
            original_count,
            "C++ does not apply search until Enter"
        );

        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("submit query");
        assert_eq!(app.mode, AppMode::Menu, "Enter must not start a scenario");
        assert!(!app.menu_state.visible_entries().is_empty());
        assert!(app.menu_state.visible_entries().iter().all(|entry| {
            let mut title = entry.title.clone();
            Markup::strip_markup(&mut title);
            title.to_lowercase().contains(&query)
        }));

        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);
        app.menu_state.set_search_text("alpha beta");
        let beta_x = layout.search_edit.x + 4 + fonts.text.measure("alpha be", false).0;
        let edit_y = layout.search_edit.y + layout.search_edit.h / 2;
        for _ in 0..2 {
            app.handle_cursor_moved(PhysicalPosition::new(f64::from(beta_x), f64::from(edit_y)))
                .expect("point inside beta");
            app.handle_mouse_button(ElementState::Pressed)
                .expect("press search edit");
            app.handle_mouse_button(ElementState::Released)
                .expect("release search edit");
        }
        assert_eq!(app.menu_state.search_edit.selected_text(), Some("beta"));

        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(layout.search_edit.x + 4),
            f64::from(edit_y),
        ))
        .expect("point at search start");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("start search selection drag");
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(layout.search_edit.x + layout.search_edit.w + 200),
            f64::from(edit_y),
        ))
        .expect("drag outside search edit");
        app.handle_mouse_button(ElementState::Released)
            .expect("release captured search drag");
        assert_eq!(
            app.menu_state.search_edit.selected_text(),
            Some("alpha beta")
        );

        app.menu_state.set_search_text("W".repeat(100));
        app.mark_menu_dirty();
        let mut frame = vec![0_u8; 800 * 600 * 4];
        app.render(&mut frame)
            .expect("render horizontally scrolled edit");
        assert!(app.menu_state.search_edit.horizontal_scroll > 0);
        app.handle_key(VirtualKeyCode::Home, ElementState::Pressed)
            .expect("move search caret home");
        app.render(&mut frame).expect("render search caret at home");
        assert!(app.menu_state.search_edit.horizontal_scroll <= 2);
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_selector_shortcuts_execute_before_conflicting_controls() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated selector shortcut user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let scenario_path = paths.scenario_dir().join("Shortcut.c4s");
        fs::create_dir_all(&scenario_path).expect("create shortcut scenario");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Shortcut Target\n",
        )
        .expect("write shortcut scenario");
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = "Shortcut.c4s".to_string();
        scenario.title = "Shortcut Target".to_string();
        scenario.path = Some(scenario_path);
        scenario.source_paths = vec![scenario.path.clone().expect("scenario path")];
        let scenarios = vec![scenario];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("selector shortcut menu");
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.menu_state = MenuState::new(menu, scenarios);
        app.open_network_host_scenario_browser();
        assert!(app.menu_state.selected_scenario().is_some());

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
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("Alt+M opens Mission Access before Comment");
        assert_eq!(
            app.game_option_input_dialog
                .as_ref()
                .expect("Mission Access input dialog")
                .purpose,
            PendingInputDialogPurpose::ScenarioMissionAccess
        );
        assert_eq!(
            app.scenario_game_options.values().comment,
            "unchanged comment"
        );
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
            .expect("cancel Mission Access");
        assert!(app.game_option_input_dialog.is_none());

        app.handle_modifiers_changed(ModifiersState::ALT | ModifiersState::CTRL)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("Ctrl+Alt+M matches neither exact selector nor option mnemonic");
        assert!(app.game_option_input_dialog.is_none());

        app.handle_modifiers_changed(ModifiersState::ALT | ModifiersState::SHIFT)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("Alt+Shift+M reaches the Comment mnemonic");
        assert_eq!(
            app.game_option_input_dialog
                .as_ref()
                .expect("Comment input dialog")
                .purpose,
            PendingInputDialogPurpose::GameOption(GameOptionInputKind::Comment)
        );
        app.game_option_input_dialog = None;
        app.game_option_input_consumed_keys.clear();
        app.game_option_consumed_keys.clear();

        app.handle_modifiers_changed(ModifiersState::ALT | ModifiersState::LOGO)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("C4KeyCodeEx ignores the OS Logo modifier");
        assert_eq!(
            app.game_option_input_dialog
                .as_ref()
                .expect("Mission Access input dialog")
                .purpose,
            PendingInputDialogPurpose::ScenarioMissionAccess
        );
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
            .expect("cancel Mission Access");

        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        app.menu_state.set_search_text("context");
        app.menu_state.set_search_focused(true);
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open the search edit context menu");
        assert!(app.context_menu.is_some());
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("an open context menu suppresses the underlying selector dialog");
        assert!(app.context_menu.is_some());
        assert!(app.game_option_input_dialog.is_none());
        assert_eq!(
            app.scenario_game_options.values().comment,
            "unchanged comment"
        );
        app.close_context_menu_silently();

        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        app.menu_state.set_search_text("");
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("F2 starts inline rename");
        assert_eq!(
            app.menu_state
                .rename_edit
                .as_ref()
                .map(|rename| rename.edit.selected_text()),
            Some(Some("Shortcut Target"))
        );
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("Escape aborts inline rename");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::Search,
            "RenameEdit restores the control focused before F2"
        );
        assert!(app.menu_state.search_focused());

        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("F5 refreshes the selector in place");
        app.handle_modifiers_changed(ModifiersState::LOGO)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("C4 ignores Logo when matching unmodified F5");
        wait_for_scenario_selector_discovery(&mut app);

        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        app.menu_state.set_search_text("alpha beta");
        app.menu_state.set_search_focused(true);
        app.menu_state.search_edit.anchor = 0;
        app.menu_state.search_edit.caret = 0;
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("selector Delete must outrank its normal-priority search edit");
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(app.menu_state.search_text(), "alpha beta");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
            .expect("decline deletion");

        // The selector binds only unmodified Delete. Ctrl+Delete remains an
        // edit operation, matching Edit::RegisterCursorOp's modifier list.
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("Ctrl+Delete reaches the focused search edit");
        assert_eq!(app.menu_state.search_text(), "beta");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("Alt+Delete matches neither selector nor search Edit");
        assert_eq!(app.menu_state.search_text(), "beta");
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_rename_restores_search_and_specific_option_focus() {
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = "Focus.c4s".to_string();
        scenario.title = "Focus Target".to_string();
        let scenarios = vec![scenario];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("rename focus menu");
        let mut app = new_menu_app(800, 600);
        app.menu_state = MenuState::new(menu, scenarios);
        app.open_scenario_browser();

        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start rename from search");
        assert!(!app.menu_state.search_focused(), "inline edit steals focus");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("abort rename back to search");
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Search);
        assert!(app.menu_state.search_focused());

        app.set_scensel_dialog_focus(ScenselDialogFocus::Options);
        app.scenario_game_options
            .set_focused_button(Some(GameOptionButton::Record));
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start rename from Record option");
        assert_eq!(app.scenario_game_options.focused_button(), None);
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("abort rename back to Record option");
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Options);
        assert_eq!(
            app.scenario_game_options.focused_button(),
            Some(GameOptionButton::Record)
        );

        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start rename before refresh");
        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("refresh aborts rename and restores its prior focus");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::Search);
        assert!(app.menu_state.search_focused());

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start rename for exact-modifier checks");
        let original_title = app
            .menu_state
            .rename_edit
            .as_ref()
            .expect("active rename")
            .edit
            .text()
            .to_string();
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set Alt modifier");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("Alt+Delete has no rename binding");
        assert_eq!(
            app.menu_state
                .rename_edit
                .as_ref()
                .expect("Alt+Delete keeps rename active")
                .edit
                .text(),
            original_title
        );
        app.handle_key(VirtualKeyCode::Z, ElementState::Pressed)
            .expect("unmatched Alt hotkey has no rename binding");
        assert!(app.menu_state.rename_edit.is_some());
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("Alt+M opens Mission Access without moving rename focus");
        assert!(app.menu_state.rename_edit.is_some());
        assert_eq!(
            app.game_option_input_dialog
                .as_ref()
                .expect("Mission Access input")
                .purpose,
            PendingInputDialogPurpose::ScenarioMissionAccess
        );
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Cancelled])
            .expect("cancel Mission Access while rename remains active");
        assert!(app.menu_state.rename_edit.is_some());
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set Control modifier");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("Control+Escape has no rename binding");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("Control+Tab has no focus-advance binding");
        assert!(app.menu_state.rename_edit.is_some());
        app.handle_modifiers_changed(ModifiersState::SHIFT)
            .expect("set Shift modifier");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("Shift+Enter has no rename binding");
        assert!(app.menu_state.rename_edit.is_some());
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("clear keyboard modifiers");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("unmodified Escape still aborts rename");
        assert!(app.menu_state.rename_edit.is_none());

        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start same-title rename before Control+F");
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set Control modifier");
        app.handle_key(VirtualKeyCode::F, ElementState::Pressed)
            .expect("Control+F causes rename focus loss");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::List,
            "RR_Deleted focus cancels the original Control+F transfer"
        );

        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("clear keyboard modifiers");
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start empty focus-loss rename");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("clear rename text");
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("empty focus-loss submission aborts");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::Search,
            "empty abort restores focus and cancels the original Tab transfer"
        );

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start rename before accepted Mission Access");
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set Alt modifier");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("open Mission Access over active rename");
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
            "MissionPass".to_string(),
        )])
        .expect("accepted Mission Access rebuilds the selector");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::Search,
            "UpdateList aborts rename and restores its saved focus before rebuilding"
        );
    }

    #[test]
    fn scensel_rename_gamepad_low_and_directions_match_dialog_bindings() {
        let mut child = FrontendScenario::fallback();
        child.identifier = "Folder.c4f/Child.c4s".to_string();
        child.title = "Child".to_string();
        child.path = None;

        let mut folder = FrontendScenario::fallback();
        folder.identifier = "Folder.c4f".to_string();
        folder.title = "Folder".to_string();
        folder.kind = ScenarioKind::Folder;
        folder.is_playable = false;
        folder.path = None;
        folder.children = vec![child];

        let scenarios = vec![folder];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("gamepad rename menu");
        let mut app = new_menu_app(800, 600);
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_scenario_browser();
        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start rename for direction routing");

        let slot = GamepadSlot::new(0);
        let source = |gamepad, event| SourcedGamepadEvent {
            gamepad,
            cluster: gamepad as u64,
            event,
        };
        app.process_sourced_gamepad_event_batch(
            [source(
                0,
                GamepadEvent::Direction {
                    slot,
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                },
            )],
            false,
        )
        .expect("disabled gamepad GUI has no dialog focus binding");
        assert!(app.menu_state.rename_edit.is_some());
        let wrong_slot = GamepadSlot::new(1);
        app.process_sourced_gamepad_event_batch(
            [source(
                1,
                GamepadEvent::Direction {
                    slot: wrong_slot,
                    button: ControlButton::Left,
                    state: ElementState::Pressed,
                },
            )],
            true,
        )
        .expect("non-primary gamepad has no dialog focus binding");
        assert!(app.menu_state.rename_edit.is_some());
        app.process_gamepad_event_batch([GamepadEvent::Direction {
            slot,
            button: ControlButton::Up,
            state: ElementState::Pressed,
        }])
        .expect("Up is inert in RenameEdit");
        assert!(app.menu_state.rename_edit.is_some());
        app.process_gamepad_event_batch([GamepadEvent::Direction {
            slot,
            button: ControlButton::Right,
            state: ElementState::Pressed,
        }])
        .expect("Right attempts focus loss");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(
            app.menu_state.dialog_focus(),
            ScenselDialogFocus::List,
            "successful RR_Deleted rename owns focus; the original advance is cancelled"
        );
        assert!(app.menu_state.current_folder().is_none());

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("restart rename for AnyLow");
        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot,
                class: GuiButtonClass::Low,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot,
                action: GamepadActionType::Cancel,
                state: ElementState::Pressed,
            },
        ])
        .expect("AnyLow aborts rename then executes dialog OK once");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(
            app.menu_state
                .current_folder()
                .map(|folder| folder.identifier.as_str()),
            Some("Folder.c4f")
        );
    }

    #[test]
    fn scensel_rename_abort_paths_do_not_mutate_and_focus_loss_commits() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated rename lifecycle user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let old_path = paths.scenario_dir().join("Old.c4s");
        let new_path = paths.scenario_dir().join("New.c4s");
        fs::create_dir_all(&old_path).expect("create original scenario");
        fs::write(old_path.join("Scenario.txt"), "[Head]\nTitle=Old\n")
            .expect("write original scenario");

        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        let index = app
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == "Old.c4s")
            .expect("original scenario row");
        app.handle_menu_input(|menu| menu.select_list_index(index))
            .expect("select original scenario");

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start gamepad-aborted rename");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("clear selected title");
        for character in "New".chars() {
            app.handle_text_input(character).expect("type replacement title");
        }
        let slot = GamepadSlot::new(0);
        app.process_gamepad_event_batch([GamepadEvent::Action {
            slot,
            action: GamepadActionType::Cancel,
            state: ElementState::Pressed,
        }])
        .expect("an abstract alias cannot bypass the raw GUI-button bindings");
        assert!(app.menu_state.rename_edit.is_some());

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
                    GamepadEvent::GuiButton {
                        slot,
                        class: GuiButtonClass::High,
                        state: ElementState::Pressed,
                    },
                ),
                source(
                    0,
                    10,
                    GamepadEvent::Action {
                        slot,
                        action: GamepadActionType::MenuToggle,
                        state: ElementState::Pressed,
                    },
                ),
            ],
            false,
        )
        .expect("disabled gamepad GUI does not bind AnyHighButton");
        assert!(app.menu_state.rename_edit.is_some());

        let wrong_slot = GamepadSlot::new(1);
        app.process_sourced_gamepad_event_batch(
            [
                source(
                    1,
                    11,
                    GamepadEvent::GuiButton {
                        slot: wrong_slot,
                        class: GuiButtonClass::High,
                        state: ElementState::Pressed,
                    },
                ),
                source(
                    1,
                    11,
                    GamepadEvent::Action {
                        slot: wrong_slot,
                        action: GamepadActionType::MenuToggle,
                        state: ElementState::Pressed,
                    },
                ),
            ],
            true,
        )
        .expect("AnyHighButton is registered only for gamepad zero");
        assert!(app.menu_state.rename_edit.is_some());

        app.process_gamepad_event_batch([
            GamepadEvent::GuiButton {
                slot,
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
            },
            GamepadEvent::Action {
                slot,
                action: GamepadActionType::MenuToggle,
                state: ElementState::Pressed,
            },
        ])
        .expect("AnyHighButton aborts and owns its alias cluster");
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);
        assert!(old_path.exists());
        assert!(!new_path.exists());

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start empty rename");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("clear title for empty submission");
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("empty Enter is an abort");
        assert!(app.menu_state.rename_edit.is_none());
        assert!(old_path.exists());
        assert!(!new_path.exists());

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start focus-loss rename");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("clear title for focus-loss submission");
        for character in "New".chars() {
            app.handle_text_input(character).expect("type focus-loss title");
        }
        app.handle_key(VirtualKeyCode::Tab, ElementState::Pressed)
            .expect("focus loss submits rename as OK");
        wait_for_scenario_selector_discovery(&mut app);
        assert!(app.menu_state.rename_edit.is_none());
        assert!(!old_path.exists());
        assert!(new_path.exists());
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| (entry.identifier.as_str(), entry.title.as_str())),
            Some(("New.c4s", "New"))
        );
        assert!(app.scenario_catalog.contains_key("New.c4s"));
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_delete_falls_through_to_search_edit_without_a_selection() {
        let menu =
            StartupMenu::new(Vec::new(), test_font(), None).expect("empty selector shortcut menu");
        let mut app = new_menu_app(800, 600);
        app.menu_state = MenuState::new(menu, Vec::new());
        app.open_scenario_browser();
        assert!(app.menu_state.selected_scenario().is_none());

        app.menu_state.set_search_text("abc");
        app.menu_state.set_search_focused(true);
        app.menu_state.search_edit.anchor = 0;
        app.menu_state.search_edit.caret = 0;
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("selector callback declines and focused Edit handles Delete");
        assert_eq!(app.menu_state.search_text(), "bc");

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("Rename declines without a selected scenario row");
        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("Refresh remains dialog-wide without a selection");
    }

    #[test]
    fn scensel_f5_rediscovers_current_folder_and_applies_live_search() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated refresh user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let folder = paths.scenario_dir().join("RefreshPack.c4f");
        let alpha = folder.join("Alpha.c4s");
        let beta = folder.join("Beta.c4s");
        let delta = folder.join("Delta.c4s");
        fs::create_dir_all(&alpha).expect("create initial refresh scenario");
        fs::create_dir_all(&beta).expect("create second initial refresh scenario");
        fs::create_dir_all(&delta).expect("create unrelated refresh scenario");
        fs::write(folder.join("Folder.txt"), "[Head]\nIndex=1\n")
            .expect("write refresh folder core");
        fs::write(
            alpha.join("Scenario.txt"),
            "[Head]\nTitle=Alpha Mission\n",
        )
            .expect("write initial refresh scenario core");
        fs::write(
            beta.join("Scenario.txt"),
            "[Head]\nTitle=Beta Mission\n",
        )
        .expect("write second initial refresh scenario core");
        fs::write(
            delta.join("Scenario.txt"),
            "[Head]\nTitle=Unrelated\n",
        )
        .expect("write unrelated refresh scenario core");

        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        app.menu_state.enter_folder("RefreshPack.c4f");
        assert_eq!(
            app.menu_state
                .current_folder()
                .map(|folder| folder.identifier.as_str()),
            Some("RefreshPack.c4f")
        );
        let beta_index = app
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == "RefreshPack.c4f/Beta.c4s")
            .expect("initial Beta row");
        app.handle_menu_input(|menu| menu.menu().select_entry_by_index(beta_index).unwrap())
            .expect("select Beta before refresh");
        app.menu_state.set_search_text("mission");
        app.menu_state.set_search_focused(true);

        let gamma = folder.join("Gamma.c4s");
        fs::create_dir_all(&gamma).expect("create refreshed scenario");
        fs::write(
            gamma.join("Scenario.txt"),
            "[Head]\nTitle=Gamma Mission\n",
        )
            .expect("write refreshed scenario core");
        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("refresh current scenario folder");

        assert_eq!(
            app.scenario_selector_loading_label().as_deref(),
            Some("Loading... (0%)"),
            "the loading book is observable before the worker can be polled"
        );
        let mut zero_percent_frame = vec![0_u8; 800 * 600 * 4];
        app.render(&mut zero_percent_frame)
            .expect("render zero-percent loading book");
        app.scenario_selector_discovery
            .as_mut()
            .expect("loading state remains installed before polling")
            .progress_percent = 37;
        app.mark_menu_dirty();
        let mut progressed_frame = vec![0_u8; 800 * 600 * 4];
        app.render(&mut progressed_frame)
            .expect("render progressed loading book");
        assert_ne!(
            zero_percent_frame, progressed_frame,
            "the visible loading label must track the percentage state"
        );
        assert_eq!(app.menu_state.search_text(), "mission");
        app.handle_key(VirtualKeyCode::Home, ElementState::Pressed)
            .expect("move loading search caret home");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("edit search without deleting the retained hidden row");
        assert_eq!(app.menu_state.search_text(), "ission");
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("RefreshPack.c4f/Beta.c4s"),
            "the old tree remains intact behind the loading book"
        );
        assert!(
            !app
                .scenario_catalog
                .contains_key("RefreshPack.c4f/Gamma.c4s"),
            "the discovered tree must not leak in before the atomic completion"
        );

        wait_for_scenario_selector_discovery(&mut app);

        assert_eq!(
            app.menu_state
                .current_folder()
                .map(|folder| folder.identifier.as_str()),
            Some("RefreshPack.c4f")
        );
        assert_eq!(
            app.menu_state
                .visible_entries()
                .iter()
                .map(|entry| entry.identifier.as_str())
                .collect::<Vec<_>>(),
            vec![
                "RefreshPack.c4f/Alpha.c4s",
                "RefreshPack.c4f/Beta.c4s",
                "RefreshPack.c4f/Gamma.c4s",
            ]
        );
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("RefreshPack.c4f/Beta.c4s")
        );
        assert_eq!(app.menu_state.search_text(), "ission");
        assert_eq!(app.menu_state.applied_search_text, "ission");
        assert!(
            app.scenario_catalog
                .contains_key("RefreshPack.c4f/Gamma.c4s")
        );
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_f2_renames_unpacked_scenario_rewrites_title_and_refocuses() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated rename user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "General", "Language", "DE")
            .expect("configure primary title language");
        let old_path = paths.scenario_dir().join("Old.c4s");
        fs::create_dir_all(&old_path).expect("create rename scenario");
        fs::write(
            old_path.join("Scenario.txt"),
            "[Head]\nTitle=Old display title\n",
        )
        .expect("write rename scenario core");
        fs::write(old_path.join("Title.txt"), "US:Old display title")
            .expect("write old title");

        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        let index = app
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == "Old.c4s")
            .expect("Old scenario row");
        app.handle_menu_input(|menu| menu.select_list_index(index))
            .expect("select Old scenario");
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start inline rename");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("abort inline rename");
        assert!(old_path.exists());
        assert!(app.menu_state.rename_edit.is_none());
        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("restart inline rename from search focus");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("focused rename edit owns bare Delete");
        assert_eq!(
            app.menu_state
                .rename_edit
                .as_ref()
                .map(|rename| rename.edit.text()),
            Some("")
        );
        assert!(app.message_dialogs.is_empty());
        for character in "New Name".chars() {
            app.handle_text_input(character).expect("type new title");
        }
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("commit inline rename");
        wait_for_scenario_selector_discovery(&mut app);

        let new_path = paths.scenario_dir().join("New Name.c4s");
        assert!(!old_path.exists());
        assert!(new_path.is_dir());
        assert_eq!(
            fs::read_to_string(new_path.join("Title.txt")).expect("read rewritten title"),
            "DE:New Name"
        );
        assert!(app.menu_state.rename_edit.is_none());
        assert_eq!(app.menu_state.dialog_focus(), ScenselDialogFocus::List);
        assert!(!app.menu_state.search_focused());
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| (entry.identifier.as_str(), entry.title.as_str())),
            Some(("New Name.c4s", "New Name"))
        );
        assert!(app.scenario_catalog.contains_key("New Name.c4s"));
        reset_cached_app_paths();
    }

    #[test]
    fn scenario_storage_renames_and_deletes_nested_packed_child() {
        let directory = tempdir().expect("packed scenario directory");
        let outer_path = directory.path().join("Campaign.c4f");
        let mut scenario = MutableGroup::new("Old.c4s");
        scenario
            .add_file("Scenario.txt", b"[Head]\nTitle=Old\n".to_vec())
            .expect("add scenario core");
        scenario
            .add_file("Title.txt", b"US:Old".to_vec())
            .expect("add old title");
        let mut chapter = MutableGroup::new("Chapter.c4f");
        chapter
            .add_child("Old.c4s", scenario)
            .expect("add scenario to nested chapter");
        let mut campaign = MutableGroup::new("Campaign.c4f");
        campaign
            .add_child("Chapter.c4f", chapter)
            .expect("add nested chapter");
        fs::write(&outer_path, campaign.pack().expect("pack campaign"))
            .expect("write campaign");

        assert_eq!(
            scenario_filename_from_title("Foo.c4s", ScenarioKind::Scenario, Path::new("Old.c4s")),
            "Fooc4s.c4s"
        );
        assert_eq!(
            scenario_filename_from_title(".!", ScenarioKind::Scenario, Path::new("Old.c4s")),
            "unnamed.c4s"
        );
        assert_eq!(
            scenario_filename_from_title("New Pack", ScenarioKind::Folder, Path::new("Old.c4f")),
            "New Pack.c4f"
        );
        assert_eq!(
            scenario_filename_from_title("New Dir", ScenarioKind::Folder, Path::new("OldDir")),
            "New Dir"
        );
        let renamed = rename_scenario_storage(
            &outer_path.join("Chapter.c4f/Old.c4s"),
            ScenarioKind::Scenario,
            "Packed New",
            "US",
        )
        .expect("rename nested packed scenario");
        assert_eq!(renamed, outer_path.join("Chapter.c4f/Packed New.c4s"));
        let campaign = Group::open(&outer_path).expect("open rewritten campaign");
        let chapter = campaign
            .open_child("Chapter.c4f")
            .expect("open rewritten chapter");
        assert!(!chapter.exists("Old.c4s"));
        let renamed_group = chapter
            .open_child("Packed New.c4s")
            .expect("open renamed packed child");
        assert_eq!(
            renamed_group.read_file("Title.txt").expect("read packed title"),
            b"US:Packed New"
        );

        delete_scenario_storage(&renamed).expect("delete nested packed scenario");
        let campaign = Group::open(&outer_path).expect("open deleted campaign");
        let chapter = campaign
            .open_child("Chapter.c4f")
            .expect("open deleted chapter");
        assert!(!chapter.exists("Packed New.c4s"));
    }

    #[test]
    fn scensel_rename_collision_is_modal_and_keeps_editor_and_storage() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated rename collision user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        for (filename, title) in [("Source.c4s", "Source"), ("Taken.c4s", "Taken")]
        {
            let path = paths.scenario_dir().join(filename);
            fs::create_dir_all(&path).expect("create collision scenario");
            fs::write(
                path.join("Scenario.txt"),
                format!("[Head]\nTitle={title}\n"),
            )
            .expect("write collision scenario");
        }
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        let index = app
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == "Source.c4s")
            .expect("source row");
        app.handle_menu_input(|menu| menu.select_list_index(index))
            .expect("select source");
        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("start collision rename");
        for character in "Taken".chars() {
            app.handle_text_input(character)
                .expect("type colliding title");
        }
        app.handle_key(VirtualKeyCode::Return, ElementState::Pressed)
            .expect("collision is handled by modal");

        assert!(paths.scenario_dir().join("Source.c4s").exists());
        assert!(paths.scenario_dir().join("Taken.c4s").exists());
        let rename = app
            .menu_state
            .rename_edit
            .as_ref()
            .expect("invalid rename editor remains");
        assert!(rename.edit.is_focused());
        assert_eq!(rename.edit.selected_text(), Some("Taken"));
        assert_eq!(app.message_dialogs.len(), 1);
        assert_eq!(
            app.message_dialogs[0].state.caption(),
            "Rename failure"
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)
            .expect("dismiss rename failure");
        let rename = app
            .menu_state
            .rename_edit
            .as_ref()
            .expect("invalid rename resumes after its error modal");
        assert!(rename.edit.is_focused());
        assert_eq!(rename.edit.selected_text(), Some("Taken"));
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_delete_confirms_exact_subject_deletes_and_selects_next() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated delete user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        for (filename, title) in [("A.c4s", "Alpha"), ("B.c4s", "Beta"), ("C.c4s", "Gamma")] {
            let path = paths.scenario_dir().join(filename);
            fs::create_dir_all(&path).expect("create delete scenario");
            fs::write(
                path.join("Scenario.txt"),
                format!("[Head]\nTitle={title}\n"),
            )
            .expect("write delete scenario");
        }
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        let index = app
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == "B.c4s")
            .expect("Beta row");
        app.handle_menu_input(|menu| menu.select_list_index(index))
            .expect("select Beta");
        app.menu_state.set_search_text("pending query that excludes Gamma");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("open scenario delete confirmation");
        assert_eq!(
            app.message_dialogs
                .last()
                .expect("delete confirmation")
                .state
                .message(),
            "Delete Scenario Beta?"
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
            .expect("confirm scenario deletion");
        wait_for_scenario_selector_discovery(&mut app);
        assert!(!paths.scenario_dir().join("B.c4s").exists());
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("C.c4s")
        );
        assert_eq!(
            app.menu_state.search_text(),
            "pending query that excludes Gamma"
        );
        assert!(!app.scenario_catalog.contains_key("B.c4s"));
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_delete_uses_original_group_warning() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated original warning user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let original_path = paths.scenario_dir().join("Original.c4s");
        fs::create_dir_all(paths.scenario_dir()).expect("create scenario directory");
        let mut original = MutableGroup::new("Original.c4s");
        original.make_original(true);
        original
            .add_file("Scenario.txt", b"[Head]\nTitle=Original\n".to_vec())
            .expect("add original scenario core");
        fs::write(&original_path, original.pack().expect("pack original scenario"))
            .expect("write original scenario");

        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        let index = app
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == "Original.c4s")
            .expect("original scenario row");
        app.handle_menu_input(|menu| menu.select_list_index(index))
            .expect("select original scenario");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("open original delete warning");
        assert_eq!(
            app.message_dialogs
                .last()
                .expect("original warning")
                .state
                .message(),
            "Scenario Original is an original file. Are your sure you want to delete it?"
        );
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::No)
            .expect("decline original deletion");
        assert!(original_path.exists());
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_delete_failure_is_nonfatal_and_keeps_row_selected() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated delete failure user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let scenario_path = paths.scenario_dir().join("Failure.c4s");
        fs::create_dir_all(&scenario_path).expect("create failing delete scenario");
        fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Failure\n",
        )
        .expect("write failing delete scenario");
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        let index = app
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == "Failure.c4s")
            .expect("failure scenario row");
        app.handle_menu_input(|menu| menu.select_list_index(index))
            .expect("select failure scenario");
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("open failing delete confirmation");
        fs::remove_dir_all(&scenario_path).expect("make confirmed deletion fail");
        app.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Yes)
            .expect("delete failure is handled");

        let failure = app.message_dialogs.last().expect("delete failure modal");
        assert_eq!(failure.state.caption(), "Delete");
        assert_eq!(failure.state.message(), "Delete failure.");
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("Failure.c4s")
        );
        assert!(app.scenario_catalog.contains_key("Failure.c4s"));
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_alt_m_updates_shared_and_persisted_mission_access() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("isolated mission access user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_network_host_scenario_browser();
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set Alt modifier");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("open Mission Access dialog");
        let dialog = app
            .game_option_input_dialog
            .as_ref()
            .expect("Mission Access dialog");
        assert_eq!(dialog.purpose, PendingInputDialogPurpose::ScenarioMissionAccess);
        assert_eq!(dialog.controller.caption(), "Mission Access");
        assert_eq!(dialog.controller.message(), "Enter mission password:");
        assert_eq!(dialog.controller.icon(), InputDialogIcon::OPTIONS);
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
            "Secret;Second".to_string(),
        )])
        .expect("grant mission access");
        wait_for_scenario_selector_discovery(&mut app);
        assert_eq!(app.mission_access.snapshot(), "Secret;Second");
        assert_eq!(
            load_configured_mission_access(&paths).expect("load persisted mission access"),
            "Secret;Second"
        );

        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("reopen Mission Access dialog");
        app.process_game_option_input_dialog_actions(vec![InputDialogAction::Accepted(
            "-secret".to_string(),
        )])
        .expect("remove mission access case-insensitively");
        wait_for_scenario_selector_discovery(&mut app);
        assert_eq!(app.mission_access.snapshot(), "Second");
        assert_eq!(
            load_configured_mission_access(&paths).expect("load updated mission access"),
            "Second"
        );
        reset_cached_app_paths();
    }

    #[test]
    fn scensel_search_context_routes_pointer_apps_focus_and_release_capture() {
        let _lock = env_lock().lock();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_LANGUAGE", Some(Path::new("US"))),
        ]);
        let paths = AppPaths::discover().expect("discover repository install");
        let mut app = GameApp::new(
            800,
            600,
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
                player_name: "Search Context Tester".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_scenario_browser();

        let fonts = app.assets.clonk_fonts.clone().expect("classic fonts");
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, &fonts);
        let label_point = PhysicalPosition::new(
            f64::from(layout.search_label.x + layout.search_label.w / 2),
            f64::from(layout.search_label.y + layout.search_label.h / 2),
        );
        app.handle_cursor_moved(label_point)
            .expect("hover Search label");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("right-down on label");
        assert!(
            app.context_menu.is_none(),
            "wooden label has no edit context"
        );

        app.menu_state.set_search_text("alpha beta");
        app.menu_state.search_edit.anchor = 0;
        app.menu_state.search_edit.caret = 5;
        assert!(!app.menu_state.search_focused());
        let edit_point = PhysicalPosition::new(
            f64::from(layout.search_edit.x + 4),
            f64::from(layout.search_edit.y + layout.search_edit.h / 2),
        );
        app.handle_cursor_moved(edit_point)
            .expect("hover search edit");
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
            .expect("Clear entry");
        app.handle_right_mouse_button(ElementState::Pressed)
            .expect("open edit context");
        let popup = app.context_menu.as_ref().expect("edit context");
        assert_eq!(popup.pointer_position(), anchor);
        assert_eq!(popup.layout().panels[0].rows.len(), expected_entries.len());
        assert_eq!(app.menu_state.search_edit.selected_text(), Some("alpha"));
        assert!(
            !app.menu_state.search_focused(),
            "right-down does not focus edit"
        );
        app.handle_right_mouse_button(ElementState::Released)
            .expect("release opening button into popup");

        let popup_margin = app
            .context_menu
            .as_ref()
            .expect("edit context")
            .layout()
            .panels[0]
            .bounds;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(popup_margin.x + 1),
            f64::from(popup_margin.y + 1),
        ))
        .expect("hover popup margin");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("press popup margin");
        assert_eq!(
            app.context_menu_pointer_capture,
            Some(ContextMenuPointerButton::Left)
        );
        app.pointer_left().expect("process cursor exit");
        assert_eq!(
            app.context_menu_pointer_capture,
            Some(ContextMenuPointerButton::Left),
            "an open popup retains capture across CursorLeft"
        );
        app.handle_cursor_moved(PhysicalPosition::new(0.0, 0.0))
            .expect("re-enter outside popup");
        app.handle_mouse_button(ElementState::Released)
            .expect("consume popup-margin release outside popup");
        assert!(app.context_menu.is_some());
        assert_eq!(app.context_menu_pointer_capture, None);

        let clear = app
            .context_menu
            .as_ref()
            .expect("edit context")
            .layout()
            .panels[0]
            .rows[clear_index]
            .rect;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(clear.x + 1),
            f64::from(clear.y + 1),
        ))
        .expect("hover Clear");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("activate Clear on left-down");
        assert!(app.context_menu.is_none());
        assert_eq!(app.menu_state.search_text(), " beta");
        assert_eq!(app.menu_state.applied_search_text, "");
        app.handle_mouse_button(ElementState::Released)
            .expect("captured activation release");
        assert!(
            !app.menu_state.search_focused(),
            "activation release must not click the underlying edit"
        );
        assert_eq!(app.context_menu_pointer_capture, None);

        app.menu_state.set_search_focused(true);
        app.menu_state.search_edit.anchor = app.menu_state.search_edit.caret;
        let before = app.menu_state.search_text().to_string();
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("open context from Apps key");
        let expected_center = GuiPoint::new(
            (layout.search_edit.x + layout.search_edit.w / 2) as f32,
            (layout.search_edit.y + layout.search_edit.h / 2) as f32,
        );
        assert_eq!(
            app.context_menu
                .as_ref()
                .expect("Apps context")
                .pointer_position(),
            expected_center
        );
        app.handle_text_input('Z')
            .expect("text is suppressed by context");
        app.handle_modifiers_changed(ModifiersState::CTRL)
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::A, ElementState::Pressed)
            .expect("Ctrl+A is suppressed by context");
        assert_eq!(app.menu_state.search_text(), before);
        assert!(app.menu_state.search_edit.selection_range().is_none());
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("set keyboard modifiers");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Pressed)
            .expect("close Apps context");
        app.handle_key(VirtualKeyCode::Escape, ElementState::Released)
            .expect("swallow closing key release");
        assert!(app.context_menu.is_none());
        assert!(app.menu_state.search_focused(), "logical focus is retained");

        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("reopen Apps context for lost-release regression");
        let select_all = app
            .context_menu
            .as_ref()
            .expect("Apps context")
            .layout()
            .panels[0]
            .rows
            .last()
            .expect("Select all row")
            .rect;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(select_all.x + 1),
            f64::from(select_all.y + 1),
        ))
        .expect("hover Select all");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("activate Select all without a matching release");
        assert!(app.context_menu.is_none());
        assert_eq!(
            app.context_menu_pointer_capture,
            Some(ContextMenuPointerButton::Left)
        );
        app.pointer_left().expect("process cursor exit");
        assert_eq!(app.context_menu_pointer_capture, None);

        app.handle_cursor_moved(edit_point)
            .expect("return pointer to search edit");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("begin fresh search click after lost release");
        assert!(app.menu_state.search_edit.dragging);
        app.handle_mouse_button(ElementState::Released)
            .expect("finish fresh search click after lost release");
        assert!(
            !app.menu_state.search_edit.dragging,
            "stale context capture must not swallow a later release"
        );

        app.menu_state.set_search_focused(false);
        app.handle_key(VirtualKeyCode::Apps, ElementState::Pressed)
            .expect("ignore Apps without edit focus");
        assert!(app.context_menu.is_none());

        let empty_entries = scensel_search_context_entries(&SearchEditState::default(), false);
        app.open_context_menu_at(empty_entries, expected_center)
            .expect("open empty classic edit context");
        let empty = app.context_menu.as_ref().expect("empty context").layout();
        assert!(empty.panels[0].rows.is_empty());
        assert_eq!(
            (empty.panels[0].bounds.w, empty.panels[0].bounds.h),
            (40, 7)
        );
        app.close_context_menu_silently();

        let assets = app.assets.scensel_assets().expect("scenario assets");
        let button_down = app
            .assets
            .dialog_image("GUIButtonDown.png")
            .expect("scenario button-down plank");
        let book = app.assets.book_fonts.clone().expect("book fonts");
        app.menu_state.set_search_text("caret");
        app.menu_state.set_search_focused(true);
        app.menu_state.search_edit.anchor = app.menu_state.search_edit.caret;
        let mut focused = Surface::new(800, 600, PixelFormat::Rgba8888);
        let mut suppressed = Surface::new(800, 600, PixelFormat::Rgba8888);
        draw_scensel_dynamic(
            &mut focused,
            &mut app.menu_state,
            &app.scenario_entry_enabled,
            &assets,
            &button_down,
            &fonts,
            &book,
            None,
            startup_gamma(),
            true,
        )
        .expect("draw focused selector");
        draw_scensel_dynamic(
            &mut suppressed,
            &mut app.menu_state,
            &app.scenario_entry_enabled,
            &assets,
            &button_down,
            &fonts,
            &book,
            None,
            startup_gamma(),
            false,
        )
        .expect("draw inactive selector");
        assert!(focused.pixels() != suppressed.pixels());
        reset_cached_app_paths();
    }

    // Wheel input is hit-tested to the right-page ScrollWindow and one SDL
    // notch advances 60 logical pixels regardless of output scale
    // (C4FullScreen.cpp:408; C4GuiContainers.cpp:612-620).
    #[test]
    fn scensel_description_wheel_scrolls_and_clamps() {
        let _lock = env_lock().lock();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");
        let user_data = tempdir().expect("isolated user data");
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(repository)),
            ("LC_USER_DATA_DIR", Some(user_data.path())),
            ("LC_LANGUAGE", Some(Path::new("US"))),
        ]);
        let paths = AppPaths::discover().expect("discover repository install");
        let mut app = GameApp::new(
            800,
            600,
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
                player_name: "Scroll Tester".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app");
        wait_for_menu(&mut app);
        app.open_scenario_browser();
        app.menu_state.stack[0].entries[0].description = Some(
            (0..100)
                .map(|index| format!("long description line {index}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        app.menu_state.refresh_menu_entries();
        let _ = app.menu_state.select_default_entry();
        let fonts = app.assets.clonk_fonts.as_deref().expect("classic fonts");
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(800, 600, fonts);
        app.menu_state.set_pointer_position(Some(GuiPoint::new(
            (layout.selection_info.x + 10) as f32,
            (layout.selection_info.y + 10) as f32,
        )));

        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("scroll one line");
        assert_eq!(app.menu_state.selection_info_scroll, 60);

        app.menu_state.selection_info_scroll = 0;
        app.handle_mouse_wheel(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -180.0)),
            3.0,
        )
        .expect("scroll physical pixels");
        assert_eq!(app.menu_state.selection_info_scroll, 60);

        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, 100.0), 1.0)
            .expect("scroll to top");
        assert_eq!(app.menu_state.selection_info_scroll, 0);

        app.menu_state
            .set_pointer_position(Some(GuiPoint::new(0.0, 0.0)));
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("ignore wheel outside description");
        assert_eq!(app.menu_state.selection_info_scroll, 0);

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
        app.handle_mouse_wheel(MouseScrollDelta::LineDelta(0.0, -1.0), 1.0)
            .expect("scroll scenario list");
        assert_eq!(app.menu_state.scenario_list_scroll(), 60);
        let mut scrolled_frame = vec![0_u8; 800 * 600 * 4];
        app.render(&mut scrolled_frame)
            .expect("render deliberately scrolled list");
        assert_eq!(
            app.menu_state.scenario_list_scroll(),
            60,
            "rendering must not snap a manually scrolled list back to its unchanged selection"
        );

        let book_fonts = app.assets.book_fonts.clone().expect("book fonts");
        let item_height = clonk_frontend::startup_scensel::scen_list_item_height(&book_fonts.text);
        let click = PhysicalPosition::new(
            f64::from(layout.list.x + 8),
            f64::from(layout.list.y + 3 + item_height / 2),
        );
        app.handle_cursor_moved(click)
            .expect("point at scrolled row");
        app.handle_mouse_button(ElementState::Released)
            .expect("select scrolled row");
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("scroll_02")
        );

        // Pressing the list track jumps the fixed pin under the pointer and
        // captures subsequent motion even outside the scrollbar.
        app.menu_state.scenario_list_scroll = 0;
        let list_bar = layout.list_scrollbar;
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(list_bar.x + 8),
            f64::from(list_bar.y + list_bar.h - 24),
        ))
        .expect("point at list track bottom");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("jump list thumb");
        let list_max_scroll = app
            .menu_state
            .scenario_list_max_scroll(layout.list.h - 6, item_height + 1);
        assert_eq!(app.menu_state.scenario_list_scroll(), list_max_scroll);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(list_bar.x - 200),
            f64::from(list_bar.y - 200),
        ))
        .expect("drag list thumb outside bar");
        assert_eq!(app.menu_state.scenario_list_scroll(), 0);
        app.handle_mouse_button(ElementState::Released)
            .expect("release captured list thumb");
        assert!(app.menu_state.scrollbar_interaction.is_none());

        // Held arrows advance their persistent bar position by one on every
        // startup draw/update instead of applying a row-sized scroll.
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(list_bar.x + 8),
            f64::from(list_bar.y + list_bar.h - 8),
        ))
        .expect("point at list bottom arrow");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("hold list bottom arrow");
        app.update().expect("first held-arrow frame");
        assert!(matches!(
            app.menu_state.scrollbar_interaction,
            Some(ScenselScrollbarInteraction {
                kind: ScenselScrollbarInteractionKind::Arrow(1),
                pin: 1,
                ..
            })
        ));
        assert!(app.menu_state.scenario_list_scroll() > 0);
        app.update().expect("second held-arrow frame");
        assert!(matches!(
            app.menu_state.scrollbar_interaction,
            Some(ScenselScrollbarInteraction { pin: 2, .. })
        ));
        app.handle_mouse_button(ElementState::Released)
            .expect("release list arrow");

        // The right description page uses the same captured fixed-thumb
        // interaction, not just wheel scrolling.
        let description_bar = clonk_frontend::startup_scensel::selection_info_scrollbar_rect(&layout);
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(description_bar.x + 8),
            f64::from(description_bar.y + description_bar.h - 24),
        ))
        .expect("point at description track bottom");
        app.handle_mouse_button(ElementState::Pressed)
            .expect("jump description thumb");
        let description_metrics = {
            let info = scensel_selection_info(&app.menu_state);
            clonk_frontend::startup_scensel::selection_info_scroll_metrics(
                &layout,
                book_fonts.as_ref(),
                &info,
            )
        };
        assert_eq!(
            app.menu_state.selection_info_scroll,
            description_metrics.max_scroll
        );
        app.handle_cursor_moved(PhysicalPosition::new(
            f64::from(description_bar.x + 200),
            f64::from(description_bar.y - 200),
        ))
        .expect("drag description thumb outside bar");
        assert_eq!(app.menu_state.selection_info_scroll, 0);
        app.handle_mouse_button(ElementState::Released)
            .expect("release description thumb");
        reset_cached_app_paths();
    }

    // C4GUI::ScrollBar keeps a fixed 16px pin between two 16px arrows.
    // Offset<->pin conversion uses integer truncation, while a track press
    // centers the pin under the pointer and begins a captured drag
    // (C4GuiContainers.cpp:343-473).
    #[test]
    fn scensel_fixed_scrollbar_geometry_matches_cpp() {
        assert_eq!(scensel_scrollbar_pin_travel(48), None);
        assert_eq!(scensel_scrollbar_pin_travel(49), Some(1));
        assert_eq!(scensel_scrollbar_pin_travel(100), Some(52));
        assert_eq!(scensel_scrollbar_pin_from_offset(0, 101, 100), Some(0));
        assert_eq!(scensel_scrollbar_pin_from_offset(50, 101, 100), Some(25));
        assert_eq!(scensel_scrollbar_pin_from_offset(101, 101, 100), Some(52));
        assert_eq!(scensel_scrollbar_offset_from_pin(0, 101, 100), Some(0));
        assert_eq!(scensel_scrollbar_offset_from_pin(26, 101, 100), Some(50));
        assert_eq!(scensel_scrollbar_offset_from_pin(52, 101, 100), Some(101));
        assert_eq!(scensel_scrollbar_jump_pin(-50, 100), Some(0));
        assert_eq!(scensel_scrollbar_jump_pin(50, 100), Some(26));
        assert_eq!(scensel_scrollbar_jump_pin(500, 100), Some(52));
        assert_eq!(scensel_scrollbar_offset_from_pin(0, 0, 100), None);
    }

    // C4GUI::ListBox::SelectEntry calls ScrollRangeInView so keyboard
    // selection remains visible, clamped against the complete item height
    // (C4GuiListBox.cpp:179-193; C4GuiContainers.cpp:549-582).
    #[test]
    fn scensel_list_scroll_keeps_selection_in_view() {
        let scenarios = (0..20)
            .map(|index| {
                let mut entry = FrontendScenario::fallback();
                entry.identifier = format!("scenario_{index:02}");
                entry.title = format!("Scenario {index:02}");
                entry
            })
            .collect::<Vec<_>>();
        let entries = build_menu_entries(&scenarios, false);
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.set_include_back(false);

        let _ = state
            .menu()
            .select_entry_by_index(19)
            .expect("select final row");
        state.ensure_list_selection_visible(100, 27, 26);
        assert_eq!(state.scenario_list_scroll(), 439);

        assert!(state.scroll_scenario_list_by(-60, 100, 27));
        assert_eq!(state.scenario_list_scroll(), 379);

        let _ = state
            .menu()
            .select_entry_by_index(0)
            .expect("select first row");
        state.ensure_list_selection_visible(100, 27, 26);
        assert_eq!(state.scenario_list_scroll(), 0);
    }

    #[test]
    fn scensel_list_keys_stop_at_ends_and_page_by_visible_rows() {
        let scenarios = (0..10)
            .map(|index| {
                let mut entry = FrontendScenario::fallback();
                entry.identifier = format!("scenario_{index:02}");
                entry.title = format!("Scenario {index:02}");
                entry
            })
            .collect::<Vec<_>>();
        let entries = build_menu_entries(&scenarios, false);
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.set_include_back(false);
        let _ = state.select_default_entry();

        assert!(state.move_list_selection_clamped(-1).is_empty());
        assert_eq!(state.menu.selected_index(), Some(0));
        let _ = state.select_list_end();
        assert_eq!(state.menu.selected_index(), Some(9));
        assert!(state.move_list_selection_clamped(1).is_empty());
        assert_eq!(state.menu.selected_index(), Some(9));
        let _ = state.select_list_home();

        // With 26px rows, 1px spacing and a 100px viewport, rows 0..=2
        // are fully visible. PageDown chooses row 2; another PageDown first
        // scrolls one viewport and chooses the last fully visible row 6.
        assert!(!state.page_list_selection(1, 100, 27, 26).is_empty());
        assert_eq!(state.menu.selected_index(), Some(2));
        assert_eq!(state.scenario_list_scroll(), 0);
        assert!(!state.page_list_selection(1, 100, 27, 26).is_empty());
        assert_eq!(state.menu.selected_index(), Some(6));
        assert_eq!(state.scenario_list_scroll(), 100);

        assert!(!state.page_list_selection(-1, 100, 27, 26).is_empty());
        assert_eq!(state.menu.selected_index(), Some(4));
        assert!(!state.page_list_selection(-1, 100, 27, 26).is_empty());
        assert_eq!(state.menu.selected_index(), Some(0));
        assert_eq!(state.scenario_list_scroll(), 0);
    }

    #[test]
    fn l057_scensel_typeahead_cycles_only_with_list_focus() {
        let scenarios = ["Thomas", "Ada", "tina", "Tori"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| {
                let mut entry = FrontendScenario::fallback();
                entry.identifier = format!("scenario_{index}");
                entry.title = title.to_string();
                entry
            })
            .collect::<Vec<_>>();
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("typeahead scenario menu");
        let mut app = new_menu_app(800, 600);
        app.menu_state = MenuState::new(menu, scenarios);
        app.open_scenario_browser();
        app.ui_sound_log.clear();

        for (character, expected) in [('T', 2), ('T', 3), ('t', 0), ('T', 2)] {
            let sound_count = app.ui_sound_log.len();
            app.handle_text_input(character)
                .expect("route scenario list character");
            assert_eq!(app.menu_state.menu.selected_index(), Some(expected));
            assert_eq!(app.ui_sound_log.len(), sound_count + 1);
            assert_eq!(app.ui_sound_log.last().map(String::as_str), Some("Command"));
        }

        let sound_count = app.ui_sound_log.len();
        app.handle_text_input('x').expect("unmatched list character");
        assert_eq!(app.menu_state.menu.selected_index(), Some(2));
        assert_eq!(app.ui_sound_log.len(), sound_count);

        app.menu_state.set_search_text("");
        app.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        app.handle_text_input('T').expect("type into scenario search");
        assert_eq!(app.menu_state.search_text(), "T");
        assert_eq!(app.menu_state.menu.selected_index(), Some(2));
        assert_eq!(app.ui_sound_log.len(), sound_count);

        app.set_scensel_dialog_focus(ScenselDialogFocus::Back);
        app.handle_text_input('T')
            .expect("character outside scenario list focus");
        assert_eq!(app.menu_state.search_text(), "T");
        assert_eq!(app.menu_state.menu.selected_index(), Some(2));
        assert_eq!(app.ui_sound_log.len(), sound_count);
    }

    #[test]
    fn l057_window_keys_map_to_shared_list_navigation_codes() {
        for (window_key, gui_key) in [
            (VirtualKeyCode::Home, KeyCode::Home),
            (VirtualKeyCode::End, KeyCode::End),
            (VirtualKeyCode::PageUp, KeyCode::PageUp),
            (VirtualKeyCode::PageDown, KeyCode::PageDown),
        ] {
            assert_eq!(map_key_code(window_key), Some(gui_key));
        }
    }

    // Selected-row -> scenario mapping honours the Back-row offset used by
    // the network lobby list.
    #[test]
    fn selected_scenario_maps_through_back_row_offset() {
        let scenarios = sample_scenarios();
        let entries = build_menu_entries(&scenarios, true);
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.menu().resize(1280.0, 720.0);

        let _ = state.menu().select_entry_by_index(0); // Back row
        assert!(state.selected_scenario().is_none());
        let _ = state.menu().select_entry_by_index(1);
        assert_eq!(
            state.selected_scenario().map(|entry| entry.title.as_str()),
            Some("Missions")
        );
    }

    // Caption above the list: current folder name, "Scenarios" at root
    // (C4StartupScenSelDlg::UpdateList, cpp:1527-1535); with no selection
    // the right page falls back to the listed folder (cpp:1566-1572).
    #[test]
    fn book_caption_and_folder_fallback_track_the_stack() {
        let scenarios = sample_scenarios();
        let entries = build_menu_entries(&scenarios, false);
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);
        state.menu().resize(1280.0, 720.0);
        state.set_include_back(false);

        assert_eq!(state.book_caption(), "Scenarios");
        assert!(state.current_folder().is_none(), "root has no folder info");

        state.enter_folder("folder_missions");
        assert_eq!(state.book_caption(), "Missions");
        assert_eq!(
            state.current_folder().map(|folder| folder.title.as_str()),
            Some("Missions")
        );

        state.leave_folder();
        assert_eq!(state.book_caption(), "Scenarios");
    }

    // List icon defaults (C4StartupScenSelDlg.cpp:705-710,951-952,1036-1037):
    // scenario Icon= clamped to the 52-icon strip else 14; .c4f folder 0;
    // plain directory 44.
    #[test]
    fn scensel_entry_icons_follow_cpp_defaults() {
        let mut scenario = FrontendScenario::fallback();
        scenario.kind = ScenarioKind::Scenario;
        scenario.icon_index = Some(15);
        assert_eq!(scensel_entry_icon(&scenario), 15);
        scenario.icon_index = Some(99);
        assert_eq!(scensel_entry_icon(&scenario), 14);
        scenario.icon_index = None;
        assert_eq!(scensel_entry_icon(&scenario), 14);

        let mut folder = FrontendScenario::fallback();
        folder.kind = ScenarioKind::Folder;
        folder.path = Some(PathBuf::from("/tmp/Fantasy.c4f"));
        assert_eq!(scensel_entry_icon(&folder), 0);
        folder.path = Some(PathBuf::from("/tmp/Downloads"));
        assert_eq!(scensel_entry_icon(&folder), 44);
    }

    fn map_test_scenario(folder: &Path, filename: &str, title: &str) -> FrontendScenario {
        let mut scenario = FrontendScenario::fallback();
        scenario.identifier = format!("Map.c4f/{filename}");
        scenario.title = title.to_string();
        scenario.description = Some(format!("Description for {title}"));
        scenario.path = Some(folder.join(filename));
        scenario
    }

    fn map_test_folder(path: &Path, children: Vec<FrontendScenario>) -> FrontendScenario {
        let mut folder = FrontendScenario::fallback();
        folder.identifier = "Map.c4f".to_string();
        folder.title = "Map Folder".to_string();
        folder.kind = ScenarioKind::Folder;
        folder.is_playable = false;
        folder.path = Some(path.to_path_buf());
        folder.children = children;
        folder
    }

    fn open_map_test_folder(app: &mut GameApp, folder: FrontendScenario) {
        let scenarios = vec![folder.clone()];
        let menu = StartupMenu::new(build_menu_entries(&scenarios, false), test_font(), None)
            .expect("FolderMap menu");
        app.menu_state = MenuState::new(menu, scenarios.clone());
        app.scenario_catalog = build_scenario_catalog(&scenarios);
        app.open_scenario_browser();
        app.handle_menu_input(|_| {
            vec![StartupMenuAction::OpenEntry(clonk_frontend::ScenarioSummary {
                identifier: folder.identifier.clone(),
                title: folder.title.clone(),
                kind: ScenarioKind::Folder,
            })]
        })
        .expect("FolderMap folder activation");
    }

    #[test]
    fn folder_map_f5_refresh_preserves_map_and_book_only_shortcuts() {
        let _lock = env_lock().lock();
        let user_data = tempdir().expect("FolderMap refresh user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        let map_path = paths.scenario_dir().join("Map.c4f");
        let alpha_path = map_path.join("Alpha.c4s");
        fs::create_dir_all(&alpha_path).expect("initial map scenario");
        fs::write(map_path.join("Folder.txt"), "[Head]\nIndex=1\n")
            .expect("map folder core");
        fs::write(
            alpha_path.join("Scenario.txt"),
            "[Head]\nTitle=Alpha Mission\n",
        )
        .expect("initial map scenario core");
        write_map_png(
            &map_path.join("FolderMap.png"),
            16,
            8,
            [20, 30, 40, 255],
        );
        fs::write(
            map_path.join("FolderMap.txt"),
            "[FolderMap]\n    [Scenario]\n    File=Alpha.c4s\n    Area=0,0,8,8\n    [Scenario]\n    File=Beta.c4s\n    Area=8,0,8,8\n",
        )
        .expect("map data");

        let mut app = new_menu_app_with_paths(800, 600, &paths);
        app.open_scenario_browser();
        app.enter_scenario_folder("Map.c4f");
        assert!(app.menu_state.current_map().is_some());

        let select_alpha = app
            .menu_state
            .activate_map_button(0)
            .expect("select Alpha map button");
        app.handle_menu_input(move |_| vec![select_alpha])
            .expect("apply Alpha selection");
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some("Map.c4f/Alpha.c4s")
        );

        app.handle_key(VirtualKeyCode::F2, ElementState::Pressed)
            .expect("map view has no inline rename shortcut");
        assert!(app.menu_state.rename_edit.is_none());
        app.handle_key(VirtualKeyCode::Delete, ElementState::Pressed)
            .expect("map view has no list delete shortcut");
        assert!(app.message_dialogs.is_empty());
        app.handle_modifiers_changed(ModifiersState::ALT)
            .expect("set map shortcut modifier");
        app.handle_key(VirtualKeyCode::M, ElementState::Pressed)
            .expect("map view has no Mission Access shortcut");
        assert!(app.game_option_input_dialog.is_none());
        app.handle_modifiers_changed(ModifiersState::empty())
            .expect("clear map shortcut modifier");

        let beta_path = map_path.join("Beta.c4s");
        fs::create_dir_all(&beta_path).expect("refreshed map scenario");
        fs::write(
            beta_path.join("Scenario.txt"),
            "[Head]\nTitle=Beta Mission\n",
        )
        .expect("refreshed map scenario core");
        app.handle_key(VirtualKeyCode::F5, ElementState::Pressed)
            .expect("refresh active map folder");
        wait_for_scenario_selector_discovery(&mut app);

        assert_eq!(
            app.menu_state
                .current_folder()
                .map(|folder| folder.identifier.as_str()),
            Some("Map.c4f")
        );
        let map = app
            .menu_state
            .current_map()
            .expect("F5 preserves FolderMap style");
        assert!(
            map.selected_entry().is_none(),
            "F5 rebuild clears the visible map selection"
        );
        assert!(map.scenarios.iter().any(|button| {
            button
                .entry
                .as_ref()
                .is_some_and(|entry| entry.identifier == "Map.c4f/Beta.c4s")
        }));
        assert!(app.scenario_catalog.contains_key("Map.c4f/Beta.c4s"));
        reset_cached_app_paths();
    }

    #[test]
    fn folder_map_disabled_opens_a_normal_book_without_inspecting_the_marker() {
        let root = tempdir().expect("FolderMap fixture");
        let map_path = root.path().join("Map.c4f");
        fs::create_dir(&map_path).expect("map folder");
        fs::write(
            map_path.join("FolderMap.txt"),
            "this is deliberately malformed and has no background",
        )
        .expect("malformed FolderMap marker");
        let alpha = map_test_scenario(&map_path, "Alpha.c4s", "Alpha");
        let folder = map_test_folder(&map_path, vec![alpha.clone()]);
        let mut app = new_menu_app(640, 480);
        app.show_folder_maps = false;

        open_map_test_folder(&mut app, folder.clone());

        assert_eq!(app.menu_state.stack.len(), 2);
        assert!(app.menu_state.current_map().is_none());
        assert_eq!(app.menu_state.visible_entries().len(), 1);
        assert_eq!(
            app.menu_state
                .selected_scenario()
                .map(|entry| entry.identifier.as_str()),
            Some(alpha.identifier.as_str())
        );
        assert_eq!(app.mode, AppMode::Menu);
        assert_eq!(app.startup_view, StartupView::ScenarioBrowser);

        let mut invalid_map_app = new_menu_app(640, 480);
        open_map_test_folder(&mut invalid_map_app, folder);
        assert!(invalid_map_app.menu_state.current_map().is_none());
        assert_eq!(invalid_map_app.menu_state.visible_entries().len(), 1);
        assert_eq!(invalid_map_app.mode, AppMode::Menu);
    }

    #[test]
    fn folder_map_parser_honors_indentation_dedents_and_first_scalar() {
        let parsed = parse_map_folder(
            "[FolderMap]\nMinResX=640\nMinResX=999\n    [Other]\n        [Scenario]\n        File=Nested.c4s\n    [Scenario]\n    File=Direct.c4s\nMinResY=480\n",
        )
        .expect("parse indentation hierarchy");
        assert_eq!(parsed.min_res_x, 640);
        assert_eq!(parsed.min_res_y, 480);
        assert_eq!(parsed.scenarios.len(), 1);
        assert_eq!(parsed.scenarios[0].filename, "Direct.c4s");
    }

    #[test]
    fn folder_map_loads_renders_titles_access_overlays_and_cpp_click_semantics() {
        let root = tempdir().expect("FolderMap fixture");
        let map_path = root.path().join("Map.c4f");
        fs::create_dir(&map_path).expect("map folder");
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
        fs::write(map_path.join("StringTblUS.txt"), "PLAY=Play\n")
            .expect("FolderMap localization table");
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
        .expect("FolderMap data");
        let alpha = map_test_scenario(&map_path, "Alpha.c4s", "Alpha Mission");
        let beta = map_test_scenario(&map_path, "Beta.c4s", "Beta Mission");
        let folder = map_test_folder(&map_path, vec![alpha.clone(), beta.clone()]);
        let mut app = new_real_menu_app(640, 480);
        app.mission_access = MissionAccessStore::new("Other; mappass ");

        open_map_test_folder(&mut app, folder);

        let map = app.menu_state.current_map().expect("map view active");
        assert_eq!(map.source_path, map_path);
        assert_eq!(map.scenarios.len(), 3);
        assert_eq!(map.scenarios[0].title, "Play Alpha Mission");
        assert_eq!(map.scenarios[1].title, "Visit Beta Mission now");
        assert!(map.scenarios[2].entry.is_none());
        assert_eq!(map.scenarios[2].title, "<c ff0000>ERROR</c>");
        assert_eq!(map.access_overlays.len(), 2);
        assert_eq!(
            map.access_overlays[0]
                .image
                .as_ref()
                .expect("unconditional access image")
                .pixels()[..4],
            [0, 0, 80, 255]
        );
        assert_eq!(
            map.access_overlays[1]
                .image
                .as_ref()
                .expect("granted access image")
                .pixels()[..4],
            [0, 0, 120, 255]
        );

        let first = app
            .menu_state
            .activate_map_button(0)
            .expect("first Alpha click");
        assert!(matches!(
            &first,
            StartupMenuAction::SelectionChanged(summary)
                if summary.identifier == alpha.identifier
        ));
        app.process_menu_actions(vec![first])
            .expect("selection action");
        assert_eq!(
            scensel_selection_info(&app.menu_state).title,
            Some("Alpha Mission")
        );
        let second = app
            .menu_state
            .activate_map_button(0)
            .expect("second Alpha click");
        let (start, _) = app
            .process_menu_actions(vec![second])
            .expect("shared DoOK path");
        assert_eq!(start.as_deref(), Some(alpha.identifier.as_str()));

        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            640,
            480,
            app.assets.clonk_fonts.as_deref().expect("GUI fonts"),
        );
        let transform = MapFolderTransform::for_map(
            app.menu_state.current_map().expect("map remains active"),
            &layout,
            640,
            480,
        );
        let (background_x, background_y) = transform.point(50, 50);
        assert!(app.handle_scensel_map_pointer_down(GuiPoint::new(
            background_x as f32,
            background_y as f32,
        )));
        assert!(app
            .menu_state
            .current_map()
            .expect("map remains active")
            .selected_entry()
            .is_none());
        let single = app
            .menu_state
            .activate_map_button(1)
            .expect("single-click Beta");
        assert!(matches!(
            single,
            StartupMenuAction::StartScenario(summary)
                if summary.identifier == beta.identifier
        ));

        let (sample_x, sample_y) = transform.point(50, 50);
        let mut frame = vec![0_u8; 640 * 480 * 4];
        app.render(&mut frame).expect("render map view");
        let pixel = app
            .graphics
            .surface()
            .get_pixel(sample_x as u32, sample_y as u32)
            .expect("background sample");
        assert_eq!([pixel.r, pixel.g, pixel.b], [20, 30, 40]);
    }

    #[test]
    fn folder_map_hides_locked_button_until_access_is_granted() {
        let _lock = env_lock().lock();
        reset_cached_app_paths();
        let user_data = tempdir().expect("locked FolderMap user data");
        let (_guard, paths) = exact_loader_test_paths(user_data.path(), None);
        persist_config_value(&paths, "General", "MissionAccess", "OtherPass")
            .expect("configure unrelated mission access");
        let map_path = paths.scenario_dir().join("Map.c4f");
        fs::create_dir_all(&map_path).expect("map folder");
        write_map_png(&map_path.join("FolderMap.png"), 8, 8, [20, 30, 40, 255]);
        fs::write(
            map_path.join("FolderMap.txt"),
            "[FolderMap]\n    [Scenario]\n    File=Locked.c4s\n    SingleClick=1\n    Area=0,0,8,8\n",
        )
        .expect("FolderMap data");
        let locked_path = map_path.join("Locked.c4s");
        fs::create_dir(&locked_path).expect("locked scenario group");
        fs::write(
            locked_path.join("Scenario.txt"),
            "[Head]\nTitle=Locked Mission\nMissionAccess=MissingPass\n",
        )
        .expect("locked scenario core");
        let mut app = new_menu_app_with_paths(640, 480, &paths);
        app.open_scenario_browser();
        app.enter_scenario_folder("Map.c4f");

        assert_eq!(
            app.menu_state
                .current_map()
                .expect("map view active")
                .scenarios
                .len(),
            0,
            "a denied real scenario has no map button or base image"
        );

        app.apply_scenario_mission_access("MissingPass")
            .expect("grant, persist, and reload mission access");
        wait_for_scenario_selector_discovery(&mut app);
        assert_eq!(app.mission_access.snapshot(), "OtherPass;MissingPass");
        app.enter_scenario_folder("Map.c4f");
        assert_eq!(
            app.menu_state
                .current_map()
                .expect("map view restored")
                .scenarios
                .len(),
            1,
            "granting the module creates the map button on rebuild"
        );
        reset_cached_app_paths();
    }

    #[test]
    fn folder_map_minimum_resolution_and_image_failures_fall_back_to_book() {
        let root = tempdir().expect("FolderMap fixture");
        let map_path = root.path().join("Map.c4f");
        fs::create_dir(&map_path).expect("map folder");
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
            .expect("resolution map");
            assert_eq!(
                load_map_folder_data(
                    &folder,
                    640,
                    480,
                    &access,
                    &["US".to_string()],
                )
                .is_some(),
                loads,
                "minimum {min_x}x{min_y}"
            );
        }

        fs::write(
            map_path.join("FolderMap.txt"),
            "[FolderMap]\n    [AccessGfx]\n    Access=Never\n    OverlayImage=Missing.png\n",
        )
        .expect("missing-image map");
        assert!(
            load_map_folder_data(
                &folder,
                640,
                480,
                &access,
                &["US".to_string()],
            )
            .is_none(),
            "even an inaccessible named image is loaded before access filtering"
        );

        fs::write(
            map_path.join("FolderMap.txt"),
            "[FolderMap]\n    [Scenario]\n    File=Alpha.c4s\n    BaseImage=Missing.png\n",
        )
        .expect("denied scenario with missing image map");
        assert!(
            load_map_folder_data(
                &folder,
                640,
                480,
                &access,
                &["US".to_string()],
            )
            .is_none(),
            "a denied scenario image is still loaded before the button is filtered"
        );
    }

    #[test]
    fn extensionless_regular_folder_ignores_folder_map_marker() {
        let root = tempdir().expect("regular-folder fixture");
        let distant_subfolder = root.path().join("Distant.c4f");
        let regular_path = distant_subfolder.join("Regular");
        let scenario_path = regular_path.join("Child.c4s");
        fs::create_dir_all(&scenario_path).expect("regular folder and child scenario");
        fs::write(distant_subfolder.join("FolderMap.txt"), "[FolderMap]\n")
            .expect("non-contiguous SubFolder marker");
        fs::write(regular_path.join("FolderMap.txt"), "[FolderMap]\n")
            .expect("irrelevant regular-folder marker");

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
        let menu = StartupMenu::new(build_menu_entries(&entries, false), test_font(), None)
            .expect("regular-folder menu");
        let mut state = MenuState::new(menu, entries);
        state.enter_folder("Regular");

        assert!(!state.configure_current_folder_map(
            true,
            640,
            480,
            &MissionAccessStore::default(),
            &["US".to_string()],
        ));
        assert!(state.current_map().is_none());
        assert_eq!(state.current_entries()[0].identifier, child.identifier);
    }

    #[test]
    fn merged_folder_map_uses_a_later_contributing_group() {
        let root = tempdir().expect("merged FolderMap fixture");
        let first = root.path().join("first/Worlds.c4f");
        let later = root.path().join("later/Worlds.c4f");
        fs::create_dir_all(&first).expect("first contributing folder");
        fs::create_dir_all(&later).expect("later contributing folder");
        fs::write(later.join("FolderMap.txt"), "[FolderMap]\n")
            .expect("later FolderMap data");
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
        .expect("later contributing map loads");
        assert_eq!(map.source_path, later);
    }

    #[test]
    fn merged_scenario_keeps_first_root_mission_access_requirement() {
        let mut unlocked = FrontendScenario::fallback();
        unlocked.identifier = "Duplicate.c4s".to_string();
        let mut locked = unlocked.clone();
        locked.mission_access = Some("Secret".to_string());

        let user_first =
            merge_frontend_scenarios(vec![unlocked.clone(), locked.clone()], false);
        assert_eq!(user_first[0].mission_access, None);

        let install_first = merge_frontend_scenarios(vec![locked, unlocked], false);
        assert_eq!(install_first[0].mission_access.as_deref(), Some("Secret"));
    }

    #[test]
    fn folder_group_rewrite_restores_original_when_commit_fails() {
        let directory = tempdir().expect("folder-group root");
        let destination = directory.path().join("Local.c4p");
        fs::create_dir(&destination).expect("create original folder group");
        fs::write(destination.join("Player.txt"), b"old player")
            .expect("write original player");
        fs::write(destination.join("Keep.dat"), b"old sentinel")
            .expect("write original sentinel");
        fs::write(destination.join(".metadata"), b"hidden sentinel")
            .expect("write ignored metadata");

        let mut replacement = MutableGroup::new("Local.c4p");
        replacement
            .add_file("Player.txt", b"new player".to_vec())
            .expect("add replacement player");
        let source = Group::from_memory(
            destination.clone(),
            replacement.pack().expect("pack replacement"),
        )
        .expect("open replacement");
        let error = replace_directory_from_same_parent_with_hook(&source, &destination, || {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "synthetic commit failure",
            ))
        })
        .expect_err("commit failure must roll the original folder group back");
        assert!(format!("{error:#}").contains("synthetic commit failure"));
        assert_eq!(
            fs::read(destination.join("Player.txt")).unwrap(),
            b"old player"
        );
        assert_eq!(
            fs::read(destination.join("Keep.dat")).unwrap(),
            b"old sentinel"
        );
        assert_eq!(
            fs::read(destination.join(".metadata")).unwrap(),
            b"hidden sentinel"
        );
        assert!(fs::read_dir(directory.path())
            .expect("enumerate folder-group root")
            .all(|entry| !entry
                .expect("folder-group root entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".Local.c4p.lc-rewrite-")));
    }

    #[test]
    fn packed_group_rewrite_safely_replaces_an_existing_directory() {
        let directory = tempdir().expect("packed replacement root");
        let destination = directory.path().join("Copy.c4s");
        fs::create_dir(&destination).expect("create previous unpacked target");
        fs::write(destination.join("Old.txt"), b"old").expect("write previous target");
        let mut replacement = MutableGroup::new("Copy.c4s");
        replacement
            .add_file("Scenario.txt", b"new".to_vec())
            .expect("add replacement entry");

        persist_console_save_group(&replacement, &destination, false)
            .expect("replace directory with a packed group");
        assert!(destination.is_file());
        assert_eq!(
            Group::open(&destination)
                .expect("open packed replacement")
                .read_file("Scenario.txt")
                .unwrap(),
            b"new"
        );
    }

    #[cfg(unix)]
    #[test]
    fn folder_group_rewrite_never_follows_nested_directory_symlinks() {
        let directory = tempdir().expect("symlink replacement root");
        let destination = directory.path().join("Local.c4p");
        let external = directory.path().join("External.c4i");
        fs::create_dir(&destination).expect("create original profile");
        fs::create_dir(&external).expect("create external crew directory");
        fs::write(external.join("Outside.dat"), b"outside").expect("write external sentinel");
        std::os::unix::fs::symlink(&external, destination.join("Hero.c4i"))
            .expect("link original crew outside profile");

        let mut replacement = MutableGroup::new("Local.c4p");
        let mut hero = MutableGroup::new("Hero.c4i");
        hero.add_file("ObjectInfo.txt", b"new crew".to_vec())
            .expect("add replacement crew core");
        replacement
            .add_child("Hero.c4i", hero)
            .expect("add replacement crew");
        persist_console_save_group(&replacement, &destination, true)
            .expect("replace linked child inside staged profile");

        assert_eq!(fs::read(external.join("Outside.dat")).unwrap(), b"outside");
        assert!(!external.join("ObjectInfo.txt").exists());
        let saved_child = destination.join("Hero.c4i");
        assert!(fs::symlink_metadata(&saved_child)
            .expect("saved child metadata")
            .file_type()
            .is_dir());
        assert_eq!(
            fs::read(saved_child.join("ObjectInfo.txt")).unwrap(),
            b"new crew"
        );
    }

    #[cfg(unix)]
    #[test]
    fn folder_group_rewrite_preserves_a_root_directory_symlink() {
        let directory = tempdir().expect("root symlink replacement");
        let physical = directory.path().join("Physical.c4p");
        let linked = directory.path().join("Linked.c4p");
        fs::create_dir(&physical).expect("create physical profile");
        fs::write(physical.join("Player.txt"), b"old").expect("write physical profile");
        std::os::unix::fs::symlink(&physical, &linked).expect("link profile");
        let mut replacement = MutableGroup::new("Linked.c4p");
        replacement
            .add_file("Player.txt", b"new".to_vec())
            .expect("add replacement player");

        persist_console_save_group(&replacement, &linked, true)
            .expect("rewrite physical folder through its stable symlink");
        assert!(fs::symlink_metadata(&linked)
            .expect("linked profile metadata")
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(physical.join("Player.txt")).unwrap(), b"new");
    }

    #[test]
    fn cache_definition_icons_distinguishes_blank_from_malformed_picture() {
        let mut app = new_running_sandbox_app();
        let mut blank = Definition::from_script("BLNK", "Blank rule", "#strict 3\n")
            .expect("blank definition compiles");
        blank.set_category(C4D_RULE);
        app.engine
            .register_definition(blank)
            .expect("blank definition registers");
        let blank_entry = GoalRuleEntry {
            definition_id: "BLNK".to_string(),
            name: "Blank rule".to_string(),
            description: None,
            fulfilled: false,
        };
        app.cache_definition_icons(std::slice::from_ref(&blank_entry))
            .expect("a loaded definition with a blank facet is valid");
        assert!(
            !app
                .ingame_menu_gfx
                .as_ref()
                .expect("blank cache initializes menu graphics")
                .definition_icons
                .contains_key("BLNK")
        );

        let temp = tempdir().expect("tempdir");
        let valid_dir = temp.path().join("Valid.c4d");
        fs::create_dir(&valid_dir).expect("valid definition directory");
        fs::write(
            valid_dir.join("DefCore.txt"),
            b"[DefCore]\nid=PCTR\nPicture=0,0,1,1\n",
        )
        .expect("valid DefCore");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
            .save(valid_dir.join("Graphics.png"))
            .expect("valid preferred graphics");
        let valid_resource = ResourceDefinitionData::load(
            &Group::open(&valid_dir).expect("open valid definition"),
        )
        .expect("valid definition resource loads");
        let mut valid = Definition::from_resource(&valid_resource)
            .expect("valid definition converts to engine data");
        valid.set_category(C4D_RULE);
        app.engine
            .register_definition(valid)
            .expect("valid definition registers");
        assert!(app
            .engine
            .try_definition_picture_image("PCTR")
            .expect("valid definition resolves")
            .is_some());
        let valid_entry = GoalRuleEntry {
            definition_id: "PCTR".to_string(),
            name: "Pictured rule".to_string(),
            description: None,
            fulfilled: false,
        };

        let malformed_dir = temp.path().join("Malformed.c4d");
        fs::create_dir(&malformed_dir).expect("malformed definition directory");
        fs::write(
            malformed_dir.join("DefCore.txt"),
            b"[DefCore]\nid=BADG\nWidth=1\nHeight=1\n",
        )
        .expect("malformed DefCore");
        fs::write(malformed_dir.join("Graphics.png"), b"not a png")
            .expect("malformed preferred graphics");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
            .save(malformed_dir.join("Graphics.bmp"))
            .expect("valid losing BMP");
        let malformed_group = Group::open(&malformed_dir).expect("open malformed definition");
        assert!(matches!(
            ResourceDefinitionData::load(&malformed_group),
            Err(ResourceDefinitionError::Graphics { path, reason })
                if path == Path::new("Graphics.png") && !reason.is_empty()
        ));

        let malformed_entry = GoalRuleEntry {
            definition_id: "BADG".to_string(),
            name: "Malformed rule".to_string(),
            description: None,
            fulfilled: false,
        };
        let error = app
            .cache_definition_icons(&[valid_entry, blank_entry, malformed_entry])
            .expect_err("a rejected graphics definition must not become a blank menu symbol");
        let EngineError::ClassicMenuParityBoundary { detail } = error else {
            panic!("unexpected malformed definition error: {error:?}");
        };
        assert_eq!(
            detail,
            "classic in-game goal/rule symbol definition `BADG` is unavailable: unknown definition `BADG`; refusing a blank symbol substitute"
        );
        assert!(
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
        let root = tempdir().expect("definition graphics fixture");
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
            fs::create_dir_all(graphics).expect("graphics group");
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
                fs::write(graphics.join(entry), graphics.to_string_lossy().as_bytes())
                    .expect("graphics entry");
            }
        }

        let scenario_group = Group::open(&scenario).expect("scenario group");
        let definition_roots = [
            Group::open(&first_pack).expect("first definition pack"),
            Group::open(&second_pack).expect("second definition pack"),
        ];
        let graphics = InstallDefinitionResolver::new(Some(Arc::new(paths)))
            .resolve_graphics_groups_with_definition_roots(&scenario_group, &definition_roots)
            .expect("graphics chain resolves");

        assert_eq!(
            graphics
                .iter()
                .map(|group| group.root().to_path_buf())
                .collect::<Vec<_>>(),
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
                .expect("winning graphics source")
        };
        assert_eq!(winner("ScenarioWins.png"), scenario_graphics);
        assert_eq!(winner("FolderWins.png"), folder_graphics);
        assert_eq!(winner("ExtraWins.png"), extra_graphics);
        assert_eq!(winner("PackWins.png"), first_graphics);
        assert_eq!(
            winner("PackTie.png"),
            first_pack.join("Graphics.c4g"),
            "RegisterMainGroups' second reversal makes the first selected definition pack win"
        );
    }
